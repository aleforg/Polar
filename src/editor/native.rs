use super::markdown::{DocumentLink, HIGHLIGHTS, LiveMarkdown};
use crate::{
    notes::tag_icons::{TagIcon, TagIcons},
    palette::{ACCENT, EDITOR, TEXT},
};

mod blocks;
mod completion;
mod cursors;
mod highlights;
mod layout;
mod math;
mod offsets;
#[cfg(debug_assertions)]
mod smoke;
mod viewport;
use gpui_kit::{Bounds, IntoElement, Pixels, Styled, Window, canvas};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject, Sel},
};
use objc2_app_kit::*;
use objc2_foundation::{
    NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString,
    NSUndoManager,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ops::Range,
    path::PathBuf,
    rc::Rc,
};

use cursors::CursorTextView;
use highlights::HighlightLayout;
use viewport::EditorViewport;

const HEADING_GUTTER: f64 = 32.;

fn utf16_boundary(source: &str, target: usize) -> Option<usize> {
    let mut offset = 0;
    for (byte, ch) in source.char_indices() {
        if offset == target {
            return Some(byte);
        }
        offset += ch.len_utf16();
        if offset > target {
            return None;
        }
    }
    (offset == target).then_some(source.len())
}

#[cfg(test)]
#[test]
fn revision_rejects_an_offset_inside_a_surrogate_pair() {
    assert_eq!(utf16_boundary("a🐻è", 2), None);
    assert_eq!(utf16_boundary("a🐻è", 3), Some("a🐻".len()));
    assert_eq!(utf16_boundary("a🐻è", 4), Some("a🐻è".len()));
}

pub(crate) enum EditorEvent {
    Changed,
    WikiCompletion,
    Follow(DocumentLink),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorMode {
    Editing,
    Reading,
    Protected,
}

pub(crate) struct RevisionSnapshot {
    pub(crate) source: String,
    pub(crate) selection: Range<usize>,
    revision: u64,
}

struct DelegateState {
    text: Retained<NSTextView>,
    highlight_layout: Retained<HighlightLayout>,
    undo: Retained<NSUndoManager>,
    busy: Cell<bool>,
    revision: Cell<u64>,
    markdown: RefCell<(String, LiveMarkdown)>,
    mode: Cell<EditorMode>,
    on_event: Box<dyn Fn(EditorEvent)>,
    checkboxes: RefCell<Vec<Retained<NSButton>>>,
    heading_labels: RefCell<Vec<Retained<NSTextField>>>,
    list_markers: RefCell<Vec<Retained<NSView>>>,
    block_views: RefCell<Vec<blocks::CachedBlock>>,
    attachments: PathBuf,
    images: RefCell<HashMap<String, Retained<NSImage>>>,
    decoration_state: RefCell<layout::DecorationState>,
    tag_buttons: RefCell<Vec<Retained<NSButton>>>,
    tag_icons: RefCell<TagIcons>,
    tag_images: RefCell<HashMap<TagIcon, Retained<NSImage>>>,
    wiki_completion: RefCell<completion::Completion>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateState]
    struct MarkdownDelegate;

    impl MarkdownDelegate {
        #[unsafe(method(chooseWikiSuggestion:))]
        fn choose_wiki_suggestion(&self, button: &NSButton) {
            if let Ok(index) = usize::try_from(button.tag()) {
                self.accept_wiki_suggestion(index);
            }
        }

        #[unsafe(method(openTableMenu:))]
        fn open_table_menu(&self, button: &NSButton) {
            self.table_menu(button);
        }

        #[unsafe(method(appendTableRow:))]
        fn append_table_row(&self, item: &NSMenuItem) {
            self.edit_table(item.tag(), crate::blocks::TableEdit::AddRow);
        }

        #[unsafe(method(appendTableColumn:))]
        fn append_table_column(&self, item: &NSMenuItem) {
            self.edit_table(item.tag(), crate::blocks::TableEdit::AddColumn);
        }

        #[unsafe(method(followTag:))]
        fn follow_tag(&self, button: &NSButton) {
            if let Ok(index) = usize::try_from(button.tag()) {
                self.follow_link(index);
            }
        }

        #[unsafe(method(toggleTask:))]
        fn toggle_task(&self, button: &NSButton) {
            let state = self.ivars();
            if state.mode.get() == EditorMode::Protected { return; }
            let text = &state.text;
            let selection = NSTextInputClient::selectedRange(&**text);
            let range = NSRange::new(button.tag() as usize, 1);
            let string = text.string();
            if range.location >= string.length() { return; }
            let value = string.substringWithRange(range).to_string();
            if matches!(value.as_str(), " " | "x" | "X") {
                let new_value = NSString::from_str(if value == " " { "x" } else { " " });
                let editable = text.isEditable();
                text.setEditable(true);
                // SAFETY: The button offset is within the current string and denotes an ASCII task marker.
                unsafe { text.insertText_replacementRange(&new_value, range); }
                text.setEditable(editable);
                text.setSelectedRange(selection);
                self.restyle(false);
            }
        }
    }

    // SAFETY: The delegate is an NSObject subclass confined to the AppKit main thread.
    unsafe impl NSObjectProtocol for MarkdownDelegate {}
    // SAFETY: The notification selector matches NSTextDelegate and is delivered on the main thread.
    unsafe impl NSTextDelegate for MarkdownDelegate {
        #[unsafe(method(textDidChange:))]
        fn changed(&self, _: &NSNotification) {
            if self.ivars().busy.get() {
                return;
            }
            self.bump_revision();
            self.restyle(false);
            self.refresh_wiki_query();
            (self.ivars().on_event)(EditorEvent::Changed);
        }

        #[unsafe(method(textDidEndEditing:))]
        fn ended_editing(&self, _: &NSNotification) {
            self.clear_wiki_completion();
        }
    }
    // SAFETY: The selection callback matches NSTextViewDelegate and reads the retained text view.
    unsafe impl NSTextViewDelegate for MarkdownDelegate {
        #[unsafe(method_id(undoManagerForTextView:))]
        fn undo_manager(&self, _: &NSTextView) -> Option<Retained<NSUndoManager>> {
            Some(self.ivars().undo.clone())
        }

        #[unsafe(method(textView:clickedOnLink:atIndex:))]
        fn clicked_link(&self, _: &NSTextView, link: &AnyObject, _: usize) -> bool {
            if let Some(index) = link.downcast_ref::<NSString>()
                .and_then(|link| link.to_string().parse::<usize>().ok())
            {
                self.follow_link(index);
            }
            true
        }

        #[unsafe(method(textView:doCommandBySelector:))]
        fn do_command(&self, _: &NSTextView, command: Sel) -> bool {
            self.wiki_command(command)
        }

        #[unsafe(method(textViewDidChangeSelection:))]
        fn selection_changed(&self, _: &NSNotification) {
            self.restyle_selection();
        }
    }
);

impl MarkdownDelegate {
    fn bump_revision(&self) {
        let revision = &self.ivars().revision;
        revision.set(revision.get().wrapping_add(1));
    }

    fn restyle_selection(&self) {
        let state = self.ivars();
        if state.busy.get() {
            return;
        }
        // Selection notifications can arrive halfway through a text edit. Its
        // textDidChange notification will style the final text synchronously,
        // before AppKit can draw it with stale fonts or formula glyph ranges.
        let unchanged = state.markdown.borrow().0 == state.text.string().to_string();
        if unchanged {
            self.restyle(false);
            self.refresh_wiki_query();
        }
    }

    fn follow_link(&self, index: usize) {
        let state = self.ivars();
        let action = state
            .markdown
            .borrow()
            .1
            .links()
            .get(index)
            .map(|(_, link)| link.clone());
        if let Some(action) = action {
            (state.on_event)(EditorEvent::Follow(action));
        }
    }
}

pub(crate) struct RichEditor {
    viewport: Retained<EditorViewport>,
    scroll: Retained<NSScrollView>,
    delegate: Retained<MarkdownDelegate>,
    wants_focus: Cell<bool>,
}

impl RichEditor {
    pub(crate) fn new(attachments: PathBuf, on_event: impl Fn(EditorEvent) + 'static) -> Rc<Self> {
        let mtm = MainThreadMarker::new().expect("GPUI runs on the main thread");
        let scroll = CursorTextView::scrollable();
        let text = scroll
            .documentView()
            .unwrap()
            .downcast::<NSTextView>()
            .unwrap();
        // SAFETY: DarkAqua is the exported AppKit appearance name.
        scroll.setAppearance(
            NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua }).as_deref(),
        );
        scroll.setBackgroundColor(&color(EDITOR));
        scroll.setDrawsBackground(true);
        scroll.setAutohidesScrollers(true);
        text.setRichText(false);
        text.setEditable(false);
        // SAFETY: The empty dictionary leaves link appearance to the Markdown decorations.
        unsafe {
            text.setLinkTextAttributes(Some(&objc2_foundation::NSDictionary::new()));
        }
        text.setAllowsUndo(true);
        text.setAutomaticQuoteSubstitutionEnabled(false);
        text.setAutomaticDashSubstitutionEnabled(false);
        text.setInsertionPointColor(Some(&color(ACCENT)));
        text.setTextColor(Some(&color(TEXT)));
        text.setBackgroundColor(&color(EDITOR));
        text.setDrawsBackground(true);
        text.setTextContainerInset(NSSize::new(HEADING_GUTTER, 0.));
        let highlight_layout = HighlightLayout::new(mtm);
        // SAFETY: Accessing the layout manager switches NSTextView to TextKit 1 before the swap.
        unsafe { text.layoutManager() };
        // SAFETY: The retained text view and its text container are accessed on the main thread.
        if let Some(container) = unsafe { text.textContainer() } {
            container.replaceLayoutManager(&highlight_layout);
            container.setLineFragmentPadding(0.);
        }
        // SAFETY: The newly allocated NSObject subclass receives its Rust ivars before super init.
        let delegate: Retained<MarkdownDelegate> = unsafe {
            let allocated = MarkdownDelegate::alloc(mtm).set_ivars(DelegateState {
                text,
                highlight_layout,
                undo: NSUndoManager::new(mtm),
                busy: Cell::new(false),
                revision: Cell::new(0),
                markdown: RefCell::default(),
                mode: Cell::new(EditorMode::Protected),
                on_event: Box::new(on_event),
                checkboxes: RefCell::default(),
                heading_labels: RefCell::default(),
                list_markers: RefCell::default(),
                block_views: RefCell::default(),
                attachments,
                images: RefCell::default(),
                decoration_state: RefCell::default(),
                tag_buttons: RefCell::default(),
                tag_icons: RefCell::default(),
                tag_images: RefCell::default(),
                wiki_completion: RefCell::default(),
            });
            msg_send![super(allocated), init]
        };
        delegate
            .ivars()
            .text
            .setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        let viewport = EditorViewport::new(mtm);
        viewport.addSubview(&scroll);
        Rc::new(Self {
            viewport,
            scroll,
            delegate,
            wants_focus: Cell::new(false),
        })
    }

    pub(crate) fn value(&self) -> String {
        self.delegate.ivars().text.string().to_string()
    }

    pub(crate) fn revision_snapshot(&self) -> Option<RevisionSnapshot> {
        let state = self.delegate.ivars();
        let text = &*state.text;
        if state.mode.get() == EditorMode::Protected
            || NSTextInputClient::hasMarkedText(text)
            || text.selectedRanges().count() != 1
        {
            return None;
        }
        let selection = NSTextInputClient::selectedRange(text);
        let end = selection.location.checked_add(selection.length)?;
        let source = self.value();
        let start = utf16_boundary(&source, selection.location)?;
        let end = utf16_boundary(&source, end)?;
        Some(RevisionSnapshot {
            source,
            selection: start..end,
            revision: state.revision.get(),
        })
    }

    pub(crate) fn revision_is_current(&self, snapshot: &RevisionSnapshot) -> bool {
        let state = self.delegate.ivars();
        state.revision.get() == snapshot.revision && state.mode.get() != EditorMode::Protected
    }

    pub(crate) fn apply_revision(
        &self,
        snapshot: &RevisionSnapshot,
        range: Range<usize>,
        replacement: &str,
    ) -> bool {
        let state = self.delegate.ivars();
        let text = &*state.text;
        if !self.revision_is_current(snapshot)
            || !text.isEditable()
            || NSTextInputClient::hasMarkedText(text)
            || self.value() != snapshot.source
        {
            return false;
        }
        let (Some(prefix), Some(original)) = (
            snapshot.source.get(..range.start),
            snapshot.source.get(range),
        ) else {
            return false;
        };
        let range = NSRange::new(
            prefix.encode_utf16().count(),
            original.encode_utf16().count(),
        );
        text.breakUndoCoalescing();
        let level = state.undo.groupingLevel();
        if level > 1 || (level == 1 && !state.undo.groupsByEvent()) {
            return false;
        }
        // The current event group can still contain a manual edit when the proposal is applied.
        if level == 1 {
            state.undo.endUndoGrouping();
        }
        state.undo.beginUndoGrouping();
        // SAFETY: The range is computed from exact UTF-8 boundaries in the current native buffer.
        unsafe {
            text.insertText_replacementRange(&NSString::from_str(replacement), range);
        }
        text.breakUndoCoalescing();
        state.undo.endUndoGrouping();
        self.focus();
        true
    }

    #[cfg(debug_assertions)]
    pub(crate) fn select_revision_fixture(&self, start: usize, length: usize) {
        self.delegate
            .ivars()
            .text
            .setSelectedRange(NSRange::new(start, length));
    }

    pub(crate) fn cached_tags(&self, value: &str) -> Option<std::collections::BTreeSet<String>> {
        let cached = self.delegate.ivars().markdown.borrow();
        // The sidebar uses GFM without math extensions. Reuse the editor's parse
        // only where those extensions cannot change tag recognition.
        if cached.0 != value || value.contains('$') {
            return None;
        }
        Some(
            cached
                .1
                .links()
                .iter()
                .filter_map(|(_, link)| {
                    if let DocumentLink::Tag(tag) = link {
                        Some(tag.clone())
                    } else {
                        None
                    }
                })
                .collect(),
        )
    }

    pub(crate) fn set_tag_icons(&self, icons: &TagIcons) {
        let mut current = self.delegate.ivars().tag_icons.borrow_mut();
        if *current != *icons {
            current.clone_from(icons);
            drop(current);
            self.delegate.restyle(true);
        }
    }

    pub(crate) fn load(&self, value: &str) {
        let _timing = crate::performance::Sample::start("load");
        self.delegate.clear_wiki_completion();
        let state = self.delegate.ivars();
        self.delegate.bump_revision();
        state.busy.set(true);
        state.text.setAllowsUndo(false);
        state.block_views.borrow_mut().clear();
        state.images.borrow_mut().clear();
        state.text.setString(&NSString::from_str(value));
        state.undo.removeAllActions();
        state.text.setAllowsUndo(true);
        state.text.setSelectedRange(NSRange::new(0, 0));
        if value.is_empty() {
            state.text.setFont(Some(&NSFont::systemFontOfSize(30.)));
        }
        state.busy.set(false);
        self.delegate.restyle(true);
        let clip = self.scroll.contentView();
        clip.scrollToPoint(NSPoint::new(0., 0.));
        self.scroll.reflectScrolledClipView(&clip);
        // A shorter note can leave pixels beyond the new glyphs in the reused AppKit layer.
        state.text.setNeedsDisplay(true);
        self.scroll.contentView().setNeedsDisplay(true);
    }

    pub(crate) fn set_mode(&self, mode: EditorMode) {
        let state = self.delegate.ivars();
        if state.mode.replace(mode) == mode {
            return;
        }
        if mode != EditorMode::Editing {
            self.delegate.clear_wiki_completion();
            self.wants_focus.set(false);
            self.blur();
        }
        state.text.setEditable(mode == EditorMode::Editing);
        self.delegate.restyle(true);
    }

    pub(crate) fn focus(&self) {
        self.wants_focus.set(true);
    }

    pub(crate) fn format(&self, prefix: &str, suffix: &str) {
        let text = &self.delegate.ivars().text;
        let selection = NSTextInputClient::selectedRange(&**text);
        let string = text.string();
        let content = string.substringWithRange(selection).to_string();
        let prefix_len = prefix.encode_utf16().count();
        let suffix_len = suffix.encode_utf16().count();
        let end = selection.location + selection.length;
        let bold_boundary = prefix == "*" && {
            let before = string
                .substringWithRange(NSRange::new(0, selection.location))
                .to_string();
            let after = string
                .substringWithRange(NSRange::new(end, string.length() - end))
                .to_string();
            before
                .bytes()
                .rev()
                .take_while(|byte| *byte == b'*')
                .count()
                .is_multiple_of(2)
                || after
                    .bytes()
                    .take_while(|byte| *byte == b'*')
                    .count()
                    .is_multiple_of(2)
        };
        let mut candidates = HIGHLIGHTS
            .iter()
            .rev()
            .filter_map(|(_, candidate, _)| (suffix == "==").then_some(*candidate))
            .chain(std::iter::once(prefix));
        let enclosing_prefix = (!bold_boundary
            && end + suffix_len <= string.length()
            && string
                .substringWithRange(NSRange::new(end, suffix_len))
                .to_string()
                == suffix)
            .then(|| {
                candidates.find(|candidate| {
                    let length = candidate.encode_utf16().count();
                    selection.location >= length
                        && string
                            .substringWithRange(NSRange::new(selection.location - length, length))
                            .to_string()
                            == *candidate
                })
            })
            .flatten();
        let (range, replacement, caret) = if let Some(enclosing_prefix) = enclosing_prefix {
            let old_len = enclosing_prefix.encode_utf16().count();
            let start = selection.location - old_len;
            let range = NSRange::new(start, old_len + selection.length + suffix_len);
            if enclosing_prefix == prefix {
                (range, content, start)
            } else {
                (
                    range,
                    format!("{prefix}{content}{suffix}"),
                    start + prefix_len,
                )
            }
        } else {
            (
                selection,
                format!("{prefix}{content}{suffix}"),
                selection.location + prefix_len,
            )
        };
        // SAFETY: The ranges come from the current AppKit string and its selection.
        unsafe {
            text.insertText_replacementRange(&NSString::from_str(&replacement), range);
        }
        text.setSelectedRange(NSRange::new(caret, selection.length));
        self.wants_focus.set(true);
    }

    pub(crate) fn insert_link(&self) {
        let text = &self.delegate.ivars().text;
        let selection = NSTextInputClient::selectedRange(&**text);
        let label = text.string().substringWithRange(selection).to_string();
        let label = if label.is_empty() {
            "Testo".to_owned()
        } else {
            label
                .replace('\\', "\\\\")
                .replace('[', "\\[")
                .replace(']', "\\]")
        };
        let url = "https://";
        let replacement = format!("[{label}]({url})");
        // SAFETY: The replacement range is the current selection in the AppKit text view.
        unsafe {
            text.insertText_replacementRange(&NSString::from_str(&replacement), selection);
        }
        let url_start = selection.location + label.encode_utf16().count() + 3;
        text.setSelectedRange(NSRange::new(url_start, url.len()));
        self.wants_focus.set(true);
    }

    pub(crate) fn prefix_line(&self, prefix: &str) {
        let text = &self.delegate.ivars().text;
        let selection = NSTextInputClient::selectedRange(&**text);
        let string = text.string();
        let line = string.lineRangeForRange(NSRange::new(selection.location, 0));
        let original = string.substringWithRange(line).to_string();
        let marker = original.split_once(' ').map_or("", |(marker, _)| marker);
        let existing = if prefix.starts_with('#')
            && (1..=6).contains(&marker.len())
            && marker.bytes().all(|byte| byte == b'#')
        {
            marker.len() + 1
        } else if original.starts_with(prefix) {
            prefix.len()
        } else {
            0
        };
        let replacement = if original.starts_with(prefix) {
            ""
        } else {
            prefix
        };
        // SAFETY: AppKit provides the current line range and both prefixes are ASCII.
        unsafe {
            text.insertText_replacementRange(
                &NSString::from_str(replacement),
                NSRange::new(line.location, existing),
            );
        }
        let start = (selection.location - line.location).saturating_sub(existing);
        let end = (selection.location - line.location + selection.length).saturating_sub(existing);
        text.setSelectedRange(NSRange::new(
            line.location + replacement.len() + start,
            end - start,
        ));
        self.wants_focus.set(true);
    }

    pub(crate) fn insert_block(&self, markdown: &str) {
        let text = &self.delegate.ivars().text;
        let selection = NSTextInputClient::selectedRange(&**text);
        let source = text.string().to_string();
        let title_end = source
            .split('\n')
            .next()
            .unwrap_or("")
            .encode_utf16()
            .count();
        let position = (selection.location + selection.length).max(title_end);
        let replacement = NSString::from_str(&format!("\n\n{markdown}\n\n"));
        // SAFETY: The insertion offset comes from the current AppKit selection and the value is an NSString.
        unsafe {
            text.insertText_replacementRange(&replacement, NSRange::new(position, 0));
        }
        self.wants_focus.set(true);
    }

    pub(crate) fn reveal(&self, offset: usize) {
        let text = &self.delegate.ivars().text;
        let source = text.string().to_string();
        if let Some(prefix) = source.get(..offset) {
            let range = NSRange::new(prefix.encode_utf16().count(), 0);
            if text.isEditable() {
                text.setSelectedRange(range);
                self.focus();
            }
            text.scrollRangeToVisible(range);
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn scroll_origin(&self) -> NSPoint {
        self.scroll.contentView().bounds().origin
    }

    #[cfg(debug_assertions)]
    pub(crate) fn is_editable(&self) -> bool {
        self.delegate.ivars().text.isEditable()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn is_visible(&self) -> bool {
        self.viewport.window().is_some() && !self.viewport.isHidden()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn viewport_frame(&self) -> NSRect {
        self.viewport.frame()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn assert_format_overlay(&self, count: usize) {
        self.viewport.assert_cutout_routes_clicks(count);
    }

    #[cfg(debug_assertions)]
    pub(crate) fn has_format_overlay(&self) -> bool {
        self.viewport
            .layer()
            .is_some_and(|layer| layer.mask().is_some())
    }

    pub(crate) fn is_focused(&self) -> bool {
        self.scroll.window().is_some_and(|window| {
            window.firstResponder().as_deref() == Some(self.delegate.ivars().text.as_ref())
        })
    }

    pub(crate) fn blur(&self) {
        self.delegate.clear_wiki_completion();
        if self.is_focused()
            && let Some(window) = self.scroll.window()
        {
            // SAFETY: The retained viewport and its parent are accessed on the main thread.
            let parent = unsafe { self.viewport.superview() };
            window.makeFirstResponder(parent.as_deref().map(AsRef::as_ref));
            self.delegate.restyle(false);
        }
    }

    pub(crate) fn undo_or_redo(&self, redo: bool) {
        if self.is_focused()
            && self.delegate.ivars().text.isEditable()
            && let Some(undo) = self.delegate.ivars().text.undoManager()
        {
            if redo {
                undo.redo();
            } else {
                undo.undo();
            }
        }
    }

    pub(crate) fn hide(&self) {
        self.blur();
        self.wants_focus.set(false);
        self.viewport.setHidden(true);
    }

    pub(crate) fn element(
        self: &Rc<Self>,
        overlays: Rc<Cell<Vec<Bounds<Pixels>>>>,
    ) -> impl IntoElement {
        let editor = self.clone();
        canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                let overlays = overlays.take();
                window.defer(cx, move |window, _| {
                    editor.place(bounds, window);
                    // SAFETY: The viewport and its parent are accessed on the AppKit main thread.
                    if let Some(parent) = unsafe { editor.viewport.superview() } {
                        let panels: Vec<_> = overlays
                            .into_iter()
                            .map(|bounds| native_rect(bounds, &parent))
                            .collect();
                        editor.viewport.set_cutouts(panels.iter().map(|&panel| {
                            editor.viewport.convertRect_fromView(panel, Some(&parent))
                        }));
                        CursorTextView::from_text(&editor.delegate.ivars().text).set_overlays(
                            panels
                                .into_iter()
                                .map(|panel| parent.convertRect_toView(panel, None))
                                .collect(),
                        );
                    }
                });
            },
        )
        .size_full()
    }

    fn place(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return;
        };
        // SAFETY: The AppKit handle points to the live GPUI NSView for this UI-thread callback.
        let parent = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
        if self.viewport.window().is_none() {
            parent.addSubview(&self.viewport);
        }
        let mut frame = native_rect(bounds, parent);
        frame.origin.x -= HEADING_GUTTER;
        frame.size.width += 2. * HEADING_GUTTER;
        if self.viewport.frame() != frame {
            self.viewport.setFrame(frame);
            self.scroll.setFrame(self.viewport.bounds());
            self.delegate.restyle(true);
        }
        self.viewport.setHidden(false);
        if self.wants_focus.replace(false)
            && let Some(window) = self.scroll.window()
        {
            let text = &self.delegate.ivars().text;
            window.makeFirstResponder(Some(text));
            self.delegate.restyle(false);
            self.delegate.refresh_wiki_query();
        }
    }
}

impl Drop for RichEditor {
    fn drop(&mut self) {
        self.viewport.removeFromSuperview();
    }
}

fn native_rect(bounds: Bounds<Pixels>, parent: &NSView) -> NSRect {
    let top = f64::from(bounds.origin.y);
    let height = f64::from(bounds.size.height);
    let y = if parent.isFlipped() {
        top
    } else {
        parent.bounds().size.height - top - height
    };
    NSRect::new(
        NSPoint::new(f64::from(bounds.origin.x), y),
        NSSize::new(f64::from(bounds.size.width), height),
    )
}

fn color(rgb: u32) -> Retained<NSColor> {
    let component = |shift| f64::from((rgb >> shift) & 255u32) / 255.;
    NSColor::colorWithSRGBRed_green_blue_alpha(component(16), component(8), component(0), 1.)
}

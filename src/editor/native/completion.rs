use objc2::{DefinedClass, MainThreadOnly, rc::Retained, runtime::Sel, sel};
use objc2_app_kit::{
    NSButton, NSEvent, NSEventModifierFlags, NSFont, NSLineBreakMode, NSTextAlignment,
    NSTextInputClient, NSView,
};
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString};

use super::{EditorEvent, EditorMode, MarkdownDelegate, RichEditor, color};
use crate::{
    notes::{Note, links},
    palette::{SIDEBAR, TEXT},
};

const ROW_HEIGHT: f64 = 26.;
const PADDING: f64 = 4.;

#[derive(PartialEq, Eq)]
struct Query {
    replacement: NSRange,
    title: String,
}

#[derive(Default)]
pub(super) struct Completion {
    query: Option<Query>,
    offered: bool,
    popup: Option<Popup>,
}

struct Popup {
    view: Retained<NSView>,
    buttons: Vec<Retained<NSButton>>,
    titles: Vec<String>,
    selected: usize,
}

impl Popup {
    fn select(&mut self, index: usize) {
        self.selected = index;
        for (index, button) in self.buttons.iter().enumerate() {
            if let Some(layer) = button.layer() {
                layer.setBackgroundColor(
                    (index == self.selected)
                        .then(|| color(0x31506a).CGColor())
                        .as_deref(),
                );
            }
        }
    }
}

impl Drop for Popup {
    fn drop(&mut self) {
        self.view.removeFromSuperview();
    }
}

impl MarkdownDelegate {
    fn wiki_query(&self) -> Option<Query> {
        let state = self.ivars();
        let text = &state.text;
        let window = text.window()?;
        if state.mode.get() != EditorMode::Editing
            || window.firstResponder().as_deref() != Some(text.as_ref())
            || NSTextInputClient::hasMarkedText(&**text)
        {
            return None;
        }
        let caret = NSTextInputClient::selectedRange(&**text);
        if caret.length != 0 || caret.location < 2 {
            return None;
        }
        let source = text.string();
        let line = source.lineRangeForRange(caret);
        let before =
            source.substringWithRange(NSRange::new(line.location, caret.location - line.location));
        let before = before.to_string();
        let (opener, title) = links::completion_prefix(&before)?;
        let start = line.location + before[..opener].encode_utf16().count();
        let markdown = state.markdown.borrow();
        let offset = markdown
            .0
            .char_indices()
            .scan(0, |utf16, (offset, ch)| {
                let pair = (*utf16, offset);
                *utf16 += ch.len_utf16();
                Some(pair)
            })
            .find_map(|(utf16, offset)| (utf16 == start).then_some(offset))?;
        if !markdown.1.allows_wiki_completion(offset) {
            return None;
        }
        let after = source.substringWithRange(NSRange::new(
            caret.location,
            line.location + line.length - caret.location,
        ));
        let after = after.to_string();
        let after = after.trim_end_matches(['\r', '\n']);
        let suffix = if let Some(end) = after.find("]]") {
            if after[..end].contains(['[', ']', '|']) {
                return None;
            }
            after[..end + 2].encode_utf16().count()
        } else {
            usize::from(after.starts_with(']'))
        };
        Some(Query {
            replacement: NSRange::new(start, caret.location - start + suffix),
            title: title.to_owned(),
        })
    }

    pub(super) fn refresh_wiki_query(&self) {
        let query = self.wiki_query();
        let request = {
            let mut completion = self.ivars().wiki_completion.borrow_mut();
            if completion.query == query {
                return;
            }
            *completion = Completion {
                query,
                ..Completion::default()
            };
            completion.query.is_some()
        };
        if request {
            (self.ivars().on_event)(EditorEvent::WikiCompletion);
        }
    }

    pub(super) fn clear_wiki_completion(&self) {
        *self.ivars().wiki_completion.borrow_mut() = Completion::default();
    }

    fn dismiss_wiki_completion(&self) -> bool {
        let mut completion = self.ivars().wiki_completion.borrow_mut();
        let pending = completion.query.is_some() && !completion.offered;
        completion.offered = true;
        completion.popup.take().is_some() || pending
    }

    pub(super) fn wiki_key_down(&self, event: &NSEvent) -> bool {
        if event.modifierFlags().intersects(
            NSEventModifierFlags::Command
                | NSEventModifierFlags::Control
                | NSEventModifierFlags::Option
                | NSEventModifierFlags::Shift,
        ) {
            return false;
        }
        // AppKit may deliver raw key events without dispatching text commands.
        let command = match event.keyCode() {
            126 => sel!(moveUp:),
            125 => sel!(moveDown:),
            36 | 76 => sel!(insertNewline:),
            48 => sel!(insertTab:),
            53 => sel!(cancelOperation:),
            _ => return false,
        };
        self.wiki_command(command)
    }

    pub(super) fn wiki_command(&self, command: Sel) -> bool {
        if command == sel!(cancelOperation:) {
            return self.dismiss_wiki_completion();
        }
        let selected = {
            let mut completion = self.ivars().wiki_completion.borrow_mut();
            let Some(popup) = completion.popup.as_mut() else {
                return false;
            };
            if command == sel!(moveUp:) {
                popup.select(popup.selected.saturating_sub(1));
                return true;
            }
            if command == sel!(moveDown:) {
                popup.select((popup.selected + 1).min(popup.titles.len() - 1));
                return true;
            }
            popup.selected
        };
        (command == sel!(insertNewline:) || command == sel!(insertTab:))
            && self.accept_wiki_suggestion(selected)
    }

    pub(super) fn accept_wiki_suggestion(&self, index: usize) -> bool {
        let state = self.ivars();
        let Some(query) = self.wiki_query() else {
            return false;
        };
        let title = {
            let completion = state.wiki_completion.borrow();
            if completion.query.as_ref() != Some(&query) {
                return false;
            }
            completion
                .popup
                .as_ref()
                .and_then(|popup| popup.titles.get(index))
                .cloned()
        };
        let Some(title) = title else {
            return false;
        };
        self.dismiss_wiki_completion();
        let link = format!("[[{title}]]");
        // SAFETY: The replacement range comes from the current text and selection in UTF-16.
        unsafe {
            state
                .text
                .insertText_replacementRange(&NSString::from_str(&link), query.replacement);
        }
        true
    }

    fn show_wiki_suggestions(&self, notes: &[Note]) {
        let state = self.ivars();
        let Some(query) = self.wiki_query() else {
            return;
        };
        let mut completion = state.wiki_completion.borrow_mut();
        if completion.query.as_ref() != Some(&query) || completion.offered {
            return;
        }
        completion.offered = true;
        let titles = links::suggested_titles(notes, &query.title);
        if titles.is_empty() {
            return;
        }
        let text = &state.text;
        let Some(window) = text.window() else {
            return;
        };
        let caret = NSTextInputClient::selectedRange(&**text);
        // SAFETY: The caret range comes from the live text view. The optional actual range is unused.
        let screen =
            unsafe { text.firstRectForCharacterRange_actualRange(caret, std::ptr::null_mut()) };
        let caret = text.convertRect_fromView(window.convertRectFromScreen(screen), None);
        let visible = text.visibleRect();
        let width = 300_f64.min(visible.size.width - PADDING * 2.);
        let below =
            visible.origin.y + visible.size.height - caret.origin.y - caret.size.height - PADDING;
        let above = caret.origin.y - visible.origin.y - PADDING;
        let max_rows = ((above.max(below) - 2. * PADDING) / ROW_HEIGHT).max(0.) as usize;
        let titles: Vec<_> = titles.into_iter().take(max_rows).collect();
        if width < 80. || titles.is_empty() {
            return;
        }
        let height = titles.len() as f64 * ROW_HEIGHT + 2. * PADDING;
        let x = caret.origin.x.clamp(
            visible.origin.x + PADDING,
            visible.origin.x + visible.size.width - width - PADDING,
        );
        let y = if height <= below {
            caret.origin.y + caret.size.height + PADDING
        } else {
            caret.origin.y - height - PADDING
        };
        let view = NSView::new(self.mtm());
        view.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)));
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setBackgroundColor(Some(&color(SIDEBAR).CGColor()));
            layer.setBorderColor(Some(&color(0x515356).CGColor()));
            layer.setBorderWidth(1.);
            layer.setCornerRadius(8.);
        }
        let mut buttons = Vec::with_capacity(titles.len());
        for (index, title) in titles.iter().enumerate() {
            // SAFETY: The delegate implements the action for NSButton on the main thread.
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(title),
                    Some(self),
                    Some(sel!(chooseWikiSuggestion:)),
                    self.mtm(),
                )
            };
            button.setBordered(false);
            button.setRefusesFirstResponder(true);
            button.setAlignment(NSTextAlignment::Left);
            button.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            button.setFont(Some(&NSFont::systemFontOfSize(13.)));
            button.setContentTintColor(Some(&color(TEXT)));
            button.setWantsLayer(true);
            if let Some(layer) = button.layer() {
                layer.setCornerRadius(5.);
            }
            button.setTag(index as isize);
            let y = height - PADDING - (index + 1) as f64 * ROW_HEIGHT;
            button.setFrame(NSRect::new(
                NSPoint::new(PADDING, y),
                NSSize::new(width - 2. * PADDING, ROW_HEIGHT),
            ));
            view.addSubview(&button);
            buttons.push(button);
        }
        let mut popup = Popup {
            view,
            buttons,
            titles,
            selected: 0,
        };
        popup.select(0);
        text.addSubview(&popup.view);
        completion.popup = Some(popup);
    }
}

impl RichEditor {
    #[cfg(debug_assertions)]
    pub(super) fn smoke_wiki_completion(&self) {
        let mut notes = crate::notes::initial_notes();
        notes[0].set_text("# Roma".into());
        notes[1].set_text("# Romanzo".into());
        notes.truncate(2);
        let text = &self.delegate.ivars().text;
        self.load("Titolo\n\n🐻 [[ro]] dopo");
        text.setSelectedRange(NSRange::new("Titolo\n\n🐻 [[ro".encode_utf16().count(), 0));
        self.show_wiki_suggestions(&notes);
        {
            let completion = self.delegate.ivars().wiki_completion.borrow();
            let popup = completion
                .popup
                .as_ref()
                .expect("show suggestions below the caret");
            assert_eq!(popup.titles, ["Roma", "Romanzo"]);
            assert_eq!(popup.buttons.len(), 2);
        }
        // SAFETY: These are standard NSTextView command selectors.
        unsafe {
            text.doCommandBySelector(sel!(moveDown:));
            text.doCommandBySelector(sel!(insertNewline:));
        }
        assert_eq!(self.value(), "Titolo\n\n🐻 [[Romanzo]] dopo");
        assert!(
            self.delegate
                .ivars()
                .wiki_completion
                .borrow()
                .popup
                .is_none()
        );

        self.load("Titolo\n\n[[ro");
        text.setSelectedRange(NSRange::new(self.value().encode_utf16().count(), 0));
        self.show_wiki_suggestions(&notes);
        assert!(self.dismiss_wiki_completion());
        self.show_wiki_suggestions(&notes);
        assert!(
            self.delegate
                .ivars()
                .wiki_completion
                .borrow()
                .popup
                .is_none()
        );
        assert_eq!(self.value(), "Titolo\n\n[[ro");

        self.load("Titolo\n\n[[ro");
        text.setSelectedRange(NSRange::new(self.value().encode_utf16().count(), 0));
        self.show_wiki_suggestions(&notes);
        let button = self
            .delegate
            .ivars()
            .wiki_completion
            .borrow()
            .popup
            .as_ref()
            .expect("show clickable suggestions")
            .buttons[0]
            .clone();
        // SAFETY: The button is retained and its action accepts an NSButton sender.
        unsafe {
            button.performClick(None);
        }
        assert_eq!(self.value(), "Titolo\n\n[[Roma]]");
        self.load("Titolo\n\n[[");
        text.setSelectedRange(NSRange::new(self.value().encode_utf16().count(), 0));
        self.show_wiki_suggestions(&notes);
        // SAFETY: This is the standard NSTextView Tab command.
        unsafe {
            text.doCommandBySelector(sel!(insertTab:));
        }
        assert_eq!(self.value(), "Titolo\n\n[[Roma]]");

        self.load("Titolo\n\n`[[ro`");
        text.setSelectedRange(NSRange::new("Titolo\n\n`[[ro".encode_utf16().count(), 0));
        self.show_wiki_suggestions(&notes);
        assert!(
            self.delegate
                .ivars()
                .wiki_completion
                .borrow()
                .popup
                .is_none()
        );
    }

    pub(crate) fn show_wiki_suggestions(&self, notes: &[Note]) {
        self.delegate.show_wiki_suggestions(notes);
    }

    pub(crate) fn dismiss_wiki_completion(&self) -> bool {
        self.delegate.dismiss_wiki_completion()
    }
}

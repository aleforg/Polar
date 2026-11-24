use std::{cell::OnceCell, ops::Range};

use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained, sel,
};
use objc2_app_kit::{
    NSAccessibility, NSBox, NSBoxType, NSButton, NSFont, NSImage, NSImageSymbolConfiguration,
    NSImageView, NSLayoutManager, NSLineBreakMode, NSMenu, NSMenuItem, NSTextField,
    NSTextInputClient, NSTitlePosition, NSView,
};
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString};

use super::{HEADING_GUTTER, MarkdownDelegate, color, cursors::Pointer, math::Graphic};
use crate::{
    attachments,
    blocks::{Block, BlockContent, Callout, CellPart, TableEdit, extend_table},
    editor::markdown::Formula,
    palette::TEXT,
};

const ROW_HEIGHT: f64 = 27.;

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = OnceCell<Retained<NSButton>>]
    struct BlockPreview;

    impl BlockPreview {
        // All other clicks go to the editor so a preview can still reveal its source.
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
            self.ivars().get().and_then(|button| {
                // SAFETY: AppKit invokes the selector on the main thread and supplies a parent-space point.
                let parent = unsafe { self.superview() };
                button.hitTest(self.convertPoint_fromView(point, parent.as_deref()))
            })
        }
    }
);

pub(super) struct CachedBlock {
    view: Retained<BlockPreview>,
    source: String,
    width: f64,
    row_height: f64,
}

impl Drop for CachedBlock {
    fn drop(&mut self) {
        self.view.removeFromSuperview();
    }
}

pub(super) struct Preview<'a> {
    pub(super) source: Range<usize>,
    pub(super) line_height: f64,
    content: Content<'a>,
}

enum Content<'a> {
    Separator,
    Table(&'a [Vec<Vec<CellPart>>]),
    Image {
        image: Retained<NSImage>,
        alt: &'a str,
        size: NSSize,
    },
    Callout(&'a Callout),
}

impl MarkdownDelegate {
    #[cfg(debug_assertions)]
    pub(super) fn assert_table_formulas(&self) {
        let views = self.ivars().block_views.borrow();
        let mut pending = vec![views[0].view.clone().into_super()];
        let mut images = 0;
        let mut invalid = false;
        let mut literal = false;
        while let Some(view) = pending.pop() {
            pending.extend(view.subviews().iter());
            if view.downcast_ref::<NSImageView>().is_some() {
                images += 1;
            }
            if let Some(label) = view.downcast_ref::<NSTextField>() {
                let text = label.stringValue().to_string();
                invalid |= text == r"$\openBearUnknown$";
                literal |= text == "$letterale$";
            }
        }
        assert_eq!(
            images, 4,
            "Render formulas in headers and mixed-content cells"
        );
        assert!(invalid, "Preserve the source of an invalid formula");
        assert!(literal, "Keep inline code literal");
    }

    pub(super) fn prepare_blocks<'a>(
        &self,
        blocks: &'a [Block],
        selection: Option<&Range<usize>>,
    ) -> Vec<Preview<'a>> {
        let state = self.ivars();
        let width = state.text.bounds().size.width - HEADING_GUTTER * 2.;
        if width < 40. {
            return Vec::new();
        }
        let image_urls: std::collections::HashSet<_> = blocks
            .iter()
            .filter_map(|block| {
                if let BlockContent::Image { url, .. } = &block.content {
                    Some(url.as_str())
                } else {
                    None
                }
            })
            .collect();
        let mut images = state.images.borrow_mut();
        images.retain(|url, _| image_urls.contains(url.as_str()));
        blocks
            .iter()
            .filter(|block| selection.is_none_or(|selection| block.is_inactive(selection)))
            .filter_map(|block| {
                let (content, line_height) = match &block.content {
                    BlockContent::Separator => (Content::Separator, ROW_HEIGHT),
                    BlockContent::Table(rows)
                        if rows
                            .first()
                            .is_some_and(|row| width >= row.len() as f64 * 28.) =>
                    {
                        (Content::Table(rows), ROW_HEIGHT)
                    }
                    BlockContent::Callout(callout) => (Content::Callout(callout), ROW_HEIGHT),
                    BlockContent::Image { url, alt } => {
                        let image = if let Some(image) = images.get(url) {
                            image.clone()
                        } else {
                            let path = attachments::resolve(&state.attachments, url)?;
                            let image = NSImage::initWithContentsOfFile(
                                NSImage::alloc(),
                                &NSString::from_str(path.to_str()?),
                            )?;
                            images.insert(url.clone(), image.clone());
                            image
                        };
                        let original = image.size();
                        if original.width <= 0. || original.height <= 0. {
                            return None;
                        }
                        let scale = (width / original.width).min(340. / original.height).min(1.);
                        let size = NSSize::new(original.width * scale, original.height * scale);
                        (Content::Image { image, alt, size }, size.height + 6.)
                    }
                    _ => return None,
                };
                Some(Preview {
                    source: block.range.clone(),
                    line_height,
                    content,
                })
            })
            .collect()
    }

    pub(super) fn place_blocks(
        &self,
        previews: &[Preview<'_>],
        offsets: &[usize],
        layout: &NSLayoutManager,
        cursors: &mut Vec<(NSRect, Pointer)>,
        value: &str,
        formulas: &[Formula],
    ) {
        let state = self.ivars();
        let mut views = state.block_views.borrow_mut();
        let mut previous = std::mem::take(&mut *views).into_iter();
        let origin = state.text.textContainerOrigin();
        let width = state.text.bounds().size.width - HEADING_GUTTER * 2.;
        let mtm = self.mtm();
        for preview in previews {
            let start = offsets[preview.source.start];
            // SAFETY: Each preview begins in the text used by this layout, and no output range is needed.
            let glyphs = unsafe {
                layout.glyphRangeForCharacterRange_actualCharacterRange(
                    NSRange::new(start, 1),
                    std::ptr::null_mut(),
                )
            };
            // SAFETY: The glyph index was obtained from this layout manager.
            let row = unsafe {
                layout.lineFragmentRectForGlyphAtIndex_effectiveRange(
                    glyphs.location,
                    std::ptr::null_mut(),
                )
            };
            let source = &value[preview.source.clone()];
            if let Some(cached) = previous.next().filter(|cached| {
                cached.source == source
                    && cached.width == width
                    && cached.row_height == row.size.height
            }) {
                let position = NSPoint::new(origin.x, origin.y + row.origin.y + 2.);
                if cached.view.frame().origin != position {
                    cached.view.setFrameOrigin(position);
                }
                if let Some(button) = cached.view.ivars().get() {
                    button.setTag(isize::try_from(preview.source.start).unwrap_or(-1));
                    if button.isEnabled() != state.text.isEditable() {
                        button.setEnabled(state.text.isEditable());
                    }
                    if button.isEnabled() {
                        cursors.push((
                            button.convertRect_toView(button.bounds(), Some(&state.text)),
                            Pointer::Arrow,
                        ));
                    }
                }
                views.push(cached);
                continue;
            }
            // SAFETY: BlockPreview subclasses NSView and the retained object is initialized on the main thread.
            let view: Retained<BlockPreview> = unsafe {
                msg_send![
                    super(BlockPreview::alloc(mtm).set_ivars(OnceCell::new())),
                    init
                ]
            };
            let height = match &preview.content {
                Content::Separator => {
                    let height = row.size.height - 4.;
                    let line = panel(width, 1., 0x474748, mtm);
                    line.setCornerRadius(0.);
                    line.setFrameOrigin(NSPoint::new(0., (height - 1.) / 2.));
                    view.addSubview(&line);
                    height
                }
                Content::Table(rows) => {
                    let height = draw_table(&view, rows, value, formulas, width, mtm);
                    // SAFETY: The retained delegate implements openTableMenu: and lives as long as its editor.
                    let button = unsafe {
                        NSButton::buttonWithTitle_target_action(
                            &NSString::from_str("⋯"),
                            Some(self),
                            Some(sel!(openTableMenu:)),
                            mtm,
                        )
                    };
                    button.setFrame(rect(width - 24., height - ROW_HEIGHT, 23., ROW_HEIGHT));
                    button.setBordered(false);
                    button.setEnabled(state.text.isEditable());
                    button.setAccessibilityLabel(Some(&NSString::from_str("Opzioni tabella")));
                    button.setTag(isize::try_from(preview.source.start).unwrap_or(-1));
                    view.addSubview(&button);
                    let _ = view.ivars().set(button);
                    height
                }
                Content::Callout(callout) => draw_callout(&view, callout, width, mtm),
                Content::Image { image, alt, size } => {
                    let photo = NSImageView::new(mtm);
                    photo.setImage(Some(image));
                    photo.setFrame(NSRect::new(NSPoint::new(0., 0.), *size));
                    photo.setAccessibilityLabel(Some(&NSString::from_str(alt)));
                    view.addSubview(&photo);
                    size.height
                }
            };
            view.setFrame(rect(origin.x, origin.y + row.origin.y + 2., width, height));
            state.text.addSubview(&view);
            if let Some(button) = view.ivars().get().filter(|button| button.isEnabled()) {
                cursors.push((
                    button.convertRect_toView(button.bounds(), Some(&state.text)),
                    Pointer::Arrow,
                ));
            }
            views.push(CachedBlock {
                view,
                source: source.to_owned(),
                width,
                row_height: row.size.height,
            });
        }
    }
    pub(super) fn table_menu(&self, button: &NSButton) {
        let menu = NSMenu::new(self.mtm());
        menu.setAutoenablesItems(false);
        for (label, action) in [
            ("Aggiungi riga", sel!(appendTableRow:)),
            ("Aggiungi colonna", sel!(appendTableColumn:)),
        ] {
            // SAFETY: Both selectors are implemented by this retained main-thread delegate.
            let item = unsafe {
                let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(self.mtm()),
                    &NSString::from_str(label),
                    Some(action),
                    &NSString::new(),
                );
                item.setTarget(Some(self));
                item
            };
            item.setTag(button.tag());
            menu.addItem(&item);
        }
        menu.popUpMenuPositioningItem_atLocation_inView(None, NSPoint::new(0., 0.), Some(button));
    }

    pub(super) fn edit_table(&self, offset: isize, edit: TableEdit) {
        let Ok(offset) = usize::try_from(offset) else {
            return;
        };
        let state = self.ivars();
        if !state.text.isEditable() {
            return;
        }
        let source = state.text.string().to_string();
        let markdown = super::LiveMarkdown::parse(&source);
        let Some(block) = markdown
            .blocks()
            .iter()
            .find(|block| block.range.start == offset)
        else {
            return;
        };
        let BlockContent::Table(rows) = &block.content else {
            return;
        };
        let Some(header) = rows.first() else { return };
        let start = source[..offset].encode_utf16().count();
        let previous = &source[block.range.clone()];
        let replacement = extend_table(previous, header.len(), edit);
        let selection = NSTextInputClient::selectedRange(&*state.text);
        let previous_len = previous.encode_utf16().count();
        let replacement_len = replacement.encode_utf16().count();
        // SAFETY: The table range was parsed from the current text and mapped to UTF-16.
        unsafe {
            state.text.insertText_replacementRange(
                &NSString::from_str(&replacement),
                NSRange::new(start, previous_len),
            );
        }
        let location = if selection.location >= start + previous_len {
            selection.location - previous_len + replacement_len
        } else {
            selection.location.min(start)
        };
        state
            .text
            .setSelectedRange(NSRange::new(location, selection.length));
    }
}

fn draw_table(
    view: &NSView,
    rows: &[Vec<Vec<CellPart>>],
    source: &str,
    formulas: &[Formula],
    width: f64,
    mtm: MainThreadMarker,
) -> f64 {
    let columns = rows.first().map_or(1, |row| row.len().max(1));
    let cell_width = width / columns as f64;
    let height = rows.len() as f64 * ROW_HEIGHT;
    view.setFrame(rect(0., 0., width, height));
    let grid = panel(width, height, 0x434548, mtm);
    for (row, cells) in rows.iter().enumerate() {
        for (column, value) in cells.iter().enumerate() {
            let y = height - (row + 1) as f64 * ROW_HEIGHT;
            let fill = match row {
                0 => 0x333537,
                n if n % 2 == 0 => 0x272829,
                _ => 0x222324,
            };
            let cell = panel(cell_width - 1., ROW_HEIGHT - 1., fill, mtm);
            cell.setCornerRadius(0.);
            cell.setFrame(rect(
                column as f64 * cell_width,
                y,
                cell_width - 1.,
                ROW_HEIGHT - 1.,
            ));
            let action_space = if row == 0 && column + 1 == columns {
                19.
            } else {
                0.
            };
            let content_width = (cell_width - 14. - action_space).max(1.);
            draw_cell(&cell, value, source, formulas, content_width, row == 0, mtm);
            grid.addSubview(&cell);
        }
    }
    view.addSubview(&grid);
    height
}

fn draw_cell(
    view: &NSView,
    parts: &[CellPart],
    source: &str,
    formulas: &[Formula],
    width: f64,
    header: bool,
    mtm: MainThreadMarker,
) {
    if let [CellPart::Text(text)] = parts {
        let label = label(text, TEXT, header, mtm);
        label.setFrame(rect(7., 4., width, 19.));
        view.addSubview(&label);
        return;
    }

    let mut x = 7.;
    let right = x + width;
    for part in parts {
        let remaining = right - x;
        if remaining < 1. {
            break;
        }

        let text = match part {
            CellPart::Text(text) => text.as_str(),
            CellPart::Formula(range) => {
                let raw = &source[range.clone()];
                let graphic = formulas
                    .iter()
                    .find(|formula| formula.range == *range)
                    .and_then(Graphic::render);
                if let Some(graphic) = graphic {
                    let size = graphic.size;
                    let scale = (14_f64 / 15.)
                        .min(remaining / size.width)
                        .min(22. / size.height);
                    let height = size.height * scale;
                    let width = size.width * scale;
                    let image = NSImageView::new(mtm);
                    image.setImage(Some(&graphic.image));
                    image.setAccessibilityLabel(Some(&NSString::from_str(raw)));
                    image.setFrame(rect(x, (ROW_HEIGHT - 1. - height) / 2., width, height));
                    view.addSubview(&image);
                    x += width;
                    continue;
                }
                raw
            }
        };
        let field = label(text, TEXT, header, mtm);
        field.sizeToFit();
        let width = field.frame().size.width.min(remaining);
        field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        field.setFrame(rect(x, 4., width, 19.));
        view.addSubview(&field);
        x += width;
    }
}

fn draw_callout(view: &NSView, callout: &Callout, width: f64, mtm: MainThreadMarker) -> f64 {
    let (ink, fill, _) = callout.kind.appearance();
    let lines: Vec<_> = callout.body.lines().collect();
    let height = (lines.len() + 1) as f64 * ROW_HEIGHT - 3.;
    let background = panel(width, height, fill, mtm);
    let line = panel(3., height, ink, mtm);
    background.addSubview(&line);
    let symbol_name = match callout.kind {
        crate::blocks::CalloutKind::Note => "info.circle",
        crate::blocks::CalloutKind::Tip => "lightbulb",
        crate::blocks::CalloutKind::Important => "exclamationmark.circle",
        crate::blocks::CalloutKind::Warning => "exclamationmark.triangle",
        crate::blocks::CalloutKind::Caution => "exclamationmark.octagon",
    };
    let icon = NSImageView::new(mtm);
    icon.setImage(
        NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(symbol_name),
            None,
        )
        .as_deref(),
    );
    icon.setSymbolConfiguration(Some(
        &NSImageSymbolConfiguration::configurationWithPointSize_weight(14., 0.),
    ));
    icon.setContentTintColor(Some(&color(ink)));
    icon.setFrame(rect(11., height - 22., 17., 19.));
    background.addSubview(&icon);
    let title = label(&callout.title, ink, true, mtm);
    title.setFrame(rect(34., height - 22., width - 46., 19.));
    background.addSubview(&title);
    for (index, body) in lines.iter().enumerate() {
        let plain = markdown::to_mdast(body, &markdown::ParseOptions::gfm())
            .map_or_else(|_| (*body).to_owned(), |node| node.to_string());
        let line = label(&plain, TEXT, false, mtm);
        line.setFrame(rect(
            12.,
            height - 22. - (index + 1) as f64 * ROW_HEIGHT,
            width - 24.,
            19.,
        ));
        background.addSubview(&line);
    }
    view.addSubview(&background);
    height
}

fn panel(width: f64, height: f64, fill: u32, mtm: MainThreadMarker) -> Retained<NSBox> {
    let panel = NSBox::new(mtm);
    panel.setBoxType(NSBoxType::Custom);
    panel.setTitlePosition(NSTitlePosition::NoTitle);
    panel.setContentViewMargins(NSSize::new(0., 0.));
    panel.setBorderWidth(0.);
    panel.setCornerRadius(6.);
    panel.setFillColor(&color(fill));
    panel.setFrame(rect(0., 0., width, height));
    panel
}

fn label(text: &str, ink: u32, bold: bool, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    let font = if bold {
        NSFont::boldSystemFontOfSize(14.)
    } else {
        NSFont::systemFontOfSize(14.)
    };
    label.setFont(Some(&font));
    label.setTextColor(Some(&color(ink)));
    label
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

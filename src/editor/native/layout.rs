use block2::RcBlock;
use gpui_kit::FontStyle;
use objc2::{
    AnyThread, DefinedClass, MainThreadOnly,
    rc::Retained,
    runtime::{AnyObject, Bool},
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::{
    NSDictionary, NSMutableDictionary, NSNumber, NSPoint, NSRange, NSRect, NSSize, NSString,
};

use super::{
    DocumentLink, EditorMode, HEADING_GUTTER, LiveMarkdown, MarkdownDelegate,
    blocks::Preview,
    color,
    cursors::{CursorTextView, Pointer},
    highlights::Highlight,
    math::{self, MathRun},
};
use crate::editor::markdown_lists::{ListLine, ListMarker};
use crate::{
    notes::tag_icons::{self, TagIcon, TagIcons},
    palette::{ACCENT, SUBTLE, TEXT},
};

use std::ops::Range;

// AppKit sets the image-to-title spacing, so add a nonbreaking space to the display title.
const TAG_ICON_GAP: &str = "\u{00a0}";
const BODY_FONT_SIZE: f64 = 15.;
const LIST_INDENT: f64 = 33.;

fn list_lane(entry: &ListLine) -> f64 {
    LIST_INDENT + entry.extra_digits as f64 * 8.5
}

fn list_indent(entry: &ListLine) -> f64 {
    entry.depth as f64 * LIST_INDENT + list_lane(entry)
}

#[derive(Default, PartialEq)]
pub(super) struct DecorationState {
    hidden: Vec<Range<usize>>,
    active_heading: Option<usize>,
    tag_pills: Vec<TagPill>,
}

#[derive(PartialEq)]
struct TagPill {
    link_index: usize,
    range: Range<usize>,
    standalone_width: Option<f64>,
    icon: TagIcon,
    visible: bool,
}

fn tag_pills(
    markdown: &LiveMarkdown,
    value: &str,
    selection: Option<&Range<usize>>,
    available_width: f64,
    icons: &TagIcons,
    previews: &[Preview<'_>],
) -> Vec<TagPill> {
    let font = NSFont::systemFontOfSize(15.);
    // SAFETY: NSFontAttributeName is the exported key for an NSFont value.
    let attributes = NSDictionary::from_slices(&[unsafe { NSFontAttributeName }], &[font.as_ref()]);
    markdown
        .links()
        .iter()
        .enumerate()
        .filter_map(|(index, (range, target))| {
            let DocumentLink::Tag(tag) = target else {
                return None;
            };
            if previews.iter().any(|preview| {
                preview.source.start <= range.start && range.end <= preview.source.end
            }) {
                return None;
            }
            let before = value[..range.start].rsplit('\n').next()?;
            let after = value[range.end..].split('\n').next()?.trim();
            let source_end = range.end + usize::from(value[range.end..].starts_with('#'));
            let visible = selection.is_none_or(|selection| {
                selection.start > source_end || selection.end < range.start
            });
            let standalone_width = (before.trim().is_empty() && matches!(after, "" | "#"))
                .then(|| {
                    let name = &value[range.start + 1..range.end];
                    let label = NSString::from_str(&format!("{TAG_ICON_GAP}{name}"));
                    // SAFETY: The attributes pair the font key with an NSFont.
                    unsafe { label.sizeWithAttributes(Some(&attributes)) }
                        .width
                        .ceil()
                        + 32.
                })
                .filter(|width| *width <= available_width);
            Some(TagPill {
                link_index: index,
                range: range.clone(),
                standalone_width,
                icon: tag_icons::resolve(icons, tag),
                visible,
            })
        })
        .collect()
}

impl MarkdownDelegate {
    fn place_decorations(
        &self,
        markdown: &LiveMarkdown,
        offsets: &[usize],
        previews: &[Preview<'_>],
        presentation: &DecorationState,
        value: &str,
    ) {
        let state = self.ivars();
        let mut checkboxes = state.checkboxes.borrow_mut();
        let mut previous_checkboxes = std::mem::take(&mut *checkboxes).into_iter();
        let mut heading_labels = state.heading_labels.borrow_mut();
        for label in heading_labels.drain(..) {
            label.removeFromSuperview();
        }
        let mut list_markers = state.list_markers.borrow_mut();
        let mut previous_markers = std::mem::take(&mut *list_markers).into_iter();
        let mut tag_buttons = state.tag_buttons.borrow_mut();
        for tag in tag_buttons.drain(..) {
            tag.removeFromSuperview();
        }
        // SAFETY: Both retained objects are retrieved from the same main-thread text view.
        let (Some(layout), Some(container)) = (unsafe { state.text.layoutManager() }, unsafe {
            state.text.textContainer()
        }) else {
            return;
        };
        layout.ensureLayoutForTextContainer(&container);
        let mut cursors = Vec::new();
        self.place_blocks(
            previews,
            offsets,
            &layout,
            &mut cursors,
            value,
            markdown.formulas(),
        );
        let origin = state.text.textContainerOrigin();
        for heading in markdown
            .headings()
            .iter()
            .filter(|heading| Some(heading.range.start) == presentation.active_heading)
        {
            let y = if heading.range.is_empty() {
                origin.y + 19.
            } else {
                let start = offsets[heading.range.start];
                let end = offsets[heading.range.end];
                // SAFETY: The character range maps to this layout’s current UTF-16 text.
                let glyphs = unsafe {
                    layout.glyphRangeForCharacterRange_actualCharacterRange(
                        NSRange::new(start, end - start),
                        std::ptr::null_mut(),
                    )
                };
                // SAFETY: The glyph index came from this layout manager after ensuring layout.
                let row = unsafe {
                    layout.lineFragmentRectForGlyphAtIndex_effectiveRange(
                        glyphs.location,
                        std::ptr::null_mut(),
                    )
                };
                origin.y + row.origin.y + (row.size.height - 16.) / 2.
            };
            let label = NSTextField::labelWithString(
                &NSString::from_str(&format!("≡{}", heading.level)),
                self.mtm(),
            );
            label.setFont(Some(&NSFont::systemFontOfSize(12.)));
            label.setTextColor(Some(&color(SUBTLE)));
            label.setFrame(NSRect::new(
                NSPoint::new(origin.x - HEADING_GUTTER, y),
                NSSize::new(26., 16.),
            ));
            state.text.addSubview(&label);
            heading_labels.push(label);
        }
        for tag in presentation.tag_pills.iter().filter(|tag| tag.visible) {
            let Some(width) = tag.standalone_width else {
                continue;
            };
            let start = offsets[tag.range.start];
            let range = NSRange::new(start, offsets[tag.range.end] - start);
            // SAFETY: The range maps to the current text storage and no output range is needed.
            let glyphs = unsafe {
                layout.glyphRangeForCharacterRange_actualCharacterRange(range, std::ptr::null_mut())
            };
            let bounds = layout.boundingRectForGlyphRange_inTextContainer(glyphs, &container);
            let name = &value[tag.range.start + 1..tag.range.end];
            let accessible = NSString::from_str(&format!("Tag {name}"));
            let mut images = state.tag_images.borrow_mut();
            let image = images.get(&tag.icon).cloned().or_else(|| {
                let symbol = super::highlights::tag_symbol(tag.icon)?;
                // The extra point of bottom padding centers the artwork on the title.
                let mut size = symbol.size();
                size.height += 1.;
                let image = NSImage::imageWithSize_flipped_drawingHandler(
                    size,
                    false,
                    &RcBlock::new(move |mut rect: NSRect| {
                        let bottom_padding = rect.size.height / size.height;
                        rect.origin.y += bottom_padding;
                        rect.size.height -= bottom_padding;
                        symbol.drawInRect(rect);
                        Bool::YES
                    }),
                );
                image.setTemplate(true);
                images.insert(tag.icon, image.clone());
                Some(image)
            });
            // SAFETY: The delegate implements followTag: and lives as long as its editor.
            let button = unsafe {
                if let Some(image) = &image {
                    NSButton::buttonWithTitle_image_target_action(
                        &NSString::from_str(&format!("{TAG_ICON_GAP}{name}")),
                        image,
                        Some(self),
                        Some(sel!(followTag:)),
                        self.mtm(),
                    )
                } else {
                    NSButton::buttonWithTitle_target_action(
                        &NSString::from_str(name),
                        Some(self),
                        Some(sel!(followTag:)),
                        self.mtm(),
                    )
                }
            };
            button.setBordered(false);
            button.setFont(Some(&NSFont::systemFontOfSize(15.)));
            button.setImagePosition(NSCellImagePosition::ImageLeft);
            button.setAlignment(NSTextAlignment::Center);
            button.setImageHugsTitle(true);
            button.setContentTintColor(Some(&color(TEXT)));
            button.setAccessibilityLabel(Some(&accessible));
            button.setTag(isize::try_from(tag.link_index).expect("link index fits in isize"));
            button.setWantsLayer(true);
            if let Some(layer) = button.layer() {
                layer.setBackgroundColor(Some(&color(0x424344).CGColor()));
                layer.setCornerRadius(13.);
            }
            button.setFrame(NSRect::new(
                NSPoint::new(
                    origin.x + bounds.origin.x,
                    origin.y + bounds.origin.y + (bounds.size.height - 26.) / 2.,
                ),
                NSSize::new(width, 26.),
            ));
            state.text.addSubview(&button);
            cursors.push((button.frame(), Pointer::Hand));
            tag_buttons.push(button);
        }
        let body_cap_height = NSFont::systemFontOfSize(BODY_FONT_SIZE).capHeight();
        for entry in markdown.list_lines() {
            let Some(marker) = &entry.marker else {
                continue;
            };
            if entry.prefix.is_empty() {
                continue;
            }
            let range = NSRange::new(offsets[entry.prefix.start], 1);
            // SAFETY: The marker offset and glyph both belong to this layout.
            let (glyph, line) = unsafe {
                let glyph = layout
                    .glyphRangeForCharacterRange_actualCharacterRange(range, std::ptr::null_mut())
                    .location;
                let line = layout
                    .lineFragmentRectForGlyphAtIndex_effectiveRange(glyph, std::ptr::null_mut());
                (glyph, line)
            };
            let baseline = line.origin.y + layout.locationForGlyphAtIndex(glyph).y;
            let center_y = origin.y + baseline - body_cap_height / 2.;
            let x = origin.x + entry.depth as f64 * LIST_INDENT;
            let ListMarker::Task { offset, checked } = marker else {
                let marker = match marker {
                    ListMarker::Bullet => {
                        let image = bullet_image(entry.depth);
                        let marker = previous_markers
                            .next()
                            .and_then(|view| match view.downcast::<NSImageView>() {
                                Ok(view) => Some(view),
                                Err(view) => {
                                    view.removeFromSuperview();
                                    None
                                }
                            })
                            .unwrap_or_else(|| NSImageView::new(self.mtm()));
                        if !marker
                            .image()
                            .is_some_and(|current| std::ptr::eq(&*current, &*image))
                        {
                            marker.setImage(Some(&image));
                        }
                        let frame = NSRect::new(NSPoint::new(x + 5., center_y - 5.), image.size());
                        if marker.frame() != frame {
                            marker.setFrame(frame);
                        }
                        marker.into_super().into_super()
                    }
                    ListMarker::Number(number) => {
                        let label = previous_markers
                            .next()
                            .and_then(|view| match view.downcast::<NSTextField>() {
                                Ok(view) => Some(view),
                                Err(view) => {
                                    view.removeFromSuperview();
                                    None
                                }
                            })
                            .unwrap_or_else(|| {
                                let label = NSTextField::labelWithString(
                                    &NSString::from_str(number),
                                    self.mtm(),
                                );
                                label.setFont(Some(&NSFont::systemFontOfSize(BODY_FONT_SIZE)));
                                label.setTextColor(Some(&color(ACCENT)));
                                label.setAlignment(NSTextAlignment::Right);
                                label
                            });
                        if label.stringValue().to_string() != *number {
                            label.setStringValue(&NSString::from_str(number));
                        }
                        let frame = NSRect::new(
                            NSPoint::new(x - 1., center_y - 9.),
                            NSSize::new(list_lane(entry) - 10., 19.),
                        );
                        if label.frame() != frame {
                            label.setFrame(frame);
                        }
                        label.into_super().into_super()
                    }
                    ListMarker::Task { .. } => unreachable!(),
                };
                // SAFETY: The marker is a retained view accessed on the main thread.
                if unsafe { marker.superview() }.is_none() {
                    state.text.addSubview(&marker);
                }
                list_markers.push(marker);
                continue;
            };
            let label = NSString::from_str(if *checked {
                "Riapri attività"
            } else {
                "Completa attività"
            });
            let symbol = NSString::from_str(if *checked {
                "checkmark.square"
            } else {
                "square"
            });
            let reusable = previous_checkboxes.next().and_then(|button| {
                if (button.state() != 0) == *checked {
                    Some(button)
                } else {
                    button.removeFromSuperview();
                    None
                }
            });
            let button = if let Some(button) = reusable {
                button
            } else if let Some(image) =
                NSImage::imageWithSystemSymbolName_accessibilityDescription(&symbol, Some(&label))
            {
                let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(21., 0.);
                let image = image.imageWithSymbolConfiguration(&config).unwrap_or(image);
                // SAFETY: The image is retained and the delegate implements toggleTask: for NSButton.
                let button = unsafe {
                    NSButton::buttonWithImage_target_action(
                        &image,
                        Some(self),
                        Some(sel!(toggleTask:)),
                        self.mtm(),
                    )
                };
                button.setBordered(false);
                button.setContentTintColor(Some(&color(if *checked {
                    0x737578
                } else {
                    0xa5a5a7
                })));
                // SAFETY: The checkbox role is an exported AppKit constant.
                button.setAccessibilityRole(Some(unsafe { NSAccessibilityCheckBoxRole }));
                // SAFETY: The checkbox accessibility value is an NSNumber with a boolean value.
                unsafe {
                    button
                        .setAccessibilityValue(Some(&NSNumber::numberWithInt(i32::from(*checked))));
                }
                button
            } else {
                // SAFETY: The delegate implements toggleTask: for NSButton and the title is an NSString.
                unsafe {
                    NSButton::checkboxWithTitle_target_action(
                        &NSString::from_str(""),
                        Some(self),
                        Some(sel!(toggleTask:)),
                        self.mtm(),
                    )
                }
            };
            button.setAccessibilityLabel(Some(&label));
            button.setTag(offsets[*offset] as isize);
            let enabled = state.mode.get() != super::EditorMode::Protected;
            if button.isEnabled() != enabled {
                button.setEnabled(enabled);
            }
            let checked_state = if *checked { 1 } else { 0 };
            if button.state() != checked_state {
                button.setState(checked_state);
            }
            let frame = NSRect::new(NSPoint::new(x - 1., center_y - 10.), NSSize::new(21., 20.));
            if button.frame() != frame {
                button.setFrame(frame);
            }
            // SAFETY: Inspect the retained checkbox's parent on the AppKit main thread.
            if unsafe { button.superview() }.is_none() {
                state.text.addSubview(&button);
            }
            if button.isEnabled() {
                cursors.push((button.frame(), Pointer::Arrow));
            }
            checkboxes.push(button);
        }
        for checkbox in previous_checkboxes {
            checkbox.removeFromSuperview();
        }
        for marker in previous_markers {
            marker.removeFromSuperview();
        }
        CursorTextView::from_text(&state.text).set_inline(cursors);
    }

    pub(super) fn restyle(&self, force: bool) {
        let _timing = crate::performance::Sample::start("restyle");
        let state = self.ivars();
        if state.busy.get() || NSTextInputClient::hasMarkedText(&*state.text) {
            return;
        }
        let value = state.text.string().to_string();
        let selection = NSTextInputClient::selectedRange(&*state.text);
        let mut offsets = vec![0; value.len() + 1];
        let mut utf16 = 0;
        for (index, ch) in value.char_indices() {
            offsets[index] = utf16;
            utf16 += ch.len_utf16();
        }
        offsets[value.len()] = utf16;
        let to_bytes = |offset: usize| {
            let mut length = 0;
            value
                .char_indices()
                .find_map(|(index, ch)| {
                    let start = length;
                    length += ch.len_utf16();
                    (offset < length || offset == start).then_some(index)
                })
                .unwrap_or(value.len())
        };
        let focused = state
            .text
            .window()
            .is_some_and(|window| window.firstResponder().as_deref() == Some(state.text.as_ref()));
        let selection = (state.mode.get() == EditorMode::Editing && focused).then(|| {
            to_bytes(selection.location)
                ..to_bytes(selection.location.saturating_add(selection.length))
        });
        let mut cached = state.markdown.borrow_mut();
        let changed = cached.0 != value;
        if changed {
            cached.1 = LiveMarkdown::parse(&value);
            math::retain_formulas(cached.1.formulas());
            cached.0 = value.clone();
        }
        let markdown = &cached.1;
        let previews = self.prepare_blocks(markdown.blocks(), selection.as_ref());
        let mut hidden: Vec<_> = markdown
            .hidden_markers(selection.as_ref())
            .into_iter()
            .cloned()
            .collect();
        hidden.extend(previews.iter().map(|preview| preview.source.clone()));
        let width = state.text.bounds().size.width - HEADING_GUTTER * 2.;
        let math: Vec<_> = markdown
            .formulas()
            .iter()
            .filter_map(|formula| {
                if selection.as_ref().is_some_and(|selection| {
                    selection.start <= formula.range.end && selection.end >= formula.range.start
                }) || previews.iter().any(|preview| {
                    preview.source.start <= formula.range.start
                        && formula.range.end <= preview.source.end
                }) {
                    return None;
                }
                let font_size = markdown
                    .headings()
                    .iter()
                    .find(|heading| heading.range.contains(&formula.range.start))
                    .map_or(BODY_FONT_SIZE, |heading| heading.size());
                let start = offsets[formula.range.start];
                let range = NSRange::new(start, offsets[formula.range.end] - start);
                let indent = markdown
                    .list_lines()
                    .iter()
                    .find(|line| line.range.contains(&formula.range.start))
                    .map_or(0., list_indent);
                Some((
                    formula,
                    MathRun::render(formula, range, width - indent, font_size)?,
                ))
            })
            .collect();
        hidden.extend(math.iter().map(|(formula, _)| formula.range.clone()));
        let active_heading = selection.as_ref().and_then(|selection| {
            markdown
                .headings()
                .iter()
                .find(|heading| {
                    selection.start >= heading.range.start && selection.start <= heading.range.end
                })
                .map(|heading| heading.range.start)
        });
        let tag_pills = tag_pills(
            markdown,
            &value,
            selection.as_ref(),
            width,
            &state.tag_icons.borrow(),
            &previews,
        );
        let presentation = DecorationState {
            hidden,
            active_heading,
            tag_pills,
        };
        if !force && !changed && *state.decoration_state.borrow() == presentation {
            return;
        }
        let decorations = markdown.decorations(selection.as_ref());
        // SAFETY: Storage is retrieved from the retained text view on the main thread.
        let Some(live_storage) = (unsafe { state.text.textStorage() }) else {
            return;
        };
        state.busy.set(true);
        let undo = state.text.undoManager();
        if let Some(undo) = &undo {
            undo.disableUndoRegistration();
        }
        // Build the desired attributes off-screen. Only differing runs are applied
        // to the live storage, preserving TextKit's layout for unchanged paragraphs.
        // SAFETY: NSTextStorage inherits the attributed-string initializer; it
        // receives a valid NSString and returns a retained storage instance.
        let storage: Retained<NSTextStorage> = unsafe {
            objc2::msg_send![NSTextStorage::alloc(), initWithString: &*NSString::from_str(&value)]
        };
        storage.beginEditing();
        let body_font = NSFont::systemFontOfSize(BODY_FONT_SIZE);
        let foreground = color(TEXT);
        let paragraph = NSMutableParagraphStyle::new();
        paragraph.setMinimumLineHeight(26.);
        let total = NSRange::new(0, utf16);
        // SAFETY: The whole range matches the current UTF-16 length and the values are AppKit attribute types.
        unsafe {
            storage.setAttributes_range(None, total);
            storage.addAttribute_value_range(NSFontAttributeName, &body_font, total);
            storage.addAttribute_value_range(NSForegroundColorAttributeName, &foreground, total);
            storage.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, total);
        }
        let font_manager = NSFontManager::sharedFontManager(self.mtm());
        let mut boundaries = vec![0, value.len()];
        for decoration in &decorations {
            boundaries.extend([decoration.range.start, decoration.range.end]);
        }
        for heading in markdown.headings() {
            boundaries.extend([heading.range.start, heading.range.end]);
        }
        for preview in &previews {
            boundaries.extend([preview.source.start, preview.source.end]);
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut attributes =
            Vec::<(RunStyle, Retained<NSMutableDictionary<NSString, AnyObject>>)>::new();
        let mut events: Vec<_> = decorations
            .iter()
            .enumerate()
            .flat_map(|(index, decoration)| {
                [
                    (decoration.range.start, true, index),
                    (decoration.range.end, false, index),
                ]
            })
            .collect();
        events.sort_unstable();
        let mut events = events.into_iter().peekable();
        let mut active = std::collections::BTreeSet::new();
        let mut hidden_ranges: Vec<_> = presentation.hidden.iter().collect();
        hidden_ranges.sort_unstable_by_key(|range| range.start);
        let mut hidden_index = 0;
        for pair in boundaries.windows(2) {
            let [start, end] = [pair[0], pair[1]];
            while events.peek().is_some_and(|event| event.0 <= start) {
                if let Some((_, entering, index)) = events.next() {
                    if entering {
                        active.insert(index);
                    } else {
                        active.remove(&index);
                    }
                }
            }
            let range = NSRange::new(offsets[start], offsets[end] - offsets[start]);
            let heading = markdown
                .headings()
                .iter()
                .find(|heading| heading.range.contains(&start));
            let mut style = RunStyle {
                size: heading.map_or(BODY_FONT_SIZE, |heading| heading.size()),
                bold: heading.is_some(),
                ..RunStyle::default()
            };
            while hidden_ranges
                .get(hidden_index)
                .is_some_and(|range| range.end <= start)
            {
                hidden_index += 1;
            }
            if hidden_ranges
                .get(hidden_index)
                .is_some_and(|range| range.contains(&start))
            {
                style.size = 0.01;
            }
            for &index in &active {
                let decoration = decorations[index].style;
                style.bold |= decoration.font_weight.is_some();
                style.italic |= decoration.font_style == Some(FontStyle::Italic);
                style.ink = decoration.color.or(style.ink);
                style.background = decoration.background_color.or(style.background);
                style.underline |= decoration.underline.is_some();
                style.strike |= decoration.strikethrough.is_some();
            }
            if style == RunStyle::default() {
                continue;
            }
            let index = if let Some(index) = attributes.iter().position(|(key, _)| key == &style) {
                index
            } else {
                let dictionary = style.attributes(&font_manager, &paragraph);
                attributes.push((style, dictionary));
                attributes.len() - 1
            };
            // SAFETY: The dictionary contains AppKit attributes and the run is within storage.
            unsafe {
                storage.setAttributes_range(Some(&attributes[index].1), range);
            }
        }
        for (index, (link, _)) in markdown.links().iter().enumerate().rev() {
            let start = offsets[link.start];
            let range = NSRange::new(start, offsets[link.end] - start);
            // SAFETY: The link range belongs to this text storage and the delegate handles the NSString identifier.
            unsafe {
                storage.addAttribute_value_range(
                    NSLinkAttributeName,
                    &NSString::from_str(&index.to_string()),
                    range,
                );
                storage.addAttribute_value_range(
                    NSCursorAttributeName,
                    &NSCursor::pointingHandCursor(),
                    range,
                );
            }
        }
        for heading in markdown.headings() {
            let range = &heading.range;
            if range.is_empty() {
                continue;
            }
            let paragraph = NSMutableParagraphStyle::new();
            if heading.level == 1 && heading.range.start == 0 {
                paragraph.setMinimumLineHeight(38.);
                paragraph.setParagraphSpacing(10.);
            } else {
                paragraph.setMinimumLineHeight(if heading.level == 1 { 48. } else { 35. });
            }
            let range = NSRange::new(
                offsets[range.start],
                offsets[range.end] - offsets[range.start],
            );
            // SAFETY: The heading range maps to current storage and the value is a paragraph style.
            unsafe {
                storage.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, range);
            }
        }
        for entry in markdown.list_lines() {
            let start = offsets[entry.range.start];
            let range = NSRange::new(start, offsets[entry.range.end] - start);
            let paragraph = NSMutableParagraphStyle::new();
            paragraph.setMinimumLineHeight(27.);
            let indent = list_indent(entry);
            paragraph.setFirstLineHeadIndent(indent);
            paragraph.setHeadIndent(indent);
            // SAFETY: The list entry belongs to current storage and the value is a paragraph style.
            unsafe {
                storage.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, range);
            }
            let hidden_font = NSFont::systemFontOfSize(0.01);
            let hidden_color = NSColor::clearColor();
            for (index, line) in value[entry.range.clone()].split_inclusive('\n').scan(
                entry.range.start,
                |start, line| {
                    let index = *start;
                    *start += line.len();
                    Some((index, line))
                },
            ) {
                let end = if index == entry.range.start {
                    entry.prefix.end
                } else {
                    index + line.len() - line.trim_start_matches([' ', '\t']).len()
                };
                let prefix = NSRange::new(offsets[index], offsets[end] - offsets[index]);
                // SAFETY: Prefix offsets are character boundaries in the current storage.
                unsafe {
                    storage.addAttribute_value_range(NSFontAttributeName, &hidden_font, prefix);
                    storage.addAttribute_value_range(
                        NSForegroundColorAttributeName,
                        &hidden_color,
                        prefix,
                    );
                }
            }
        }
        for preview in &previews {
            let start = offsets[preview.source.start];
            let end = offsets[preview.source.end];
            let range = NSRange::new(start, end - start);
            let paragraph = NSMutableParagraphStyle::new();
            paragraph.setMinimumLineHeight(preview.line_height);
            // SAFETY: Both attributes apply to a valid UTF-16 source range and use native AppKit types.
            unsafe {
                storage.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, range);
                storage.addAttribute_value_range(
                    NSForegroundColorAttributeName,
                    &NSColor::clearColor(),
                    range,
                );
            }
        }
        let mut inline_icons = Vec::new();
        let mut highlights = Vec::new();
        for tag in &presentation.tag_pills {
            let start = offsets[tag.range.start];
            let range = NSRange::new(start, offsets[tag.range.end] - start);
            // SAFETY: The tag's range maps to the current text storage.
            unsafe { storage.removeAttribute_range(NSBackgroundColorAttributeName, range) };
            if tag.standalone_width.is_some() {
                let paragraph = NSMutableParagraphStyle::new();
                paragraph.setMinimumLineHeight(26.);
                paragraph.setParagraphSpacingBefore(12.);
                // SAFETY: This range belongs to the current storage.
                unsafe {
                    if tag.visible {
                        storage.addAttribute_value_range(
                            NSForegroundColorAttributeName,
                            &NSColor::clearColor(),
                            range,
                        );
                    }
                    storage.addAttribute_value_range(
                        NSParagraphStyleAttributeName,
                        &paragraph,
                        range,
                    );
                }
            } else if tag.visible {
                let hash = NSRange::new(start, 1);
                // SAFETY: The leading hash is one UTF-16 unit in the current storage.
                unsafe {
                    storage.addAttribute_value_range(
                        NSForegroundColorAttributeName,
                        &NSColor::clearColor(),
                        hash,
                    );
                    storage.addAttribute_value_range(
                        NSKernAttributeName,
                        &NSNumber::new_f64(11.),
                        hash,
                    );
                }
                inline_icons.push((start, tag.icon));
                highlights.push(Highlight {
                    range,
                    fill: 0x424344,
                    height: 23.,
                    radius: 11.5,
                });
            }
        }
        inline_icons.sort_unstable_by_key(|(start, _)| *start);
        if state.highlight_layout.replace_tags(inline_icons) {
            state.text.setNeedsDisplay(true);
        }
        for (formula, run) in &math {
            math::apply_layout(&storage, &value, &offsets, formula, run);
        }
        let math_changed = state
            .highlight_layout
            .replace_math(math.into_iter().map(|(_, run)| run).collect());
        highlights.extend(markdown.highlights().iter().filter_map(|(range, fill)| {
            if previews.iter().any(|preview| {
                preview.source.start <= range.start && range.end <= preview.source.end
            }) {
                return None;
            }
            let size = markdown
                .headings()
                .iter()
                .find(|heading| heading.range.contains(&range.start))
                .map_or(BODY_FONT_SIZE, |heading| heading.size());
            let start = offsets[range.start];
            Some(Highlight {
                range: NSRange::new(start, offsets[range.end] - start),
                fill: *fill,
                height: size * 0.5 + 16.5,
                radius: 3.,
            })
        }));
        if state.highlight_layout.replace_highlights(highlights) {
            state.text.setNeedsDisplay(true);
        }
        storage.endEditing();
        apply_changed_attributes(&live_storage, &storage);
        if math_changed {
            // SAFETY: The full UTF-16 range matches the current storage and no output range is needed.
            unsafe {
                state
                    .highlight_layout
                    .invalidateGlyphsForCharacterRange_changeInLength_actualCharacterRange(
                        total,
                        0,
                        std::ptr::null_mut(),
                    );
                state
                    .highlight_layout
                    .invalidateLayoutForCharacterRange_actualCharacterRange(
                        total,
                        std::ptr::null_mut(),
                    );
            }
            state.text.setNeedsDisplay(true);
        }
        self.place_decorations(markdown, &offsets, &previews, &presentation, &value);
        state.decoration_state.replace(presentation);
        if let Some(undo) = &undo {
            undo.enableUndoRegistration();
        }
        state.busy.set(false);
    }
}

#[derive(PartialEq)]
struct RunStyle {
    size: f64,
    bold: bool,
    italic: bool,
    ink: Option<gpui_kit::Hsla>,
    background: Option<gpui_kit::Hsla>,
    underline: bool,
    strike: bool,
}

impl Default for RunStyle {
    fn default() -> Self {
        Self {
            size: BODY_FONT_SIZE,
            bold: false,
            italic: false,
            ink: None,
            background: None,
            underline: false,
            strike: false,
        }
    }
}

impl RunStyle {
    fn attributes(
        &self,
        manager: &NSFontManager,
        paragraph: &NSParagraphStyle,
    ) -> Retained<NSMutableDictionary<NSString, AnyObject>> {
        let result = NSMutableDictionary::new();
        let mut font = if self.bold {
            NSFont::boldSystemFontOfSize(self.size)
        } else {
            NSFont::systemFontOfSize(self.size)
        };
        if self.italic {
            font = manager.convertFont_toHaveTrait(&font, NSFontTraitMask::ItalicFontMask);
        }
        let foreground = self.ink.map_or_else(|| color(TEXT), hsla);
        // SAFETY: Each exported attribute key receives its documented AppKit value type.
        unsafe {
            result.insert(NSFontAttributeName, font.as_ref());
            result.insert(NSForegroundColorAttributeName, foreground.as_ref());
            result.insert(NSParagraphStyleAttributeName, paragraph.as_ref());
            if let Some(background) = self.background {
                result.insert(NSBackgroundColorAttributeName, hsla(background).as_ref());
            }
            if self.underline {
                result.insert(
                    NSUnderlineStyleAttributeName,
                    NSNumber::numberWithInt(1).as_ref(),
                );
            }
            if self.strike {
                result.insert(
                    NSStrikethroughStyleAttributeName,
                    NSNumber::numberWithInt(1).as_ref(),
                );
            }
        }
        result
    }
}

fn apply_changed_attributes(live: &NSTextStorage, desired: &NSTextStorage) {
    let _timing = crate::performance::Sample::start("attributes");
    let mut index = 0;
    live.beginEditing();
    while index < desired.length() {
        let mut old_range = NSRange::default();
        let mut new_range = NSRange::default();
        // SAFETY: Both storages contain the same UTF-16 string; index is in bounds
        // and AppKit initializes the effective ranges through these valid pointers.
        let (old, new) = unsafe {
            (
                live.attributesAtIndex_effectiveRange(index, &mut old_range),
                desired.attributesAtIndex_effectiveRange(index, &mut new_range),
            )
        };
        let end =
            (old_range.location + old_range.length).min(new_range.location + new_range.length);
        if !old.isEqualToDictionary(&new) {
            // SAFETY: The intersection of the two attribute runs is within both storages.
            unsafe {
                live.setAttributes_range(Some(&new), NSRange::new(index, end - index));
            }
        }
        index = end;
    }
    live.endEditing();
}

fn bullet_image(depth: usize) -> Retained<NSImage> {
    thread_local! {
        static IMAGES: [Retained<NSImage>; 3] = std::array::from_fn(create_bullet_image);
    }
    IMAGES.with(|images| images[depth % 3].clone())
}

fn create_bullet_image(shape: usize) -> Retained<NSImage> {
    NSImage::imageWithSize_flipped_drawingHandler(
        NSSize::new(10., 10.),
        false,
        &RcBlock::new(move |_rect: NSRect| {
            let ink = color(ACCENT);
            ink.set();
            match shape {
                0 => NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
                    NSPoint::new(2., 2.),
                    NSSize::new(6., 6.),
                ))
                .fill(),
                1 => {
                    let path = NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
                        NSPoint::new(2.5, 2.5),
                        NSSize::new(5., 5.),
                    ));
                    path.setLineWidth(1.);
                    path.stroke();
                }
                _ => {
                    let path = NSBezierPath::bezierPath();
                    path.moveToPoint(NSPoint::new(5., 1.));
                    path.lineToPoint(NSPoint::new(9., 5.));
                    path.lineToPoint(NSPoint::new(5., 9.));
                    path.lineToPoint(NSPoint::new(1., 5.));
                    path.closePath();
                    path.fill();
                }
            }
            Bool::YES
        }),
    )
}

fn hsla(ink: gpui_kit::Hsla) -> Retained<NSColor> {
    let rgba: gpui_kit::Rgba = ink.into();
    NSColor::colorWithSRGBRed_green_blue_alpha(
        rgba.r.into(),
        rgba.g.into(),
        rgba.b.into(),
        rgba.a.into(),
    )
}

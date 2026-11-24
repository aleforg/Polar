use block2::RcBlock;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::ProtocolObject,
};
use objc2_app_kit::*;
use objc2_foundation::{
    NSArray, NSIntersectionRange, NSNotFound, NSObjectProtocol, NSPoint, NSRange, NSRect, NSString,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    ptr::{self, NonNull},
};

use super::{color, math::MathRun};
use crate::{notes::tag_icons::TagIcon, palette::TEXT};

#[derive(PartialEq)]
pub(super) struct Highlight {
    pub(super) range: NSRange,
    pub(super) fill: u32,
    pub(super) height: f64,
    pub(super) radius: f64,
}

#[derive(Default)]
pub(super) struct LayoutState {
    highlights: Vec<Highlight>,
    math: Vec<MathRun>,
    tags: Vec<(usize, TagIcon)>,
    tag_images: HashMap<TagIcon, Retained<NSImage>>,
}

define_class!(
    #[unsafe(super(NSLayoutManager))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RefCell<LayoutState>]
    pub(super) struct HighlightLayout;

    impl HighlightLayout {
        #[unsafe(method(drawBackgroundForGlyphRange:atPoint:))]
        fn draw_background(&self, glyphs: NSRange, origin: NSPoint) {
            self.draw_highlights(glyphs, origin);
            // SAFETY: Forward the original AppKit drawing arguments to draw selection and other backgrounds.
            unsafe { msg_send![super(self), drawBackgroundForGlyphRange: glyphs, atPoint: origin] }
        }

        #[unsafe(method(drawGlyphsForGlyphRange:atPoint:))]
        fn draw_glyphs(&self, glyphs: NSRange, origin: NSPoint) {
            // SAFETY: AppKit supplies the glyph range and drawing origin.
            unsafe { msg_send![super(self), drawGlyphsForGlyphRange: glyphs, atPoint: origin] }
            self.draw_math(glyphs, origin);
            self.draw_tags(glyphs, origin);
        }
    }

    // SAFETY: The subclass inherits NSObject conformance from NSLayoutManager.
    unsafe impl NSObjectProtocol for HighlightLayout {}

    // SAFETY: The layout manager calls its delegate on the main thread for its own text storage.
    unsafe impl NSLayoutManagerDelegate for HighlightLayout {
        #[unsafe(method(layoutManager:shouldGenerateGlyphs:properties:characterIndexes:font:forGlyphRange:))]
        unsafe fn generate_glyphs(
            &self,
            manager: &NSLayoutManager,
            glyphs: NonNull<u16>,
            properties: NonNull<NSGlyphProperty>,
            indexes: NonNull<usize>,
            font: &NSFont,
            range: NSRange,
        ) -> usize {
            let state = self.ivars().borrow();
            if state.math.is_empty() || range.length == 0 {
                return 0;
            }
            // SAFETY: AppKit provides a parallel buffer of range.length entries for each pointer.
            let characters = unsafe { std::slice::from_raw_parts(indexes.as_ptr(), range.length) };
            if !characters.iter().any(|&index| math_at(&state.math, index).is_some()) {
                return 0;
            }
            // SAFETY: The input buffers are valid for this callback and are copied before mutation.
            let mut glyphs = unsafe { std::slice::from_raw_parts(glyphs.as_ptr(), range.length) }.to_vec();
            let mut properties = unsafe { std::slice::from_raw_parts(properties.as_ptr(), range.length) }.to_vec();
            for (index, &character) in characters.iter().enumerate() {
                if let Some(math) = math_at(&state.math, character) {
                    if character == math.range.location {
                        glyphs[index] = u16::MAX;
                        properties[index] = NSGlyphProperty::ControlCharacter;
                    } else if !properties[index].contains(NSGlyphProperty::ControlCharacter) {
                        glyphs[index] = 0;
                        properties[index] = NSGlyphProperty::Null;
                    }
                }
            }
            // SAFETY: All three buffers contain exactly range.length entries and the manager copies them.
            unsafe {
                manager.setGlyphs_properties_characterIndexes_font_forGlyphRange(
                    NonNull::new(glyphs.as_mut_ptr()).unwrap(),
                    NonNull::new(properties.as_mut_ptr()).unwrap(),
                    indexes,
                    font,
                    range,
                );
            }
            range.length
        }

        #[unsafe(method(layoutManager:shouldUseAction:forControlCharacterAtIndex:))]
        fn control_action(
            &self,
            _manager: &NSLayoutManager,
            action: NSControlCharacterAction,
            character: usize,
        ) -> NSControlCharacterAction {
            if math_at(&self.ivars().borrow().math, character).is_some_and(|math| math.range.location == character) {
                NSControlCharacterAction::Whitespace
            } else {
                action
            }
        }

        #[unsafe(method(layoutManager:boundingBoxForControlGlyphAtIndex:forTextContainer:proposedLineFragment:glyphPosition:characterIndex:))]
        fn control_bounds(
            &self,
            _manager: &NSLayoutManager,
            _glyph: usize,
            _container: &NSTextContainer,
            _row: NSRect,
            _position: NSPoint,
            character: usize,
        ) -> NSRect {
            let state = self.ivars().borrow();
            math_at(&state.math, character).map_or(NSRect::default(), |math| {
                NSRect::new(NSPoint::default(), math.size)
            })
        }
    }
);

impl HighlightLayout {
    pub(super) fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: The layout manager receives its Rust ivars before the superclass initializes it.
        let layout: Retained<Self> =
            unsafe { msg_send![super(Self::alloc(mtm).set_ivars(RefCell::default())), init] };
        layout.setDelegate(Some(ProtocolObject::from_ref(&*layout)));
        layout
    }

    pub(super) fn replace_highlights(&self, highlights: Vec<Highlight>) -> bool {
        let mut state = self.ivars().borrow_mut();
        let previous = &mut state.highlights;
        if *previous == highlights {
            return false;
        }
        *previous = highlights;
        true
    }

    pub(super) fn replace_math(&self, math: Vec<MathRun>) -> bool {
        let mut state = self.ivars().borrow_mut();
        let changed = state.math.len() != math.len()
            || state.math.iter().zip(&math).any(|(old, new)| {
                old.range != new.range
                    || old.size != new.size
                    || old.baseline != new.baseline
                    || old.display != new.display
                    || !std::rc::Rc::ptr_eq(&old.graphic, &new.graphic)
            });
        state.math = math;
        changed
    }

    fn draw_math(&self, glyphs: NSRange, origin: NSPoint) {
        let state = self.ivars().borrow();
        if state.math.is_empty() || glyphs.length == 0 {
            return;
        }
        // SAFETY: The glyph range is the one AppKit supplied for the draw call.
        let visible =
            unsafe { self.characterRangeForGlyphRange_actualGlyphRange(glyphs, ptr::null_mut()) };
        for math in state
            .math
            .iter()
            .filter(|math| NSIntersectionRange(visible, math.range).length != 0)
        {
            // SAFETY: Each formula begins at a valid UTF-16 location in the current text storage.
            let (glyph, row) = unsafe {
                let glyph = self
                    .glyphRangeForCharacterRange_actualCharacterRange(
                        NSRange::new(math.range.location, 1),
                        ptr::null_mut(),
                    )
                    .location;
                let row =
                    self.lineFragmentRectForGlyphAtIndex_effectiveRange(glyph, ptr::null_mut());
                (glyph, row)
            };
            let position = self.locationForGlyphAtIndex(glyph);
            let (x, y) = if math.display {
                (
                    (row.size.width - math.size.width) / 2.,
                    (row.size.height - math.size.height) / 2.,
                )
            } else {
                (position.x, position.y - math.baseline)
            };
            let rect = NSRect::new(
                NSPoint::new(origin.x + row.origin.x + x, origin.y + row.origin.y + y),
                math.size,
            );
            // SAFETY: The formula is a retained AppKit image and no drawing hints are required.
            unsafe {
                math.graphic
                    .image
                    .drawInRect_fromRect_operation_fraction_respectFlipped_hints(
                        rect,
                        NSRect::default(),
                        NSCompositingOperation::SourceOver,
                        1.,
                        true,
                        None,
                    );
            }
        }
    }

    pub(super) fn replace_tags(&self, tags: Vec<(usize, TagIcon)>) -> bool {
        let mut state = self.ivars().borrow_mut();
        if state.tags == tags {
            return false;
        }
        for &(_, icon) in &tags {
            if let std::collections::hash_map::Entry::Vacant(entry) = state.tag_images.entry(icon)
                && let Some(image) = tag_symbol(icon)
            {
                let palette = NSArray::from_retained_slice(&[color(TEXT)]);
                let tint = NSImageSymbolConfiguration::configurationWithPaletteColors(&palette);
                entry.insert(image.imageWithSymbolConfiguration(&tint).unwrap_or(image));
            }
        }
        state.tags = tags;
        true
    }

    fn draw_tags(&self, glyphs: NSRange, origin: NSPoint) {
        let state = self.ivars().borrow();
        if state.tags.is_empty() || glyphs.length == 0 {
            return;
        }
        // SAFETY: AppKit supplies the glyph range for this draw call.
        let visible =
            unsafe { self.characterRangeForGlyphRange_actualGlyphRange(glyphs, ptr::null_mut()) };
        let first = state
            .tags
            .partition_point(|(index, _)| *index < visible.location);
        for &(index, icon) in state.tags[first..]
            .iter()
            .take_while(|(index, _)| *index < visible.location + visible.length)
        {
            let Some(image) = state.tag_images.get(&icon) else {
                continue;
            };
            // SAFETY: The index is the leading hash in the current text storage.
            let (glyph, row) = unsafe {
                let glyph = self
                    .glyphRangeForCharacterRange_actualCharacterRange(
                        NSRange::new(index, 1),
                        ptr::null_mut(),
                    )
                    .location;
                (
                    glyph,
                    self.lineFragmentRectForGlyphAtIndex_effectiveRange(glyph, ptr::null_mut()),
                )
            };
            let position = self.locationForGlyphAtIndex(glyph);
            let size = image.size();
            let rect = NSRect::new(
                NSPoint::new(
                    origin.x + row.origin.x + position.x + 2.,
                    origin.y + row.origin.y + position.y - 5.5 - size.height / 2.,
                ),
                size,
            );
            // SAFETY: The symbol is retained and draws in the current AppKit graphics context.
            unsafe {
                image.drawInRect_fromRect_operation_fraction_respectFlipped_hints(
                    rect,
                    NSRect::default(),
                    NSCompositingOperation::SourceOver,
                    1.,
                    true,
                    None,
                );
            }
        }
    }

    fn draw_highlights(&self, glyphs: NSRange, origin: NSPoint) {
        let state = self.ivars().borrow();
        let highlights = &state.highlights;
        if highlights.is_empty() || glyphs.length == 0 {
            return;
        }
        let Some(container) = self.textContainers().firstObject() else {
            return;
        };
        // SAFETY: AppKit supplies a valid draw range and no optional output range is needed.
        let visible =
            unsafe { self.characterRangeForGlyphRange_actualGlyphRange(glyphs, ptr::null_mut()) };
        let width = container.size().width;
        NSGraphicsContext::saveGraphicsState_class();
        for highlight in highlights
            .iter()
            .filter(|highlight| NSIntersectionRange(visible, highlight.range).length != 0)
        {
            // SAFETY: The highlight's range comes from the current UTF-16 text storage.
            let highlighted_glyphs = unsafe {
                self.glyphRangeForCharacterRange_actualCharacterRange(
                    highlight.range,
                    ptr::null_mut(),
                )
            };
            let fill = color(highlight.fill);
            let height = highlight.height;
            let radius = highlight.radius;
            fill.setFill();
            self.enumerateEnclosingRectsForGlyphRange_withinSelectedGlyphRange_inTextContainer_usingBlock(
                highlighted_glyphs,
                NSRange::new(NSNotFound as usize, 0),
                &container,
                &RcBlock::new(move |mut rect: NSRect, _| {
                    if rect.size.width > 0. {
                        let height = height.min(rect.size.height);
                        let left = (rect.origin.x - 3.).max(0.);
                        let right = (rect.origin.x + rect.size.width + 3.).min(width);
                        rect.origin.x = origin.x + left;
                        // TextKit centers the enclosing rect above the visible glyphs.
                        rect.origin.y += origin.y + (rect.size.height - height) / 2. + 4.5;
                        rect.size.width = right - left;
                        rect.size.height = height;
                        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, radius, radius).fill();
                    }
                }),
            );
        }
        NSGraphicsContext::restoreGraphicsState_class();
    }
}

fn math_at(math: &[MathRun], index: usize) -> Option<&MathRun> {
    let index_after = math.partition_point(|math| math.range.location <= index);
    let entry = math.get(index_after.checked_sub(1)?)?;
    (index < entry.range.location + entry.range.length).then_some(entry)
}

pub(super) fn tag_symbol(icon: TagIcon) -> Option<Retained<NSImage>> {
    let system_symbol = |icon: TagIcon| {
        NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(icon.symbol()),
            None,
        )
    };
    let image = system_symbol(icon).or_else(|| system_symbol(TagIcon::Hash))?;
    let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(11., 0.);
    Some(image.imageWithSymbolConfiguration(&config).unwrap_or(image))
}

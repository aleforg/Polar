use super::offsets::Offsets;
use objc2::{AnyThread, extern_class, extern_methods, rc::Retained};
use objc2_app_kit::NSImage;
use objc2_foundation::{NSData, NSMutableAttributedString, NSObject, NSRange, NSSize, NSString};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::editor::markdown::Formula;

#[link(name = "JavaScriptCore", kind = "framework")]
unsafe extern "C" {}

extern_class!(
    #[unsafe(super(NSObject))]
    struct JSContext;
);

extern_class!(
    #[unsafe(super(NSObject))]
    struct JSValue;
);

impl JSContext {
    extern_methods!(
        #[unsafe(method(new))]
        fn new() -> Retained<Self>;

        #[unsafe(method(evaluateScript:))]
        fn evaluate(&self, source: &NSString) -> Option<Retained<JSValue>>;
    );
}

impl JSValue {
    extern_methods!(
        #[unsafe(method(toString))]
        fn string(&self) -> Option<Retained<NSString>>;
    );
}

const ENGINE: &str = include_str!("../../../vendor/mathjax/svg.js");
const RECENT_FORMULAS: usize = 512;
const RECENT_SVG_BYTES: usize = 8 * 1024 * 1024;

struct CachedFormula {
    graphic: Option<Rc<Graphic>>,
    last_used: u64,
}

impl CachedFormula {
    fn svg_bytes(&self) -> usize {
        self.graphic.as_ref().map_or(0, |graphic| graphic.svg_bytes)
    }
}

#[derive(Default)]
struct Renderer {
    context: Option<Retained<JSContext>>,
    #[cfg(debug_assertions)]
    render_calls: usize,
    cache: HashMap<(String, bool), CachedFormula>,
    access: u64,
}

thread_local! {
    static RENDERER: RefCell<Renderer> = RefCell::default();
}

pub(super) struct Graphic {
    pub(super) image: Retained<NSImage>,
    pub(super) size: NSSize,
    baseline: f64,
    svg_bytes: usize,
}

impl Graphic {
    pub(super) fn render(formula: &Formula) -> Option<Rc<Self>> {
        if formula.value.len() > 8192 {
            return None;
        }
        RENDERER.with_borrow_mut(|renderer| renderer.get(formula))
    }
}

pub(super) struct MathRun {
    pub(super) range: NSRange,
    pub(super) graphic: Rc<Graphic>,
    pub(super) size: NSSize,
    pub(super) baseline: f64,
    pub(super) display: bool,
}

impl MathRun {
    pub(super) fn render(
        formula: &Formula,
        range: NSRange,
        available_width: f64,
        font_size: f64,
    ) -> Option<Self> {
        if available_width < 16. {
            return None;
        }
        let graphic = Graphic::render(formula)?;
        let scale = (font_size / 15.).min(available_width / graphic.size.width);
        Some(Self {
            range,
            size: NSSize::new(graphic.size.width * scale, graphic.size.height * scale),
            baseline: graphic.baseline * scale,
            graphic,
            display: formula.display,
        })
    }
}

#[cfg(debug_assertions)]
pub(super) fn render_count() -> usize {
    RENDERER.with_borrow(|renderer| renderer.render_calls)
}

pub(super) fn retain_formulas(formulas: &[Formula]) {
    RENDERER.with_borrow_mut(|renderer| renderer.retain(formulas));
}

#[cfg(debug_assertions)]
pub(super) fn assert_document_cache() {
    let formulas: Vec<_> = (0..180)
        .map(|index| Formula {
            range: 0..0,
            value: format!("x_{{{index}}} + 1"),
            display: false,
        })
        .collect();
    RENDERER.with_borrow_mut(|renderer| {
        renderer.retain(&formulas);
        let graphics: Vec<_> = formulas
            .iter()
            .map(|formula| renderer.get(formula).expect("valid synthetic formula"))
            .collect();
        for (formula, previous) in formulas.iter().zip(&graphics) {
            let current = renderer.get(formula).expect("cached formula");
            assert!(
                Rc::ptr_eq(previous, &current),
                "A document with more than 128 formulas must reuse every rendered formula"
            );
        }
        renderer.retain(&[]);
        assert!(Rc::ptr_eq(
            &graphics[0],
            &renderer.get(&formulas[0]).unwrap()
        ));
        let extra: Vec<_> = (0..RECENT_FORMULAS + 1)
            .map(|index| Formula {
                range: 0..0,
                value: format!("invalid-{index}"),
                display: false,
            })
            .collect();
        for formula in &extra {
            renderer.access += 1;
            renderer.cache.insert(
                (formula.value.clone(), formula.display),
                CachedFormula {
                    graphic: None,
                    last_used: renderer.access,
                },
            );
        }
        renderer.retain(&formulas[..1]);
        assert_eq!(renderer.cache.len(), RECENT_FORMULAS + 1);
        assert!(Rc::ptr_eq(
            &graphics[0],
            &renderer.get(&formulas[0]).unwrap()
        ));
        assert!(
            !renderer
                .cache
                .contains_key(&(formulas[1].value.clone(), false))
        );
    });
}

impl Renderer {
    fn retain(&mut self, formulas: &[Formula]) {
        // The active document is never subject to eviction. Keep a bounded
        // pool of recent previews to make switching back to a note cheap.
        let active: HashSet<_> = formulas
            .iter()
            .map(|formula| (formula.value.as_str(), formula.display))
            .collect();
        let mut inactive = Vec::new();
        let mut bytes = 0_usize;
        for (key, cached) in &self.cache {
            if !active.contains(&(key.0.as_str(), key.1)) {
                let cost = key.0.len() + cached.svg_bytes();
                bytes += cost;
                inactive.push((key, cached.last_used, cost));
            }
        }
        if inactive.len() <= RECENT_FORMULAS && bytes <= RECENT_SVG_BYTES {
            return;
        }
        inactive.sort_unstable_by_key(|&(_, last_used, _)| last_used);
        let mut count = inactive.len();
        let evicted: Vec<_> = inactive
            .into_iter()
            .take_while(|&(_, _, cost)| {
                if count <= RECENT_FORMULAS && bytes <= RECENT_SVG_BYTES {
                    return false;
                }
                count -= 1;
                bytes -= cost;
                true
            })
            .map(|(key, _, _)| key.clone())
            .collect();
        for key in evicted {
            self.cache.remove(&key);
        }
    }

    fn get(&mut self, formula: &Formula) -> Option<Rc<Graphic>> {
        let key = (formula.value.clone(), formula.display);
        self.access += 1;
        if let Some(cached) = self.cache.get_mut(&key) {
            cached.last_used = self.access;
            return cached.graphic.clone();
        }
        let graphic = self.render(formula).map(Rc::new);
        self.cache.insert(
            key,
            CachedFormula {
                graphic: graphic.clone(),
                last_used: self.access,
            },
        );
        graphic
    }

    fn render(&mut self, formula: &Formula) -> Option<Graphic> {
        let _timing = crate::performance::Sample::start("math_render");
        #[cfg(debug_assertions)]
        {
            self.render_calls += 1;
        }
        let context = self.context.get_or_insert_with(|| {
            let context = JSContext::new();
            context.evaluate(&NSString::from_str(ENGINE));
            context
        });
        let tex = serde_json::to_string(&formula.value).ok()?;
        let script = NSString::from_str(&format!("openBearMath({tex}, {})", formula.display));
        let output = context.evaluate(&script)?.string()?.to_string();
        let (width, height, baseline, svg): (f64, f64, f64, String) =
            serde_json::from_str(&output).ok()?;
        let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(svg.as_bytes()))?;
        Some(Graphic {
            image,
            size: NSSize::new(width, height),
            baseline,
            svg_bytes: svg.len(),
        })
    }
}

pub(super) fn apply_layout(
    storage: &NSMutableAttributedString,
    text: &str,
    offsets: &Offsets,
    formula: &Formula,
    run: &MathRun,
) {
    use objc2_app_kit::*;
    use objc2_foundation::NSMutableCopying;
    use std::ptr;

    let head = NSRange::new(run.range.location, 1);
    // SAFETY: The first character is the dollar delimiter in the current storage.
    unsafe {
        let source_font = storage
            .attribute_atIndex_effectiveRange(NSFontAttributeName, head.location, ptr::null_mut())
            .and_then(|font| font.downcast::<NSFont>().ok());
        if source_font.is_some_and(|font| font.pointSize() < 1.) {
            storage.addAttribute_value_range(
                NSFontAttributeName,
                &NSFont::systemFontOfSize(15.),
                head,
            );
        }
    }

    let paragraph_start = text[..formula.range.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let first_end = text[formula.range.start..]
        .find('\n')
        .map_or(text.len(), |len| formula.range.start + len);
    if formula.display || run.size.height > 22. {
        let range = offsets.range(paragraph_start, first_end);
        // SAFETY: All indices are UTF-16 offsets in the active text storage and the style is copied before editing.
        unsafe {
            let paragraph = storage
                .attribute_atIndex_effectiveRange(
                    NSParagraphStyleAttributeName,
                    run.range.location,
                    ptr::null_mut(),
                )
                .and_then(|style| style.downcast::<NSParagraphStyle>().ok())
                .map_or_else(NSMutableParagraphStyle::new, |style| style.mutableCopy());
            if formula.display {
                paragraph.setMinimumLineHeight(run.size.height + 26.);
                paragraph.setAlignment(NSTextAlignment::Center);
            } else {
                paragraph
                    .setMinimumLineHeight(paragraph.minimumLineHeight().max(run.size.height + 8.));
            }
            storage.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, range);
        }
    }
    if formula.display && first_end < formula.range.end {
        let compact = NSMutableParagraphStyle::new();
        compact.setMinimumLineHeight(1.);
        let range = offsets.range(first_end + 1, formula.range.end);
        // SAFETY: The compact style applies only to the hidden formula source lines.
        unsafe {
            storage.addAttribute_value_range(NSParagraphStyleAttributeName, &compact, range);
            storage.addAttribute_value_range(
                NSFontAttributeName,
                &NSFont::systemFontOfSize(0.01),
                range,
            );
        }
    }
}

use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::NSView;
use objc2_core_graphics::CGMutablePath;
use objc2_foundation::{NSIntersectionRect, NSObjectProtocol, NSPoint, NSPointInRect, NSRect};
use objc2_quartz_core::{CAShapeLayer, CATransaction, kCAFillRuleEvenOdd};
use std::{cell::RefCell, ptr};

#[derive(PartialEq)]
pub(super) struct Cutouts {
    bounds: NSRect,
    rects: Vec<NSRect>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RefCell<Option<Cutouts>>]
    pub(super) struct EditorViewport;

    impl EditorViewport {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
            if self.is_cutout_hit(point) {
                None
            } else {
                // SAFETY: AppKit supplies the point in the coordinate space expected by NSView.
                unsafe { msg_send![super(self), hitTest: point] }
            }
        }
    }

    // SAFETY: The viewport is an NSView subclass confined to the AppKit main thread.
    unsafe impl NSObjectProtocol for EditorViewport {}
);

impl EditorViewport {
    pub(super) fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: The allocated NSView subclass receives its Rust ivars before super init.
        let view: Retained<Self> =
            unsafe { msg_send![super(Self::alloc(mtm).set_ivars(RefCell::default())), init] };
        view.setWantsLayer(true);
        view
    }

    fn is_cutout_hit(&self, point: NSPoint) -> bool {
        self.ivars().borrow().as_ref().is_some_and(|cutouts| {
            // SAFETY: AppKit calls hitTest on the main thread with a point in the parent view.
            let parent = unsafe { self.superview() };
            let point = self.convertPoint_fromView(point, parent.as_deref());
            cutouts.rects.iter().any(|rect| NSPointInRect(point, *rect))
        })
    }

    pub(super) fn set_cutouts(&self, overlays: impl Iterator<Item = NSRect>) {
        let bounds = self.bounds();
        let rects: Vec<_> = overlays
            .map(|rect| NSIntersectionRect(bounds, rect))
            .filter(|rect| !rect.is_empty())
            .collect();
        let cutouts = (!rects.is_empty()).then_some(Cutouts { bounds, rects });
        let Some(layer) = self.layer() else {
            return;
        };
        if *self.ivars().borrow() == cutouts {
            return;
        }

        // GPUI paints below the native view. Reveal each disjoint overlay and route its clicks
        // to GPUI without moving the scroll view or touching its selection.
        let mask = cutouts.as_ref().map(|cutouts| {
            let path = CGMutablePath::new();
            // SAFETY: The path is newly allocated and a null transform denotes identity.
            unsafe {
                CGMutablePath::add_rect(Some(&path), ptr::null(), cutouts.bounds);
                for rect in &cutouts.rects {
                    CGMutablePath::add_rect(Some(&path), ptr::null(), *rect);
                }
            }
            let mask = CAShapeLayer::new();
            mask.setFrame(bounds);
            mask.setPath(Some(&path));
            // SAFETY: This is the exported Core Animation even-odd fill rule.
            mask.setFillRule(unsafe { kCAFillRuleEvenOdd });
            mask
        });
        self.ivars().replace(cutouts);
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        // SAFETY: The new mask has no superlayer and is used only by this view.
        unsafe { layer.setMask(mask.as_deref().map(AsRef::as_ref)) };
        CATransaction::commit();
    }
}

#[cfg(debug_assertions)]
impl EditorViewport {
    pub(super) fn assert_cutout_routes_clicks(&self, count: usize) {
        use objc2_core_graphics::CGPath;

        let cutouts = self.ivars().borrow();
        let cutouts = cutouts.as_ref().expect("the panel overlaps the editor");
        assert_eq!(cutouts.rects.len(), count);
        // SAFETY: The viewport is mounted in the smoke-test window on the main thread.
        let parent = unsafe { self.superview() }.unwrap();
        let mask = self
            .layer()
            .unwrap()
            .mask()
            .unwrap()
            .downcast::<CAShapeLayer>()
            .unwrap();
        let path = mask.path().unwrap();
        let outside = NSPoint::new(
            cutouts.bounds.origin.x + 16.,
            cutouts.bounds.origin.y + cutouts.bounds.size.height / 2.,
        );
        for (point, inside) in cutouts
            .rects
            .iter()
            .map(|rect| {
                (
                    NSPoint::new(
                        rect.origin.x + rect.size.width / 2.,
                        rect.origin.y + rect.size.height / 2.,
                    ),
                    true,
                )
            })
            .chain([(outside, false)])
        {
            assert_eq!(
                self.hitTest(self.convertPoint_toView(point, Some(&parent)))
                    .is_none(),
                inside
            );
            // SAFETY: The mask supplies the retained path and null denotes the identity transform.
            assert_eq!(
                unsafe { CGPath::contains_point(Some(&path), ptr::null(), point, true) },
                !inside
            );
        }
    }
}

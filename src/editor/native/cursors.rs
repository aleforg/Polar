use objc2::{
    AnyThread, ClassType, DefinedClass, MainThreadOnly, define_class, msg_send,
    rc::{Allocated, Retained},
    runtime::AnyObject,
    sel,
};
use objc2_app_kit::{
    NSColor, NSCursor, NSEvent, NSScrollView, NSSelectionAffinity, NSTextContainer, NSTextView,
    NSTrackingArea, NSTrackingAreaOptions, NSView,
};
use objc2_foundation::{NSPoint, NSPointInRect, NSRange, NSRect, NSString};

use std::cell::RefCell;

const CARET_WIDTH: f64 = 2.5;

#[derive(Clone, Copy)]
pub(super) enum Pointer {
    Arrow,
    Hand,
}

#[derive(Default)]
pub(super) struct CursorRects {
    inline: Vec<(NSRect, Pointer)>,
    overlays: Vec<NSRect>,
    caret: Option<NSRect>,
    caret_view: Option<Retained<CaretView>>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    struct CaretView;

    impl CaretView {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> Option<Retained<NSView>> {
            None
        }
    }
);

define_class!(
    #[unsafe(super(NSTextView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RefCell<CursorRects>]
    pub(super) struct CursorTextView;

    impl CursorTextView {
        #[unsafe(method_id(initWithFrame:textContainer:))]
        fn init(
            this: Allocated<Self>,
            frame: NSRect,
            container: Option<&NSTextContainer>,
        ) -> Retained<Self> {
            // SAFETY: Initialize the Rust ivars before invoking NSTextView's designated initializer.
            unsafe {
                msg_send![super(this.set_ivars(RefCell::default())), initWithFrame: frame, textContainer: container]
            }
        }

        #[unsafe(method(setSelectedRange:affinity:stillSelecting:))]
        fn select_range(&self, range: NSRange, affinity: NSSelectionAffinity, selecting: bool) {
            // SAFETY: Forward AppKit's selection unchanged before updating its presentation.
            unsafe {
                let _: () = msg_send![super(self), setSelectedRange: range, affinity: affinity, stillSelecting: selecting];
            }
            // AppKit defers selection notifications during mouse tracking. Reveal the
            // source now, including while the button remains pressed or a drag starts.
            if selecting && let Some(delegate) = self.delegate() {
                let object: &objc2::runtime::AnyObject = (*delegate).as_ref();
                if let Some(delegate) = object.downcast_ref::<super::MarkdownDelegate>() {
                    delegate.restyle_selection();
                }
            }
        }

        #[unsafe(method(drawInsertionPointInRect:color:turnedOn:))]
        fn update_insertion_point(&self, mut rect: NSRect, color: &NSColor, on: bool) {
            if !on {
                self.hide_insertion_point();
                return;
            }
            let caret_view = self.caret_view();
            self.ivars().borrow_mut().caret = Some(rect);
            let trim = (rect.size.height * 0.08).clamp(0., 3.);
            // TextKit centers its insertion rectangle above the visible letterforms.
            rect.origin.y += trim / 2. + 5.;
            rect.size.height -= trim;
            let scale = self.window().map_or(1., |window| window.backingScaleFactor());
            let left = rect.origin.x - (CARET_WIDTH - rect.size.width) / 2.;
            rect.origin.x = (left * scale).round() / scale;
            rect.size.width = CARET_WIDTH;
            caret_view.setFrame(rect);
            if let Some(layer) = caret_view.layer() {
                layer.setBackgroundColor(Some(&color.CGColor()));
            }
            caret_view.setHidden(false);
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty: NSRect) {
            // SAFETY: AppKit supplies the dirty rectangle for the current graphics context.
            unsafe { msg_send![super(self), drawRect: dirty] }
            // TextKit erases the blink by repainting its original narrow rectangle.
            if self.ivars().borrow().caret == Some(dirty) || !self.shouldDrawInsertionPoint() {
                self.hide_insertion_point();
            }
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let timing = crate::performance::Sample::start("key");
            if timing.is_some() {
                let queued = objc2_foundation::NSProcessInfo::processInfo().systemUptime() - event.timestamp();
                eprintln!("POLAR_PROFILE input_queue {:.3}", queued * 1000.);
            }
            if let Some(delegate) = self.delegate() {
                let object: &objc2::runtime::AnyObject = (*delegate).as_ref();
                if object.downcast_ref::<super::MarkdownDelegate>()
                    .is_some_and(|delegate| delegate.wiki_key_down(event)) {
                    return;
                }
            }
            // SAFETY: Forward the native keyboard event to NSTextView unchanged.
            unsafe { msg_send![super(self), keyDown: event] }
        }

        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, value: &AnyObject, range: NSRange) {
            // Some input sources send arrow keys as control characters rather than commands.
            if let Some(string) = value.downcast_ref::<NSString>()
                && string.length() == 1
                && let Some(command) = match string.characterAtIndex(0) {
                    0x001e | 0xf700 => Some(sel!(moveUp:)),
                    0x001f | 0xf701 => Some(sel!(moveDown:)),
                    _ => None,
                }
                && let Some(delegate) = self.delegate()
            {
                let object: &AnyObject = (*delegate).as_ref();
                if object.downcast_ref::<super::MarkdownDelegate>()
                    .is_some_and(|delegate| delegate.wiki_command(command))
                {
                    return;
                }
            }
            // SAFETY: Forward the original AppKit input object and replacement range unchanged.
            unsafe { msg_send![super(self), insertText: value, replacementRange: range] }
        }

        #[unsafe(method(cursorUpdate:))]
        fn cursor_update(&self, event: &NSEvent) {
            if !self.set_control_cursor(event) {
                // SAFETY: AppKit supplies the original cursor update event.
                unsafe { msg_send![super(self), cursorUpdate: event] }
            }
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) {
            // SAFETY: AppKit supplies the original mouse event.
            unsafe { msg_send![super(self), mouseEntered: event] }
            self.set_control_cursor(event);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            // SAFETY: AppKit supplies the original mouse event.
            unsafe { msg_send![super(self), mouseMoved: event] }
            self.set_control_cursor(event);
        }

    }
);

impl CursorTextView {
    pub(super) fn scrollable() -> Retained<NSScrollView> {
        // SAFETY: This NSTextView subclass uses the inherited AppKit factory and runs on the main thread.
        let scroll: Retained<NSScrollView> =
            unsafe { msg_send![Self::class(), scrollableTextView] };
        let text = scroll.documentView().unwrap().downcast::<Self>().unwrap();
        // Track the editor even where the GPUI overlay handles clicks.
        // SAFETY: The tracking owner is the view itself and accepts the mouse and cursor events.
        let tracking = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                NSRect::default(),
                NSTrackingAreaOptions::MouseMoved
                    | NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::CursorUpdate
                    | NSTrackingAreaOptions::ActiveInKeyWindow
                    | NSTrackingAreaOptions::InVisibleRect,
                Some(&text),
                None,
            )
        };
        text.addTrackingArea(&tracking);
        scroll
    }

    pub(super) fn from_text(text: &NSTextView) -> &Self {
        text.downcast_ref::<Self>().expect("Polar text view")
    }

    pub(super) fn set_inline(&self, rects: Vec<(NSRect, Pointer)>) {
        self.ivars().borrow_mut().inline = rects;
    }

    pub(super) fn set_overlays(&self, rects: Vec<NSRect>) {
        self.ivars().borrow_mut().overlays = rects;
    }

    fn caret_view(&self) -> Retained<CaretView> {
        if let Some(view) = &self.ivars().borrow().caret_view {
            return view.clone();
        }
        // SAFETY: AppKit initializes the allocated NSView subclass on the main thread.
        let view: Retained<CaretView> =
            unsafe { msg_send![super(CaretView::alloc(self.mtm()).set_ivars(())), init] };
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(CARET_WIDTH / 2.);
        }
        self.addSubview(&view);
        self.ivars().borrow_mut().caret_view = Some(view.clone());
        view
    }

    fn hide_insertion_point(&self) {
        let view = {
            let mut state = self.ivars().borrow_mut();
            if state.caret.take().is_none() {
                return;
            }
            state.caret_view.clone()
        };
        if let Some(view) = view {
            view.setHidden(true);
        }
    }

    fn set_control_cursor(&self, event: &NSEvent) -> bool {
        let window_point = event.locationInWindow();
        let point = self.convertPoint_fromView(window_point, None);
        let rects = self.ivars().borrow();
        let pointer = if rects
            .overlays
            .iter()
            .any(|&rect| NSPointInRect(window_point, rect))
        {
            Some(Pointer::Arrow)
        } else {
            rects
                .inline
                .iter()
                .find_map(|&(rect, pointer)| NSPointInRect(point, rect).then_some(pointer))
        };

        match pointer {
            Some(Pointer::Arrow) => NSCursor::arrowCursor().set(),
            Some(Pointer::Hand) => NSCursor::pointingHandCursor().set(),
            None => return false,
        }
        true
    }
}

use crate::palette::MUTED;
use gpui_kit::{
    Div, ElementId, InteractiveElement, MouseButton, ParentElement, Styled, base::Button,
    component::Icon, div, px, rgb,
};

pub(super) fn icon(name: &'static str, size: f32, color: u32) -> Icon {
    Icon::default()
        .path(format!("icons/{name}.svg"))
        .size(px(size))
        .text_color(rgb(color))
}

pub(super) fn tool(id: impl Into<ElementId>, label: &'static str, glyph: &'static str) -> Button {
    Button::new(id)
        .accessibility_label(label)
        .flex()
        .items_center()
        .justify_center()
        .h(px(28.))
        .w(px(30.))
        .rounded(px(6.))
        .child(icon(glyph, 17., MUTED))
}

pub(super) fn drag_area() -> Div {
    div()
        .h_full()
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count == 2 {
                window.zoom_window();
            } else {
                window.start_window_move();
            }
        })
}

pub(super) fn menu_button(label: &'static str) -> Button {
    Button::new(label)
        .accessibility_label(label)
        .w_full()
        .h(px(27.))
        .px(px(8.))
        .rounded(px(5.))
        .justify_start()
        .text_left()
        .text_size(px(13.))
        .child(label)
}

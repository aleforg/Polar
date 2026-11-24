use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, Pixels, StatefulInteractiveElement,
    Styled, Window, anchored,
    base::Button,
    component::{
        input::Input,
        menu::{ContextMenuExt, PopupMenuItem},
        tooltip::Tooltip,
    },
    deferred, div, point,
    prelude::FluentBuilder as _,
    px, rgb,
};

use super::{Notebook, widgets::icon};
use crate::{
    notes::tag_icons::TagIcon,
    palette::{ACCENT, EDITOR, MUTED, RULE, TEXT},
};

pub(super) struct TagPicker {
    tag: String,
    choice: Option<TagIcon>,
    row_y: Pixels,
    expanded: bool,
}

impl Notebook {
    pub(super) fn tag_context_menu(
        &self,
        row: Button,
        tag: &str,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let tag = tag.to_owned();
        let notebook = cx.weak_entity();
        row.context_menu(move |menu, window, _| {
            let row_y = window.mouse_position().y;
            let tag = tag.clone();
            let notebook = notebook.clone();
            menu.item(
                PopupMenuItem::new("Modifica tag")
                    .icon(icon("pencil", 14., MUTED))
                    .on_click(move |_, window, cx| {
                        let _ = notebook.update(cx, |this, cx| {
                            this.open_tag_picker(&tag, row_y, window, cx);
                        });
                    }),
            )
        })
    }

    fn open_tag_picker(
        &mut self,
        tag: &str,
        row_y: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.source.blur();
        self.workspace_focus.focus(window, cx);
        self.tag_icon_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.tag_picker = Some(TagPicker {
            tag: tag.to_owned(),
            choice: self.tag_icons.get(tag).copied(),
            row_y,
            expanded: false,
        });
        cx.notify();
    }

    fn save_tag_icon(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.tag_picker.take() else {
            return;
        };
        if self.tag_icons.get(&picker.tag).copied() != picker.choice {
            match picker.choice {
                Some(choice) => self.tag_icons.insert(picker.tag, choice),
                None => self.tag_icons.remove(&picker.tag),
            };
            self.source.set_tag_icons(&self.tag_icons);
            self.changed(cx);
            self.flush(cx);
        }
        cx.notify();
    }

    pub(super) fn tag_picker(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let picker = self.tag_picker.as_ref()?;
        let selected = picker
            .choice
            .unwrap_or_else(|| TagIcon::automatic(&picker.tag));
        let query = self.tag_icon_search.read(cx).value().trim().to_lowercase();
        let position = if picker.expanded {
            point(px(174.), picker.row_y - px(182.))
        } else {
            point(px(12.), picker.row_y + px(16.))
        };
        let panel = div()
            .id("edit-tag-panel")
            .occlude()
            .w(px(314.))
            .px(px(15.))
            .py(px(12.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .rounded(px(15.))
            .border_1()
            .border_color(rgb(RULE))
            .bg(rgb(EDITOR))
            .shadow_lg()
            .text_size(px(13.))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.tag_picker = None;
                cx.notify();
            }))
            .child(
                div()
                    .text_center()
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .child("Modifica tag"),
            )
            .child(
                div()
                    .h(px(29.))
                    .px(px(8.))
                    .line_height(px(29.))
                    .rounded(px(5.))
                    .bg(rgb(0x2c2d2e))
                    .truncate()
                    .child(picker.tag.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        Button::new("tag-icon-swatch")
                            .accessibility_label("Scegli icona del tag")
                            .tooltip(|window, cx| {
                                Tooltip::new("Scegli icona del tag").build(window, cx)
                            })
                            .h(px(25.))
                            .px(px(8.))
                            .gap(px(4.))
                            .rounded(px(5.))
                            .bg(rgb(0x424344))
                            .child(icon(selected.glyph(), 16., TEXT))
                            .child(icon(
                                if picker.expanded {
                                    "chevron-up"
                                } else {
                                    "chevron-down"
                                },
                                11.,
                                TEXT,
                            ))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(picker) = &mut this.tag_picker {
                                    picker.expanded = !picker.expanded;
                                    cx.notify();
                                }
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("cancel-tag-icon")
                            .h(px(25.))
                            .px(px(12.))
                            .rounded(px(5.))
                            .bg(rgb(0x424344))
                            .child("Annulla")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.tag_picker = None;
                                this.workspace_focus.focus(window, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("save-tag-icon")
                            .h(px(25.))
                            .px(px(13.))
                            .rounded(px(5.))
                            .bg(rgb(ACCENT))
                            .text_color(rgb(0xffffff))
                            .child("Fatto")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.workspace_focus.focus(window, cx);
                                this.save_tag_icon(cx);
                            })),
                    ),
            )
            .when(picker.expanded, |panel| {
                let choices = std::iter::once((None, "sparkles", "Automatica"))
                    .filter(|_| query.is_empty() || "automatica".contains(&query))
                    .chain(
                        TagIcon::ALL
                            .iter()
                            .copied()
                            .filter(|choice| choice.matches(&query))
                            .map(|choice| (Some(choice), choice.glyph(), choice.label())),
                    );
                let mut tiles = div()
                    .id("tag-icon-results")
                    .h(px(238.))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .id("tag-icon-grid")
                            .flex()
                            .flex_wrap()
                            .gap(px(8.))
                            .children(choices.map(|(choice, glyph, label)| {
                                let active = picker.choice == choice;
                                Button::new(label)
                                    .accessibility_label(label)
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(label).build(window, cx)
                                    })
                                    .size(px(38.))
                                    .rounded_full()
                                    .bg(rgb(if active { ACCENT } else { EDITOR }))
                                    .hover(|button| {
                                        button.bg(rgb(if active { ACCENT } else { 0x343537 }))
                                    })
                                    .child(icon(glyph, 21., if active { EDITOR } else { MUTED }))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(picker) = &mut this.tag_picker {
                                            picker.choice = choice;
                                            cx.notify();
                                        }
                                    }))
                            })),
                    );
                if !query.is_empty()
                    && !TagIcon::ALL.iter().any(|choice| choice.matches(&query))
                    && !"automatica".contains(&query)
                {
                    tiles = tiles.child(
                        div()
                            .mt(px(16.))
                            .text_center()
                            .text_color(rgb(MUTED))
                            .child("Nessuna icona"),
                    );
                }
                panel.child(tiles).child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .h(px(32.))
                        .px(px(9.))
                        .rounded_full()
                        .border_1()
                        .border_color(rgb(RULE))
                        .child(icon("search", 15., MUTED))
                        .child(
                            div().flex_1().child(
                                Input::new(&self.tag_icon_search)
                                    .appearance(false)
                                    .bordered(false)
                                    .focus_bordered(false)
                                    .aria_label("Ricerca icone")
                                    .h(px(28.))
                                    .p_0()
                                    .text_size(px(13.)),
                            ),
                        ),
                )
            });
        Some(
            deferred(
                anchored()
                    .position(position)
                    .snap_to_window_with_margin(px(8.))
                    .child(panel),
            )
            .with_priority(3),
        )
    }
}

use super::{
    Notebook, Panel,
    navigation::{NOTE_LIST_WIDTH, SIDEBAR_WIDTH},
    widgets::icon,
};
use crate::{blocks::CalloutKind, editor::markdown::HIGHLIGHTS, palette::SUBTLE};
use gpui_kit::{
    Context, Div, ParentElement, Pixels, Styled, Window, base::Button, div,
    prelude::FluentBuilder as _, px, relative, rgb,
};

const BAR_HEIGHT: f32 = 30.;
const BAR_ICON: u32 = 0xcbd2d9;
const ACTIVE: u32 = 0x41464d;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FormatGroup {
    Heading,
    List,
    Highlight,
    More,
    Todo,
    Callout,
}

impl Notebook {
    pub(super) fn format_bar(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let group = match self.panel {
            Panel::FormatOptions(group) => Some(group),
            _ => None,
        };
        let group_button = |group, label, glyph, chevron, cx: &mut Context<Self>| {
            bar_button(label, glyph, chevron)
                .selected(
                    self.panel == Panel::FormatOptions(group)
                        || (group == FormatGroup::List
                            && matches!(
                                self.panel,
                                Panel::FormatOptions(FormatGroup::Todo | FormatGroup::Callout)
                            )),
                )
                .styles(|styles| styles.selected(|style| style.bg(rgb(ACTIVE))))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.panel == Panel::FormatOptions(group) {
                        this.panel = Panel::Format;
                        cx.notify();
                    } else {
                        this.toggle_panel(Panel::FormatOptions(group), cx);
                    }
                }))
        };

        let bar = div()
            .flex()
            .items_center()
            .w_full()
            .h(px(BAR_HEIGHT))
            .px(px(4.))
            .rounded(px(8.))
            .bg(rgb(0x23262b))
            .child(group_button(
                FormatGroup::Heading,
                "Titoli",
                "heading",
                true,
                cx,
            ))
            .child(
                bar_button("Elenco attività", "square-check", false).on_click(
                    cx.listener(|this, _, window, cx| this.format("- [ ] ", "", window, cx)),
                ),
            )
            .child(group_button(
                FormatGroup::List,
                "Liste e blocchi",
                "list",
                true,
                cx,
            ))
            .child(div().w(px(18.)).min_w_0())
            .child(
                bar_button("Grassetto", "bold", false).on_click(
                    cx.listener(|this, _, window, cx| this.format("**", "**", window, cx)),
                ),
            )
            .child(
                bar_button("Corsivo", "italic", false)
                    .on_click(cx.listener(|this, _, window, cx| this.format("*", "*", window, cx))),
            )
            .child(group_button(
                FormatGroup::Highlight,
                "Evidenziatore",
                "highlighter",
                true,
                cx,
            ))
            .child(div().w(px(15.)).min_w_0())
            .child(
                bar_button("Link", "link", false)
                    .on_click(cx.listener(|this, _, window, cx| this.insert_link(window, cx))),
            )
            .child(
                bar_button("Tabella", "table", false)
                    .on_click(cx.listener(|this, _, window, cx| this.insert_table(window, cx))),
            )
            .child(
                bar_button("Immagine…", "images", false)
                    .on_click(cx.listener(|this, _, window, cx| this.choose_image(window, cx))),
            )
            .child(
                group_button(
                    FormatGroup::More,
                    "Altri stili",
                    "ellipsis-vertical",
                    false,
                    cx,
                )
                .w(px(26.)),
            );

        let editor_width = window.viewport_size().width
            - px(if self.panes == 3 { SIDEBAR_WIDTH } else { 0. })
            - px(if self.panes >= 2 { NOTE_LIST_WIDTH } else { 0. });
        let bar_width = px(386.).min(editor_width * 0.96);
        let right_edge = (editor_width + bar_width) / 2. - px(8.);
        div()
            .absolute()
            .bottom(px(19.))
            .left_0()
            .right_0()
            .mx_auto()
            .w(bar_width)
            .child(bar)
            .children(group.map(|group| self.format_menu(group, bar_width, right_edge, cx)))
    }

    fn format_menu(
        &self,
        group: FormatGroup,
        bar_width: Pixels,
        right_edge: Pixels,
        cx: &mut Context<Self>,
    ) -> Div {
        let menu = div()
            .absolute()
            .bottom(px(BAR_HEIGHT + 3.))
            .p(px(5.))
            .rounded(px(11.))
            .border_1()
            .border_color(rgb(0x47494b))
            .bg(rgb(0x363738))
            .shadow_lg()
            .text_color(rgb(0xe2e3e4));

        match group {
            FormatGroup::Heading => {
                let mut menu = menu.left_0().w(px(142.));
                for (label, glyph, prefix) in [
                    ("Titolo 1", "heading-1", "# "),
                    ("Titolo 2", "heading-2", "## "),
                    ("Titolo 3", "heading-3", "### "),
                    ("Titolo 4", "heading-4", "#### "),
                    ("Titolo 5", "heading-5", "##### "),
                    ("Titolo 6", "heading-6", "###### "),
                ] {
                    menu = menu.child(
                        style_item(label, Some(icon(glyph, 12., BAR_ICON)), "").on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.format(prefix, "", window, cx)
                            }),
                        ),
                    );
                }
                menu
            }
            FormatGroup::List => {
                let mut menu = menu.left(relative(0.18)).w(px(230.));
                for (label, glyph, prefix) in [
                    ("Lista", "list", "- "),
                    ("Lista ordinata", "list-ordered", "1. "),
                    ("Citazione in blocco", "quote", "> "),
                ] {
                    menu = menu.child(
                        style_item(label, Some(icon(glyph, 13., BAR_ICON)), "").on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.format(prefix, "", window, cx)
                            }),
                        ),
                    );
                }
                for (label, glyph, group) in [
                    ("ToDo", "square-check", FormatGroup::Todo),
                    ("Riquadro", "square", FormatGroup::Callout),
                ] {
                    menu = menu.child(
                        style_item(label, Some(icon(glyph, 13., BAR_ICON)), "›").on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.toggle_panel(Panel::FormatOptions(group), cx)
                            }),
                        ),
                    );
                }
                menu.child(separator()).child(
                    style_item("Separatore", Some(icon("minus", 13., BAR_ICON)), "").on_click(
                        cx.listener(|this, _, window, cx| this.insert_block("---", window, cx)),
                    ),
                )
            }
            FormatGroup::Todo => menu
                .left(relative(0.18))
                .w(px(174.))
                .child(back_to_lists(cx))
                .child(
                    style_item("Da fare", Some(icon("square", 13., BAR_ICON)), "").on_click(
                        cx.listener(|this, _, window, cx| this.format("- [ ] ", "", window, cx)),
                    ),
                )
                .child(
                    style_item("Completato", Some(icon("square-check", 13., BAR_ICON)), "")
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.format("- [x] ", "", window, cx)
                            }),
                        ),
                ),
            FormatGroup::Callout => {
                let mut menu = menu
                    .left(relative(0.18))
                    .w(px(195.))
                    .child(back_to_lists(cx));
                for kind in CalloutKind::ALL {
                    menu = menu.child(
                        style_item(kind.label(), Some(icon("quote", 13., BAR_ICON)), "").on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.insert_callout(kind, window, cx)
                            }),
                        ),
                    );
                }
                menu
            }
            FormatGroup::Highlight => {
                let mut menu = menu.left(relative(0.51)).w(px(117.));
                for (index, (label, prefix, color)) in HIGHLIGHTS.into_iter().enumerate() {
                    if index == 1 {
                        menu = menu.child(separator());
                    }
                    menu = menu.child(
                        style_item(label, Some(icon("circle", 10., color)), "").on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.format(prefix, "==", window, cx)
                            }),
                        ),
                    );
                }
                menu
            }
            FormatGroup::More => {
                let width = px(228.);
                let left = (bar_width - px(27.)).min(right_edge - width);
                let mut menu = menu.left(left).w(width);
                for (label, key, prefix, suffix) in [
                    ("Sottolineato", "⌘U", "<u>", "</u>"),
                    ("Barrato", "", "~~", "~~"),
                    ("Codice", "", "`", "`"),
                    ("Matematica", "", "$", "$"),
                    ("Blocco di matematica", "", "\n\n$$\n", "\n$$\n\n"),
                    ("Link Wiki", "", "[[", "]]"),
                ] {
                    menu = menu.child(style_item(label, None, key).on_click(cx.listener(
                        move |this, _, window, cx| this.format(prefix, suffix, window, cx),
                    )));
                    if label == "Codice" {
                        menu = menu.child(style_item("Blocco di codice", None, "").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.insert_block("```\n\n```", window, cx)
                            }),
                        ));
                    }
                }
                menu.child(separator()).child(
                    style_item("Nascondi barra di stile", None, "").on_click(
                        cx.listener(|this, _, _, cx| this.toggle_panel(Panel::Format, cx)),
                    ),
                )
            }
        }
    }
}

fn bar_button(label: &'static str, glyph: &'static str, chevron: bool) -> Button {
    Button::new(label)
        .accessibility_label(label)
        .min_w_0()
        .w(px(if chevron { 39. } else { 34. }))
        .h(px(24.))
        .rounded(px(4.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(1.))
                .child(icon(glyph, 15., BAR_ICON))
                .when(chevron, |item| {
                    item.child(icon("chevron-down", 8., BAR_ICON))
                }),
        )
}

fn style_item(
    label: &'static str,
    symbol: Option<gpui_kit::component::Icon>,
    shortcut: &'static str,
) -> Button {
    Button::new(label)
        .accessibility_label(label)
        .w_full()
        .h(px(24.))
        .px(px(9.))
        .rounded(px(5.))
        .justify_start()
        .child(
            div()
                .flex()
                .w_full()
                .items_center()
                .gap(px(8.))
                .text_size(px(13.))
                .children(symbol)
                .child(div().flex_1().child(label))
                .child(div().text_color(rgb(SUBTLE)).child(shortcut)),
        )
}

fn separator() -> Div {
    div().mx(px(9.)).my(px(3.)).h(px(1.)).bg(rgb(0x505153))
}

fn back_to_lists(cx: &mut Context<Notebook>) -> Button {
    style_item(
        "Liste e blocchi",
        Some(icon("chevron-left", 13., BAR_ICON)),
        "",
    )
    .on_click(
        cx.listener(|this, _, _, cx| {
            this.toggle_panel(Panel::FormatOptions(FormatGroup::List), cx)
        }),
    )
}

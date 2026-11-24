use std::{cell::Cell, rc::Rc};

use polar_note_revision::Action;

use super::{
    Notebook, Panel,
    revision::Pane,
    widgets::{drag_area, icon, menu_button, tool},
};
use crate::palette::{ACCENT, EDITOR, MUTED, RULE, SIDEBAR, SUBTLE, TEXT, TITLE};
use gpui_kit::{
    Context, Div, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder as _, px, rgb,
};
use gpui_kit::{FontWeight, base::Button};

impl Notebook {
    pub(super) fn editor(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mode_toggle = self
            .selected()
            .is_some_and(|note| note.can_edit() && !self.editing)
            .then(|| {
                tool("edit-mode", "Rendi modificabile", "pencil-off")
                    .on_click(cx.listener(|this, _, window, cx| this.start_editing(window, cx)))
            });
        let toolbar = div()
            .h(px(52.))
            .flex()
            .items_center()
            .justify_between()
            .px(px(8.))
            .child(drag_area().flex_1().when(self.panes == 1, |left| {
                left.ml(px(70.))
                    .flex()
                    .gap(px(6.))
                    .child(
                        tool(
                            "show-panes",
                            "Mostra tag, note ed editor",
                            "panel-left-open",
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.panes = 3;
                            cx.notify();
                        })),
                    )
                    .child(
                        tool("new-in-focus", "Nuova nota", "square-pen")
                            .on_click(cx.listener(|this, _, window, cx| this.new_note(window, cx))),
                    )
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(35.))
                    .rounded_full()
                    .bg(rgb(0x38393a))
                    .border_1()
                    .border_color(rgb(0x494a4b))
                    .px(px(5.))
                    .gap(px(6.))
                    .children(mode_toggle)
                    .when(
                        self.selected()
                            .is_some_and(|note| note.read_only && !note.archived && !note.trashed),
                        |tools| {
                            tools.child(
                                tool("unlock-note", "Rendi modificabile", "pencil-off").on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.toggle_read_only(window, cx)
                                    }),
                                ),
                            )
                        },
                    )
                    .when(
                        self.selected().is_some_and(|note| note.can_edit()),
                        |tools| {
                            tools.child(
                                Button::new("format")
                                    .accessibility_label("Formattazione")
                                    .h(px(28.))
                                    .px(px(10.))
                                    .rounded_full()
                                    .text_size(px(14.))
                                    .text_color(rgb(MUTED))
                                    .selected(self.panel.is_format())
                                    .styles(|styles| {
                                        styles.selected(|style| style.bg(rgb(0x4e4f50)))
                                    })
                                    .child(
                                        div()
                                            .flex()
                                            .gap(px(1.))
                                            .child(div().font_weight(FontWeight::BOLD).child("B"))
                                            .child(div().italic().child("I"))
                                            .child(div().underline().child("U")),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_panel(Panel::Format, cx)
                                    })),
                            )
                        },
                    )
                    .child(
                        tool("statistics", "Statistiche", "info").on_click(
                            cx.listener(|this, _, _, cx| this.toggle_panel(Panel::Info, cx)),
                        ),
                    )
                    .child(
                        tool("note-actions", "Azioni della nota", "ellipsis-vertical").on_click(
                            cx.listener(|this, _, _, cx| this.toggle_panel(Panel::Actions, cx)),
                        ),
                    ),
            );
        let overlays = Rc::new(Cell::new(Vec::new()));
        let mut area = div()
            .relative()
            .min_w_0()
            .flex_1()
            .flex()
            .flex_col()
            .bg(rgb(EDITOR))
            .child(toolbar)
            .children(self.revision_panel(window, cx));
        if let Some(error) = &self.startup_error {
            area = area.child(div().p(px(24.)).text_color(rgb(MUTED)).child(error.clone()));
        } else if self.selected().is_some() {
            area = area.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .min_w_0()
                    .justify_center()
                    .px(px(44.))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(650.))
                            .h_full()
                            .child(self.source.element(overlays.clone())),
                    ),
            );
        } else {
            area = area.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(SUBTLE))
                    .child("Nessuna nota selezionata"),
            );
        }
        if let Some(status) = &self.status {
            area = area.child(
                div()
                    .px(px(18.))
                    .py(px(8.))
                    .text_color(rgb(MUTED))
                    .bg(rgb(SIDEBAR))
                    .child(status.clone()),
            );
        }
        let overlay = match self.panel {
            Panel::Format | Panel::FormatOptions(_) => Some(self.format_bar(window, cx)),
            Panel::Info | Panel::Actions => Some(self.popover(cx)),
            Panel::None => None,
        };
        area.children(overlay.map(|overlay| {
            overlay.on_children_prepainted(move |bounds, _, _| overlays.set(bounds))
        }))
    }

    fn revision_panel(&self, window: &Window, cx: &mut Context<Self>) -> Option<Div> {
        let mut card = div()
            .id("revision")
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(650.))
            .rounded(px(15.))
            .border_1()
            .border_color(rgb(0x494a4b))
            .bg(rgb(SIDEBAR))
            .overflow_hidden()
            .text_size(px(13.));
        let (glyph, title, detail, dismiss) = match &self.revision.pane {
            Pane::Hidden => return None,
            Pane::Running(target) => (
                "sparkles",
                target.title(),
                Some("Elaborazione sul Mac…"),
                "Annulla",
            ),
            Pane::Message(message) => ("info", *message, None, "Chiudi"),
            Pane::Preview { target, .. } => {
                ("sparkles", target.title(), Some(target.scope()), "Annulla")
            }
        };
        let dismiss_button = Button::new("dismiss-revision")
            .accessibility_label(dismiss)
            .track_focus(&self.revision.dismiss_focus)
            .h(px(28.))
            .px(px(11.))
            .rounded_full()
            .text_color(rgb(MUTED))
            .child(dismiss)
            .on_click(cx.listener(|this, _, window, cx| {
                this.dismiss_revision(window, cx);
            }));
        let header = div()
            .flex()
            .items_center()
            .gap(px(9.))
            .px(px(14.))
            .py(px(10.))
            .child(icon(
                glyph,
                16.,
                if detail.is_some() { ACCENT } else { MUTED },
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(rgb(TITLE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title)
                    .children(detail.map(|detail| {
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .font_weight(FontWeight::NORMAL)
                            .child(detail)
                    })),
            );
        if let Pane::Preview {
            target,
            replacement,
            notices,
            formatting,
        } = &self.revision.pane
        {
            let show_original = self.revision.show_original;
            let version = |label: &'static str, original| {
                Button::new(label)
                    .accessibility_label(label)
                    .selected(show_original == original)
                    .h(px(27.))
                    .px(px(10.))
                    .rounded_full()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .styles(|styles| {
                        styles.selected(|style| style.bg(rgb(0x494a4b)).text_color(rgb(TEXT)))
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.revision.show_original = original;
                        cx.notify();
                    }))
            };
            card = card
                .child(
                    header.child(
                        div()
                            .flex()
                            .rounded_full()
                            .bg(rgb(EDITOR))
                            .p(px(2.))
                            .child(version("Proposta", false))
                            .child(version("Originale", true)),
                    ),
                )
                .child(
                    div()
                        .id(if show_original {
                            "revision-original"
                        } else {
                            "revision-proposal"
                        })
                        .max_h((window.viewport_size().height * 0.30).clamp(px(70.), px(220.)))
                        .overflow_y_scroll()
                        .border_t_1()
                        .border_color(rgb(RULE))
                        .bg(rgb(EDITOR))
                        .px(px(16.))
                        .py(px(12.))
                        .text_size(px(14.))
                        .line_height(px(21.))
                        .text_color(rgb(TEXT))
                        .child(if show_original {
                            target.original().to_owned()
                        } else {
                            replacement.clone()
                        }),
                );
            if !notices.is_empty() || *formatting {
                card = card.child(
                    div()
                        .id("revision-notices")
                        .max_h(px(68.))
                        .overflow_y_scroll()
                        .border_t_1()
                        .border_color(rgb(RULE))
                        .px(px(16.))
                        .py(px(8.))
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .when(*formatting, |notes| {
                            notes.child("Formattazione: chiusura del grassetto.")
                        })
                        .children(notices.iter().map(|notice| {
                            div().child(format!("{} «{}»", notice.message, notice.source))
                        })),
                );
            }
            card = card.child(
                div()
                    .flex()
                    .justify_end()
                    .items_center()
                    .gap(px(5.))
                    .border_t_1()
                    .border_color(rgb(RULE))
                    .px(px(12.))
                    .py(px(8.))
                    .child(dismiss_button)
                    .child(
                        Button::new("apply-revision")
                            .accessibility_label("Applica revisione")
                            .h(px(28.))
                            .px(px(13.))
                            .rounded_full()
                            .bg(rgb(ACCENT))
                            .text_color(rgb(EDITOR))
                            .font_weight(FontWeight::SEMIBOLD)
                            .hover(|style| style.bg(rgb(0x72b3e4)))
                            .child("Applica")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.apply_revision(window, cx)),
                            ),
                    ),
            );
        } else {
            card = card.child(header.child(dismiss_button));
        }
        Some(
            div()
                .flex()
                .justify_center()
                .px(px(12.))
                .pb(px(12.))
                .child(card),
        )
    }

    fn popover(&self, cx: &mut Context<Self>) -> Div {
        let mut panel = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(6.))
            .rounded(px(if self.panel == Panel::Info { 19. } else { 9. }))
            .shadow_lg()
            .bg(rgb(if self.panel == Panel::Info {
                EDITOR
            } else {
                0x303133
            }))
            .border_1()
            .border_color(rgb(0x505153));
        let note = self.selected();
        match self.panel {
            Panel::Info => {
                panel = panel.child(self.info_panel(cx));
            }
            Panel::Actions => {
                if let Some(note) = note {
                    if self.editing && note.can_edit() {
                        panel = panel.child(
                            menu_button("Correggi selezione")
                                .disabled(self.revision.busy())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.start_revision(Action::Correct, window, cx)
                                })),
                        );
                    }
                    if note.can_edit() {
                        panel = panel.child(
                            menu_button("Organizza appunti")
                                .disabled(self.revision.busy())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.start_revision(Action::Organize, window, cx)
                                })),
                        );
                    }
                    panel =
                        panel
                            .child(
                                menu_button(if self.editing {
                                    "Torna alla sola lettura"
                                } else {
                                    "Modifica Markdown"
                                })
                                .disabled(!note.can_edit())
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.panel = Panel::None;
                                        if this.editing {
                                            this.stop_editing(window, cx);
                                        } else {
                                            this.start_editing(window, cx);
                                        }
                                    },
                                )),
                            )
                            .child(
                                menu_button(if note.pinned {
                                    "Rimuovi da Fissata"
                                } else {
                                    "Fissa in alto"
                                })
                                .disabled(note.trashed)
                                .on_click(cx.listener(|this, _, window, cx| this.pin(window, cx))),
                            )
                            .child(menu_button("Duplica nota").disabled(note.trashed).on_click(
                                cx.listener(|this, _, window, cx| this.duplicate_note(window, cx)),
                            ))
                            .child(
                                menu_button(if note.archived {
                                    "Rimuovi dall’archivio"
                                } else {
                                    "Archivia"
                                })
                                .disabled(note.trashed)
                                .on_click(cx.listener(
                                    |this, _, window, cx| this.toggle_archive(window, cx),
                                )),
                            )
                            .child(
                                menu_button("Esporta nota…")
                                    .on_click(cx.listener(|this, _, _, cx| this.export(cx))),
                            )
                            .child(
                                menu_button(if note.trashed {
                                    "Ripristina"
                                } else {
                                    "Sposta nel Cestino"
                                })
                                .on_click(cx.listener(
                                    |this, _, window, cx| this.trash_or_restore(window, cx),
                                )),
                            );
                }
                for (panes, label) in [
                    (1, "Mostra solo l’editor"),
                    (2, "Mostra note ed editor"),
                    (3, "Mostra tag, note ed editor"),
                ] {
                    panel = panel.child(menu_button(label).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.panes = panes;
                            this.panel = Panel::None;
                            cx.notify();
                        },
                    )));
                }
            }
            _ => {}
        }
        div()
            .absolute()
            .top(px(if self.panel == Panel::Info { 53. } else { 47. }))
            .right(px(if self.panel == Panel::Info { 14. } else { 10. }))
            .w(px(if self.panel == Panel::Info {
                300.
            } else {
                225.
            }))
            .child(panel)
    }
}

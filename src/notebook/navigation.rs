use super::{
    Notebook, Panel,
    widgets::{drag_area, icon, menu_button, tool},
};
use crate::{
    notes::{self, Filter, tag_icons},
    palette::{ACCENT, LIST, MUTED, RULE, SELECTED, SIDEBAR, SUBTLE, TEXT, TITLE},
};
use chrono::{DateTime, Local};
use gpui_kit::{
    ClickEvent, Context, Focusable, InteractiveElement, IntoElement, ObjectFit, ParentElement,
    StatefulInteractiveElement, Styled, StyledImage, Window, div, img, prelude::FluentBuilder as _,
    px, rgb,
};
use gpui_kit::{
    FontWeight, SharedString,
    base::Button,
    component::{input::Input, popover::Popover},
};
use std::collections::BTreeSet;

pub(super) const SIDEBAR_WIDTH: f32 = 188.;
pub(super) const NOTE_LIST_WIDTH: f32 = 314.;

impl Notebook {
    pub(super) fn step_note(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.panes < 2
            || self.panel != Panel::None
            || self.tag_picker.is_some()
            || self.source.is_focused()
            || (self.search_open && self.search.read(cx).focus_handle(cx).is_focused(window))
        {
            cx.propagate();
            return;
        }
        self.sync_editor(cx);
        let results = notes::visible(&self.notes, &self.filter, &self.query);
        let index = match results
            .iter()
            .position(|note| Some(note.id) == self.selected)
        {
            Some(current) => current.checked_add_signed(offset),
            None if offset < 0 => results.len().checked_sub(1),
            None => Some(0),
        };
        let Some((index, id)) =
            index.and_then(|index| results.get(index).map(|note| (index, note.id)))
        else {
            return;
        };
        self.select_note(Some(id));
        self.workspace_focus.focus(window, cx);
        self.note_list_scroll.scroll_to_item(index);
        cx.notify();
    }

    pub(super) fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = crate::performance::Sample::start("sidebar");
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(52.))
            .px(px(12.))
            .child(drag_area().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon("cloud-off", 18., MUTED))
                    .child(self.layout_menu(cx)),
            );
        let filters = [
            (Filter::Untagged, "Senza Tag", "inbox"),
            (Filter::Todo, "ToDo", "square-check"),
            (Filter::Today, "Oggi", "calendar-days"),
            (Filter::Pinned, "Fissata", "pin"),
        ];
        let mut rows = div()
            .id("sidebar-rows")
            .flex()
            .flex_col()
            .px(px(8.))
            .pt(px(11.))
            .overflow_y_scroll()
            .child(
                div()
                    .relative()
                    .child(self.filter_row(Filter::Notes, "Note".into(), "file-text", 30., cx))
                    .child(
                        tool(
                            "toggle-note-group",
                            "Espandi o comprimi Note",
                            if self.expanded_notes {
                                "chevron-down"
                            } else {
                                "chevron-right"
                            },
                        )
                        .absolute()
                        .left(px(1.))
                        .top_0()
                        .w(px(28.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.expanded_notes = !this.expanded_notes;
                            cx.notify();
                        })),
                    )
                    .child(div().absolute().right(px(8.)).top(px(6.)).child(icon(
                        "chevrons-up-down",
                        15.,
                        MUTED,
                    ))),
            );
        if self.expanded_notes {
            rows = rows.children(filters.into_iter().map(|(filter, label, glyph)| {
                self.filter_row(filter, label.into(), glyph, 42., cx)
            }));
        }
        if self.notes.iter().any(|note| note.archived && !note.trashed)
            || self.filter == Filter::Archive
        {
            rows =
                rows.child(self.filter_row(Filter::Archive, "Archivio".into(), "archive", 30., cx));
        }
        rows = rows.child(self.filter_row(Filter::Trash, "Cestino".into(), "trash", 30., cx));
        let tags = notes::all_tags(self.notes.iter().filter(|note| {
            !self.indexing_tags || self.selected == Some(note.id) || note.tags_cached()
        }));
        let entries = tags
            .iter()
            .flat_map(|tag| {
                tag.match_indices('/')
                    .map(|(offset, _)| &tag[..offset])
                    .chain(std::iter::once(tag.as_str()))
            })
            .collect::<BTreeSet<_>>();
        rows = rows.child(div().h(px(14.)));
        for tag in &entries {
            let visible = tag
                .match_indices('/')
                .all(|(offset, _)| self.expanded_tags.contains(&tag[..offset]));
            if !visible {
                continue;
            }
            let indent = tag.bytes().filter(|byte| *byte == b'/').count() as f32 * 15.;
            let label = tag.rsplit('/').next().unwrap_or(tag);
            let button = self.filter_row(
                Filter::Tag((*tag).to_owned()),
                label.into(),
                tag_icons::resolve(&self.tag_icons, tag).glyph(),
                30. + indent,
                cx,
            );
            let mut row = div()
                .relative()
                .child(self.tag_context_menu(button, tag, cx));
            if entries.iter().any(|child| {
                child
                    .strip_prefix(tag)
                    .is_some_and(|suffix| suffix.starts_with('/'))
            }) {
                let tag = (*tag).to_owned();
                let glyph = if self.expanded_tags.contains(&tag) {
                    "chevron-down"
                } else {
                    "chevron-right"
                };
                row = row.child(
                    tool(
                        SharedString::from(format!("toggle-tag-{tag}")),
                        "Espandi o comprimi tag",
                        glyph,
                    )
                    .absolute()
                    .left(px(indent + 1.))
                    .top_0()
                    .w(px(28.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded_tags.remove(&tag) {
                            this.expanded_tags.insert(tag.clone());
                        }
                        cx.notify();
                    })),
                );
            }
            rows = rows.child(row);
        }
        div()
            .flex()
            .flex_col()
            .w(px(SIDEBAR_WIDTH))
            .flex_shrink_0()
            .bg(rgb(SIDEBAR))
            .child(header)
            .child(rows)
    }

    fn filter_row(
        &self,
        filter: Filter,
        label: String,
        glyph: &'static str,
        indent: f32,
        cx: &mut Context<Self>,
    ) -> Button {
        let active = self.filter == filter;
        let color = if active { TEXT } else { MUTED };
        Button::new(SharedString::from(format!("filter-{filter:?}")))
            .accessibility_label(label.clone())
            .flex()
            .items_center()
            .w_full()
            .h(px(28.))
            .gap(px(14.))
            .justify_start()
            .pl(px(indent))
            .rounded(px(8.))
            .text_size(px(14.))
            .text_color(rgb(color))
            .when(active, |row| row.bg(rgb(0x505153)))
            .child(icon(glyph, 17., color))
            .child(div().line_height(px(20.)).truncate().child(label))
            .on_click(
                cx.listener(move |this, _, window, cx| this.set_filter(filter.clone(), window, cx)),
            )
    }

    fn layout_menu(&self, cx: &Context<Self>) -> Popover {
        let notebook = cx.weak_entity();
        Popover::new("sidebar-layout")
            .trigger(tool(
                "layout-options",
                "Opzioni dei pannelli",
                "sliders-horizontal",
            ))
            .content(move |_, _, cx| {
                div().w(px(195.)).flex().flex_col().children(
                    [
                        (3, "Tag, note ed editor"),
                        (2, "Note ed editor"),
                        (1, "Solo editor"),
                    ]
                    .map(|(panes, label)| {
                        let notebook = notebook.clone();
                        menu_button(label).on_click(cx.listener(move |popover, _, window, cx| {
                            let _ = notebook.update(cx, |this, cx| {
                                this.panes = panes;
                                this.panel = Panel::None;
                                cx.notify();
                            });
                            popover.dismiss(window, cx);
                        }))
                    }),
                )
            })
    }

    fn list_section_menu(&self, cx: &Context<Self>) -> Popover {
        let notebook = cx.weak_entity();
        let label = match &self.filter {
            Filter::Tag(tag) => tag.rsplit('/').next().unwrap_or(tag),
            filter => section_label(filter),
        };
        let has_archive =
            self.notes.iter().any(|note| note.archived) || self.filter == Filter::Archive;
        Popover::new("list-sections")
            .trigger(
                Button::new("list-section")
                    .accessibility_label(format!("Sezione · {label}"))
                    .h(px(32.))
                    .px(px(4.))
                    .rounded(px(6.))
                    .text_size(px(15.))
                    .font_weight(FontWeight::BOLD)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(3.))
                            .child(div().max_w(px(110.)).truncate().child(label.to_owned()))
                            .child(icon("chevron-down", 12., MUTED)),
                    ),
            )
            .content(move |_, _, cx| {
                div().w(px(155.)).flex().flex_col().children(
                    [
                        Filter::Notes,
                        Filter::Untagged,
                        Filter::Todo,
                        Filter::Today,
                        Filter::Pinned,
                        Filter::Archive,
                        Filter::Trash,
                    ]
                    .into_iter()
                    .filter(|filter| *filter != Filter::Archive || has_archive)
                    .map(|filter| {
                        let notebook = notebook.clone();
                        menu_button(section_label(&filter)).on_click(cx.listener(
                            move |popover, _, window, cx| {
                                let _ = notebook.update(cx, |this, cx| {
                                    this.set_filter(filter.clone(), window, cx)
                                });
                                popover.dismiss(window, cx);
                            },
                        ))
                    }),
                )
            })
    }

    pub(super) fn note_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let left_tool = if self.panes == 2 {
            Some(
                tool("show-sidebar", "Mostra barra laterale", "panel-left").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.panes = 3;
                        cx.notify();
                    },
                )),
            )
        } else {
            None
        };
        let search = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(52.))
            .pl(px(12.))
            .pr(px(8.))
            .children(left_tool)
            .when(!self.search_open, |header| {
                header
                    .child(self.list_section_menu(cx))
                    .child(drag_area().flex_1())
                    .child(
                        tool("new-note", "Nuova nota", "square-pen")
                            .size(px(35.))
                            .rounded_full()
                            .bg(rgb(0x303133))
                            .mr(px(8.))
                            .on_click(cx.listener(|this, _, window, cx| this.new_note(window, cx))),
                    )
                    .child(
                        tool("open-search", "Ricerca", "search")
                            .size(px(35.))
                            .rounded_full()
                            .bg(rgb(0x303133))
                            .on_click(cx.listener(|this, _, window, cx| this.find(window, cx))),
                    )
            })
            .when(self.search_open, |header| {
                header.child(
                    div()
                        .flex()
                        .items_center()
                        .flex_1()
                        .min_w_0()
                        .h(px(34.))
                        .rounded_full()
                        .border_1()
                        .border_color(rgb(RULE))
                        .px(px(8.))
                        .gap(px(5.))
                        .child(icon("search", 15., MUTED))
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&self.search)
                                    .appearance(false)
                                    .bordered(false)
                                    .focus_bordered(false)
                                    .h(px(28.))
                                    .aria_label("Ricerca")
                                    .p_0()
                                    .text_size(px(14.)),
                            ),
                        )
                        .child(
                            tool("close-search", "Chiudi ricerca", "circle-x")
                                .w(px(20.))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.close_search(window, cx);
                                })),
                        ),
                )
            });
        let results = notes::visible(&self.notes, &self.filter, &self.query);
        let mut rows = div()
            .id("note-list-rows")
            .track_scroll(&self.note_list_scroll)
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(10.))
            .pt(px(3.));
        if results.is_empty() {
            rows = rows.child(
                div()
                    .py(px(24.))
                    .text_center()
                    .text_color(rgb(SUBTLE))
                    .child("Nessuna nota"),
            );
        }
        for note in results {
            let id = note.id;
            let active = self.selected == Some(id);
            let title = note.title();
            let button = Button::new(("note-row", id))
                .accessibility_label(title.to_owned())
                .w_full()
                .flex()
                .flex_col()
                .items_start()
                .justify_start()
                .text_left()
                .px(px(13.))
                .pt(px(10.))
                .pb(px(10.))
                .min_h(px(103.))
                .rounded(px(6.))
                .when(active, |row| row.bg(rgb(SELECTED)))
                .child(
                    div()
                        .w_full()
                        .line_clamp(2)
                        .text_ellipsis()
                        .line_height(px(18.))
                        .text_size(px(14.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(TITLE))
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .flex()
                        .w_full()
                        .min_h(px(36.))
                        .mt(px(4.))
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .line_clamp(2)
                                .text_ellipsis()
                                .line_height(px(19.))
                                .text_size(px(14.))
                                .text_color(rgb(MUTED))
                                .child(note.snippet()),
                        )
                        .children(
                            self.store
                                .as_ref()
                                .and_then(|store| note.thumbnail(&store.attachments()))
                                .map(|path| {
                                    img(path)
                                        .size(px(40.))
                                        .rounded(px(5.))
                                        .object_fit(ObjectFit::Cover)
                                }),
                        ),
                )
                .child(
                    div()
                        .mt(px(4.))
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .text_size(px(12.))
                        .text_color(rgb(SUBTLE))
                        .when(note.pinned, |row| row.child(icon("pin", 13., ACCENT)))
                        .child(date_label(note.modified)),
                )
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    this.sync_editor(cx);
                    this.select_note(Some(id));
                    this.panel = Panel::None;
                    if event.click_count() == 2 {
                        this.start_editing(window, cx);
                    }
                    cx.notify();
                }));
            rows = rows.child(
                div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .child(button)
                    .when(!active, |row| {
                        row.child(div().mx(px(13.)).h(px(1.)).flex_shrink_0().bg(rgb(RULE)))
                    }),
            );
        }
        div()
            .flex()
            .flex_col()
            .w(px(NOTE_LIST_WIDTH))
            .flex_shrink_0()
            .bg(rgb(LIST))
            .border_r_1()
            .border_color(rgb(RULE))
            .child(search)
            .child(rows)
    }
}

fn section_label(filter: &Filter) -> &'static str {
    match filter {
        Filter::Notes => "Note",
        Filter::Untagged => "Senza tag",
        Filter::Todo => "ToDo",
        Filter::Today => "Oggi",
        Filter::Pinned => "Fissata",
        Filter::Archive => "Archivio",
        Filter::Trash => "Cestino",
        Filter::Tag(_) => "Tag",
    }
}

fn date_label(timestamp: u64) -> String {
    let Some(date) =
        DateTime::from_timestamp(timestamp as i64, 0).map(|date| date.with_timezone(&Local))
    else {
        return String::new();
    };
    if notes::now().saturating_sub(timestamp) < 60 {
        return "Ora".into();
    }
    use chrono::Datelike as _;
    let months = [
        "gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic",
    ];
    format!(
        "{} {} {}",
        date.day(),
        months[date.month0() as usize],
        date.year()
    )
}

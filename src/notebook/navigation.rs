use super::{
    Notebook, Panel,
    widgets::{drag_area, icon, menu_button, tool},
};
use crate::{
    notes::{self, Filter, Note, tag_icons},
    palette::{ACCENT, LIST, MUTED, RULE, SELECTED, SIDEBAR, SUBTLE, TEXT, TITLE},
};
use chrono::{DateTime, Local};
use gpui_kit::{
    ClickEvent, Context, Div, Focusable, IntoElement, ObjectFit, ParentElement, ScrollStrategy,
    Styled, StyledImage, Window, div, img, prelude::FluentBuilder as _, px, rgb, uniform_list,
};
use gpui_kit::{
    FontWeight, SharedString,
    base::Button,
    component::{input::Input, popover::Popover},
};
use std::{collections::BTreeMap, ops::Range};

pub(super) const SIDEBAR_WIDTH: f32 = 188.;
pub(super) const NOTE_LIST_WIDTH: f32 = 314.;

enum SidebarRow {
    NoteGroup,
    Filter(Filter, &'static str, &'static str),
    Tag { name: String, has_children: bool },
    Spacer,
}

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
        let results = notes::visible_indices(&self.notes, &self.filter, &self.query);
        let index = match results
            .iter()
            .position(|&index| Some(self.notes[index].id) == self.selected)
        {
            Some(current) => current.checked_add_signed(offset),
            None if offset < 0 => results.len().checked_sub(1),
            None => Some(0),
        };
        let Some((index, id)) =
            index.and_then(|index| results.get(index).map(|&note| (index, self.notes[note].id)))
        else {
            return;
        };
        self.select_note(Some(id));
        self.workspace_focus.focus(window, cx);
        self.note_list_scroll
            .scroll_to_item(index, ScrollStrategy::Nearest);
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
        let mut rows = vec![SidebarRow::NoteGroup];
        if self.expanded_notes {
            rows.extend([
                SidebarRow::Filter(Filter::Untagged, "Senza Tag", "inbox"),
                SidebarRow::Filter(Filter::Todo, "ToDo", "square-check"),
                SidebarRow::Filter(Filter::Today, "Oggi", "calendar-days"),
                SidebarRow::Filter(Filter::Pinned, "Fissata", "pin"),
            ]);
        }
        if self.notes.iter().any(|note| note.archived && !note.trashed)
            || self.filter == Filter::Archive
        {
            rows.push(SidebarRow::Filter(Filter::Archive, "Archivio", "archive"));
        }
        rows.push(SidebarRow::Filter(Filter::Trash, "Cestino", "trash"));
        let tags = notes::all_tags(self.notes.iter().filter(|note| {
            !self.indexing_tags || self.selected == Some(note.id) || note.tags_cached()
        }));
        let mut entries = BTreeMap::new();
        for tag in tags {
            for (offset, _) in tag.match_indices('/') {
                entries.insert(&tag[..offset], true);
            }
            entries.entry(tag).or_insert(false);
        }
        if !entries.is_empty() {
            rows.push(SidebarRow::Spacer);
        }
        rows.extend(
            entries
                .into_iter()
                .filter(|(tag, _)| {
                    tag.match_indices('/')
                        .all(|(offset, _)| self.expanded_tags.contains(&tag[..offset]))
                })
                .map(|(tag, has_children)| SidebarRow::Tag {
                    name: tag.to_owned(),
                    has_children,
                }),
        );
        let list = uniform_list(
            "sidebar-rows",
            rows.len(),
            cx.processor(move |this, range: Range<usize>, _, cx| {
                rows[range]
                    .iter()
                    .map(|row| this.sidebar_row(row, cx))
                    .collect()
            }),
        );
        div()
            .flex()
            .flex_col()
            .w(px(SIDEBAR_WIDTH))
            .flex_shrink_0()
            .bg(rgb(SIDEBAR))
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .px(px(8.))
                    .pt(px(11.))
                    .child(list.size_full()),
            )
    }

    fn sidebar_row(&self, row: &SidebarRow, cx: &mut Context<Self>) -> Div {
        let item = div().relative().w_full().h(px(28.));
        match row {
            SidebarRow::NoteGroup => item
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
            SidebarRow::Filter(filter, label, glyph) => {
                let indent = if matches!(filter, Filter::Archive | Filter::Trash) {
                    30.
                } else {
                    42.
                };
                item.child(self.filter_row(filter.clone(), (*label).into(), glyph, indent, cx))
            }
            SidebarRow::Tag { name, has_children } => {
                let indent = name.bytes().filter(|byte| *byte == b'/').count() as f32 * 15.;
                let label = name.rsplit('/').next().unwrap_or(name);
                let button = self.filter_row(
                    Filter::Tag(name.clone()),
                    label.into(),
                    tag_icons::resolve(&self.tag_icons, name).glyph(),
                    30. + indent,
                    cx,
                );
                item.child(self.tag_context_menu(button, name, cx))
                    .when(*has_children, |item| {
                        let glyph = if self.expanded_tags.contains(name) {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        };
                        let name = name.clone();
                        item.child(
                            tool(
                                SharedString::from(format!("toggle-tag-{name}")),
                                "Espandi o comprimi tag",
                                glyph,
                            )
                            .absolute()
                            .left(px(indent + 1.))
                            .top_0()
                            .w(px(28.))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if !this.expanded_tags.remove(&name) {
                                        this.expanded_tags.insert(name.clone());
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                    })
            }
            SidebarRow::Spacer => item,
        }
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
        let results = notes::visible_indices(&self.notes, &self.filter, &self.query);
        let mut rows = div()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
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
        } else {
            let count = results.len();
            rows = rows.child(
                uniform_list(
                    "note-list-rows",
                    count,
                    cx.processor(move |this, range: Range<usize>, _, cx| {
                        results[range]
                            .iter()
                            .map(|&index| this.note_row(&this.notes[index], cx))
                            .collect()
                    }),
                )
                .track_scroll(&self.note_list_scroll)
                .size_full(),
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

    fn note_row(&self, note: &Note, cx: &Context<Self>) -> Div {
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
            .pt(px(8.))
            .pb(px(8.))
            .h_full()
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
                                img(crate::attachments::preview::list_image(path))
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
                    .line_height(px(15.))
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
        div()
            .relative()
            .w_full()
            .h(px(116.))
            .child(button)
            .when(!active, |row| {
                row.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left(px(13.))
                        .right(px(13.))
                        .h(px(1.))
                        .bg(rgb(RULE)),
                )
            })
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

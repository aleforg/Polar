mod statistics;

use chrono::{DateTime, Local};
use gpui_kit::{
    Context, Div, FontWeight, InteractiveElement, ParentElement, StatefulInteractiveElement,
    Styled, base::Button, div, prelude::FluentBuilder as _, px, rgb,
};
use markdown::{ParseOptions, mdast::Node, to_mdast};
use statistics::Statistics;

use super::{Notebook, Panel, widgets::icon};
use crate::{
    notes::links,
    palette::{MUTED, RULE, SELECTED, SUBTLE, TEXT},
};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum InfoTab {
    Stats,
    Outline,
    Backlinks,
}

pub(super) struct OutlineEntry {
    pub(super) offset: usize,
    level: u8,
    label: String,
}

pub(super) fn outline(text: &str) -> Vec<OutlineEntry> {
    let mut headings: Vec<_> = to_mdast(text, &ParseOptions::gfm())
        .ok()
        .and_then(|node| match node {
            Node::Root(root) => Some(root.children),
            _ => None,
        })
        .into_iter()
        .flatten()
        .filter_map(|node| {
            let Node::Heading(heading) = &node else {
                return None;
            };
            Some(OutlineEntry {
                offset: heading.position.as_ref()?.start.offset,
                level: heading.depth,
                label: node.to_string(),
            })
        })
        .collect();
    if headings.first().is_none_or(|heading| heading.offset != 0)
        && let Some(title) = text.lines().next().filter(|line| !line.trim().is_empty())
    {
        headings.insert(
            0,
            OutlineEntry {
                offset: 0,
                level: 1,
                label: title.trim().to_owned(),
            },
        );
    }
    headings
}

impl Notebook {
    pub(super) fn jump_to_heading(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.source.reveal(offset);
        self.panel = Panel::None;
        cx.notify();
    }

    pub(super) fn info_panel(&self, cx: &mut Context<Self>) -> Div {
        let mut tabs = div()
            .flex()
            .gap(px(3.))
            .p(px(3.))
            .rounded(px(7.))
            .bg(rgb(0x262729));
        let mut title = "";
        for (tab, id, label, glyph) in [
            (
                InfoTab::Stats,
                "stats",
                "Statistiche",
                "chart-no-axes-column",
            ),
            (InfoTab::Outline, "outline", "Indice", "list"),
            (InfoTab::Backlinks, "backlinks", "Rimandi", "reply"),
        ] {
            let active = self.info_tab == tab;
            if active {
                title = label;
            }
            tabs = tabs.child(
                Button::new(id)
                    .accessibility_label(label)
                    .flex_1()
                    .h(px(26.))
                    .rounded(px(5.))
                    .text_size(px(12.))
                    .text_color(rgb(if active { TEXT } else { MUTED }))
                    .when(active, |button| button.bg(rgb(0x505153)))
                    .child(icon(glyph, 16., if active { TEXT } else { MUTED }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.info_tab = tab;
                        cx.notify();
                    })),
            );
        }
        let mut panel = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(4.))
            .child(
                div()
                    .py(px(3.))
                    .text_center()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(MUTED))
                    .child(title),
            )
            .child(tabs);
        let Some(note) = self.selected() else {
            return panel.child(
                div()
                    .p(px(8.))
                    .text_color(rgb(SUBTLE))
                    .child("Nessuna nota"),
            );
        };
        match self.info_tab {
            InfoTab::Stats => {
                let stats = Statistics::for_note(&note.text);
                let rows = [
                    [
                        ("Parole", stats.words.to_string(), "message-square"),
                        ("Caratteri", stats.characters.to_string(), "type"),
                    ],
                    [
                        ("Paragrafi", stats.paragraphs.to_string(), "pilcrow"),
                        ("Tempo di lettura", stats.reading_time(), "clock"),
                    ],
                ];
                panel = panel
                    .children(rows.into_iter().map(|row| {
                        div().flex().gap(px(7.)).children(row.into_iter().map(
                            |(label, value, glyph)| {
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .gap(px(9.))
                                    .p(px(10.))
                                    .rounded(px(7.))
                                    .bg(rgb(0x2c2d2f))
                                    .child(
                                        div()
                                            .flex()
                                            .justify_between()
                                            .items_center()
                                            .text_size(px(22.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(rgb(TEXT))
                                            .child(value)
                                            .child(icon(glyph, 14., SUBTLE)),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(rgb(MUTED))
                                            .child(label),
                                    )
                            },
                        ))
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .rounded(px(7.))
                            .bg(rgb(0x2c2d2f))
                            .child(
                                date_card("Ultima modifica", note.modified)
                                    .border_b_1()
                                    .border_color(rgb(RULE)),
                            )
                            .children(note.created.map(|created| date_card("Creazione", created))),
                    );
            }
            InfoTab::Outline => {
                let headings = outline(&note.text);
                let mut items = div()
                    .id("note-outline")
                    .flex()
                    .flex_col()
                    .max_h(px(320.))
                    .overflow_y_scroll();
                if headings.is_empty() {
                    items = items.child(
                        div()
                            .p(px(8.))
                            .text_color(rgb(SUBTLE))
                            .child("Nessun titolo"),
                    );
                }
                for heading in headings {
                    let offset = heading.offset;
                    let label = if heading.label.is_empty() {
                        "Senza titolo".to_owned()
                    } else {
                        heading.label
                    };
                    items = items.child(
                        Button::new(("outline-heading", offset))
                            .accessibility_label(label.clone())
                            .w_full()
                            .h(px(26.))
                            .flex_shrink_0()
                            .justify_start()
                            .rounded(px(5.))
                            .pl(px(7. + f32::from(heading.level.saturating_sub(1)) * 9.))
                            .text_size(px(13.))
                            .text_color(rgb(TEXT))
                            .hover(|row| row.bg(rgb(SELECTED)))
                            .border_b_1()
                            .border_color(rgb(RULE))
                            .when(heading.level == 1, |row| row.font_weight(FontWeight::BOLD))
                            .child(div().truncate().child(label))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.jump_to_heading(offset, cx);
                            })),
                    );
                }
                panel = panel.child(items);
            }
            InfoTab::Backlinks => {
                let mut items = div()
                    .id("note-backlinks")
                    .flex()
                    .flex_col()
                    .max_h(px(320.))
                    .overflow_y_scroll();
                let mut count = 0;
                for candidate in self
                    .notes
                    .iter()
                    .filter(|candidate| candidate.id != note.id && !candidate.trashed)
                {
                    if !links::wiki_links(&candidate.text)
                        .iter()
                        .any(|link| links::target(&self.notes, link.title) == Some(note.id))
                    {
                        continue;
                    }
                    count += 1;
                    let id = candidate.id;
                    let title = candidate.title().to_owned();
                    items = items.child(
                        Button::new(("backlink", id))
                            .accessibility_label(title.clone())
                            .w_full()
                            .h(px(26.))
                            .flex_shrink_0()
                            .justify_start()
                            .rounded(px(5.))
                            .pl(px(7.))
                            .text_size(px(13.))
                            .text_color(rgb(TEXT))
                            .hover(|row| row.bg(rgb(SELECTED)))
                            .child(div().truncate().child(title))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.follow_note(id, window, cx)
                            })),
                    );
                }
                if count == 0 {
                    items = items.child(
                        div()
                            .p(px(8.))
                            .text_color(rgb(SUBTLE))
                            .child("Nessuna nota contiene un rimando"),
                    );
                }
                panel = panel.child(items);
            }
        }
        panel
    }
}

fn date_card(label: &'static str, timestamp: u64) -> Div {
    let date = i64::try_from(timestamp)
        .ok()
        .and_then(|timestamp| DateTime::from_timestamp(timestamp, 0))
        .map(|date| {
            date.with_timezone(&Local)
                .format("%d/%m/%Y · %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "—".to_owned());
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .p(px(9.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .text_size(px(13.))
                .text_color(rgb(TEXT))
                .font_weight(FontWeight::SEMIBOLD)
                .child(date)
                .child(icon("calendar-days", 14., SUBTLE)),
        )
        .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child(label))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_keeps_duplicate_headings_and_skips_code() {
        let text = "Titolo 🐻

## Itinerario

```
## Codice
```

### Itinerario
";
        let entries = outline(text);
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.level, entry.label.as_str()))
                .collect::<Vec<_>>(),
            [(1, "Titolo 🐻"), (2, "Itinerario"), (3, "Itinerario")]
        );
        assert_eq!(entries[2].offset, text.find("### Itinerario").unwrap());
    }
}

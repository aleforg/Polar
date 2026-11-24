use gpui_kit::{
    FontStyle, FontWeight, HighlightStyle, StrikethroughStyle, UnderlineStyle,
    base::input::TextDecoration, px, rgb,
};
use markdown::{ParseOptions, mdast::Node, to_mdast};
use std::ops::Range;

use super::markdown_lists::{self, ListLine, ListMarker};

use crate::{
    blocks::{Block, BlockContent},
    notes,
    palette::{ACCENT, MUTED, TITLE},
};

pub(super) fn syntax_tree(text: &str) -> Option<Node> {
    let mut options = ParseOptions::gfm();
    options.constructs.math_flow = true;
    options.constructs.math_text = true;
    to_mdast(text, &options).ok()
}

pub(crate) const HIGHLIGHTS: [(&str, &str, u32); 6] = [
    ("Predefinito", "==", 0xcbd2d9),
    ("Verde", "==🟢", 0x9adf99),
    ("Rosso", "==🔴", 0xfca8a5),
    ("Blu", "==🔵", 0x8bc1ff),
    ("Giallo", "==🟡", 0xffd385),
    ("Viola", "==🟣", 0xe9acf4),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DocumentLink {
    Web(String),
    Wiki(String),
    Tag(String),
}

pub(super) struct Formula {
    pub(super) range: Range<usize>,
    pub(super) value: String,
    pub(super) display: bool,
}

#[derive(Default)]
pub(crate) struct LiveMarkdown {
    text_styles: Vec<TextDecoration>,
    markers: Vec<(Range<usize>, Range<usize>)>,
    highlights: Vec<(Range<usize>, u32)>,
    headings: Vec<Heading>,
    list_lines: Vec<ListLine>,
    formulas: Vec<Formula>,
    blocks: Vec<Block>,
    links: Vec<(Range<usize>, DocumentLink)>,
    literal_ranges: Vec<Range<usize>>,
}

pub(crate) struct Heading {
    pub(crate) range: Range<usize>,
    pub(crate) level: u8,
}

impl Heading {
    pub(crate) fn size(&self) -> f64 {
        match self.level {
            1 => 30.,
            2 => 23.,
            3 => 20.,
            _ => 17.,
        }
    }
}

impl LiveMarkdown {
    pub(crate) fn parse(text: &str) -> Self {
        let _timing = crate::performance::Sample::start("parse");
        let mut result = Self::default();
        if let Some(root) = syntax_tree(text) {
            result.blocks = root
                .children()
                .into_iter()
                .flatten()
                .filter_map(|node| Block::parse(node, text))
                .collect();
            result.visit(&root, text);
            markdown_lists::collect(&root, text, 0, &mut result.list_lines);
            let completed: Vec<_> = result
                .list_lines
                .iter()
                .filter_map(|line| {
                    matches!(line.marker, Some(ListMarker::Task { checked: true, .. }))
                        .then_some(line.prefix.end..line.range.end)
                })
                .collect();
            for range in completed {
                result.style(
                    range,
                    HighlightStyle {
                        color: Some(rgb(MUTED).into()),
                        ..Default::default()
                    },
                );
            }
        }
        let title_end = text.lines().next().map_or(0, str::len);
        if result
            .headings
            .first()
            .is_none_or(|heading| heading.range.start != 0)
        {
            result.headings.insert(
                0,
                Heading {
                    range: 0..title_end,
                    level: 1,
                },
            );
            result.style(0..title_end, heading());
        }
        result
    }

    pub(super) fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub(super) fn list_lines(&self) -> &[ListLine] {
        &self.list_lines
    }

    pub(crate) fn headings(&self) -> &[Heading] {
        &self.headings
    }

    pub(super) fn formulas(&self) -> &[Formula] {
        &self.formulas
    }

    pub(super) fn allows_wiki_completion(&self, offset: usize) -> bool {
        !self
            .literal_ranges
            .iter()
            .any(|range| range.contains(&offset))
    }

    pub(super) fn links(&self) -> &[(Range<usize>, DocumentLink)] {
        &self.links
    }

    pub(super) fn highlights(&self) -> &[(Range<usize>, u32)] {
        &self.highlights
    }

    pub(crate) fn hidden_markers(&self, selection: Option<&Range<usize>>) -> Vec<&Range<usize>> {
        self.markers
            .iter()
            .filter_map(|(marker, construct)| {
                selection
                    .is_none_or(|selection| {
                        selection.start > construct.end || selection.end < construct.start
                    })
                    .then_some(marker)
            })
            .collect()
    }

    pub(crate) fn decorations(&self, selection: Option<&Range<usize>>) -> Vec<TextDecoration> {
        let mut styles = self.text_styles.clone();
        for (marker, construct) in &self.markers {
            let active = selection.is_some_and(|selection| {
                selection.start <= construct.end && selection.end >= construct.start
            });
            let color = rgb(if active { MUTED } else { 0x303030 }).into();
            styles.push(TextDecoration::new(
                marker.clone(),
                HighlightStyle {
                    color: Some(color),
                    ..Default::default()
                },
            ));
        }
        styles
    }

    fn style(&mut self, range: Range<usize>, style: HighlightStyle) {
        if !range.is_empty() {
            self.text_styles.push(TextDecoration::new(range, style));
        }
    }

    fn surround(&mut self, range: &Range<usize>, content: Range<usize>) {
        for marker in [range.start..content.start, content.end..range.end] {
            if !marker.is_empty() {
                self.markers.push((marker, range.clone()));
            }
        }
    }

    fn highlight(&mut self, text: &str, range: Range<usize>) {
        let mut offset = range.start;
        for line in text[range].split_inclusive('\n') {
            let mut opening = None;
            for (index, _) in line.match_indices("==") {
                let before = &line.as_bytes()[..index];
                if before
                    .iter()
                    .rev()
                    .take_while(|&&byte| byte == b'\\')
                    .count()
                    % 2
                    != 0
                    || before.last() == Some(&b'=')
                    || line.as_bytes().get(index + 2) == Some(&b'=')
                {
                    continue;
                }
                let Some(start) = opening.take() else {
                    opening = Some(index);
                    continue;
                };
                let (_, prefix, _) = HIGHLIGHTS
                    .iter()
                    .skip(1)
                    .find(|(_, prefix, _)| line[start..].starts_with(prefix))
                    .copied()
                    .unwrap_or(HIGHLIGHTS[0]);
                let content_start = start + prefix.len();
                if content_start < index && !line[content_start..index].trim().is_empty() {
                    let content = offset + content_start..offset + index;
                    self.surround(&(offset + start..offset + index + 2), content.clone());
                    let fill = match prefix {
                        "==🟢" => 0x3f5e3e,
                        "==🔴" => 0x6b4041,
                        "==🟡" => 0x635333,
                        "==🟣" => 0x604263,
                        _ => 0x2e5073,
                    };
                    self.highlights.push((content, fill));
                }
            }
            offset += line.len();
        }
    }

    fn visit(&mut self, node: &Node, text: &str) {
        if let Some(pos) = node.position() {
            let range = pos.start.offset..pos.end.offset;
            if matches!(
                node,
                Node::Code(_)
                    | Node::InlineCode(_)
                    | Node::Math(_)
                    | Node::InlineMath(_)
                    | Node::Link(_)
                    | Node::LinkReference(_)
                    | Node::Image(_)
                    | Node::ImageReference(_)
            ) {
                self.literal_ranges.push(range.clone());
            }
            let content = node.children().and_then(|children| {
                Some(
                    children.first()?.position()?.start.offset
                        ..children.last()?.position()?.end.offset,
                )
            });
            let style = match node {
                Node::Heading(_)
                    if self.blocks.iter().any(|block| {
                        matches!(block.content, BlockContent::Separator)
                            && block.range.start > range.start
                            && block.range.end == range.end
                    }) =>
                {
                    None
                }
                Node::Heading(h) => {
                    self.headings.push(Heading {
                        range: range.clone(),
                        level: h.depth,
                    });
                    Some(heading())
                }
                Node::Strong(_) => Some(HighlightStyle {
                    font_weight: Some(FontWeight::BOLD),
                    ..Default::default()
                }),
                Node::Emphasis(_) => Some(HighlightStyle {
                    font_style: Some(FontStyle::Italic),
                    ..Default::default()
                }),
                Node::Delete(_) => Some(HighlightStyle {
                    strikethrough: Some(StrikethroughStyle {
                        thickness: px(1.),
                        color: None,
                    }),
                    ..Default::default()
                }),
                Node::Link(_) | Node::LinkReference(_) => {
                    if let Node::Link(link) = node
                        && let Some(label) = &content
                        && (link.url.starts_with("https://") || link.url.starts_with("http://"))
                    {
                        self.links
                            .push((label.clone(), DocumentLink::Web(link.url.clone())));
                    }
                    Some(HighlightStyle {
                        color: Some(rgb(ACCENT).into()),
                        underline: Some(UnderlineStyle {
                            thickness: px(1.),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                }
                Node::Math(math) => {
                    let source = &text[range.clone()];
                    let fence = source.bytes().take_while(|&byte| byte == b'$').count();
                    let closed = source.lines().count() >= 3
                        && source.lines().last().is_some_and(|line| {
                            let line = line.trim();
                            line.len() >= fence && line.bytes().all(|byte| byte == b'$')
                        });
                    if closed && !math.value.trim().is_empty() {
                        self.formulas.push(Formula {
                            range: range.clone(),
                            value: math.value.clone(),
                            display: true,
                        });
                    }
                    None
                }
                Node::InlineMath(math) => {
                    let source = &text[range.clone()];
                    let double = source.starts_with("$$");
                    let after = text[range.end..].chars().next();
                    if !source.contains('\n')
                        && !math.value.is_empty()
                        && (double
                            || (math.value.trim() == math.value
                                && !after.is_some_and(|ch| ch.is_ascii_digit())))
                    {
                        let display = double
                            && text[..range.start]
                                .rsplit('\n')
                                .next()
                                .unwrap_or("")
                                .trim()
                                .is_empty()
                            && text[range.end..]
                                .split('\n')
                                .next()
                                .unwrap_or("")
                                .trim()
                                .is_empty();
                        self.formulas.push(Formula {
                            range: range.clone(),
                            value: math.value.clone(),
                            display,
                        });
                    }
                    None
                }
                Node::InlineCode(_) | Node::Code(_) => {
                    self.style(
                        range.clone(),
                        HighlightStyle {
                            background_color: Some(rgb(0x323335).into()),
                            ..Default::default()
                        },
                    );
                    if matches!(node, Node::InlineCode(_)) {
                        let fence = text[range.clone()]
                            .bytes()
                            .take_while(|&c| c == b'`')
                            .count();
                        self.surround(&range, range.start + fence..range.end - fence);
                    }
                    None
                }
                Node::Text(_) => {
                    self.highlight(text, range.clone());
                    for link in notes::links::in_text(text, range.clone()) {
                        self.surround(&link.range, link.label.clone());
                        self.links.push((
                            link.label.clone(),
                            DocumentLink::Wiki(link.title.to_owned()),
                        ));
                        self.style(
                            link.label,
                            HighlightStyle {
                                color: Some(rgb(ACCENT).into()),
                                ..Default::default()
                            },
                        );
                    }
                    for tag in notes::tags::in_text(&text[range.clone()]) {
                        let absolute = range.start + tag.range.start..range.start + tag.range.end;
                        let visible = absolute.start..absolute.start + 1 + tag.name.len();
                        self.surround(&absolute, visible.clone());
                        self.links
                            .push((visible.clone(), DocumentLink::Tag(tag.name.to_lowercase())));
                        self.style(
                            visible,
                            HighlightStyle {
                                color: Some(rgb(0xd8d9da).into()),
                                background_color: Some(rgb(0x424344).into()),
                                ..Default::default()
                            },
                        );
                    }
                    None
                }
                _ => None,
            };
            if let (Some(style), Some(content)) = (style, content) {
                if matches!(node, Node::Heading(_)) {
                    for marker in [range.start..content.start, content.end..range.end] {
                        if marker.len() > 1 {
                            let reveal = marker.start + 1..marker.end - 1;
                            self.markers.push((marker, reveal));
                        }
                    }
                } else {
                    self.surround(&range, content.clone());
                }
                self.style(content, style);
            }
        }
        if let Some(children) = node.children() {
            let mut underline_start = None;
            for child in children {
                if let Node::Html(html) = child
                    && let Some(pos) = &html.position
                {
                    if html.value.eq_ignore_ascii_case("<u>") {
                        underline_start = Some(pos.start.offset..pos.end.offset);
                    } else if html.value.eq_ignore_ascii_case("</u>")
                        && let Some(start) = underline_start.take()
                    {
                        let range = start.start..pos.end.offset;
                        let content = start.end..pos.start.offset;
                        self.surround(&range, content.clone());
                        self.style(
                            content,
                            HighlightStyle {
                                underline: Some(UnderlineStyle {
                                    thickness: px(1.),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            },
                        );
                    }
                }
                self.visit(child, text);
            }
        }
    }
}

fn heading() -> HighlightStyle {
    HighlightStyle {
        color: Some(rgb(TITLE).into()),
        font_weight: Some(FontWeight::BOLD),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_dashes_as_separators_with_or_without_a_blank_line() {
        let text =
            "Titolo\n\nPrima **forte**\n---\nDopo\n\n---\n\n```\n---\n```\n\nSottotitolo\n--";
        let markdown = LiveMarkdown::parse(text);
        assert_eq!(markdown.blocks().len(), 2);
        for block in markdown.blocks() {
            assert!(matches!(block.content, BlockContent::Separator));
            assert_eq!(&text[block.range.clone()], "---");
        }
        assert_eq!(markdown.headings().len(), 2);
        assert!(text[markdown.headings()[1].range.clone()].starts_with("Sottotitolo"));
        assert!(markdown.decorations(None).iter().any(|decoration| {
            &text[decoration.range.clone()] == "forte"
                && decoration.style.font_weight == Some(FontWeight::BOLD)
        }));
    }

    #[test]
    fn parses_closed_math_without_mistaking_code_and_prices() {
        let text = "Titolo\n\n$E=mc^2$ e `$codice$`. Prezzi $5 o $10.\n\n$$\n\\sum_{k=1}^n k\n$$\n\n$$x^2$$\n\n$$\ninterrotta";
        let markdown = LiveMarkdown::parse(text);
        let formulas: Vec<_> = markdown
            .formulas()
            .iter()
            .map(|formula| (formula.value.as_str(), formula.display))
            .collect();
        assert_eq!(
            formulas,
            [("E=mc^2", false), ("\\sum_{k=1}^n k", true), ("x^2", true)]
        );
    }

    #[test]
    fn keeps_heading_levels_and_ignores_fenced_code() {
        let text =
            "Titolo\n\n## Secondo\n\n### Terzo\n\nTesto\n\n```\n#### Codice\n```\n\n###### Sesto";
        let markdown = LiveMarkdown::parse(text);
        let headings: Vec<_> = markdown
            .headings()
            .iter()
            .map(|h| (h.level, &text[h.range.clone()]))
            .collect();
        assert_eq!(
            headings,
            [
                (1, "Titolo"),
                (2, "## Secondo"),
                (3, "### Terzo"),
                (6, "###### Sesto")
            ]
        );
        let explicit = LiveMarkdown::parse("## Primo");
        assert_eq!(explicit.headings().len(), 1);
        assert_eq!(explicit.headings()[0].level, 2);
    }

    #[test]
    fn highlights_plain_and_colored_markdown() {
        let text = "Titolo\n\n==🟢caffè== e ==tè==, `==codice==`";
        let markdown = LiveMarkdown::parse(text);
        let highlights: Vec<_> = markdown
            .highlights()
            .iter()
            .map(|(range, fill)| (&text[range.clone()], *fill))
            .collect();
        assert_eq!(highlights, [("caffè", 0x3f5e3e), ("tè", 0x2e5073)]);
        assert!(
            markdown
                .hidden_markers(None)
                .iter()
                .any(|range| &text[(**range).clone()] == "==🟢")
        );
    }

    #[test]
    fn formats_unicode_and_reveals_only_active_markers() {
        let text =
            "Titolo 🐻\n\n## Sezione\n\n**caffè** e *tè* #viaggi\n\n- [x] Fatto\n\n`#codice`";
        let markdown = LiveMarkdown::parse(text);
        let bold = text.find("caffè").unwrap();
        let styles = markdown.decorations(Some(&(bold..bold)));
        assert!(
            styles.iter().any(|d| &text[d.range.clone()] == "caffè"
                && d.style.font_weight == Some(FontWeight::BOLD))
        );
        assert!(styles.iter().any(
            |d| &text[d.range.clone()] == "tè" && d.style.font_style == Some(FontStyle::Italic)
        ));
        assert!(
            styles
                .iter()
                .any(|d| &text[d.range.clone()] == "Fatto"
                    && d.style.color == Some(rgb(MUTED).into()))
        );
        assert!(
            styles
                .iter()
                .any(|d| &text[d.range.clone()] == "#viaggi" && d.style.background_color.is_some())
        );
        let marker = |styles: &[TextDecoration]| {
            styles
                .iter()
                .find(|d| d.range == (bold - 2..bold))
                .unwrap()
                .style
                .color
                .unwrap()
        };
        assert_eq!(marker(&styles), rgb(MUTED).into());
        assert_eq!(
            marker(&markdown.decorations(Some(&(0..0)))),
            rgb(0x303030).into()
        );
        assert!(!styles.iter().any(|d| &text[d.range.clone()] == "#codice"));
    }
}

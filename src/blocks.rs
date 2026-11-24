use markdown::mdast::Node;
use std::ops::Range;

pub(crate) const TABLE: &str = "| Colonna 1 | Colonna 2 |\n| --- | --- |\n|  |  |\n|  |  |";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CalloutKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl CalloutKind {
    pub(crate) const ALL: [Self; 5] = [
        Self::Note,
        Self::Tip,
        Self::Important,
        Self::Warning,
        Self::Caution,
    ];

    pub(crate) fn keyword(self) -> &'static str {
        match self {
            Self::Note => "NOTE",
            Self::Tip => "TIP",
            Self::Important => "IMPORTANT",
            Self::Warning => "WARNING",
            Self::Caution => "CAUTION",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Note => "Nota",
            Self::Tip => "Suggerimento",
            Self::Important => "Importante",
            Self::Warning => "Avviso",
            Self::Caution => "Attenzione",
        }
    }

    pub(crate) fn appearance(self) -> (u32, u32, &'static str) {
        match self {
            Self::Note => (0x6ba4e9, 0x252f3c, "info"),
            Self::Tip => (0x72b68a, 0x26352d, "lightbulb"),
            Self::Important => (0xb997e6, 0x342b41, "circle-alert"),
            Self::Warning => (0xe4af60, 0x3d3224, "triangle-alert"),
            Self::Caution => (0xe67d84, 0x3d282e, "octagon-alert"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Callout {
    pub(crate) kind: CalloutKind,
    pub(crate) title: String,
    pub(crate) body: String,
}

impl Callout {
    pub(crate) fn parse(source: &str) -> Option<Self> {
        let mut lines = source.lines();
        let (marker, title) = lines
            .next()?
            .trim_start()
            .strip_prefix('>')?
            .trim_start()
            .strip_prefix("[!")?
            .split_once(']')?;
        let kind = CalloutKind::ALL
            .into_iter()
            .find(|kind| marker.eq_ignore_ascii_case(kind.keyword()))?;
        let title = title.trim();
        let body = lines
            .map(|line| {
                line.trim_start()
                    .strip_prefix('>')
                    .map_or(line, |line| line.strip_prefix(' ').unwrap_or(line))
            })
            .collect::<Vec<_>>()
            .join("\n");
        Some(Self {
            kind,
            title: if title.is_empty() {
                kind.label()
            } else {
                title
            }
            .into(),
            body,
        })
    }
}

#[derive(Clone, Copy)]
pub(super) enum TableEdit {
    AddRow,
    AddColumn,
}

pub(super) fn extend_table(source: &str, columns: usize, edit: TableEdit) -> String {
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    match edit {
        TableEdit::AddRow => format!("{source}{newline}|{}", "   |".repeat(columns)),
        TableEdit::AddColumn => source
            .lines()
            .enumerate()
            .map(|(index, line)| {
                let mut line = line.trim_end().to_owned();
                let trailing_pipe = line.strip_suffix('|').is_some_and(|before| {
                    before
                        .bytes()
                        .rev()
                        .take_while(|byte| *byte == b'\\')
                        .count()
                        .is_multiple_of(2)
                });
                if !trailing_pipe {
                    line.push_str(" |");
                }
                line.push_str(if index == 1 { " --- |" } else { "   |" });
                line
            })
            .collect::<Vec<_>>()
            .join(newline),
    }
}

#[derive(Debug)]
pub(crate) enum CellPart {
    Text(String),
    Formula(Range<usize>),
}

fn collect_cell_parts(node: &Node, parts: &mut Vec<CellPart>) {
    if let Node::InlineMath(math) = node
        && let Some(position) = &math.position
    {
        parts.push(CellPart::Formula(
            position.start.offset..position.end.offset,
        ));
    } else if let Some(children) = node.children() {
        for child in children {
            collect_cell_parts(child, parts);
        }
    } else {
        let text = node.to_string();
        if let Some(CellPart::Text(previous)) = parts.last_mut() {
            previous.push_str(&text);
        } else if !text.is_empty() {
            parts.push(CellPart::Text(text));
        }
    }
}

#[derive(Debug)]
pub(crate) enum BlockContent {
    Separator,
    Table(Vec<Vec<Vec<CellPart>>>),
    Image { url: String, alt: String },
    Callout(Callout),
}

#[derive(Debug)]
pub(crate) struct Block {
    pub(crate) range: Range<usize>,
    pub(crate) content: BlockContent,
}

impl Block {
    pub(crate) fn parse(node: &Node, source: &str) -> Option<Self> {
        let pos = node.position()?;
        let range = pos.start.offset..pos.end.offset;
        let content = match node {
            Node::ThematicBreak(_) => BlockContent::Separator,
            Node::Heading(heading) if heading.depth == 2 => {
                // Bear treats a line of dashes as a separator even after a paragraph.
                let (_, line) = source.get(range.clone())?.rsplit_once('\n')?;
                let dashes = line.trim();
                if dashes.len() < 3 || !dashes.bytes().all(|byte| byte == b'-') {
                    return None;
                }
                return Some(Self {
                    range: range.end - line.len()..range.end,
                    content: BlockContent::Separator,
                });
            }
            Node::Table(table) => BlockContent::Table(
                table
                    .children
                    .iter()
                    .filter_map(|row| {
                        let Node::TableRow(row) = row else {
                            return None;
                        };
                        Some(
                            row.children
                                .iter()
                                .map(|cell| {
                                    let mut parts = Vec::new();
                                    collect_cell_parts(cell, &mut parts);
                                    parts
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            ),
            Node::Paragraph(paragraph) => {
                let [Node::Image(image)] = paragraph.children.as_slice() else {
                    return None;
                };
                BlockContent::Image {
                    url: image.url.clone(),
                    alt: image.alt.clone(),
                }
            }
            Node::Blockquote(_) => {
                BlockContent::Callout(Callout::parse(source.get(range.clone())?)?)
            }
            _ => return None,
        };
        Some(Self { range, content })
    }

    pub(crate) fn is_inactive(&self, selection: &Range<usize>) -> bool {
        selection.start > self.range.end || selection.end < self.range.start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use markdown::{ParseOptions, to_mdast};

    #[test]
    fn adds_table_dimensions_without_rewriting_cell_markup() {
        let source = "| **Nome** | Valore |\n| :--- | ---: |\n| A \\| B | 10 |";
        let taller = extend_table(source, 2, TableEdit::AddRow);
        assert!(taller.starts_with(source));
        assert!(taller.ends_with("|   |   |"));
        let wider = extend_table(source, 2, TableEdit::AddColumn);
        assert!(wider.contains("| :--- | ---: | --- |"));
        assert!(wider.contains("| A \\| B | 10 |   |"));
        let root = to_mdast(&wider, &ParseOptions::gfm()).unwrap();
        let table = Block::parse(&root.children().unwrap()[0], &wider).unwrap();
        assert!(matches!(table.content, BlockContent::Table(rows) if rows[0].len() == 3));
    }

    #[test]
    fn parses_inserted_blocks_and_preserves_callout_markdown() {
        let source = format!(
            "{TABLE}\n\n> [!warning] Verifica\n> **Prima**\n>\n> - Dopo\n\n![Foto](attachments/image-demo.png)"
        );
        let root = to_mdast(&source, &ParseOptions::gfm()).unwrap();
        let blocks: Vec<_> = root
            .children()
            .unwrap()
            .iter()
            .filter_map(|node| Block::parse(node, &source))
            .collect();
        assert!(
            matches!(&blocks[0].content, BlockContent::Table(rows) if rows.len() == 3 && matches!(rows[0][1].as_slice(), [CellPart::Text(text)] if text == "Colonna 2"))
        );
        assert!(
            matches!(&blocks[1].content, BlockContent::Callout(c) if c.kind == CalloutKind::Warning && c.title == "Verifica" && c.body == "**Prima**\n\n- Dopo")
        );
        assert!(
            matches!(&blocks[2].content, BlockContent::Image { url, alt } if url == "attachments/image-demo.png" && alt == "Foto")
        );
    }
}

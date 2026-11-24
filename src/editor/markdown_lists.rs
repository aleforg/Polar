use markdown::mdast::Node;
use std::ops::Range;

pub(super) enum ListMarker {
    Bullet,
    Number(String),
    Task { offset: usize, checked: bool },
}

pub(super) struct ListLine {
    pub(super) range: Range<usize>,
    pub(super) prefix: Range<usize>,
    pub(super) depth: usize,
    pub(super) extra_digits: usize,
    pub(super) marker: Option<ListMarker>,
}

pub(super) fn collect(node: &Node, source: &str, depth: usize, lines: &mut Vec<ListLine>) {
    if let Node::List(list) = node {
        for child in &list.children {
            let Node::ListItem(item) = child else {
                continue;
            };
            let Some(position) = &item.position else {
                continue;
            };
            let start = line_start(source, position.start.offset);
            let end = position.end.offset;
            let first = item
                .children
                .first()
                .filter(|node| !matches!(node, Node::List(_)));
            let first_position = first.and_then(Node::position);
            let body = first_position.map_or_else(
                || source[start..end].find('\n').map_or(end, |len| start + len),
                |position| position.start.offset,
            );
            let marker = item
                .checked
                .and_then(|checked| {
                    let offset = source[start..body].find('[')? + start + 1;
                    Some(ListMarker::Task { offset, checked })
                })
                .unwrap_or_else(|| {
                    if list.ordered {
                        let marker = source[position.start.offset..body]
                            .split_whitespace()
                            .next()
                            .unwrap_or("1.");
                        ListMarker::Number(marker.to_owned())
                    } else {
                        ListMarker::Bullet
                    }
                });
            let extra_digits = match &marker {
                ListMarker::Number(marker) => marker.len().saturating_sub(3),
                _ => 0,
            };
            lines.push(ListLine {
                range: start..first_position.map_or(body, |position| position.end.offset),
                prefix: start..body,
                depth,
                extra_digits,
                marker: Some(marker),
            });
            for (index, node) in item.children.iter().enumerate() {
                if index > 0
                    && matches!(node, Node::Paragraph(_))
                    && let Some(position) = node.position()
                {
                    let body = position.start.offset;
                    let start = line_start(source, body);
                    lines.push(ListLine {
                        range: start..position.end.offset,
                        prefix: start..body,
                        depth,
                        extra_digits,
                        marker: None,
                    });
                }
                collect(node, source, depth + 1, lines);
            }
        }
    } else if let Some(children) = node.children() {
        for child in children {
            collect(child, source, depth, lines);
        }
    }
}

fn line_start(source: &str, offset: usize) -> usize {
    source[..offset].rfind('\n').map_or(0, |index| index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use markdown::{ParseOptions, to_mdast};

    #[test]
    fn preserves_typed_numbers_and_reserves_room_for_long_labels() {
        let source = "1. First\n42. Second\n1000. Third";
        let tree = to_mdast(source, &ParseOptions::gfm()).unwrap();
        let mut lines = Vec::new();
        collect(&tree, source, 0, &mut lines);
        assert!(matches!(&lines[1].marker, Some(ListMarker::Number(marker)) if marker == "42."));
        assert!(matches!(&lines[2].marker, Some(ListMarker::Number(marker)) if marker == "1000."));
        assert_eq!(lines[2].extra_digits, 2);
    }

    #[test]
    fn keeps_depth_order_continuations_and_task_offsets() {
        let source = "7. First\n   - Nested\n   - [x] Done\n\n     Continuation\n8. Last";
        let tree = to_mdast(source, &ParseOptions::gfm()).unwrap();
        let mut lines = Vec::new();
        collect(&tree, source, 0, &mut lines);
        assert_eq!(
            lines.iter().map(|line| line.depth).collect::<Vec<_>>(),
            [0, 1, 1, 1, 0]
        );
        assert!(matches!(&lines[0].marker, Some(ListMarker::Number(marker)) if marker == "7."));
        assert!(matches!(&lines[4].marker, Some(ListMarker::Number(marker)) if marker == "8."));
        assert!(
            matches!(lines[2].marker, Some(ListMarker::Task { offset, checked: true }) if &source[offset..offset + 1] == "x")
        );
        assert!(lines[3].marker.is_none());
    }
}

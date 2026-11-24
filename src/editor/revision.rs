use std::{collections::HashMap, ops::Range};

use markdown::mdast::Node;
use polar_note_revision::Action;

use super::markdown::syntax_tree;
use crate::{blocks::Block, notes};

pub(crate) struct Selection {
    pub(crate) range: Range<usize>,
    pub(crate) units: Vec<Range<usize>>,
}

fn offsets(node: &Node) -> Option<Range<usize>> {
    let pos = node.position()?;
    Some(pos.start.offset..pos.end.offset)
}

fn content_offsets(node: &Node, source: &str) -> Option<Range<usize>> {
    let range = offsets(node)?;
    // The parser can include the newline after a final list item in its position.
    let content = source.get(range.clone())?.trim_end_matches(['\r', '\n']);
    Some(range.start..range.start + content.len())
}

fn inline_supported(node: &Node, source: &str) -> bool {
    match node {
        Node::Text(_) => offsets(node).is_some_and(|range| {
            let text = &source[range.clone()];
            !text.contains("==")
                && notes::links::in_text(source, range).is_empty()
                && notes::tags::in_text(text).next().is_none()
        }),
        Node::Break(_) => true,
        Node::Strong(_) | Node::Emphasis(_) | Node::Delete(_) => node
            .children()
            .is_some_and(|children| children.iter().all(|node| inline_supported(node, source))),
        _ => false,
    }
}

fn supported(node: &Node, source: &str) -> bool {
    match node {
        Node::List(list) => list.children.iter().all(|item| {
            matches!(item, Node::ListItem(item) if item.checked.is_none()
                && matches!(item.children.as_slice(), [child @ Node::Paragraph(_)] if supported(child, source)))
        }),
        Node::Paragraph(_) | Node::Heading(_) => Block::parse(node, source).is_none() && node.children()
            .is_some_and(|children| children.iter().all(|node| inline_supported(node, source))),
        _ => false,
    }
}

fn split<'a>(
    nodes: &'a [Node],
    source: &str,
    range: &Range<usize>,
) -> Option<(&'a [Node], &'a [Node], &'a [Node])> {
    let first = nodes.iter().position(|node| {
        content_offsets(node, source).is_some_and(|node| node.end > range.start)
    })?;
    let last = nodes
        .iter()
        .position(|node| offsets(node).is_some_and(|node| node.start >= range.end))
        .unwrap_or(nodes.len());
    if first >= last
        || offsets(&nodes[first])?.start < range.start
        || content_offsets(&nodes[last - 1], source)?.end > range.end
    {
        return None;
    }
    Some((&nodes[..first], &nodes[first..last], &nodes[last..]))
}

fn units(nodes: &[Node]) -> impl Iterator<Item = &Node> {
    nodes.iter().flat_map(|node| match node {
        Node::List(list) => list.children.as_slice(),
        _ => std::slice::from_ref(node),
    })
}

fn supported_selection(nodes: &[Node], source: &str, action: Action) -> bool {
    nodes.iter().all(|node| supported(node, source))
        && (matches!(action, Action::Correct)
            || nodes.iter().any(|node| !matches!(node, Node::Heading(_))))
}

pub(crate) fn selection(
    source: &str,
    range: &Range<usize>,
    action: Action,
) -> Result<Selection, &'static str> {
    let title_end = source.find('\n').map_or(source.len(), |index| index + 1);
    if range.is_empty() {
        return Err("Seleziona uno o più blocchi completi nel corpo della nota.");
    }
    let root = syntax_tree(source).ok_or("Impossibile leggere il Markdown della nota.")?;
    let nodes = root
        .children()
        .ok_or("La nota non contiene blocchi selezionabili.")?;
    if range.start < title_end
        || nodes.iter().any(|node| {
            offsets(node).is_some_and(|node| node.start < title_end && node.end > range.start)
        })
    {
        return Err(
            "Il titolo è escluso. Se condivide un paragrafo con il testo, aggiungi una riga vuota dopo il titolo.",
        );
    }
    let (_, selected, _) = split(nodes, source, range)
        .ok_or("Seleziona blocchi Markdown interi, compresi i marcatori.")?;
    if !supported_selection(selected, source, action) {
        return Err(
            "Seleziona paragrafi, intestazioni nel corpo o liste piatte. Per organizzare, includi almeno un paragrafo o una lista.",
        );
    }
    Ok(selection_for_blocks(selected, source))
}

pub(crate) fn organization_passages(source: &str) -> Result<Vec<Selection>, &'static str> {
    let title_end = source.find('\n').map_or(source.len(), |index| index + 1);
    let root = syntax_tree(source).ok_or("Impossibile leggere il Markdown della nota.")?;
    let nodes = root
        .children()
        .ok_or("La nota non contiene blocchi selezionabili.")?;
    let passages: Vec<_> = nodes
        .split(|node| {
            !offsets(node).is_some_and(|range| range.start >= title_end) || !supported(node, source)
        })
        .filter(|nodes| nodes.iter().any(|node| !matches!(node, Node::Heading(_))))
        .map(|nodes| selection_for_blocks(nodes, source))
        .collect();
    if passages.is_empty() {
        if nodes.iter().any(|node| {
            offsets(node).is_some_and(|range| range.start < title_end && range.end > title_end)
        }) {
            return Err(
                "Aggiungi una riga vuota dopo il titolo per separarlo dal corpo della nota.",
            );
        }
        return Err("La nota non contiene paragrafi o liste semplici da organizzare.");
    }
    Ok(passages)
}

fn selection_for_blocks(selected: &[Node], source: &str) -> Selection {
    let start = offsets(&selected[0]).expect("positioned block").start;
    let end = content_offsets(&selected[selected.len() - 1], source)
        .expect("positioned block")
        .end;
    let units = units(selected)
        .filter_map(|node| content_offsets(node, source))
        .map(|unit| unit.start - start..unit.end - start)
        .collect();
    Selection {
        range: start..end,
        units,
    }
}

fn clear_positions(node: &mut Node) {
    node.position_set(None);
    if let Some(children) = node.children_mut() {
        children.iter_mut().for_each(clear_positions);
    }
}

fn outside_unchanged(original: &[Node], revised: &[Node], shift: isize) -> bool {
    original.len() == revised.len()
        && original.iter().zip(revised).all(|(before, after)| {
            let Some(range) = offsets(before) else {
                return false;
            };
            let Some(updated) = offsets(after) else {
                return false;
            };
            if range.start.checked_add_signed(shift) != Some(updated.start)
                || range.end.checked_add_signed(shift) != Some(updated.end)
            {
                return false;
            }
            let (mut before, mut after) = (before.clone(), after.clone());
            clear_positions(&mut before);
            clear_positions(&mut after);
            before == after
        })
}

fn markers(text: &str) -> Vec<u8> {
    text.bytes()
        .filter(|byte| matches!(byte, b'*' | b'_' | b'~' | b'\\'))
        .collect()
}

fn inline_formats(node: &Node, source: &str, formats: &mut Vec<u8>) {
    match node {
        Node::Strong(_) | Node::Emphasis(_) | Node::Delete(_) => {
            if let Some(range) = offsets(node) {
                formats.push(source.as_bytes()[range.start]);
                formats.push(if matches!(node, Node::Strong(_)) {
                    2
                } else {
                    1
                });
            }
        }
        Node::Break(_) => formats.extend([b'\n', 1]),
        _ => {}
    }
    if let Some(children) = node.children() {
        children
            .iter()
            .for_each(|child| inline_formats(child, source, formats));
    }
}

fn correction_preserves_blocks(before: &Node, after: &Node, original: &str, revised: &str) -> bool {
    let Some((old, new)) = offsets(before).zip(offsets(after)) else {
        return false;
    };
    let old = &original[old];
    let new = &revised[new];
    match (before, after) {
        (Node::List(a), Node::List(b)) => {
            a.ordered == b.ordered
                && a.start == b.start
                && a.spread == b.spread
                && a.children.len() == b.children.len()
                && a.children
                    .iter()
                    .zip(&b.children)
                    .all(|(a, b)| correction_preserves_blocks(a, b, original, revised))
        }
        (Node::ListItem(a), Node::ListItem(b)) => {
            old.split_whitespace().next() == new.split_whitespace().next()
                && a.spread == b.spread
                && correction_preserves_blocks(&a.children[0], &b.children[0], original, revised)
        }
        (Node::Heading(a), Node::Heading(b)) => {
            a.depth == b.depth
                && old.bytes().take_while(|&c| c == b'#').count()
                    == new.bytes().take_while(|&c| c == b'#').count()
                && old.rsplit_once('\n').map(|(_, line)| line.trim())
                    == new.rsplit_once('\n').map(|(_, line)| line.trim())
                && preserves_inline(before, after, original, revised, false)
        }
        (Node::Paragraph(_), Node::Paragraph(_)) => {
            preserves_inline(before, after, original, revised, true)
        }
        _ => false,
    }
}

fn preserves_inline(
    before: &Node,
    after: &Node,
    original: &str,
    revised: &str,
    repair: bool,
) -> bool {
    let (Some(old), Some(new)) = (offsets(before), offsets(after)) else {
        return false;
    };
    let (old, new) = (&original[old], &revised[new]);
    let (old_markers, new_markers) = (markers(old), markers(new));
    let (mut old_formats, mut new_formats) = (Vec::new(), Vec::new());
    inline_formats(before, original, &mut old_formats);
    inline_formats(after, revised, &mut new_formats);
    if old_markers == new_markers && old_formats == new_formats {
        return true;
    }
    if !repair || old.contains('\\') || new.contains('\\') {
        return false;
    }
    let mut formats = new_formats.chunks_exact(2);
    if !old_formats
        .chunks_exact(2)
        .all(|existing| formats.any(|new| new == existing))
    {
        return false;
    }
    if old_markers == new_markers {
        return new_formats.len() > old_formats.len()
            && old_formats.iter().filter(|&&c| c == b'~').count()
                == new_formats.iter().filter(|&&c| c == b'~').count();
    }
    old_markers == b"**" && new_markers == b"****" && new_formats == [b'*', 2]
}

fn original_units_survive(before: &[Node], after: &[Node], original: &str, revised: &str) -> bool {
    let mut remaining = HashMap::new();
    for node in units(before) {
        let Some(range) = content_offsets(node, original) else {
            return false;
        };
        *remaining.entry(&original[range]).or_insert(0) += 1;
    }
    for node in units(after) {
        let Some(range) = content_offsets(node, revised) else {
            return false;
        };
        if let Some(count) = remaining
            .get_mut(&revised[range])
            .filter(|count| **count > 0)
        {
            *count -= 1;
        } else if !matches!(node, Node::Heading(_)) {
            return false;
        }
    }
    remaining.values().all(|&count| count == 0)
}

pub(crate) fn validate(
    source: &str,
    range: &Range<usize>,
    replacement: &str,
    action: Action,
) -> bool {
    if source.get(range.clone()).is_none()
        || replacement.trim().is_empty()
        || replacement.starts_with(['\r', '\n'])
        || replacement.ends_with(['\r', '\n'])
        || replacement.contains('\0')
    {
        return false;
    }
    let mut revised = source.to_owned();
    revised.replace_range(range.clone(), replacement);
    let replacement_range = range.start..range.start + replacement.len();
    let (Some(original_root), Some(revised_root)) = (syntax_tree(source), syntax_tree(&revised))
    else {
        return false;
    };
    let Some((before, selected, after)) = original_root
        .children()
        .and_then(|nodes| split(nodes, source, range))
    else {
        return false;
    };
    let Some((new_before, proposed, new_after)) = revised_root
        .children()
        .and_then(|nodes| split(nodes, &revised, &replacement_range))
    else {
        return false;
    };
    if !outside_unchanged(before, new_before, 0)
        || !outside_unchanged(
            after,
            new_after,
            replacement.len() as isize - range.len() as isize,
        )
        || !supported_selection(proposed, &revised, action)
    {
        return false;
    }
    match action {
        Action::Correct => {
            selected.len() == proposed.len()
                && selected.iter().zip(proposed).all(|(before, after)| {
                    correction_preserves_blocks(before, after, source, &revised)
                })
        }
        Action::Organize => original_units_survive(selected, proposed, source, &revised),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_complete_body_blocks_and_keeps_the_title_out() {
        for source in [
            "Titolo 🐻\nCorpo",
            "Titolo 🐻\n\nCorpo",
            "# Titolo 🐻\nCorpo",
        ] {
            let start = source.find("Corpo").unwrap();
            assert_eq!(
                selection(source, &(start..source.len()), Action::Correct).is_ok(),
                source != "Titolo 🐻\nCorpo"
            );
        }
        for body in [
            "- Uno\n  - Sotto",
            "- [ ] Da fare",
            "Sottotitolo\n---",
            "[Link](https://example.com)",
            "Testo #tag",
            "Vedi [[Nota]]",
        ] {
            let source = format!("Titolo\n\n{body}");
            assert!(
                selection(&source, &(8..source.len()), Action::Correct).is_err(),
                "{body}"
            );
        }
        let source = "Titolo\n\n## Intestazione";
        assert!(selection(source, &(8..source.len()), Action::Correct).is_ok());
        assert!(selection(source, &(8..source.len()), Action::Organize).is_err());
        assert!(selection(source, &(11..source.len()), Action::Correct).is_err());
    }

    #[test]
    fn finds_organization_passages_without_selecting_protected_blocks() {
        let source = "# Titolo 🐻\n## Note\n\nPrimo testo.\n\n- Voce\n  - Annidata\n\n1. Uno\n2. Due\n\n[Link](https://example.com)\n\n## Fine";
        let passages = organization_passages(source).unwrap();
        assert_eq!(
            passages
                .iter()
                .map(|passage| &source[passage.range.clone()])
                .collect::<Vec<_>>(),
            ["## Note\n\nPrimo testo.", "1. Uno\n2. Due"]
        );
        assert!(organization_passages("Titolo\nCorpo attaccato").is_err());
        assert!(organization_passages("# Titolo\n## Solo una sezione").is_err());
        assert!(organization_passages("Titolo\n\n- Voce\n  - Annidata").is_err());
    }

    #[test]
    fn validates_corrections_in_context() {
        for (before, replacement) in [
            ("Riunone **martedì", "Riunione **martedì**"),
            ("Da tenere ** testo**.", "Da tenere **testo**."),
        ] {
            let source = format!("Titolo 🐻\n\n{before}\n\nDopo.");
            let start = source.find(before).unwrap();
            let range = start..start + before.len();
            assert!(validate(&source, &range, replacement, Action::Correct));
            for invalid in [
                "- Nuovo punto",
                "Corretto #tag",
                "```\nCambia il contesto",
                "[r]: https://example.com",
            ] {
                assert!(
                    !validate(&source, &range, invalid, Action::Correct),
                    "{invalid}"
                );
            }
        }
        let source = "Titolo\n\n\\**letterale**";
        assert!(!validate(
            source,
            &(8..source.len()),
            "**letterale**",
            Action::Correct
        ));
    }

    #[test]
    fn organization_reuses_each_original_unit() {
        let source = "Titolo 🐻\n\n- Uno\n  continua qui\n- Due\n- Uno\n\nDopo.";
        let start = source.find("- Uno").unwrap();
        let end = source.find("\n\nDopo.").unwrap();
        let selected = selection(source, &(start..end), Action::Organize).unwrap();
        let text = &source[selected.range.clone()];
        assert_eq!(
            selected
                .units
                .iter()
                .map(|r| &text[r.clone()])
                .collect::<Vec<_>>(),
            ["- Uno\n  continua qui", "- Due", "- Uno"]
        );
        assert!(validate(
            source,
            &(start..end),
            "## Uno\n\n- Uno\n  continua qui\n- Uno\n\n## Due\n\n- Due",
            Action::Organize
        ));
        for invalid in ["- Due\n- Uno", "## Uno\n\n## Due", "- Due\n- Tre\n- Uno"] {
            assert!(
                !validate(source, &(start..end), invalid, Action::Organize),
                "{invalid}"
            );
        }

        let source = "Titolo\n\n1. Sale: chiedere capienza\n2. Preventivo: chiedi a Luca\n3. Sale: forse giovedì\n4. Preventivo: IVA esclusa";
        let proposal = "## Sale\n\n1. Sale: chiedere capienza\n\n3. Sale: forse giovedì\n\n## Preventivo\n\n2. Preventivo: chiedi a Luca\n\n4. Preventivo: IVA esclusa";
        let passages = organization_passages(source).unwrap();
        assert!(validate(
            source,
            &passages[0].range,
            proposal,
            Action::Organize
        ));
    }
}

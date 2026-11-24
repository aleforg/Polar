use super::Note;
use markdown::{ParseOptions, mdast::Node, to_mdast};
use std::ops::Range;

pub(crate) struct WikiLink<'a> {
    pub(crate) range: Range<usize>,
    pub(crate) label: Range<usize>,
    pub(crate) title: &'a str,
}

/// Finds the title being typed after an unclosed wiki-link opener.
pub(crate) fn completion_prefix(before: &str) -> Option<(usize, &str)> {
    let start = before.rfind("[[")?;
    let title = &before[start + 2..];
    if before[..start].ends_with('[')
        || escaped_opener(before, start)
        || title.contains(['[', ']', '|'])
    {
        return None;
    }
    Some((start, title))
}

fn escaped_opener(text: &str, start: usize) -> bool {
    text[..start]
        .bytes()
        .rev()
        .take_while(|&byte| byte == b'\\')
        .count()
        % 2
        != 0
}

pub(crate) fn suggested_titles(notes: &[Note], query: &str) -> Vec<String> {
    let query = query.trim().to_lowercase();
    let mut matches: Vec<_> = notes
        .iter()
        .filter(|note| !note.trashed)
        .filter_map(|note| {
            let title = note.title();
            let normalized = title.to_lowercase();
            let index = normalized.find(&query)?;
            (!title.contains(['[', ']', '|'])).then_some((index != 0, normalized, title, note))
        })
        .collect();
    matches.sort_unstable_by(|a, b| {
        (a.0, &a.1)
            .cmp(&(b.0, &b.1))
            .then_with(|| (b.3.modified, b.3.id).cmp(&(a.3.modified, a.3.id)))
    });
    matches.dedup_by(|a, b| a.1 == b.1);
    matches
        .into_iter()
        .filter(|(_, _, title, _)| {
            let source = format!("[[{title}]]");
            wiki_links(&source)
                .iter()
                .any(|link| link.title == *title && link.range == (0..source.len()))
        })
        .take(6)
        .map(|(_, _, title, _)| title.to_owned())
        .collect()
}

pub(crate) fn in_text(text: &str, range: Range<usize>) -> Vec<WikiLink<'_>> {
    let mut links = Vec::new();
    let mut cursor = range.start;
    while let Some(open) = text[cursor..range.end].find("[[") {
        let start = cursor + open;
        let body_start = start + 2;
        let Some(close) = text[body_start..range.end].find("]]") else {
            break;
        };
        let end = body_start + close;
        cursor = end + 2;
        let inner = &text[body_start..end];
        if escaped_opener(text, start) || inner.contains(['[', ']', '\n', '\r']) {
            continue;
        }
        let (title, label) = inner.split_once('|').unwrap_or((inner, inner));
        if title.trim().is_empty() || label.trim().is_empty() {
            continue;
        }
        let label_start = if title.len() == inner.len() {
            body_start
        } else {
            body_start + title.len() + 1
        };
        links.push(WikiLink {
            range: start..cursor,
            label: label_start..end,
            title: title.trim(),
        });
    }
    links
}

pub(crate) fn wiki_links(text: &str) -> Vec<WikiLink<'_>> {
    if !text.contains("[[") {
        return Vec::new();
    }
    let mut options = ParseOptions::gfm();
    options.constructs.math_text = true;
    options.constructs.math_flow = true;
    let Ok(root) = to_mdast(text, &options) else {
        return Vec::new();
    };
    let mut links = Vec::new();
    let mut pending = vec![&root];
    while let Some(node) = pending.pop() {
        match node {
            Node::Text(text_node) => {
                if let Some(pos) = &text_node.position {
                    links.extend(in_text(text, pos.start.offset..pos.end.offset));
                }
            }
            Node::Link(_) | Node::LinkReference(_) | Node::Image(_) => {}
            _ => {
                if let Some(children) = node.children() {
                    pending.extend(children.iter().rev());
                }
            }
        }
    }
    links
}

pub(crate) fn target(notes: &[Note], title: &str) -> Option<u64> {
    let title = title.to_lowercase();
    notes
        .iter()
        .filter(|note| !note.trashed && note.title().to_lowercase() == title)
        .max_by_key(|note| (note.modified, note.id))
        .map(|note| note.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_only_open_wiki_titles() {
        assert_eq!(completion_prefix("🐻 [[Ro"), Some((5, "Ro")));
        assert_eq!(completion_prefix("[["), Some((0, "")));
        for text in [r"\[[Roma", "[[[Roma", "[[Roma|alias", "[[Roma]]"] {
            assert_eq!(completion_prefix(text), None, "{text}");
        }
    }

    #[test]
    fn prefers_title_prefixes_and_excludes_trash_and_duplicates() {
        let mut notes = crate::notes::initial_notes();
        notes[0].text = "# Appunti su Roma".into();
        notes[1].text = "# Romanzo".into();
        notes[2].text = "# ROMA".into();
        notes[3].text = "# Roma".into();
        notes[3].modified = notes[2].modified + 1;
        notes[4].text = "# Roma sparita".into();
        notes[4].trashed = true;
        notes[5].text = "# Roma *corsivo*".into();
        assert_eq!(
            suggested_titles(&notes, "rOma"),
            ["Roma", "Romanzo", "Appunti su Roma"]
        );
    }

    #[test]
    fn resolves_aliases_without_linking_code_or_missing_notes() {
        let notes = crate::notes::initial_notes();
        let text =
            "Vedi [[Appunti di viaggio|l’itinerario]] e `[[Letture da esplorare]]`. [[Assente]]";
        let links = wiki_links(text);
        assert_eq!(links.len(), 2);
        assert_eq!(&text[links[0].label.clone()], "l’itinerario");
        assert_eq!(target(&notes, links[0].title), Some(1));
        assert_eq!(target(&notes, links[1].title), None);
    }
}

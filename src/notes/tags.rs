use markdown::{ParseOptions, mdast::Node, to_mdast};
use std::{collections::BTreeSet, ops::Range};

pub(crate) struct Tag<'a> {
    pub(crate) range: Range<usize>,
    pub(crate) name: &'a str,
}

pub(crate) fn tags(text: &str) -> BTreeSet<String> {
    if !may_have_tags(text) {
        return BTreeSet::new();
    }
    let Ok(root) = to_mdast(text, &ParseOptions::gfm()) else {
        return BTreeSet::new();
    };
    let mut tags = BTreeSet::new();
    let mut pending = vec![&root];
    while let Some(node) = pending.pop() {
        if let Node::Text(node) = node
            && let Some(pos) = &node.position
        {
            tags.extend(
                in_text(&text[pos.start.offset..pos.end.offset]).map(|tag| tag.name.to_lowercase()),
            );
        } else if let Some(children) = node.children() {
            pending.extend(children);
        }
    }
    tags
}

pub(crate) fn may_have_tags(text: &str) -> bool {
    text.match_indices('#')
        .any(|(offset, _)| text[offset + 1..].starts_with(tag_character))
}

pub(crate) fn in_text(text: &str) -> impl Iterator<Item = Tag<'_>> {
    let mut end = 0;
    text.match_indices('#').filter_map(move |(start, _)| {
        if start < end
            || text[..start]
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_whitespace() && !matches!(c, '(' | '[' | '"' | '\''))
        {
            return None;
        }
        let after = &text[start + 1..];
        let word_end = after
            .find(|c: char| !tag_character(c))
            .unwrap_or(after.len());
        let word = after[..word_end].trim_end_matches('/');
        if !valid(word) {
            return None;
        }
        if let Some(close) = after.find('#') {
            let name = &after[..close];
            if name == name.trim()
                && valid(name)
                && name.chars().all(|c| c == ' ' || tag_character(c))
            {
                end = start + 2 + close;
                return Some(Tag {
                    range: start..end,
                    name,
                });
            }
        }
        end = start + 1 + word.len();
        Some(Tag {
            range: start..end,
            name: word,
        })
    })
}

fn tag_character(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '/' | '-' | '_')
}

fn valid(name: &str) -> bool {
    name.split('/').all(|part| !part.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaced_and_nested_tags_exclude_code_and_escaped_hashes() {
        let text =
            "Titolo\n\n## Sezione #Una città/Idee# #Altra\n\n`#codice` \\#letterale ##doppio";
        assert_eq!(
            tags(text),
            BTreeSet::from(["altra".into(), "una città/idee".into()])
        );
        assert_eq!(in_text("#Una città#").next().unwrap().name, "Una città");
        assert!(!may_have_tags(
            "# Titolo\n\n## Sottotitolo\n\nTesto con # e testo."
        ));
        assert_eq!(tags("**#Lettūra**"), BTreeSet::from(["lettūra".into()]));
    }
}

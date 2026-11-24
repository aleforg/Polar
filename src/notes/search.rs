use super::Note;
use markdown::{ParseOptions, mdast::Node, to_mdast};

pub(super) struct Search {
    title_only: bool,
    terms: Vec<Term>,
}

struct Term {
    value: String,
    excluded: bool,
    phrase: bool,
}

impl Search {
    pub(super) fn new(query: &str) -> Self {
        let mut title_only = false;
        let mut terms = Vec::new();
        let mut value = String::new();
        let mut quoted = false;
        let mut phrase = false;
        let mut finish = |value: &mut String, phrase: bool| {
            if !phrase && value == "@title" {
                title_only = true;
            } else if !value.is_empty() {
                let excluded = value.len() > 1 && value.starts_with('-');
                if excluded {
                    value.remove(0);
                }
                terms.push(Term {
                    value: std::mem::take(value),
                    excluded,
                    phrase,
                });
            }
            value.clear();
        };

        for character in query.to_lowercase().chars() {
            match character {
                '"' | '“' | '”' => {
                    quoted = !quoted;
                    phrase = true;
                }
                c if c.is_whitespace() && !quoted => {
                    finish(&mut value, phrase);
                    phrase = false;
                }
                c => value.push(c),
            }
        }
        finish(&mut value, phrase);
        Self { title_only, terms }
    }

    pub(super) fn matches(&self, note: &Note) -> bool {
        if self.terms.is_empty() {
            return true;
        }
        let text = if self.title_only {
            note.title()
        } else {
            &note.text
        }
        .to_lowercase();
        self.terms.iter().all(|term| {
            let matches = match (term.phrase, term.value.as_str()) {
                (false, "@pinned") => note.pinned,
                (false, "@readonly") => note.read_only,
                (false, "@tagged") => !note.tags().is_empty(),
                (false, "@task") => {
                    let features = note.search_features();
                    features.todo || features.done
                }
                (false, "@code") => note.search_features().code,
                (false, "@untagged") => note.tags().is_empty(),
                (false, "@todo") => note.search_features().todo,
                (false, "@done") => note.search_features().done && !note.search_features().todo,
                (false, "@wikilinks") => !super::links::wiki_links(&note.text).is_empty(),
                (false, "@images") => note.search_features().images,
                (_, value) => text.contains(value),
            };
            matches != term.excluded
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Features {
    pub(super) todo: bool,
    done: bool,
    code: bool,
    images: bool,
}

impl Note {
    pub(super) fn search_features(&self) -> Features {
        let mut cached = self.search_cache.borrow_mut();
        if cached.0 != self.text {
            *cached = (self.text.clone(), Features::parse(&self.text));
        }
        cached.1
    }
}

impl Features {
    fn parse(text: &str) -> Self {
        let mut result = Self::default();
        let Ok(root) = to_mdast(text, &ParseOptions::gfm()) else {
            return result;
        };
        let mut pending = vec![&root];
        while let Some(node) = pending.pop() {
            match node {
                Node::ListItem(item) => match item.checked {
                    Some(false) => result.todo = true,
                    Some(true) => result.done = true,
                    None => {}
                },
                Node::Code(_) | Node::InlineCode(_) => result.code = true,
                Node::Image(_) => result.images = true,
                _ => {}
            }
            if let Some(children) = node.children() {
                pending.extend(children);
            }
        }
        result
    }
}

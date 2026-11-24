use markdown::{ParseOptions, mdast::Node, to_mdast};

pub(super) struct Statistics {
    pub(super) words: usize,
    pub(super) characters: usize,
    pub(super) paragraphs: usize,
}

impl Statistics {
    pub(super) fn for_note(source: &str) -> Self {
        let mut visible = String::new();
        if let Ok(root) = to_mdast(source, &ParseOptions::gfm()) {
            write_visible(&root, &mut visible);
        } else {
            visible.push_str(source);
        }
        Self {
            words: visible.split_whitespace().count(),
            characters: visible
                .chars()
                .filter(|ch| !matches!(ch, '\r' | '\n'))
                .count(),
            paragraphs: visible
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count(),
        }
    }

    pub(super) fn reading_time(&self) -> String {
        let seconds = self.words.saturating_add(2) / 5;
        match (seconds / 60, seconds % 60) {
            (0, seconds) => format!("{seconds} s"),
            (minutes, 0) => format!("{minutes} min"),
            (minutes, seconds) => format!("{minutes} min {seconds} s"),
        }
    }
}

fn write_visible(node: &Node, text: &mut String) {
    match node {
        Node::Text(value) => text.push_str(&value.value),
        Node::InlineCode(value) => text.push_str(&value.value),
        Node::Code(value) => {
            text.push_str(&value.value);
            text.push('\n');
        }
        Node::Break(_) => text.push('\n'),
        _ => {
            if let Some(children) = node.children() {
                for child in children {
                    write_visible(child, text);
                }
            }
            match node {
                Node::Heading(_) | Node::Paragraph(_) | Node::TableRow(_) => text.push('\n'),
                Node::TableCell(_) => text.push(' '),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_observed_bear_fixture() {
        let stats = Statistics::for_note(
            "# Appunti di viaggio · Verifica OpenBear\n\n## Itinerario\nUn paragrafo con **grassetto** e *corsivo*.\n\n- [ ] Preparare lo zaino\n- [x] Controllare gli orari\n\n#verifica-openbear/idee",
        );
        assert_eq!(
            (stats.words, stats.characters, stats.paragraphs),
            (20, 147, 6)
        );
        assert_eq!(stats.reading_time(), "4 s");
    }
}

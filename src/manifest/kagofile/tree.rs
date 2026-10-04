//! Kagofile text to a tree of lines. one statement per line, words split
//! on whitespace, `"..."` for words with spaces, `#` starts a comment, and
//! deeper indentation makes a line part of the block above it.

use crate::diagnostic::{self, Diagnostic, Location};

/// one word and where it starts in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    /// byte offset of the word, the opening quote for quoted words.
    pub at: usize,
    pub quoted: bool,
}

/// a line and the lines indented under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub words: Vec<Word>,
    pub children: Vec<Node>,
}

impl Node {
    /// first word, the keyword.
    #[must_use]
    pub fn keyword(&self) -> &str {
        self.words.first().map_or("", |w| w.text.as_str())
    }

    /// words after the keyword.
    #[must_use]
    pub fn args(&self) -> &[Word] {
        self.words.get(1..).unwrap_or(&[])
    }

    #[must_use]
    pub fn at(&self) -> usize {
        self.words.first().map_or(0, |w| w.at)
    }
}

/// parses `source` into top-level nodes.
///
/// # Errors
///
/// on an unclosed quote or a tab in indentation.
pub fn parse(file: &str, source: &str) -> Result<Vec<Node>, Diagnostic> {
    let error = |at: usize, message: &str| {
        Diagnostic::error(diagnostic::MANIFEST_SYNTAX, message).at(Location::at(file, source, at))
    };
    // (indent, node) for every open block, outermost first.
    let mut open: Vec<(usize, Node)> = Vec::new();
    let mut roots = Vec::new();
    let mut offset = 0;
    for raw in source.split_inclusive('\n') {
        let start = offset;
        offset += raw.len();
        let line = raw.trim_end_matches(['\n', '\r']);
        let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
        if line[..indent].contains('\t') {
            return Err(error(start, "indent with spaces, not tabs"));
        }
        let words = words(line, start).map_err(|at| error(at, "unclosed quote"))?;
        if words.is_empty() {
            continue;
        }
        let node = Node {
            words,
            children: Vec::new(),
        };
        // close every block this line is not inside of.
        while let Some((depth, _)) = open.last() {
            if indent > *depth {
                break;
            }
            let (_, done) = open.pop().expect("checked above");
            attach(&mut open, &mut roots, done);
        }
        open.push((indent, node));
    }
    while let Some((_, done)) = open.pop() {
        attach(&mut open, &mut roots, done);
    }
    Ok(roots)
}

fn attach(open: &mut [(usize, Node)], roots: &mut Vec<Node>, node: Node) {
    match open.last_mut() {
        Some((_, parent)) => parent.children.push(node),
        None => roots.push(node),
    }
}

/// splits one line into words. `Err` holds the offset of an unclosed quote.
fn words(line: &str, base: usize) -> Result<Vec<Word>, usize> {
    let mut out = Vec::new();
    let mut chars = line.char_indices().peekable();
    while let Some(&(i, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '#' {
            break;
        } else if c == '"' {
            chars.next();
            let mut text = String::new();
            let mut closed = false;
            while let Some((_, c)) = chars.next() {
                match c {
                    '"' => {
                        closed = true;
                        break;
                    }
                    '\\' => match chars.next() {
                        Some((_, 'n')) => text.push('\n'),
                        Some((_, 't')) => text.push('\t'),
                        Some((_, other)) => text.push(other),
                        None => break,
                    },
                    other => text.push(other),
                }
            }
            if !closed {
                return Err(base + i);
            }
            out.push(Word {
                text,
                at: base + i,
                quoted: true,
            });
        } else {
            let mut text = String::new();
            while let Some(&(_, c)) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                text.push(c);
                chars.next();
            }
            out.push(Word {
                text,
                at: base + i,
                quoted: false,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(node: &Node) -> Vec<&str> {
        node.words.iter().map(|w| w.text.as_str()).collect()
    }

    #[test]
    fn blocks_by_indent() {
        let src = "package dev.eira.ozone\n  name Ozone   # comment\n  version 0.1.0\n\napp ozone\n  windows a.exe\n    deeper x\n  linux a\nshortcut\n";
        let nodes = parse("Kagofile", src).unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(texts(&nodes[0]), ["package", "dev.eira.ozone"]);
        assert_eq!(texts(&nodes[0].children[0]), ["name", "Ozone"]);
        assert_eq!(nodes[1].children.len(), 2);
        assert_eq!(texts(&nodes[1].children[0].children[0]), ["deeper", "x"]);
        assert_eq!(nodes[2].keyword(), "shortcut");
    }

    #[test]
    fn quoted_words_and_offsets() {
        let src = "path optional \"Add \\\"ozone\\\" to PATH\"\n";
        let nodes = parse("Kagofile", src).unwrap();
        let label = &nodes[0].words[2];
        assert_eq!(label.text, "Add \"ozone\" to PATH");
        assert!(label.quoted);
        assert_eq!(label.at, src.find('"').unwrap());
        assert_eq!(nodes[0].words[1].at, 5);
    }

    #[test]
    fn hash_inside_a_word_is_not_a_comment() {
        let nodes = parse("Kagofile", "fetch https://x/a#b\n").unwrap();
        assert_eq!(nodes[0].args()[0].text, "https://x/a#b");
    }

    #[test]
    fn errors_point_at_the_problem() {
        let e = parse("Kagofile", "name \"open\n").unwrap_err();
        assert_eq!(e.location.unwrap().column, 6);
        let e = parse("Kagofile", "app x\n\twindows a\n").unwrap_err();
        assert_eq!(e.location.unwrap().line, 2);
    }
}

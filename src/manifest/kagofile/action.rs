//! `action`: a named block of Kagofile lines with parameters, used like
//! any statement. kago's library defines some, like `steamvr`; a Kagofile
//! or a file it `use`s can define more.
//!
//! ```text
//! action steamvr dir to={app}/steamvr/$dir.name
//!   copy $dir -> $to
//! ```
//!
//! a call fills in `$dir` and `$to` and runs the lines as if written in
//! its place: `steamvr build:driver` or `steamvr build:driver to={data}/d`.
//! `$dir.name` is the last part of the path.

use super::tree::{Node, Word};

#[derive(Debug, Clone)]
pub struct Action {
    params: Vec<Param>,
    body: Vec<Node>,
}

#[derive(Debug, Clone)]
struct Param {
    name: String,
    /// may use the parameters before it.
    default: Option<String>,
}

/// a problem and the offset it is at.
pub type Problem = (usize, String);

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl Action {
    /// `action <name> <param> [<param>=<default>]...` and the lines under
    /// it.
    ///
    /// # Errors
    ///
    /// on a bad name, a required parameter after one with a default, or no
    /// lines.
    pub fn define(node: &Node) -> Result<(String, Self), Problem> {
        let usage = "expected `action <name> [<param>] [<param>=<default>]` with its lines indented under it";
        let Some((name, rest)) = node.args().split_first() else {
            return Err((node.at(), usage.to_owned()));
        };
        if !valid_name(&name.text) {
            return Err((
                name.at,
                format!(
                    "`{}` is not an action name; use a-z, 0-9, `-` and `_`",
                    name.text
                ),
            ));
        }
        let mut params: Vec<Param> = Vec::new();
        for word in rest {
            let (param, default) = match word.text.split_once('=') {
                Some((p, d)) => (p, Some(d.to_owned())),
                None => (word.text.as_str(), None),
            };
            if !valid_name(param) || params.iter().any(|p| p.name == param) {
                return Err((word.at, format!("`{param}` is not a new parameter name")));
            }
            if default.is_none() && params.iter().any(|p| p.default.is_some()) {
                return Err((
                    word.at,
                    format!("`{param}` needs a default, like the parameters before it"),
                ));
            }
            params.push(Param {
                name: param.to_owned(),
                default,
            });
        }
        if node.children.is_empty() {
            return Err((node.at(), usage.to_owned()));
        }
        Ok((
            name.text.clone(),
            Self {
                params,
                body: node.children.clone(),
            },
        ))
    }

    /// the lines of a call with `args`, parameters filled in. every word
    /// is placed at `at`, the call, so problems in them point there.
    ///
    /// # Errors
    ///
    /// on a missing, unknown or extra argument.
    pub fn call(&self, args: &[Word], at: usize) -> Result<Vec<Node>, Problem> {
        let mut given: Vec<Option<String>> = vec![None; self.params.len()];
        let mut next = 0;
        for word in args {
            let named =
                word.text
                    .split_once('=')
                    .filter(|_| !word.quoted)
                    .and_then(|(name, value)| {
                        let i = self.params.iter().position(|p| p.name == name)?;
                        Some((i, value.to_owned()))
                    });
            let (i, value) = match named {
                Some(n) => n,
                None if next < self.params.len() => {
                    next += 1;
                    (next - 1, word.text.clone())
                }
                None => {
                    return Err((word.at, format!("`{}` is one argument too many", word.text)));
                }
            };
            if given[i].replace(value).is_some() {
                return Err((word.at, format!("`{}` is given twice", self.params[i].name)));
            }
        }
        let mut values: Vec<(String, String)> = Vec::new();
        for (param, value) in self.params.iter().zip(given) {
            let value = match (value, &param.default) {
                (Some(v), _) => v,
                (None, Some(default)) => fill(default, &values),
                (None, None) => {
                    return Err((at, format!("missing `{}`", param.name)));
                }
            };
            values.push((param.name.clone(), value));
        }
        Ok(self
            .body
            .iter()
            .map(|node| filled(node, &values, at))
            .collect())
    }

    /// what a call looks like, for messages.
    #[must_use]
    pub fn usage(&self, name: &str) -> String {
        let params = self.params.iter().map(|p| match &p.default {
            Some(_) => format!(" [{}=...]", p.name),
            None => format!(" <{}>", p.name),
        });
        std::iter::once(name.to_owned()).chain(params).collect()
    }
}

fn filled(node: &Node, values: &[(String, String)], at: usize) -> Node {
    Node {
        words: node
            .words
            .iter()
            .map(|w| Word {
                text: fill(&w.text, values),
                at,
                quoted: w.quoted,
            })
            .collect(),
        children: node
            .children
            .iter()
            .map(|c| filled(c, values, at))
            .collect(),
    }
}

/// every node moved to `at`, for lines from another file: their problems
/// point at the line that brought them in.
#[must_use]
pub fn moved(node: &Node, at: usize) -> Node {
    filled(node, &[], at)
}

/// `$name` replaced by its value and `$name.name` by the last part of
/// it; any other `$` stays.
fn fill(text: &str, values: &[(String, String)]) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(after.len());
        let Some((_, value)) = values.iter().find(|(name, _)| *name == after[..len]) else {
            out.push('$');
            rest = after;
            continue;
        };
        rest = &after[len..];
        if let Some(tail) = rest.strip_prefix(".name") {
            out.push_str(last_part(value));
            rest = tail;
        } else {
            out.push_str(value);
        }
    }
    out.push_str(rest);
    out
}

/// last path segment of a reference, after any `source:`.
fn last_part(reference: &str) -> &str {
    let path = reference.split_once(':').map_or(reference, |(_, p)| p);
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(src: &str) -> Action {
        let nodes = super::super::tree::parse("t", src).unwrap();
        Action::define(&nodes[0]).unwrap().1
    }

    fn call(a: &Action, args: &str) -> Result<Vec<Vec<String>>, String> {
        let line = format!("x {args}");
        let node = &super::super::tree::parse("t", &line).unwrap()[0];
        let flat = |n: &Node| n.words.iter().map(|w| w.text.clone()).collect::<Vec<_>>();
        a.call(node.args(), 7)
            .map(|nodes| nodes.iter().map(flat).collect())
            .map_err(|(_, e)| e)
    }

    #[test]
    fn calls_fill_in_parameters_and_defaults() {
        let a = action(
            "action steamvr dir to={app}/steamvr/$dir.name\n  copy $dir -> $to\n  json f add list $to\n",
        );
        let lines = call(&a, "motion:driver/build/aira").unwrap();
        assert_eq!(
            lines[0],
            [
                "copy",
                "motion:driver/build/aira",
                "->",
                "{app}/steamvr/aira"
            ]
        );
        assert_eq!(lines[1][4], "{app}/steamvr/aira");
        let lines = call(&a, "d to={data}/x").unwrap();
        assert_eq!(lines[0][3], "{data}/x");
        let lines = call(&a, "to={data}/x dir=d").unwrap();
        assert_eq!(lines[0][1], "d");
    }

    #[test]
    fn bad_calls_say_why() {
        let a = action("action a one two=2\n  copy $one $two $other\n");
        assert_eq!(call(&a, "").unwrap_err(), "missing `one`");
        assert!(call(&a, "1 2 3").unwrap_err().contains("too many"));
        assert!(call(&a, "1 one=2").unwrap_err().contains("twice"));
        assert_eq!(call(&a, "1").unwrap()[0], ["copy", "1", "2", "$other"]);
    }

    #[test]
    fn bad_definitions_say_why() {
        let define = |src: &str| {
            let nodes = super::super::tree::parse("t", src).unwrap();
            Action::define(&nodes[0]).map(|_| ()).map_err(|(_, e)| e)
        };
        assert!(define("action a\n").is_err(), "no lines");
        assert!(
            define("action a b=1 c\n  copy x\n")
                .unwrap_err()
                .contains("default")
        );
        assert!(define("action a b b\n  copy x\n").is_err());
        assert!(define("action a.b\n  copy x\n").is_err());
    }
}

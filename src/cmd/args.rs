/// positionals plus `--flag value` options, in any order. options listed
/// in `known` take one value and may repeat.
pub struct Args {
    pub positional: Vec<String>,
    options: Vec<(String, String)>,
    switches: Vec<String>,
}

impl Args {
    pub fn parse(args: &[String], known: &[&str]) -> Result<Self, String> {
        Self::parse_with(args, known, &[])
    }

    /// like `parse`, with `switches` that take no value, like `--force`.
    pub fn parse_with(args: &[String], known: &[&str], switches: &[&str]) -> Result<Self, String> {
        let mut positional = Vec::new();
        let mut options = Vec::new();
        let mut on = Vec::new();
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            if switches.contains(&arg.as_str()) {
                on.push(arg.clone());
            } else if let Some(flag) = known.iter().find(|k| **k == arg) {
                let value = it.next().ok_or_else(|| format!("`{flag}` needs a value"))?;
                options.push(((*flag).to_owned(), value.clone()));
            } else if arg.starts_with("--") {
                return Err(format!("unknown option `{arg}`"));
            } else {
                positional.push(arg.clone());
            }
        }
        Ok(Self {
            positional,
            options,
            switches: on,
        })
    }

    /// whether the switch `flag` was given.
    pub fn flag(&self, flag: &str) -> bool {
        self.switches.iter().any(|s| s == flag)
    }

    pub fn first(&self) -> Option<&str> {
        self.positional.first().map(String::as_str)
    }

    /// last value of `flag`.
    pub fn value<'a>(&'a self, flag: &'a str) -> Option<&'a str> {
        self.all(flag).last()
    }

    pub fn all<'a>(&'a self, flag: &'a str) -> impl DoubleEndedIterator<Item = &'a str> + 'a {
        self.options
            .iter()
            .filter(move |(k, _)| k == flag)
            .map(|(_, v)| v.as_str())
    }

    /// at most one positional.
    pub fn at_most_one(&self) -> Result<(), String> {
        match self.positional.as_slice() {
            [] | [_] => Ok(()),
            [_, extra, ..] => Err(format!("unexpected argument `{extra}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn options_anywhere() {
        let a = Args::parse(
            &args(&[
                "--target",
                "windows-x86_64",
                "app",
                "--target",
                "linux-x86_64",
            ]),
            &["--target"],
        )
        .unwrap();
        assert_eq!(a.first(), Some("app"));
        assert_eq!(a.all("--target").count(), 2);
        assert_eq!(a.value("--target"), Some("linux-x86_64"));
    }

    #[test]
    fn rejects_unknown_and_missing() {
        assert!(Args::parse(&args(&["--nope"]), &["--target"]).is_err());
        assert!(Args::parse(&args(&["--target"]), &["--target"]).is_err());
    }
}

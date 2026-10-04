//! `${name}` substitution in Kagofile paths and URLs. text only, no logic.

use std::collections::BTreeMap;

use crate::manifest::Manifest;
use crate::package::Target;

/// the variables a Kagofile can use for one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vars(BTreeMap<&'static str, String>);

impl Vars {
    #[must_use]
    pub fn new(manifest: &Manifest, target: Target) -> Self {
        Self(BTreeMap::from([
            ("id", manifest.package.id.to_string()),
            ("name", manifest.package.name.clone()),
            ("version", manifest.package.version.to_string()),
            ("os", target.os().to_string()),
            ("arch", target.arch().to_string()),
            (
                "libc",
                target.libc().map_or_else(String::new, |l| l.to_string()),
            ),
            ("target", target.to_string()),
            ("triple", target.triple().to_owned()),
            ("exe", target.exe_suffix().to_owned()),
        ]))
    }

    /// `text` with every `${name}` replaced. `$$` is a literal `$`.
    ///
    /// # Errors
    ///
    /// on an unknown name or an unclosed `${`.
    pub fn expand(&self, text: &str) -> Result<String, String> {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(i) = rest.find('$') {
            out.push_str(&rest[..i]);
            let after = &rest[i + 1..];
            if let Some(more) = after.strip_prefix('$') {
                out.push('$');
                rest = more;
            } else if let Some(open) = after.strip_prefix('{') {
                let close = open
                    .find('}')
                    .ok_or_else(|| format!("unclosed `${{` in `{text}`"))?;
                let name = &open[..close];
                let value = self.0.get(name).ok_or_else(|| {
                    format!(
                        "unknown variable `${{{name}}}`, expected one of {}",
                        self.0
                            .keys()
                            .map(|k| format!("${{{k}}}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
                out.push_str(value);
                rest = &open[close + 1..];
            } else {
                out.push('$');
                rest = after;
            }
        }
        out.push_str(rest);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::AUREA;

    #[test]
    fn expands_known_names() {
        let m = Manifest::parse("Kagofile", AUREA).0.unwrap();
        let v = Vars::new(&m, "windows-x86_64".parse().unwrap());
        assert_eq!(
            v.expand("https://x/v${version}/aurea-${triple}${exe}.zip")
                .unwrap(),
            "https://x/v1.4.0/aurea-x86_64-pc-windows-msvc.exe.zip"
        );
        assert_eq!(v.expand("cost $$5 and $x").unwrap(), "cost $5 and $x");
        assert!(v.expand("${nope}").unwrap_err().contains("${version}"));
        assert!(v.expand("${version").is_err());
    }
}

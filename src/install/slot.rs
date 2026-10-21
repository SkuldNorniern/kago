//! where a change really goes on this machine: its place with the roots,
//! variables and scope of one install filled in.

use std::path::PathBuf;

use super::{Roots, Scope};
use crate::manifest::{Place, Root, SemanticPath, Value};

/// a place resolved for one install.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "in", rename_all = "kebab-case")]
pub enum Slot {
    Json {
        file: PathBuf,
        key: Vec<String>,
    },
    /// `machine` is `HKLM`, else `HKCU`.
    Registry {
        machine: bool,
        key: String,
        name: String,
    },
    /// everyone's variable, else the user's.
    Environment {
        machine: bool,
        name: String,
    },
}

impl std::fmt::Display for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hive = |machine: bool| if machine { "HKLM" } else { "HKCU" };
        match self {
            Self::Json { file, key } => write!(f, "{} {}", file.display(), key.join(".")),
            Self::Registry { machine, key, name } => {
                write!(f, r"{}\{key} {name}", hive(*machine))
            }
            Self::Environment { name, .. } => write!(f, "%{name}%"),
        }
    }
}

/// `place` for an install in `scope`. `variable` reads an environment
/// variable for `%NAME%` in a file location.
///
/// # Errors
///
/// when a root or variable has no value here, or the place needs an
/// install for everyone and this one is not.
pub fn slot(
    place: &Place,
    scope: Scope,
    roots: &Roots,
    variable: &dyn Fn(&str) -> Option<String>,
) -> Result<Slot, String> {
    let machine = scope == Scope::Machine;
    match place {
        Place::Json { file, key } => Ok(Slot::Json {
            file: location(file, roots, variable)?,
            key: key.clone(),
        }),
        Place::Registry { key, name } => {
            let (hive, rest) = key.split_once('\\').unwrap_or((key, ""));
            let named = match hive.to_ascii_uppercase().as_str() {
                "HKCU" | "HKEY_CURRENT_USER" => Some(false),
                "HKLM" | "HKEY_LOCAL_MACHINE" => Some(true),
                _ => None,
            };
            let (in_machine, key) = match named {
                Some(m) => (m, rest.to_owned()),
                None => (machine, key.clone()),
            };
            if in_machine && !machine {
                return Err(format!(
                    r"HKLM\{key} needs an install for everyone; leave out `HKLM\` to follow the install"
                ));
            }
            Ok(Slot::Registry {
                machine: in_machine,
                key,
                name: name.clone(),
            })
        }
        Place::Environment { name } => Ok(Slot::Environment {
            machine,
            name: name.clone(),
        }),
    }
}

/// a file location written `{root}/...` or `%VARIABLE%/...`.
///
/// # Errors
///
/// when the root or variable has no value here.
pub fn location(
    text: &str,
    roots: &Roots,
    variable: &dyn Fn(&str) -> Option<String>,
) -> Result<PathBuf, String> {
    if text.starts_with('{') {
        let path: SemanticPath = text
            .parse()
            .map_err(|e: crate::manifest::PathError| e.to_string())?;
        return roots.resolve(&path);
    }
    let (name, rest) = text
        .strip_prefix('%')
        .and_then(|t| t.split_once('%'))
        .ok_or_else(|| format!("`{text}` starts with neither a root nor a variable"))?;
    let base = variable(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| format!("%{name}% is not set"))?;
    let mut out = base;
    out.extend(rest.split(['/', '\\']).filter(|s| !s.is_empty()));
    Ok(out)
}

/// `value` with the install's roots filled in: a whole `{root}/...` path
/// becomes the real path, and a root inside longer text its folder.
#[must_use]
pub fn fill(value: &Value, roots: &Roots) -> Value {
    let Some(text) = value.as_str() else {
        return value.clone();
    };
    Value::text(fill_text(text, roots))
}

/// as [`fill`], for text like a program argument.
#[must_use]
pub fn fill_text(text: &str, roots: &Roots) -> String {
    if let Ok(path) = text.parse::<SemanticPath>()
        && let Ok(real) = roots.resolve(&path)
    {
        return real.display().to_string();
    }
    // a root inside longer text, like `"{app}/x.exe" "%1"`: the path it
    // starts runs to a quote, space, comma or semicolon, and gets this
    // OS's separators, as programs reading it expect.
    let mut out = String::new();
    let mut rest = text;
    'scan: while let Some(open) = rest.find('{') {
        for root in Root::ALL {
            let token = format!("{{{}}}", root.as_str());
            if let Some(after) = rest[open..].strip_prefix(token.as_str())
                && let Some(dir) = roots.0.get(&root)
            {
                let end = after
                    .find(['"', '\'', ' ', ',', ';'])
                    .unwrap_or(after.len());
                out.push_str(&rest[..open]);
                out.push_str(&dir.display().to_string());
                out.push_str(&after[..end].replace('/', std::path::MAIN_SEPARATOR_STR));
                rest = &after[end..];
                continue 'scan;
            }
        }
        out.push_str(&rest[..=open]);
        rest = &rest[open + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn roots() -> Roots {
        let base = std::env::temp_dir().join("kago-slot");
        Roots(BTreeMap::from([
            (Root::App, base.join("app")),
            (Root::Config, base.join("config")),
        ]))
    }

    fn env(name: &str) -> Option<String> {
        (name == "HOME").then(|| std::env::temp_dir().display().to_string())
    }

    #[test]
    fn locations_take_roots_and_variables() {
        let r = roots();
        let json = |file: &str| Place::Json {
            file: file.into(),
            key: vec!["k".into()],
        };
        let Slot::Json { file, .. } =
            slot(&json("{config}/a.json"), Scope::User, &r, &env).unwrap()
        else {
            panic!()
        };
        assert_eq!(file, r.0[&Root::Config].join("a.json"));
        let Slot::Json { file, .. } =
            slot(&json("%HOME%/x/a.json"), Scope::User, &r, &env).unwrap()
        else {
            panic!()
        };
        assert_eq!(file, std::env::temp_dir().join("x").join("a.json"));
        assert!(slot(&json("%NOPE%/a.json"), Scope::User, &r, &env).is_err());
    }

    #[test]
    fn registry_follows_the_install_unless_named() {
        let r = roots();
        let reg = |key: &str| Place::Registry {
            key: key.into(),
            name: "v".into(),
        };
        let machine = |s: Slot| matches!(s, Slot::Registry { machine: true, .. });
        assert!(!machine(
            slot(&reg(r"Software\X"), Scope::User, &r, &env).unwrap()
        ));
        assert!(machine(
            slot(&reg(r"Software\X"), Scope::Machine, &r, &env).unwrap()
        ));
        assert!(!machine(
            slot(&reg(r"HKCU\Software\X"), Scope::Machine, &r, &env).unwrap()
        ));
        assert!(slot(&reg(r"HKLM\Software\X"), Scope::User, &r, &env).is_err());
        let Slot::Registry { key, .. } =
            slot(&reg(r"HKLM\Software\X"), Scope::Machine, &r, &env).unwrap()
        else {
            panic!()
        };
        assert_eq!(key, r"Software\X");
    }

    #[test]
    fn values_get_real_paths() {
        let r = roots();
        let app = r.0[&Root::App].clone();
        assert_eq!(
            fill(&Value::text("{app}/steamvr/aira"), &r),
            Value::text(app.join("steamvr").join("aira").display().to_string())
        );
        assert_eq!(
            fill_text("--home={app}", &r),
            format!("--home={}", app.display())
        );
        assert_eq!(fill(&Value::word("3", false), &r), Value::word("3", false));
        let exe = app.join("bin").join("aira.exe").display().to_string();
        assert_eq!(
            fill_text("\"{app}/bin/aira.exe\" \"%1\" {nope}/x", &r),
            format!("\"{exe}\" \"%1\" {{nope}}/x")
        );
    }
}

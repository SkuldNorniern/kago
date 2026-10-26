//! `json`, `registry` and `env`: values the install sets in places it does
//! not own, like another program's settings file. each is put back at
//! uninstall.

use super::policy::keywords;
use super::selector::Selector;
use crate::diagnostic::Location;

/// where a value lives.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "in", rename_all = "kebab-case")]
pub enum Place {
    /// a key in a JSON file. `file` starts with a root like `{config}` or a
    /// variable like `%LOCALAPPDATA%`; `key` is the path of names into it.
    Json { file: String, key: Vec<String> },
    /// a registry value. `key` starts with `HKCU\` or `HKLM\`, or with
    /// neither to follow the install: `HKCU` per user, `HKLM` for everyone.
    /// an empty `name` is the key's default value.
    Registry { key: String, name: String },
    /// an environment variable, the user's or everyone's as the install is.
    Environment { name: String },
}

impl Place {
    /// how the Kagofile names this kind of place.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Json { .. } => "json",
            Self::Registry { .. } => "registry",
            Self::Environment { .. } => "env",
        }
    }
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json { file, key } => write!(f, "{file} {}", key.join(".")),
            Self::Registry { key, name } if name.is_empty() => write!(f, "{key} (default)"),
            Self::Registry { key, name } => write!(f, "{key} {name}"),
            Self::Environment { name } => write!(f, "%{name}%"),
        }
    }
}

keywords! {
    /// what a change does to its place. `set` sets the value, uninstall
    /// restores it; `add` adds a list item once, uninstall removes it; `init`
    /// sets a value only when there is none and keeps it at uninstall.
    Edit { Set => "set", Add => "add", Init => "init" }
}

/// a value as JSON: text, a number, true or false, or whatever a file held
/// before. `{app}` and the other roots in text become real paths at
/// install.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Value(pub serde_json::Value);

// serde_json never holds NaN, so equality is total.
impl Eq for Value {}

impl Value {
    /// a Kagofile word: a whole number, `true` or `false` when unquoted,
    /// text otherwise.
    #[must_use]
    pub fn word(text: &str, quoted: bool) -> Self {
        if !quoted {
            if let Ok(n) = text.parse::<i64>() {
                return Self(n.into());
            }
            match text {
                "true" => return Self(true.into()),
                "false" => return Self(false.into()),
                _ => {}
            }
        }
        Self(text.into())
    }

    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self(serde_json::Value::String(text.into()))
    }

    /// the text, when it is text.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        self.0.as_str()
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            serde_json::Value::String(s) => f.write_str(s),
            other => write!(f, "{other}"),
        }
    }
}

/// one `set`, `add` or `init` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub place: Place,
    pub edit: Edit,
    pub value: Value,
    pub component: Option<String>,
    pub when: Selector,
    pub at: Location,
}

//! `check`: how to tell whether something is on the machine, like a
//! runtime a component is for or a prerequisite installed already.

/// a test of the machine.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Check {
    /// a registry key, like `HKLM\SOFTWARE\...`, and optionally a value in
    /// it that has to be there and not 0 or empty.
    Registry { key: String, value: Option<String> },
    /// a file, with `%VARIABLES%` like `%ProgramFiles%` expanded.
    File { path: String },
    /// a key in a JSON file that holds something: not missing, null,
    /// false, 0, empty text or an empty list.
    Json { file: String, key: Vec<String> },
}

/// a check with the name people know the thing by, like `SteamVR`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Detect {
    pub name: String,
    pub check: Check,
}

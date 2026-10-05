//! `prerequisite`: another vendor's installer the package needs on the
//! machine first, like the Visual C++ runtime. it ships inside the setup
//! and runs only when its check says it is missing.

use super::Check;
use crate::diagnostic::Location;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prerequisite {
    pub id: String,
    /// shown while it installs.
    pub name: String,
    /// build-side installer, like `vcredist:vc_redist.x64.exe`.
    pub file: String,
    pub arguments: Vec<String>,
    pub check: Check,
    /// its installer needs administrator rights.
    pub admin: bool,
    pub at: Location,
}

/// what kago fills in for a prerequisite it knows: name, arguments,
/// check and whether it needs administrator rights. the file always comes
/// from the Kagofile, pinned by its sha256.
#[must_use]
pub fn preset(id: &str) -> Option<(&'static str, Vec<String>, Check, bool)> {
    let args = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect();
    match id {
        // the Visual C++ 2015-2022 runtime, x64. a newer one installed
        // already makes its installer exit with 1638, which counts as done.
        "vcredist" => Some((
            "Microsoft Visual C++ Runtime",
            args(&["/install", "/quiet", "/norestart"]),
            Check::Registry {
                key: r"HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64".to_owned(),
                value: Some("Installed".to_owned()),
            },
            true,
        )),
        // the Evergreen WebView2 runtime, through its bootstrapper.
        "webview2" => Some((
            "Microsoft Edge WebView2 Runtime",
            args(&["/silent", "/install"]),
            Check::Registry {
                key: r"HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}".to_owned(),
                value: Some("pv".to_owned()),
            },
            true,
        )),
        _ => None,
    }
}

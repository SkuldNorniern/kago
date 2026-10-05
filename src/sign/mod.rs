//! Authenticode signing for `kago build`, through the Windows SDK's
//! signtool: the setup, the runtime it installs as the uninstaller, and
//! the package's own programs. see `Docs/authenticode`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::manifest::{Sign, SignKey};

/// signs files with one key, as the Kagofile says.
#[derive(Debug)]
pub struct Signer {
    tool: PathBuf,
    /// where the key is, as signtool arguments.
    key: Vec<String>,
    timestamp: String,
}

impl Signer {
    /// finds signtool and reads the key's settings, like a password from
    /// the environment. relative files are under `base`.
    ///
    /// # Errors
    ///
    /// when signtool is missing or a setting cannot be read.
    pub fn new(sign: &Sign, base: &Path) -> Result<Self, String> {
        let tool = signtool().ok_or(
            "signtool not found; install the Windows SDK, or set KAGO_SIGNTOOL to signtool.exe",
        )?;
        let key = match &sign.key {
            SignKey::Certificate {
                thumbprint,
                machine,
            } => {
                let mut key = vec!["/sha1".to_owned(), thumbprint.value()?.replace(' ', "")];
                if *machine {
                    key.push("/sm".to_owned());
                }
                key
            }
            SignKey::Pfx { file, password } => {
                let mut key = vec!["/f".to_owned(), base.join(file).display().to_string()];
                if let Some(name) = password {
                    let password = std::env::var(name)
                        .map_err(|_| format!("environment variable {name} is not set"))?;
                    key.extend(["/p".to_owned(), password]);
                }
                key
            }
            SignKey::Azure { metadata, dlib } => vec![
                "/dlib".to_owned(),
                dlib.clone(),
                "/dmdf".to_owned(),
                base.join(metadata).display().to_string(),
            ],
        };
        Ok(Self {
            tool,
            key,
            timestamp: sign.timestamp.clone(),
        })
    }

    /// signs `file` in place. `description` is what UAC shows as the
    /// program's name. timestamp servers are flaky, so it tries thrice.
    ///
    /// # Errors
    ///
    /// with signtool's own message when signing fails.
    pub fn sign(&self, file: &Path, description: &str, url: Option<&str>) -> Result<(), String> {
        let mut args: Vec<String> = ["sign", "/fd", "SHA256", "/td", "SHA256", "/tr"]
            .map(str::to_owned)
            .into();
        args.push(self.timestamp.clone());
        args.extend(self.key.iter().cloned());
        args.extend(["/d".to_owned(), description.to_owned()]);
        if let Some(url) = url {
            args.extend(["/du".to_owned(), url.to_owned()]);
        }
        args.push(file.display().to_string());
        let mut last = String::new();
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_secs(2));
            }
            let out = Command::new(&self.tool)
                .args(&args)
                .output()
                .map_err(|e| format!("cannot run {}: {e}", self.tool.display()))?;
            if out.status.success() {
                return Ok(());
            }
            last = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Err(format!(
            "signing {} failed: {}",
            file.display(),
            last.trim().replace(self.key_secret(), "***")
        ))
    }

    /// the password, so messages never show it.
    fn key_secret(&self) -> &str {
        self.key
            .iter()
            .position(|a| a == "/p")
            .and_then(|i| self.key.get(i + 1))
            .map_or("\u{0}", String::as_str)
    }

    /// whether Windows trusts the signature on `file`: false for a test
    /// certificate that does not chain to a trusted root.
    #[must_use]
    pub fn trusted(&self, file: &Path) -> bool {
        Command::new(&self.tool)
            .args(["verify", "/pa", "/q"])
            .arg(file)
            .output()
            .is_ok_and(|o| o.status.success())
    }
}

/// signtool: `KAGO_SIGNTOOL`, else the newest Windows SDK's, else PATH.
fn signtool() -> Option<PathBuf> {
    if let Some(tool) = std::env::var_os("KAGO_SIGNTOOL") {
        return Some(PathBuf::from(tool));
    }
    let kits = std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .map(|p| p.join("Windows Kits").join("10").join("bin"));
    let newest = kits.and_then(|kits| {
        let mut versions: Vec<PathBuf> = std::fs::read_dir(kits)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.path().join("x64").join("signtool.exe"))
            .filter(|p| p.is_file())
            .collect();
        versions.sort_by_key(|p| sdk_version(p));
        versions.pop()
    });
    newest.or_else(|| {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join("signtool.exe"))
                .find(|p| p.is_file())
        })
    })
}

/// `10.0.26100.0` out of `...\bin\10.0.26100.0\x64\signtool.exe`, as
/// numbers, so 10.0.26100 sorts after 10.0.9600.
fn sdk_version(signtool: &Path) -> Vec<u32> {
    signtool
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .map(|v| v.split('.').filter_map(|p| p.parse().ok()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_sdks_sort_last() {
        let v = |s: &str| sdk_version(&PathBuf::from(format!("C:/k/bin/{s}/x64/signtool.exe")));
        assert!(v("10.0.26100.0") > v("10.0.9600.0"));
        assert_eq!(v("10.0.22621.0"), [10, 0, 22621, 0]);
    }
}

//! updates from where releases are published. `kago build` writes a
//! `latest.json` next to the setup; an installed package remembers that
//! file's URL, and `kago update` reads it, downloads the newer setup,
//! checks its sha256 and runs it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::package::{PackageId, Target, Version};

/// file name of the feed `kago build` writes.
pub const FEED: &str = "latest.json";

/// `latest.json`: the newest version and its setup for each target.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Feed {
    pub format: u32,
    pub package: PackageId,
    pub version: Version,
    /// by target, like `windows-x86_64`.
    pub setups: BTreeMap<String, Setup>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Setup {
    /// a file name next to the feed, or a full URL.
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

/// a newer version to install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub version: Version,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

impl Feed {
    /// a feed for one setup. `kago build` merges it into any feed of the
    /// same version already in the output folder, so one file covers
    /// every target.
    #[must_use]
    pub fn new(package: PackageId, version: Version, target: Target, setup: Setup) -> Self {
        Self {
            format: 1,
            package,
            version,
            setups: BTreeMap::from([(target.to_string(), setup)]),
        }
    }

    /// what to install over `installed` on `target`, from a feed found at
    /// `at` (after redirects). `None` when nothing newer is there.
    ///
    /// # Errors
    ///
    /// when the feed is for another package, or newer but without a setup
    /// for this target.
    pub fn offer(
        &self,
        package: &PackageId,
        installed: &Version,
        target: Target,
        at: &str,
    ) -> Result<Option<Offer>, String> {
        if &self.package != package {
            return Err(format!(
                "the update feed is for {}, not {package}",
                self.package
            ));
        }
        if self.version.cmp_precedence(installed) != std::cmp::Ordering::Greater {
            return Ok(None);
        }
        let setup = self
            .setups
            .get(&target.to_string())
            .ok_or_else(|| format!("{package} {} has no setup for {target} yet", self.version))?;
        Ok(Some(Offer {
            version: self.version.clone(),
            url: next_to(at, &setup.file),
            sha256: setup.sha256.clone(),
            size: setup.size,
        }))
    }
}

/// `file` next to the URL `at`, unless it is a URL itself.
fn next_to(at: &str, file: &str) -> String {
    if file.contains("://") {
        return file.to_owned();
    }
    match at.rfind('/') {
        Some(slash) => format!("{}/{file}", &at[..slash]),
        None => file.to_owned(),
    }
}

/// reads the feed at `url` into `dir`. returns it with the URL it really
/// came from: GitHub's `releases/latest` redirects to the release itself,
/// and the setup has to come from that same release.
///
/// # Errors
///
/// when the download fails or the file is not a feed.
pub fn read(url: &str, dir: &Path) -> Result<(Feed, String), String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join(FEED);
    let out = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--write-out",
            "%{url_effective}",
            "--output",
        ])
        .arg(&file)
        .arg(url)
        .output()
        .map_err(|e| format!("cannot run curl to read {url}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cannot read the update feed {url}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let at = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let text = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let feed =
        serde_json::from_str(&text).map_err(|e| format!("{url} is not an update feed: {e}"))?;
    Ok((feed, if at.is_empty() { url.to_owned() } else { at }))
}

/// downloads `offer` into `dir`, checked against its sha256.
///
/// # Errors
///
/// when the download fails or does not match.
pub fn download(offer: &Offer, dir: &Path) -> Result<PathBuf, String> {
    let file = crate::payload::fetch(&offer.url, Some(&offer.sha256), dir, "")?;
    let name = offer.url.rsplit('/').next().unwrap_or("setup.exe");
    let named = dir.join(name);
    fs::rename(&file, &named).map_err(|e| format!("{}: {e}", named.display()))?;
    Ok(named)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(version: &str) -> Feed {
        Feed::new(
            PackageId::new("dev.eira.ozone").unwrap(),
            version.parse().unwrap(),
            "windows-x86_64".parse().unwrap(),
            Setup {
                file: "Ozone-1.2.0-setup-windows-x86_64.exe".into(),
                sha256: "ab".repeat(32),
                size: 10,
            },
        )
    }

    #[test]
    fn offers_only_newer_versions_from_the_same_release() {
        let id = PackageId::new("dev.eira.ozone").unwrap();
        let target = "windows-x86_64".parse().unwrap();
        let at = "https://github.com/o/r/releases/download/v1.2.0/latest.json";
        let installed: Version = "1.1.0".parse().unwrap();
        let offer = feed("1.2.0")
            .offer(&id, &installed, target, at)
            .unwrap()
            .unwrap();
        assert_eq!(
            offer.url,
            "https://github.com/o/r/releases/download/v1.2.0/Ozone-1.2.0-setup-windows-x86_64.exe"
        );
        assert_eq!(feed("1.1.0").offer(&id, &installed, target, at), Ok(None));
        assert_eq!(feed("1.0.0").offer(&id, &installed, target, at), Ok(None));
        let other = PackageId::new("dev.eira.other").unwrap();
        assert!(feed("1.2.0").offer(&other, &installed, target, at).is_err());
        let arm = "windows-aarch64".parse().unwrap();
        assert!(feed("1.2.0").offer(&id, &installed, arm, at).is_err());
    }

    #[test]
    fn full_urls_stay() {
        assert_eq!(
            next_to("https://a/b/latest.json", "https://c/d.exe"),
            "https://c/d.exe"
        );
        assert_eq!(
            next_to("https://a/b/latest.json", "d.exe"),
            "https://a/b/d.exe"
        );
    }

    #[test]
    fn feed_json_round_trips() {
        let f = feed("1.2.0");
        let text = serde_json::to_string(&f).unwrap();
        assert!(text.contains("\"version\":\"1.2.0\""));
        assert_eq!(serde_json::from_str::<Feed>(&text).unwrap(), f);
    }
}

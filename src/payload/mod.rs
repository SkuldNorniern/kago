//! where file sources are on disk for one build: plain paths next to the
//! Kagofile, `name:path` through a declared source, `${...}` expanded for
//! the target. fetched sources are downloaded, checked and unpacked under
//! `.kago/` next to the Kagofile.

mod fetch;
mod unpack;
mod vars;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub use fetch::{fetch, file_url};
pub use vars::Vars;

use crate::install::Sources;
use crate::manifest::{Manifest, SourceKind};
use crate::package::Target;

pub struct Payload<'a> {
    manifest: &'a Manifest,
    base: PathBuf,
    target: Target,
    vars: Vars,
    /// source name to its prepared directory, filled on first use.
    ready: RefCell<BTreeMap<String, PathBuf>>,
}

impl<'a> Payload<'a> {
    #[must_use]
    pub fn new(manifest: &'a Manifest, base: &Path, target: Target) -> Self {
        Self {
            manifest,
            base: base.to_path_buf(),
            target,
            vars: Vars::new(manifest, target),
            ready: RefCell::new(BTreeMap::new()),
        }
    }

    /// directory a source's paths are relative to, prepared once.
    fn root(&self, name: &str) -> Result<PathBuf, String> {
        if let Some(dir) = self.ready.borrow().get(name) {
            return Ok(dir.clone());
        }
        let source = self
            .manifest
            .source(name)
            .ok_or_else(|| format!("unknown source `{name}`"))?;
        let dir = match &source.kind {
            SourceKind::Dir(path) => self.base.join(self.vars.expand(path)?),
            SourceKind::Fetch(url) => {
                let url = self.vars.expand(url)?;
                let target = self.target.to_string();
                let expected = source
                    .sha256
                    .get(&Some(target.clone()))
                    .or_else(|| source.sha256.get(&None))
                    .map(String::as_str);
                let kago = self.base.join(".kago");
                let file = fetch::fetch(&url, expected, &kago.join("cache"), &target)?;
                let digest = file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_owned();
                let dir = kago.join("sources").join(name).join(&target);
                let marker = dir.join(".kago-sha256");
                if fs::read_to_string(&marker).ok().as_deref() != Some(digest.as_str()) {
                    let _ = fs::remove_dir_all(&dir);
                    let named = file.with_file_name(url.rsplit('/').next().unwrap_or("download"));
                    // keep the URL's file name for `unpack none`.
                    let unpack_as = source.unpack.for_url(&url);
                    let from = if unpack_as == crate::manifest::Unpack::None {
                        fs::copy(&file, &named).map_err(|e| e.to_string())?;
                        named
                    } else {
                        file.clone()
                    };
                    unpack::unpack(&from, unpack_as, &dir)
                        .map_err(|e| format!("unpacking source `{name}`: {e}"))?;
                    fs::write(&marker, &digest).map_err(|e| e.to_string())?;
                }
                dir
            }
        };
        self.ready.borrow_mut().insert(name.to_owned(), dir.clone());
        Ok(dir)
    }
}

impl Sources for Payload<'_> {
    fn locate(&self, reference: &str) -> Result<PathBuf, String> {
        let reference = self.vars.expand(reference)?;
        match reference.split_once(':') {
            Some((name, path)) if self.manifest.source(name).is_some() => {
                Ok(self.root(name)?.join(path))
            }
            _ => Ok(self.base.join(reference)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kago-payload-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn dir_sources_and_vars() {
        let dir = temp("dir");
        let src = "package dev.x.tool\n  version 2.0.0\nsource build\n  dir out/${triple}\napp tool\n  windows build:tool${exe}\n";
        let m = Manifest::parse("Kagofile", src).0.unwrap();
        let p = Payload::new(&m, &dir, "windows-x86_64".parse().unwrap());
        assert_eq!(
            p.locate("build:tool${exe}").unwrap(),
            dir.join("out/x86_64-pc-windows-msvc").join("tool.exe")
        );
        assert_eq!(p.locate("docs/${version}").unwrap(), dir.join("docs/2.0.0"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fetch_checks_and_unpacks() {
        if std::process::Command::new("curl")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = temp("fetch");
        // a zip to "download" over file://
        let zip_path = dir.join("tool-2.0.0.zip");
        let mut w = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
        w.start_file("tool.exe", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut w, b"tool").unwrap();
        w.finish().unwrap();
        let (_, digest) = crate::install::hash(&zip_path).unwrap();
        let url = file_url(&dir.join("tool-${version}.zip"));

        let manifest = |sha: &str| {
            format!(
                "package dev.x.tool\n  version 2.0.0\nsource release\n  fetch {url}\n  sha256 {sha}\n  unpack zip\napp tool\n  windows release:tool.exe\n"
            )
        };
        let m = Manifest::parse("Kagofile", &manifest(&digest.to_string()))
            .0
            .unwrap();
        let p = Payload::new(&m, &dir, "windows-x86_64".parse().unwrap());
        let tool = p.locate("release:tool.exe").unwrap();
        assert_eq!(fs::read(tool).unwrap(), b"tool");

        let wrong = "0".repeat(64);
        let m = Manifest::parse("Kagofile", &manifest(&wrong)).0.unwrap();
        let p = Payload::new(&m, &dir, "windows-x86_64".parse().unwrap());
        let e = p.locate("release:tool.exe").unwrap_err();
        assert!(e.contains("does not match"), "{e}");
        fs::remove_dir_all(dir).unwrap();
    }
}

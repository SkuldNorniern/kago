use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use super::operation::{Digest, Operation};
use super::{Resolved, Scope};
use crate::diagnostic::{self, Diagnostic, Diagnostics};
use crate::manifest::{Downgrade, File, Manifest, SemanticPath};
use crate::package::{PackageId, Target, Version};

/// everything one install does, in order. same manifest, target, scope,
/// components and payload give the same plan.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InstallPlan {
    pub package: PackageId,
    pub name: String,
    pub version: Version,
    pub publisher: Option<String>,
    pub target: Target,
    pub scope: Scope,
    pub components: Vec<String>,
    /// whether this may replace a newer installed version.
    pub downgrade: Downgrade,
    pub operations: Vec<Operation>,
}

impl InstallPlan {
    /// turns a resolution into concrete steps. reads the payload under
    /// `base` (the manifest directory) to expand directories and hash files.
    #[must_use]
    pub fn build(
        manifest: &Manifest,
        resolved: &Resolved,
        target: Target,
        scope: Scope,
        base: &Path,
    ) -> (Option<Self>, Diagnostics) {
        let mut diagnostics = Diagnostics::default();
        let mut files = Vec::new();
        for f in &resolved.files {
            if let Err(d) = payload(f, base, &mut files) {
                diagnostics.push(d);
            }
        }
        files.sort_by(|a, b| a.target.cmp(&b.target));

        let mut directories = BTreeSet::new();
        for f in &files {
            let mut dir = f.target.parent();
            while !dir.relative().is_empty() && directories.insert(dir.clone()) {
                dir = dir.parent();
            }
        }

        let mut operations: Vec<Operation> = directories
            .into_iter()
            .map(|path| Operation::CreateDirectory { path })
            .collect();
        operations.extend(files.into_iter().map(|f| Operation::InstallFile {
            source: f.source,
            target: f.target,
            size: f.size,
            sha256: f.sha256,
            ownership: f.ownership,
        }));
        operations.extend(resolved.paths.iter().map(|p| Operation::AddPathEntry {
            directory: p.directory.clone(),
            scope: scope.of(p.scope),
        }));
        operations.extend(
            resolved
                .shortcuts
                .iter()
                .map(|s| Operation::CreateShortcut {
                    name: s.shortcut.name.clone(),
                    target: s.target.clone(),
                    location: s.shortcut.location,
                    arguments: s.shortcut.arguments.clone(),
                }),
        );
        operations.extend(resolved.services.iter().map(|s| Operation::InstallService {
            id: s.service.id.clone(),
            command: s.command.clone(),
            arguments: s.service.arguments.clone(),
            scope: scope.of(s.service.scope),
        }));
        let entrypoint = manifest.application.entrypoint.as_ref().and_then(|id| {
            resolved
                .files
                .iter()
                .find(|f| f.logical_id.as_ref() == Some(id))
                .map(|f| f.target.clone())
        });
        operations.push(Operation::RegisterApplication {
            display_name: manifest.application.display_name.clone(),
            entrypoint,
        });

        if diagnostics.has_errors() {
            return (None, diagnostics);
        }
        let plan = Self {
            package: manifest.package.id.clone(),
            name: manifest.package.name.clone(),
            version: manifest.package.version.clone(),
            publisher: manifest.package.publisher.clone(),
            target,
            scope,
            components: resolved.components.iter().map(str::to_owned).collect(),
            downgrade: manifest.install.downgrade,
            operations,
        };
        (Some(plan), diagnostics)
    }

    /// total payload bytes.
    #[must_use]
    pub fn payload_size(&self) -> u64 {
        self.operations
            .iter()
            .map(|op| match op {
                Operation::InstallFile { size, .. } => *size,
                _ => 0,
            })
            .sum()
    }
}

impl fmt::Display for InstallPlan {
    /// dry-run listing.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} {} ({})", self.name, self.version, self.package)?;
        writeln!(f, "target: {}", self.target)?;
        writeln!(f, "scope: {}", self.scope)?;
        writeln!(f, "components: {}", self.components.join(", "))?;
        writeln!(f, "payload: {} bytes", self.payload_size())?;
        writeln!(f)?;
        for op in &self.operations {
            writeln!(f, "  {op}")?;
        }
        Ok(())
    }
}

/// one payload file after directory expansion.
struct Payload {
    source: PathBuf,
    target: SemanticPath,
    size: u64,
    sha256: Digest,
    ownership: crate::manifest::Ownership,
}

/// `file` as payload entries. a directory source becomes every file under
/// it, sorted, mapped below `file.target`.
fn payload(file: &File, base: &Path, out: &mut Vec<Payload>) -> Result<(), Diagnostic> {
    let source = base.join(&file.source);
    let error = |message: String| {
        Diagnostic::error(diagnostic::INSTALL_SOURCE, message).at(file.at.clone())
    };
    let meta = fs::metadata(&source)
        .map_err(|e| error(format!("cannot read payload `{}`: {e}", file.source)))?;
    if meta.is_file() {
        out.push(entry(&source, file.target.clone(), file)?);
        return Ok(());
    }
    let mut stack = vec![(source, file.target.clone())];
    while let Some((dir, target)) = stack.pop() {
        let mut children: Vec<_> = fs::read_dir(&dir)
            .and_then(Iterator::collect::<io::Result<Vec<_>>>)
            .map_err(|e| error(format!("cannot list `{}`: {e}", dir.display())))?;
        children.sort_by_key(fs::DirEntry::file_name);
        for child in children {
            let name = child.file_name();
            let Some(name) = name.to_str().filter(|n| crate::manifest::valid_segment(n)) else {
                return Err(error(format!(
                    "payload name `{}` under `{}` is not portable",
                    name.to_string_lossy(),
                    dir.display()
                )));
            };
            let path = child.path();
            let child_target = target.join(name);
            if path.is_dir() {
                stack.push((path, child_target));
            } else {
                out.push(entry(&path, child_target, file)?);
            }
        }
    }
    Ok(())
}

fn entry(source: &Path, target: SemanticPath, file: &File) -> Result<Payload, Diagnostic> {
    let (size, sha256) = hash(source).map_err(|e| {
        Diagnostic::error(
            diagnostic::INSTALL_SOURCE,
            format!("cannot read payload `{}`: {e}", source.display()),
        )
        .at(file.at.clone())
    })?;
    Ok(Payload {
        source: source.to_path_buf(),
        target,
        size,
        sha256,
        ownership: file.ownership,
    })
}

/// size and sha-256 of a file, streamed.
///
/// # Errors
///
/// when the file cannot be read.
pub fn hash(path: &Path) -> io::Result<(u64, Digest)> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    let mut size = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((size, Digest(hasher.finalize().into())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::{Selection, resolve};
    use crate::manifest::{Context, tests::AUREA};

    /// temp payload tree matching the AUREA example.
    fn payload_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kago-plan-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dist/windows-x86_64")).unwrap();
        fs::create_dir_all(dir.join("themes/dark")).unwrap();
        fs::write(dir.join("dist/windows-x86_64/aurea.exe"), b"MZ fake").unwrap();
        fs::write(dir.join("themes/light.toml"), b"light").unwrap();
        fs::write(dir.join("themes/dark/main.toml"), b"dark").unwrap();
        dir
    }

    fn plan(dir: &Path) -> (Option<InstallPlan>, Diagnostics) {
        let m = Manifest::parse("Kagofile", AUREA).0.unwrap();
        let target: Target = "windows-x86_64".parse().unwrap();
        let r = resolve(&m, &Context::target(target), Selection::defaults(&m))
            .0
            .unwrap();
        InstallPlan::build(&m, &r, target, Scope::User, dir)
    }

    #[test]
    fn builds_ordered_plan() {
        let dir = payload_dir("ordered");
        let (p, d) = plan(&dir);
        assert!(d.is_empty(), "{d:?}");
        let p = p.unwrap();
        let lines: Vec<String> = p.operations.iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "mkdir     {app}/bin",
                "mkdir     {app}/themes",
                "mkdir     {app}/themes/dark",
                "file      {app}/bin/aurea.exe (7 bytes)",
                "file      {app}/themes/dark/main.toml (4 bytes)",
                "file      {app}/themes/light.toml (5 bytes)",
                "path      {app}/bin (user)",
                "shortcut  Aurea -> {app}/bin/aurea.exe (applications)",
                "register  Aurea",
            ]
        );
        assert_eq!(p.payload_size(), 16);
        assert_eq!(p.components, ["cli", "core"]);
        // same input, same plan.
        assert_eq!(plan(&dir).0.unwrap(), p);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hashes_payload() {
        let dir = payload_dir("hash");
        let (size, digest) = hash(&dir.join("themes/light.toml")).unwrap();
        assert_eq!(size, 5);
        assert_eq!(
            digest.to_string(),
            "99a7026172d42714d0293e5598ec1c8e8260d7d0c4b8582ad002c1883494e216"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_payload_is_an_error() {
        let dir = payload_dir("missing");
        fs::remove_file(dir.join("dist/windows-x86_64/aurea.exe")).unwrap();
        let (p, d) = plan(&dir);
        assert!(p.is_none());
        assert_eq!(d.iter().next().unwrap().code, diagnostic::INSTALL_SOURCE);
        fs::remove_dir_all(dir).unwrap();
    }
}

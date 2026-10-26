//! Kago Package Container v1: one target's install plan plus its payload.
//!
//! ```text
//! "KAGOPKG1"  u64 header length  header json  payload bytes...
//! ```
//!
//! the header holds the plan with each file source replaced by its payload
//! index, and the offset, packed size, size and sha-256 of every payload
//! entry. each entry is one zstd frame. nothing in it is an absolute install
//! path; roots are resolved at install time.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::install::{Digest, InstallPlan, Operation, hash};
use crate::manifest::{FolderChoice, SemanticPath, SetupLook};

pub const MAGIC: &[u8; 8] = b"KAGOPKG1";
pub const FORMAT: u32 = 1;

/// more than any real plan needs.
const MAX_HEADER: u64 = 64 << 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub format: u32,
    pub plan: InstallPlan,
    pub payload: Vec<Entry>,
    #[serde(default)]
    pub setup: Setup,
    /// files the setup window uses, stored as they are after the payload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<Asset>,
}

/// a named file for the setup window, like the logo PNG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    /// from the start of the payload.
    pub offset: u64,
    /// bytes in the container, zstd compressed.
    pub size: u64,
}

/// how the setup is branded; images are assets named `logo`, `header` and
/// `side`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupBrand {
    pub text: Option<String>,
    pub accent: Option<[u8; 3]>,
    pub welcome: Option<String>,
    pub finish: Option<String>,
    pub link: Option<(String, String)>,
}

/// what the setup window shows besides the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Setup {
    pub look: SetupLook,
    pub license: Option<LicenseText>,
    /// whether the setup offers a folder choice.
    #[serde(default = "choose")]
    pub folder: FolderChoice,
    #[serde(default)]
    pub brand: SetupBrand,
    /// what the package is, in a line, shown under its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

const fn choose() -> FolderChoice {
    FolderChoice::Choose
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            look: SetupLook::Modern,
            license: None,
            folder: FolderChoice::Choose,
            brand: SetupBrand::default(),
            description: None,
        }
    }
}

/// the license to show before installing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseText {
    pub text: String,
    /// whether installing needs the user to accept it.
    pub accept: bool,
}

/// one payload file, relative to the end of the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub offset: u64,
    /// bytes in the container, zstd compressed.
    pub packed: u64,
    /// bytes once unpacked.
    pub size: u64,
    pub sha256: Digest,
}

/// zstd level for payload. high, since builds run once and installs many times.
const LEVEL: i32 = 19;

/// writes `plan`, its payload and the setup window's `assets`. returns the
/// bytes written.
///
/// # Errors
///
/// when a payload file cannot be read or `out` cannot be written.
pub fn write(
    plan: &InstallPlan,
    setup: &Setup,
    assets: &[(String, Vec<u8>)],
    out: &mut impl Write,
) -> io::Result<u64> {
    let mut plan = plan.clone();
    // their installers go in as assets; build paths stay out.
    for p in &mut plan.prerequisites {
        p.source = PathBuf::new();
    }
    let mut packed = Vec::new();
    let mut payload = Vec::new();
    let mut offset = 0;
    for op in &mut plan.operations {
        if let Operation::InstallFile {
            source,
            size,
            sha256,
            ..
        } = op
        {
            let index = PathBuf::from(payload.len().to_string());
            let bytes = zstd::encode_all(fs::File::open(&*source)?, LEVEL)?;
            *source = index;
            payload.push(Entry {
                offset,
                packed: bytes.len() as u64,
                size: *size,
                sha256: *sha256,
            });
            offset += bytes.len() as u64;
            packed.push(bytes);
        }
    }
    let mut stored = Vec::new();
    let mut packed_assets = Vec::new();
    for (name, bytes) in assets {
        let bytes = zstd::encode_all(bytes.as_slice(), LEVEL)?;
        stored.push(Asset {
            name: name.clone(),
            offset,
            size: bytes.len() as u64,
        });
        offset += bytes.len() as u64;
        packed_assets.push(bytes);
    }
    let header = serde_json::to_vec(&Header {
        format: FORMAT,
        plan,
        payload,
        setup: setup.clone(),
        assets: stored,
    })
    .map_err(io::Error::other)?;
    out.write_all(MAGIC)?;
    out.write_all(&(header.len() as u64).to_le_bytes())?;
    out.write_all(&header)?;
    for bytes in &packed {
        out.write_all(bytes)?;
    }
    for bytes in &packed_assets {
        out.write_all(bytes)?;
    }
    Ok(16 + header.len() as u64 + offset)
}

/// an opened container inside some file.
pub struct Container {
    file: fs::File,
    payload_start: u64,
    pub header: Header,
}

impl Container {
    /// the asset called `name`, if the container has one.
    ///
    /// # Errors
    ///
    /// when it cannot be read.
    pub fn asset(&mut self, name: &str) -> io::Result<Option<Vec<u8>>> {
        let Some(asset) = self.header.assets.iter().find(|a| a.name == name) else {
            return Ok(None);
        };
        // a damaged size must not ask for more than the file holds.
        let end = self
            .payload_start
            .saturating_add(asset.offset)
            .saturating_add(asset.size);
        if end > self.file.metadata()?.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("container: asset {name} runs past the end"),
            ));
        }
        let size = usize::try_from(asset.size).map_err(io::Error::other)?;
        let mut bytes = vec![0; size];
        self.file
            .seek(SeekFrom::Start(self.payload_start + asset.offset))?;
        self.file.read_exact(&mut bytes)?;
        zstd::decode_all(bytes.as_slice()).map(Some)
    }

    /// reads the container that starts at `offset` in `path`.
    ///
    /// # Errors
    ///
    /// when the bytes there are not a supported container.
    pub fn open(path: &Path, offset: u64) -> io::Result<Self> {
        let bad =
            |what: &str| io::Error::new(io::ErrorKind::InvalidData, format!("container: {what}"));
        let mut file = fs::File::open(path)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut magic = [0; 8];
        let mut len = [0; 8];
        file.read_exact(&mut magic)?;
        file.read_exact(&mut len)?;
        if &magic != MAGIC {
            return Err(bad("bad magic"));
        }
        let len = u64::from_le_bytes(len);
        // a damaged length must not ask for any amount of memory.
        if len > MAX_HEADER {
            return Err(bad("header too large"));
        }
        let len = usize::try_from(len).map_err(|_| bad("header too large"))?;
        let mut json = vec![0; len];
        file.read_exact(&mut json)?;
        let header: Header = serde_json::from_slice(&json).map_err(|e| bad(&e.to_string()))?;
        if header.format != FORMAT {
            return Err(bad(&format!("format {} is not supported", header.format)));
        }
        Ok(Self {
            file,
            payload_start: offset + 16 + len as u64,
            header,
        })
    }

    /// writes every payload file into `dir`, checks each hash, and returns
    /// the plan with sources pointing at the extracted files.
    ///
    /// # Errors
    ///
    /// when a payload entry is short, corrupt or cannot be written.
    pub fn extract(&mut self, dir: &Path) -> io::Result<InstallPlan> {
        self.extract_observed(dir, &mut |_, _, _| {})
    }

    /// [`Self::extract`], telling `observe` about each file before it is
    /// unpacked: its index, the file count and where it will be installed.
    ///
    /// # Errors
    ///
    /// as [`Self::extract`].
    pub fn extract_observed(
        &mut self,
        dir: &Path,
        observe: &mut dyn FnMut(usize, usize, &SemanticPath),
    ) -> io::Result<InstallPlan> {
        let plan = self.header.plan.clone();
        self.extract_plan(plan, dir, observe)
    }

    /// unpacks only the files `plan` installs, like the header's plan after
    /// [`InstallPlan::select`], so unticked components are never unpacked.
    ///
    /// # Errors
    ///
    /// as [`Self::extract`].
    pub fn extract_plan(
        &mut self,
        mut plan: InstallPlan,
        dir: &Path,
        observe: &mut dyn FnMut(usize, usize, &SemanticPath),
    ) -> io::Result<InstallPlan> {
        fs::create_dir_all(dir)?;
        let total = plan
            .operations
            .iter()
            .filter(|op| matches!(op, Operation::InstallFile { .. }))
            .count();
        let mut done = 0;
        for op in &mut plan.operations {
            let Operation::InstallFile { source, target, .. } = op else {
                continue;
            };
            observe(done, total, target);
            done += 1;
            let index: usize = source
                .to_str()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "container: bad payload index")
                })?;
            let entry = *self.header.payload.get(index).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "container: payload index out of range",
                )
            })?;
            let out = dir.join(index.to_string());
            self.file
                .seek(SeekFrom::Start(self.payload_start + entry.offset))?;
            let part = (&mut self.file).take(entry.packed);
            // never more than the entry says, whatever a damaged frame holds.
            let mut unpacked = zstd::Decoder::new(part)?.take(entry.size + 1);
            let copied = io::copy(&mut unpacked, &mut fs::File::create(&out)?)?;
            let (size, digest) = hash(&out)?;
            if copied != entry.size || size != entry.size || digest != entry.sha256 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("container: payload {index} is corrupt"),
                ));
            }
            *source = out;
        }
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::{Scope, Selection, resolve};
    use crate::manifest::{Context, Manifest, tests::AUREA};

    #[test]
    fn write_open_extract() {
        let dir = std::env::temp_dir().join(format!("kago-container-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dist/windows-x86_64")).unwrap();
        fs::create_dir_all(dir.join("themes")).unwrap();
        fs::write(dir.join("dist/windows-x86_64/aurea.exe"), b"MZ app").unwrap();
        fs::write(dir.join("themes/a.toml"), b"theme").unwrap();
        let m = Manifest::parse("Kagofile", AUREA).0.unwrap();
        let target = "windows-x86_64".parse().unwrap();
        let r = resolve(&m, &Context::target(target), Selection::defaults(&m))
            .0
            .unwrap();
        let plan = InstallPlan::build(&m, &r, target, Scope::User, dir.as_path())
            .0
            .unwrap();

        let path = dir.join("out.bin");
        let mut out = fs::File::create(&path).unwrap();
        out.write_all(b"stub").unwrap();
        let assets = [("logo".to_owned(), b"png bytes".to_vec())];
        write(&plan, &Setup::default(), &assets, &mut out).unwrap();
        drop(out);

        let mut c = Container::open(&path, 4).unwrap();
        assert_eq!(c.header.payload.len(), 2);
        assert_eq!(c.asset("logo").unwrap().as_deref(), Some(&b"png bytes"[..]));
        assert_eq!(c.asset("side").unwrap(), None);
        let mut seen = Vec::new();
        let extracted = c
            .extract_observed(&dir.join("staging"), &mut |i, n, target| {
                seen.push((i, n, target.to_string()));
            })
            .unwrap();
        assert_eq!(
            seen,
            [
                (0, 2, "{app}/bin/aurea.exe".to_owned()),
                (1, 2, "{app}/themes/a.toml".to_owned())
            ]
        );
        let sources: Vec<Vec<u8>> = extracted
            .operations
            .iter()
            .filter_map(|op| match op {
                Operation::InstallFile { source, .. } => Some(fs::read(source).unwrap()),
                _ => None,
            })
            .collect();
        assert_eq!(sources, [b"MZ app".to_vec(), b"theme".to_vec()]);
        assert_eq!(extracted.package, plan.package);

        // a plan without the theme unpacks only the app.
        let mut only_app = c.header.plan.clone();
        only_app.operations.retain(|op| {
            !matches!(op, Operation::InstallFile { target, .. } if target.to_string().contains("themes"))
        });
        let partial = dir.join("partial");
        c.extract_plan(only_app, &partial, &mut |_, _, _| {})
            .unwrap();
        assert_eq!(fs::read_dir(&partial).unwrap().count(), 1);

        // flip one payload byte: extraction must refuse.
        let mut bytes = fs::read(&path).unwrap();
        // the last payload byte; the asset comes after it.
        let asset = zstd::encode_all(&b"png bytes"[..], LEVEL).unwrap().len();
        let last = bytes.len() - 1 - asset;
        bytes[last] ^= 0xff;
        fs::write(&path, bytes).unwrap();
        let mut c = Container::open(&path, 4).unwrap();
        assert!(c.extract(&dir.join("staging2")).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}

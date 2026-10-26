//! pinned downloads. a download is only used when its sha-256 matches the
//! Kagofile; matching downloads are cached by that hash.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::install::{Digest, hash};

/// downloads `url` into `cache` unless a file with `expected` is already
/// there. returns the cached file.
///
/// # Errors
///
/// when the download fails, `expected` is missing (the error then names the
/// real hash to pin), or the hash does not match.
pub fn fetch(
    url: &str,
    expected: Option<&str>,
    cache: &Path,
    pin_hint: &str,
) -> Result<PathBuf, String> {
    if !(url.starts_with("https://") || url.starts_with("http://") || url.starts_with("file://")) {
        return Err(format!("`{url}` is not an https, http or file URL"));
    }
    fs::create_dir_all(cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let expected = match expected {
        Some(hex) => Some(hex.parse::<Digest>().map_err(|e| format!("sha256: {e}"))?),
        None => None,
    };
    if let Some(digest) = expected {
        let cached = cache.join(digest.to_string());
        if hash(&cached).is_ok_and(|(_, d)| d == digest) {
            return Ok(cached);
        }
    }
    let partial = cache.join(format!("download-{}.partial", std::process::id()));
    let mut curl = Command::new("curl");
    curl.args(["--fail", "--location", "--silent", "--show-error"]);
    // an https link never goes on over plain http.
    if url.starts_with("https://") {
        curl.args(["--proto-redir", "=https"]);
    }
    let out = curl
        .arg("--output")
        .arg(&partial)
        .arg(url)
        .output()
        .map_err(|e| format!("cannot run curl to fetch {url}: {e}"))?;
    if !out.status.success() {
        let _ = fs::remove_file(&partial);
        return Err(format!(
            "fetching {url} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let (_, got) = hash(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
    let Some(digest) = expected else {
        let _ = fs::remove_file(&partial);
        return Err(format!(
            "{url} has no pinned sha256; it downloaded as {got}. add `sha256 {pin_hint} {got}` to the source"
        ));
    };
    if got != digest {
        let _ = fs::remove_file(&partial);
        return Err(format!(
            "{url} does not match its pinned sha256\n  expected {digest}\n  got      {got}"
        ));
    }
    let cached = cache.join(digest.to_string());
    fs::rename(&partial, &cached).map_err(|e| format!("{}: {e}", cached.display()))?;
    Ok(cached)
}

/// `file://` URL for a local path, for tests and local mirrors.
#[must_use]
pub fn file_url(path: &Path) -> String {
    let path = path.display().to_string().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

//! archives to directories. entries that would land outside the target
//! (absolute paths, `..`) are refused, not skipped.

use std::fs;
use std::io;
use std::path::Path;

use crate::manifest::Unpack;

/// extracts `archive` into `dir`, which must not exist yet.
///
/// # Errors
///
/// when the archive is broken or has an entry escaping `dir`.
pub fn unpack(archive: &Path, format: Unpack, dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    match format {
        Unpack::Auto | Unpack::None => {
            let name = archive
                .file_name()
                .ok_or_else(|| io::Error::other("download has no file name"))?;
            fs::copy(archive, dir.join(name)).map(|_| ())
        }
        Unpack::Zip => zip(archive, dir),
        Unpack::TarGz => tar_gz(archive, dir),
    }
}

fn zip(archive: &Path, dir: &Path) -> io::Result<()> {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive)?).map_err(io::Error::other)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(io::Error::other)?;
        let Some(relative) = entry.enclosed_name() else {
            return Err(io::Error::other(format!(
                "archive entry `{}` points outside the archive",
                entry.name()
            )));
        };
        let out = dir.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        io::copy(&mut entry, &mut fs::File::create(&out)?)?;
    }
    Ok(())
}

fn tar_gz(archive: &Path, dir: &Path) -> io::Result<()> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(archive)?));
    for entry in tar.entries()? {
        let mut entry = entry?;
        // `unpack_in` refuses entries that escape `dir` by returning false.
        if !entry.unpack_in(dir)? {
            return Err(io::Error::other(format!(
                "archive entry `{}` points outside the archive",
                entry.path()?.display()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("kago-unpack-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn zip_round_trip_and_escape() {
        let dir = temp("zip");
        let archive = dir.join("a.zip");
        let mut w = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        w.start_file("bin/app.exe", opts).unwrap();
        w.write_all(b"app").unwrap();
        w.finish().unwrap();
        unpack(&archive, Unpack::Zip, &dir.join("out")).unwrap();
        assert_eq!(fs::read(dir.join("out/bin/app.exe")).unwrap(), b"app");

        let evil = dir.join("evil.zip");
        let mut w = zip::ZipWriter::new(fs::File::create(&evil).unwrap());
        w.start_file("../escape.txt", opts).unwrap();
        w.write_all(b"x").unwrap();
        w.finish().unwrap();
        assert!(unpack(&evil, Unpack::Zip, &dir.join("out2")).is_err());
        assert!(!dir.join("escape.txt").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tar_gz_round_trip() {
        let dir = temp("tgz");
        let archive = dir.join("a.tar.gz");
        let gz = flate2::write::GzEncoder::new(
            fs::File::create(&archive).unwrap(),
            flate2::Compression::default(),
        );
        let mut b = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o755);
        header.set_cksum();
        b.append_data(&mut header, "bin/app", &b"app"[..]).unwrap();
        b.into_inner().unwrap().finish().unwrap();
        unpack(&archive, Unpack::TarGz, &dir.join("out")).unwrap();
        assert_eq!(fs::read(dir.join("out/bin/app")).unwrap(), b"app");
        fs::remove_dir_all(dir).unwrap();
    }
}

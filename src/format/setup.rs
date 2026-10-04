//! setup executable: the kago runtime with a container appended.
//!
//! ```text
//! stub executable  pad to 8  container  u64 offset  u64 length  "KAGOEND1"
//! ```
//!
//! on Windows the trailer sits right before the Authenticode certificate
//! table when the file is signed, so signing after build keeps it findable.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::container;
use crate::install::InstallPlan;

pub const TRAILER: &[u8; 8] = b"KAGOEND1";
const TRAILER_LEN: u64 = 24;

/// where a container sits inside an executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Embedded {
    /// end of the stub, where the runtime alone ends.
    pub stub_len: u64,
    pub offset: u64,
    pub len: u64,
}

/// writes `stub` + container for `plan` to `out`. returns its size.
///
/// # Errors
///
/// when the stub or payload cannot be read or `out` cannot be written.
pub fn build(stub: &Path, plan: &InstallPlan, out: &Path) -> io::Result<u64> {
    let stub_bytes = fs::read(stub)?;
    if embedded_in(&stub_bytes)?.is_some() {
        return Err(io::Error::other(
            "the stub already carries a package; build from a plain kago",
        ));
    }
    let tmp = out.with_extension("partial");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(&stub_bytes)?;
    let stub_len = stub_bytes.len() as u64;
    let pad = (8 - stub_len % 8) % 8;
    file.write_all(&[0; 8][..usize::try_from(pad).unwrap_or(0)])?;
    let offset = stub_len + pad;
    let len = container::write(plan, &mut file)?;
    file.write_all(&offset.to_le_bytes())?;
    file.write_all(&len.to_le_bytes())?;
    file.write_all(TRAILER)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, out)?;
    Ok(offset + len + TRAILER_LEN)
}

/// container inside the executable at `path`, if it has one.
///
/// # Errors
///
/// when the file cannot be read.
pub fn embedded(path: &Path) -> io::Result<Option<Embedded>> {
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mut head = vec![0; usize::try_from(len.min(4096)).unwrap_or(4096)];
    file.read_exact(&mut head)?;
    let end = overlay_end(&head, len);
    if end < TRAILER_LEN {
        return Ok(None);
    }
    let mut trailer = [0; 24];
    file.seek(SeekFrom::Start(end - TRAILER_LEN))?;
    file.read_exact(&mut trailer)?;
    Ok(parse_trailer(&trailer, end))
}

fn embedded_in(bytes: &[u8]) -> io::Result<Option<Embedded>> {
    let len = bytes.len() as u64;
    let end = overlay_end(&bytes[..bytes.len().min(4096)], len);
    if end < TRAILER_LEN {
        return Ok(None);
    }
    let start = usize::try_from(end - TRAILER_LEN).map_err(io::Error::other)?;
    let trailer: [u8; 24] = bytes[start..start + 24].try_into().expect("24 bytes");
    Ok(parse_trailer(&trailer, end))
}

fn parse_trailer(trailer: &[u8; 24], end: u64) -> Option<Embedded> {
    if &trailer[16..] != TRAILER {
        return None;
    }
    let offset = u64::from_le_bytes(trailer[..8].try_into().expect("8 bytes"));
    let len = u64::from_le_bytes(trailer[8..16].try_into().expect("8 bytes"));
    let fits = offset.checked_add(len)? == end - TRAILER_LEN;
    fits.then_some(Embedded {
        stub_len: offset,
        offset,
        len,
    })
}

/// end of appended data: the start of the PE certificate table when the
/// file is a signed PE, the file end otherwise.
fn overlay_end(head: &[u8], file_len: u64) -> u64 {
    let u16_at = |i: usize| head.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |i: usize| {
        head.get(i..i + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let certificate_table = || -> Option<u64> {
        if head.get(..2)? != b"MZ" {
            return None;
        }
        let pe = usize::try_from(u32_at(0x3c)?).ok()?;
        if head.get(pe..pe + 4)? != b"PE\0\0" {
            return None;
        }
        let optional = pe + 24;
        let directories = match u16_at(optional)? {
            0x10b => optional + 96,
            0x20b => optional + 112,
            _ => return None,
        };
        // directory 4 is the certificate table; its address is a file offset.
        let at = u64::from(u32_at(directories + 4 * 8)?);
        let size = u64::from(u32_at(directories + 4 * 8 + 4)?);
        (at != 0 && size != 0 && at + size <= file_len).then_some(at)
    };
    certificate_table().unwrap_or(file_len)
}

/// copies only the runtime part of a setup executable to `out`.
///
/// # Errors
///
/// when either file cannot be accessed.
pub fn extract_stub(setup: &Path, embedded: Embedded, out: &Path) -> io::Result<()> {
    let mut from = fs::File::open(setup)?.take(embedded.stub_len);
    let mut to = fs::File::create(out)?;
    io::copy(&mut from, &mut to)?;
    to.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailer_round_trip_without_pe() {
        let mut bytes = b"not a pe file".to_vec();
        let offset = bytes.len() as u64;
        bytes.extend_from_slice(b"CONTAINER");
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&9u64.to_le_bytes());
        bytes.extend_from_slice(TRAILER);
        let e = embedded_in(&bytes).unwrap().unwrap();
        assert_eq!((e.offset, e.len, e.stub_len), (offset, 9, offset));
        assert_eq!(
            embedded_in(b"plain file with no trailer at all").unwrap(),
            None
        );
    }

    #[test]
    fn signed_pe_ends_at_certificate_table() {
        // tiny PE32+ header with a certificate table at 0x300, 0x40 long.
        let mut head = vec![0u8; 0x400];
        head[..2].copy_from_slice(b"MZ");
        head[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        head[0x80..0x84].copy_from_slice(b"PE\0\0");
        let optional = 0x80 + 24;
        head[optional..optional + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        let dir = optional + 112 + 4 * 8;
        head[dir..dir + 4].copy_from_slice(&0x300u32.to_le_bytes());
        head[dir + 4..dir + 8].copy_from_slice(&0x40u32.to_le_bytes());
        assert_eq!(overlay_end(&head, 0x340), 0x300);
        assert_eq!(
            overlay_end(&head[..0x100], 0x100),
            0x100,
            "table past the end is ignored"
        );
    }

    #[test]
    fn this_kago_has_no_package() {
        let me = std::env::current_exe().unwrap();
        assert_eq!(embedded(&me).unwrap(), None);
    }
}

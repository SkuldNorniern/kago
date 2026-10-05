//! reading Windows executables: the DLLs one imports. used to warn about
//! runtimes a fresh Windows does not have.

/// names of the DLLs `bytes` imports, lowercase. empty when it is not a PE
/// file or has no import table.
#[must_use]
pub fn imports(bytes: &[u8]) -> Vec<String> {
    imports_in(bytes).unwrap_or_default()
}

fn imports_in(b: &[u8]) -> Option<Vec<String>> {
    let u16_at = |i: usize| b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    let u32_at = |i: usize| {
        b.get(i..i + 4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    };
    if b.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(0x3c)? as usize;
    if b.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let sections = usize::from(u16_at(pe + 6)?);
    let optional_size = usize::from(u16_at(pe + 20)?);
    let optional = pe + 24;
    let data_dirs = optional + if u16_at(optional)? == 0x20b { 112 } else { 96 };
    let import_rva = u32_at(data_dirs + 8)?;
    if import_rva == 0 {
        return Some(Vec::new());
    }
    let table = optional + optional_size;
    // file offset of a relative virtual address, through the section table.
    let offset = |rva: u32| -> Option<usize> {
        (0..sections).find_map(|i| {
            let s = table + i * 40;
            let (virtual_size, address) = (u32_at(s + 8)?, u32_at(s + 12)?);
            let (raw_size, raw) = (u32_at(s + 16)?, u32_at(s + 20)?);
            let span = virtual_size.max(raw_size);
            (address <= rva && rva < address + span).then(|| (rva - address + raw) as usize)
        })
    };
    let mut out = Vec::new();
    let mut entry = offset(import_rva)?;
    // a zeroed descriptor ends the table; cap it so a broken file cannot
    // run on forever.
    for _ in 0..4096 {
        let name_rva = u32_at(entry + 12)?;
        if name_rva == 0 {
            break;
        }
        let start = offset(name_rva)?;
        let len = b.get(start..)?.iter().position(|c| *c == 0)?;
        out.push(String::from_utf8_lossy(&b[start..start + len]).to_ascii_lowercase());
        entry += 20;
    }
    Some(out)
}

/// whether `bytes` is a PE file with an icon in its resources, which
/// Explorer, shortcuts and the taskbar show for it.
#[must_use]
pub fn has_icon(b: &[u8]) -> bool {
    has_icon_in(b).unwrap_or(false)
}

fn has_icon_in(b: &[u8]) -> Option<bool> {
    let u16_at = |i: usize| b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    let u32_at = |i: usize| {
        b.get(i..i + 4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    };
    if b.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(0x3c)? as usize;
    let sections = usize::from(u16_at(pe + 6)?);
    let optional_size = usize::from(u16_at(pe + 20)?);
    let optional = pe + 24;
    let data_dirs = optional + if u16_at(optional)? == 0x20b { 112 } else { 96 };
    let resource_rva = u32_at(data_dirs + 16)?;
    if resource_rva == 0 {
        return Some(false);
    }
    let table = optional + optional_size;
    let root = (0..sections).find_map(|i| {
        let s = table + i * 40;
        let (virtual_size, address) = (u32_at(s + 8)?, u32_at(s + 12)?);
        let (raw_size, raw) = (u32_at(s + 16)?, u32_at(s + 20)?);
        let span = virtual_size.max(raw_size);
        (address <= resource_rva && resource_rva < address + span)
            .then(|| (resource_rva - address + raw) as usize)
    })?;
    // the root directory lists resource types; RT_GROUP_ICON is id 14.
    let named = usize::from(u16_at(root + 12)?);
    let ids = usize::from(u16_at(root + 14)?);
    Some((named..named + ids).any(|i| u32_at(root + 16 + i * 8) == Some(14)))
}

/// whether `bytes` is a PE file with an Authenticode signature: its
/// certificate table, data directory 4, is filled in.
#[must_use]
pub fn is_signed(b: &[u8]) -> bool {
    let u16_at = |i: usize| b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    let u32_at = |i: usize| {
        b.get(i..i + 4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    };
    let signed = || -> Option<bool> {
        if b.get(..2)? != b"MZ" {
            return None;
        }
        let pe = u32_at(0x3c)? as usize;
        let optional = pe + 24;
        let data_dirs = optional + if u16_at(optional)? == 0x20b { 112 } else { 96 };
        Some(u32_at(data_dirs + 4 * 8 + 4)? != 0)
    };
    signed().unwrap_or(false)
}

/// the Visual C++ runtime DLLs among `imports`: not part of Windows, so a
/// fresh machine cannot start the program without them.
#[must_use]
pub fn needs_vc_runtime(imports: &[String]) -> Vec<&str> {
    imports
        .iter()
        .map(String::as_str)
        .filter(|dll| {
            dll.starts_with("vcruntime") || dll.starts_with("msvcp") || dll.starts_with("concrt")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_an_executable_imports_nothing() {
        assert!(imports(b"hello").is_empty());
        assert!(imports(b"MZ").is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn this_test_binary_imports_kernel32() {
        let me = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        let dlls = imports(&me);
        assert!(dlls.iter().any(|d| d == "kernel32.dll"), "{dlls:?}");
    }

    #[test]
    fn icons_need_a_resource_table() {
        assert!(!has_icon(b"not a program"));
    }

    #[test]
    fn unsigned_test_binaries_and_non_programs() {
        let me = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        assert!(!is_signed(&me), "cargo does not sign");
        assert!(!is_signed(b"not a program"));
    }

    #[test]
    fn picks_out_the_vc_runtime() {
        let dlls = ["kernel32.dll", "vcruntime140.dll", "msvcp140.dll"].map(str::to_owned);
        assert_eq!(
            needs_vc_runtime(&dlls),
            ["vcruntime140.dll", "msvcp140.dll"]
        );
    }
}

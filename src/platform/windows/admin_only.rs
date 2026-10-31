//! kago's state for everyone, in `%ProgramData%\Kago`, writable by
//! administrators only: users can add files to new folders there, and a
//! planted journal would make an elevated kago run its undo programs.

use std::path::Path;

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
    ProgressInvokeNever, SDDL_REVISION_1, SE_FILE_OBJECT, TREE_SEC_INFO_RESET,
    TreeSetNamedSecurityInfoW,
};
use windows::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
    GetSecurityDescriptorOwner, IsWellKnownSid, OWNER_SECURITY_INFORMATION,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
    WinBuiltinAdministratorsSid, WinLocalSystemSid,
};
use windows::core::{BOOL, HSTRING};

/// owned by Administrators; SYSTEM and Administrators may change it, users
/// may only read, nothing inherited from `%ProgramData%`.
const SDDL: &str = "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";

/// a security descriptor, freed on drop.
struct Descriptor(PSECURITY_DESCRIPTOR);

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: allocated by the system with LocalAlloc, freed once.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0.0)));
        }
    }
}

/// makes `dir` (created when missing) writable by administrators only,
/// with everything in it, unless it already is.
///
/// # Errors
///
/// when it cannot be created or its owner and rights cannot be set.
pub fn admin_only(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if locked(dir) {
        return Ok(());
    }
    // the reset below would make what a user planted look like ours.
    drop_foreign(dir)?;
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: the string lives across the call; `sd` is an out pointer.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(SDDL),
            SDDL_REVISION_1,
            &raw mut sd,
            None,
        )
    }
    .map_err(|e| e.message())?;
    let sd = Descriptor(sd);
    let (mut owner, mut dacl) = (PSID::default(), std::ptr::null_mut::<ACL>());
    let (mut present, mut defaulted) = (BOOL::default(), BOOL::default());
    // SAFETY: `sd` is a valid descriptor; the out pointers point into it.
    unsafe {
        GetSecurityDescriptorOwner(sd.0, &raw mut owner, &raw mut defaulted)
            .and_then(|()| {
                GetSecurityDescriptorDacl(sd.0, &raw mut present, &raw mut dacl, &raw mut defaulted)
            })
            .map_err(|e| e.message())?;
    }
    // SAFETY: owner and dacl point into `sd`, alive across the call. reset
    // replaces whatever a user set on files already in there.
    let e = unsafe {
        TreeSetNamedSecurityInfoW(
            &HSTRING::from(dir),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            Some(owner),
            None,
            Some(dacl),
            None,
            TREE_SEC_INFO_RESET,
            None,
            ProgressInvokeNever,
            None,
        )
    };
    e.ok().map_err(|e| {
        format!(
            "cannot make {} writable by administrators only: {}",
            dir.display(),
            e.message()
        )
    })
}

/// removes everything under `dir` no administrator made: links, and what
/// any other account owns.
fn drop_foreign(dir: &Path) -> Result<(), String> {
    let failed = |path: &Path, e: std::io::Error| format!("{}: {e}", path.display());
    for entry in std::fs::read_dir(dir).map_err(|e| failed(dir, e))? {
        let entry = entry.map_err(|e| failed(dir, e))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| failed(&path, e))?;
        if !kind.is_symlink() && rights(&path).is_some_and(|(admin, _)| admin) {
            if kind.is_dir() {
                drop_foreign(&path)?;
            }
            continue;
        }
        let removed = if kind.is_file() {
            std::fs::remove_file(&path)
        } else {
            // a link goes alone, never what it points at.
            std::fs::remove_dir_all(&path)
        };
        removed.map_err(|e| failed(&path, e))?;
    }
    Ok(())
}

/// whether `dir` is owned by Administrators or SYSTEM with its rights its
/// own: then only an administrator could have put anything in it.
fn locked(dir: &Path) -> bool {
    rights(dir) == Some((true, true))
}

/// whether Administrators or SYSTEM own `path`, and whether its rights are
/// its own, not inherited. `None` when they cannot be read.
fn rights(path: &Path) -> Option<(bool, bool)> {
    let mut owner = PSID::default();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: the name lives across the call; out pointers are valid.
    let e = unsafe {
        GetNamedSecurityInfoW(
            &HSTRING::from(path),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&raw mut owner),
            None,
            None,
            None,
            &raw mut sd,
        )
    };
    if e.is_err() {
        return None;
    }
    let sd = Descriptor(sd);
    let (mut control, mut revision) = (0u16, 0u32);
    // SAFETY: `sd` is valid; owner points into it.
    unsafe {
        let admin = IsWellKnownSid(owner, WinBuiltinAdministratorsSid).as_bool()
            || IsWellKnownSid(owner, WinLocalSystemSid).as_bool();
        let protected = GetSecurityDescriptorControl(sd.0, &raw mut control, &raw mut revision)
            .is_ok()
            && control & SE_DACL_PROTECTED.0 != 0;
        Some((admin, protected))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a temp folder this process owns; when elevated its files may read as
    /// an administrator's, so only the link is sure to go.
    #[test]
    fn links_and_what_others_made_go() {
        let dir = std::env::temp_dir().join(format!("kago-foreign-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let elsewhere = dir.join("elsewhere");
        let store = dir.join("store");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(elsewhere.join("keep.txt"), b"x").unwrap();
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(store.join("link"))
            .arg(&elsewhere)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        drop_foreign(&store).unwrap();
        assert!(!store.join("link").exists());
        assert!(
            elsewhere.join("keep.txt").is_file(),
            "what it points at stays"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

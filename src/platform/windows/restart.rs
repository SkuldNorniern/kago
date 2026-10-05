//! programs holding installed files open, through the Restart Manager:
//! which they are, and closing them so an update or uninstall can replace
//! or remove the files. what Inno Setup's `CloseApplications` does.

use std::path::PathBuf;

use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::RestartManager::{
    CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmForceShutdown, RmGetList,
    RmRegisterResources, RmShutdown, RmStartSession,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

/// a Restart Manager session over some files, ended on drop.
struct Session(u32);

impl Session {
    fn over(files: &[PathBuf]) -> Result<Self, String> {
        let mut handle = 0u32;
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        // SAFETY: the key buffer has the size the API asks for.
        let started = unsafe { RmStartSession(&raw mut handle, None, PWSTR(key.as_mut_ptr())) };
        check(started, "start")?;
        let session = Self(handle);
        let names: Vec<HSTRING> = files.iter().map(|f| HSTRING::from(f.as_path())).collect();
        let pointers: Vec<PCWSTR> = names.iter().map(|n| PCWSTR(n.as_ptr())).collect();
        // SAFETY: `names` outlives the call; the pointers point into it.
        let registered = unsafe { RmRegisterResources(handle, Some(&pointers), None, None) };
        check(registered, "register files")?;
        Ok(session)
    }

    /// the programs using the files.
    fn programs(&self) -> Result<Vec<String>, String> {
        let mut infos: Vec<RM_PROCESS_INFO> = Vec::new();
        loop {
            let mut needed = 0u32;
            let mut count = u32::try_from(infos.len()).unwrap_or(0);
            let mut reasons = 0u32;
            // SAFETY: `infos` holds `count` entries for the API to fill.
            let listed = unsafe {
                RmGetList(
                    self.0,
                    &raw mut needed,
                    &raw mut count,
                    Some(infos.as_mut_ptr()),
                    &raw mut reasons,
                )
            };
            if listed == ERROR_MORE_DATA {
                infos.resize(needed as usize, RM_PROCESS_INFO::default());
                continue;
            }
            check(listed, "list programs")?;
            infos.truncate(count as usize);
            break;
        }
        let mut names: Vec<String> = infos
            .iter()
            .map(|info| {
                let end = info
                    .strAppName
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(info.strAppName.len());
                String::from_utf16_lossy(&info.strAppName[..end])
            })
            .filter(|n| !n.is_empty())
            .collect();
        names.sort();
        names.dedup();
        Ok(names)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: the handle came from RmStartSession and is ended once.
        let _ = unsafe { RmEndSession(self.0) };
    }
}

fn check(result: WIN32_ERROR, what: &str) -> Result<(), String> {
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!(
            "cannot {what} with the Restart Manager: {}",
            windows::core::Error::from(result.to_hresult()).message()
        ))
    }
}

/// names of the programs that have any of `files` open.
pub fn programs_using(files: &[PathBuf]) -> Result<Vec<String>, String> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    Session::over(files)?.programs()
}

/// asks the programs using `files` to close, the way Windows does at sign
/// out, and ends the ones that do not.
pub fn close_programs(files: &[PathBuf]) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }
    let session = Session::over(files)?;
    if session.programs()?.is_empty() {
        return Ok(());
    }
    // SAFETY: a live session; no status callback.
    let asked = unsafe { RmShutdown(session.0, 0, None) };
    if asked == ERROR_SUCCESS && settled(files)? {
        return Ok(());
    }
    // SAFETY: as above.
    let forced = unsafe { RmShutdown(session.0, RmForceShutdown.0.cast_unsigned(), None) };
    check(forced, "close programs")?;
    if settled(files)? {
        return Ok(());
    }
    Err(format!(
        "still running: {}",
        programs_using(files)?.join(", ")
    ))
}

/// whether every program let go of the files within a few seconds: a
/// closing program takes a moment to exit. a new session each time, as
/// the one that closed them keeps listing them as stopped.
fn settled(files: &[PathBuf]) -> Result<bool, String> {
    for _ in 0..30 {
        if programs_using(files)?.is_empty() {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(false)
}

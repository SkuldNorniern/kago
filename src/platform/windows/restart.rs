//! programs holding installed files open, through the Restart Manager:
//! which they are, and closing them so an update or uninstall can replace
//! or remove the files. what Inno Setup's `CloseApplications` does.

use std::path::PathBuf;

use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::RestartManager::{
    CCH_RM_SESSION_KEY, RM_APP_TYPE, RM_PROCESS_INFO, RmCritical, RmEndSession, RmExplorer,
    RmForceShutdown, RmGetList, RmMainWindow, RmOtherWindow, RmRegisterResources, RmShutdown,
    RmStartSession,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

/// a program holding one of the files.
struct Holder {
    name: String,
    kind: RM_APP_TYPE,
}

impl Holder {
    /// has a window, so it may be holding unsaved work.
    fn windowed(&self) -> bool {
        self.kind == RmMainWindow || self.kind == RmOtherWindow
    }

    /// closing it would take the desktop or Windows down with it.
    fn system(&self) -> bool {
        self.kind == RmExplorer || self.kind == RmCritical
    }
}

/// files registered per call: a big package at once can be refused.
const BATCH: usize = 1000;

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
        for batch in pointers.chunks(BATCH) {
            // SAFETY: `names` outlives the call; the pointers point into it.
            let registered = unsafe { RmRegisterResources(handle, Some(batch), None, None) };
            check(registered, "register files")?;
        }
        Ok(session)
    }

    /// the programs using the files.
    fn holders(&self) -> Result<Vec<Holder>, String> {
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
        Ok(infos
            .iter()
            .map(|info| {
                let end = info
                    .strAppName
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(info.strAppName.len());
                let name = String::from_utf16_lossy(&info.strAppName[..end]);
                Holder {
                    // one without a name still holds the file.
                    name: if name.is_empty() {
                        format!("process {}", info.Process.dwProcessId)
                    } else {
                        name
                    },
                    kind: info.ApplicationType,
                }
            })
            .collect())
    }
}

fn names<'a>(holders: impl Iterator<Item = &'a Holder>) -> Vec<String> {
    let mut names: Vec<String> = holders.map(|h| h.name.clone()).collect();
    names.sort();
    names.dedup();
    names
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
    Ok(names(Session::over(files)?.holders()?.iter()))
}

/// asks the programs using `files` to close, the way Windows does at sign
/// out. one with a window that stays may be asking to save work, so it is
/// reported, never ended; ones without a window are ended.
pub fn close_programs(files: &[PathBuf]) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }
    let session = Session::over(files)?;
    let holders = session.holders()?;
    if holders.is_empty() {
        return Ok(());
    }
    if holders.iter().any(Holder::system) {
        return Err(format!(
            "in use by {}, which cannot be closed; restart the computer and try again",
            names(holders.iter().filter(|h| h.system())).join(", ")
        ));
    }
    // SAFETY: a live session; no status callback.
    let asked = unsafe { RmShutdown(session.0, 0, None) };
    if asked == ERROR_SUCCESS && settled(files, ASKED)? {
        return Ok(());
    }
    let left = Session::over(files)?.holders()?;
    if left.iter().any(Holder::windowed) {
        return Err(format!(
            "still open: {}; close it, saving your work, and try again",
            names(left.iter().filter(|h| h.windowed())).join(", ")
        ));
    }
    // SAFETY: as above.
    let forced = unsafe { RmShutdown(session.0, RmForceShutdown.0.cast_unsigned(), None) };
    check(forced, "close programs")?;
    if settled(files, FORCED)? {
        return Ok(());
    }
    Err(format!(
        "still running: {}",
        programs_using(files)?.join(", ")
    ))
}

/// tenths of a second a program gets to save and exit when asked.
const ASKED: u32 = 150;
/// and to be gone when ended.
const FORCED: u32 = 30;

/// whether every program let go of the files within `tries` tenths of a
/// second. a new session each time, as the one that closed them keeps
/// listing them as stopped.
fn settled(files: &[PathBuf], tries: u32) -> Result<bool, String> {
    for _ in 0..tries {
        if programs_using(files)?.is_empty() {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(false)
}

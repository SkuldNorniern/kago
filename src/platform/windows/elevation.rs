//! administrator rights for installs that are for everyone: whether this
//! process has them, and starting a program with the UAC prompt.

use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcessToken, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{
    SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

/// whether this process runs with administrator rights.
pub fn is_elevated() -> bool {
    // SAFETY: the token handle is closed below; the out buffer is a
    // TOKEN_ELEVATION of the size passed.
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some((&raw mut elevation).cast()),
            u32::try_from(size_of::<TOKEN_ELEVATION>()).unwrap_or(4),
            &raw mut size,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// starts `exe` with `arguments` after the UAC prompt. with `wait`, returns
/// its exit code once it ends; without, 0 once it started.
pub fn run_elevated(exe: &Path, arguments: &[String], wait: bool) -> Result<u32, String> {
    let file = HSTRING::from(exe);
    let parameters = HSTRING::from(super::shortcut::join_arguments(arguments));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: u32::try_from(size_of::<SHELLEXECUTEINFOW>()).unwrap_or(0),
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: every string in `info` lives until the call returns; the
    // process handle it returns is closed below.
    unsafe {
        ShellExecuteExW(&raw mut info).map_err(|e| {
            if e.code() == windows::Win32::Foundation::ERROR_CANCELLED.to_hresult() {
                "administrator rights were not given".to_owned()
            } else {
                format!("cannot start {} as administrator: {e}", exe.display())
            }
        })?;
        let process = info.hProcess;
        if process.is_invalid() {
            return Ok(0);
        }
        let mut code = 0u32;
        if wait {
            WaitForSingleObject(process, INFINITE);
            let _ = GetExitCodeProcess(process, &raw mut code);
        }
        let _ = CloseHandle(process);
        Ok(code)
    }
}

//! one setup or uninstaller per package at a time, through a named mutex,
//! and a message box for saying so before any window is up.

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK, MessageBoxW};
use windows::core::HSTRING;

/// held while this process is the one working on the package.
pub struct Instance(HANDLE);

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: the mutex is owned by this thread and closed once.
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

/// takes the mutex `name`, waiting up to `wait_ms` for another holder to
/// let go. `None` while another process holds it.
pub fn claim(name: &str, wait_ms: u32) -> Option<Instance> {
    // global, so setups in other sessions (RDP, user switching) exclude each other.
    let name = HSTRING::from(format!("Global\\{name}"));
    // SAFETY: a plain named mutex; the handle is closed by `Instance`, or
    // below when it is not ours.
    unsafe {
        // created by an elevated process, it may not open for this one:
        // that is someone else holding it too.
        let handle = CreateMutexW(None, false, &name).ok()?;
        let waited = WaitForSingleObject(handle, wait_ms);
        if waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED {
            Some(Instance(handle))
        } else {
            let _ = CloseHandle(handle);
            None
        }
    }
}

/// a plain message box with an OK button.
pub fn tell(title: &str, text: &str) {
    // SAFETY: both strings live until the call returns.
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from(title),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

//! the shell's folder picker, `IFileOpenDialog` with `FOS_PICKFOLDERS`.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
    SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::core::HSTRING;

/// asks for a folder, starting in `start` or the nearest folder above it
/// that exists. `None` when cancelled or when the dialog cannot open.
pub fn pick(owner: Option<isize>, title: &str, start: Option<&Path>) -> Option<PathBuf> {
    // SAFETY: COM is initialized for this thread and released below; the
    // returned path is copied before the shell's buffer is freed.
    unsafe {
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let picked = (|| -> windows::core::Result<PathBuf> {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetOptions(dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)?;
            dialog.SetTitle(&HSTRING::from(title))?;
            if let Some(dir) = start.and_then(|s| s.ancestors().find(|a| a.is_dir())) {
                let item: windows::core::Result<IShellItem> =
                    SHCreateItemFromParsingName(&HSTRING::from(dir), None);
                if let Ok(item) = item {
                    dialog.SetFolder(&item)?;
                }
            }
            let owner = owner.map(|h| HWND(h as *mut _));
            dialog.Show(owner)?;
            let raw = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = raw.to_string().map(PathBuf::from);
            CoTaskMemFree(Some(raw.0 as *const _));
            path.map_err(|_| windows::core::Error::empty())
        })();
        if init.is_ok() {
            CoUninitialize();
        }
        picked.ok()
    }
}

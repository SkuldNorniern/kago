//! writes icon resources into an executable on disk.

use std::path::Path;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::LibraryLoader::{
    BeginUpdateResourceW, EndUpdateResourceW, UpdateResourceW,
};
use windows::core::{HSTRING, PCWSTR};

use crate::format::ico::IconGroup;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
/// first group, which Explorer shows for the file.
const MAIN_ICON: u16 = 1;
const NEUTRAL: u16 = 0;

/// integer resource id as the `PCWSTR` the API expects (`MAKEINTRESOURCE`).
fn id(n: u16) -> PCWSTR {
    PCWSTR(usize::from(n) as *const u16)
}

pub fn set_icon(exe: &Path, icon: &IconGroup) -> Result<(), String> {
    let err = |e: windows::core::Error| format!("cannot set icon on {}: {e}", exe.display());
    // SAFETY: handle comes from BeginUpdateResourceW and is ended exactly
    // once; data slices outlive each call.
    unsafe {
        let update: HANDLE = BeginUpdateResourceW(&HSTRING::from(exe), false).map_err(err)?;
        let result = (|| {
            for (n, image) in icon.images.iter().enumerate() {
                let n = u16::try_from(n + 1).map_err(|_| windows::core::Error::empty())?;
                UpdateResourceW(
                    update,
                    id(RT_ICON),
                    id(n),
                    NEUTRAL,
                    Some(image.as_ptr().cast()),
                    u32::try_from(image.len()).unwrap_or(u32::MAX),
                )?;
            }
            UpdateResourceW(
                update,
                id(RT_GROUP_ICON),
                id(MAIN_ICON),
                NEUTRAL,
                Some(icon.group.as_ptr().cast()),
                u32::try_from(icon.group.len()).unwrap_or(u32::MAX),
            )
        })();
        // discard on failure, commit otherwise.
        EndUpdateResourceW(update, result.is_err()).map_err(err)?;
        result.map_err(err)
    }
}

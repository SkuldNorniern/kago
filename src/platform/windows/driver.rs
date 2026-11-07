//! driver packages and the software devices they drive, through `SetupAPI`,
//! as `pnputil /add-driver` and `devcon install` do. each step needs
//! administrator rights.

use std::path::Path;

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DICD_GENERATE_ID, DIF_REGISTERDEVICE, DIF_REMOVE, DIGCF_ALLCLASSES, DIGCF_PRESENT, HDEVINFO,
    INSTALLFLAG_FORCE, MAX_CLASS_NAME_LEN, SP_COPY_NOOVERWRITE, SP_DEVINFO_DATA, SPDRP_HARDWAREID,
    SPOST_PATH, SUOI_FORCEDELETE, SetupCopyOEMInfW, SetupDiCallClassInstaller,
    SetupDiCreateDeviceInfoList, SetupDiCreateDeviceInfoW, SetupDiDestroyDeviceInfoList,
    SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW,
    SetupDiGetDeviceRegistryPropertyW, SetupDiGetINFClassW, SetupDiOpenDeviceInfoW,
    SetupDiSetDeviceRegistryPropertyW, SetupUninstallOEMInfW, UpdateDriverForPlugAndPlayDevicesW,
};
use windows::Win32::Foundation::{ERROR_FILE_EXISTS, ERROR_NO_MORE_ITEMS, GetLastError};
use windows::core::{GUID, HSTRING, PCWSTR, PWSTR};

use crate::platform::DriverAdded;

/// a device information set, destroyed on drop.
struct Set(HDEVINFO);

impl Drop for Set {
    fn drop(&mut self) {
        // SAFETY: the set came from SetupAPI and is destroyed once.
        let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

fn wide_to_string(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// copies the package at `inf` into the driver store. `new` is false when
/// the same package was there already, which then stays its owner's.
pub fn add(inf: &Path) -> Result<DriverAdded, String> {
    let source = HSTRING::from(inf.as_os_str());
    let mut name = [0u16; 260];
    let mut component = PWSTR::null();
    // SAFETY: the buffers outlive the call.
    let first = unsafe {
        SetupCopyOEMInfW(
            &source,
            PCWSTR::null(),
            SPOST_PATH,
            SP_COPY_NOOVERWRITE,
            Some(&mut name),
            None,
            Some(&raw mut component),
        )
    };
    let new = match first {
        Ok(()) => true,
        // the same package is in the store already; the name is filled in.
        Err(e) if e.code() == ERROR_FILE_EXISTS.to_hresult() => false,
        Err(e) => {
            return Err(format!(
                "cannot add driver {}: {}; is it signed, and are you an administrator?",
                inf.display(),
                e.message()
            ));
        }
    };
    let full = wide_to_string(&name);
    // the published name is the file name, like `oem12.inf`.
    let published = full
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(full.as_str())
        .to_owned();
    if published.is_empty() {
        return Err(format!(
            "driver {}: Windows gave it no name in the driver store",
            inf.display()
        ));
    }
    Ok(DriverAdded { published, new })
}

/// takes the package `published`, like `oem12.inf`, out of the driver
/// store, also from devices still using it. gone already is fine.
pub fn remove(published: &str) -> Result<(), String> {
    let name = HSTRING::from(published);
    // SAFETY: plain call with a live string.
    let ok = unsafe { SetupUninstallOEMInfW(&name, SUOI_FORCEDELETE, None) };
    if ok.as_bool() {
        return Ok(());
    }
    // SAFETY: reads this thread's last error.
    let error = unsafe { GetLastError() };
    // not in the store any more: ERROR_FILE_NOT_FOUND or SetupAPI's
    // ERROR_NOT_INSTALLED.
    if error.0 == 2 || error.0 == 0xE000_1000 {
        return Ok(());
    }
    Err(format!(
        "cannot remove driver {published} from the driver store: {}",
        windows::core::Error::from(error.to_hresult()).message()
    ))
}

/// the hardware ids of a device, from its `REG_MULTI_SZ` property.
fn hardware_ids(set: &Set, device: &SP_DEVINFO_DATA) -> Vec<String> {
    let mut buffer = vec![0u8; 2048];
    // SAFETY: the set and device are live; the buffer outlives the call.
    let read = unsafe {
        SetupDiGetDeviceRegistryPropertyW(
            set.0,
            device,
            SPDRP_HARDWAREID,
            None,
            Some(&mut buffer),
            None,
        )
    };
    if read.is_err() {
        return Vec::new();
    }
    let wide: Vec<u16> = buffer
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes(*b))
        .collect();
    wide.split(|c| *c == 0)
        .filter(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

fn instance_id(set: &Set, device: &SP_DEVINFO_DATA) -> Result<String, String> {
    let mut buffer = [0u16; 512];
    // SAFETY: the set and device are live; the buffer outlives the call.
    unsafe { SetupDiGetDeviceInstanceIdW(set.0, device, Some(&mut buffer), None) }
        .map_err(|e| format!("cannot read a device's instance id: {}", e.message()))?;
    Ok(wide_to_string(&buffer))
}

/// the instance id of a present device with hardware id `id`, if any.
pub fn find(id: &str) -> Result<Option<String>, String> {
    // SAFETY: plain call; the set is destroyed by `Set`.
    let set = unsafe {
        SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_ALLCLASSES | DIGCF_PRESENT)
    }
    .map(Set)
    .map_err(|e| format!("cannot list devices: {}", e.message()))?;
    for index in 0.. {
        let mut device = SP_DEVINFO_DATA {
            cbSize: u32::try_from(size_of::<SP_DEVINFO_DATA>()).unwrap_or(u32::MAX),
            ..Default::default()
        };
        // SAFETY: the set is live; `device` outlives the call.
        if let Err(e) = unsafe { SetupDiEnumDeviceInfo(set.0, index, &raw mut device) } {
            if e.code() == ERROR_NO_MORE_ITEMS.to_hresult() {
                break;
            }
            return Err(format!("cannot list devices: {}", e.message()));
        }
        if hardware_ids(&set, &device)
            .iter()
            .any(|h| h.eq_ignore_ascii_case(id))
        {
            return instance_id(&set, &device).map(Some);
        }
    }
    Ok(None)
}

/// installs the driver at `inf` on every present device with hardware id
/// `id`, as Windows would when such hardware turns up.
pub fn update(id: &str, inf: &Path) -> Result<(), String> {
    let mut reboot = windows::core::BOOL(0);
    // SAFETY: the strings and flag outlive the call.
    unsafe {
        UpdateDriverForPlugAndPlayDevicesW(
            None,
            &HSTRING::from(id),
            &HSTRING::from(inf.as_os_str()),
            INSTALLFLAG_FORCE,
            Some(&raw mut reboot),
        )
    }
    .map_err(|e| {
        format!(
            "cannot install driver {} on device {id}: {}",
            inf.display(),
            e.message()
        )
    })
}

/// makes a root-enumerated device with hardware id `id`, of the class
/// `inf` declares, and installs the driver on it. returns its instance id.
/// a device that cannot get its driver is removed again.
pub fn create(id: &str, inf: &Path) -> Result<String, String> {
    let mut class = GUID::zeroed();
    let mut class_name = [0u16; MAX_CLASS_NAME_LEN as usize];
    // SAFETY: the buffers outlive the call.
    unsafe {
        SetupDiGetINFClassW(
            &HSTRING::from(inf.as_os_str()),
            &raw mut class,
            &mut class_name,
            None,
        )
    }
    .map_err(|e| {
        format!(
            "cannot read the class of {}: {}",
            inf.display(),
            e.message()
        )
    })?;
    // SAFETY: the set is destroyed by `Set`.
    let set = unsafe { SetupDiCreateDeviceInfoList(Some(&raw const class), None) }
        .map(Set)
        .map_err(|e| format!("cannot make device {id}: {}", e.message()))?;
    let mut device = SP_DEVINFO_DATA {
        cbSize: u32::try_from(size_of::<SP_DEVINFO_DATA>()).unwrap_or(u32::MAX),
        ..Default::default()
    };
    // SAFETY: the set, strings and `device` outlive the call.
    unsafe {
        SetupDiCreateDeviceInfoW(
            set.0,
            PCWSTR::from_raw(class_name.as_ptr()),
            &raw const class,
            PCWSTR::null(),
            None,
            DICD_GENERATE_ID,
            Some(&raw mut device),
        )
    }
    .map_err(|e| format!("cannot make device {id}: {}", e.message()))?;
    // REG_MULTI_SZ: the id, then two zeros.
    let mut ids: Vec<u8> = id.encode_utf16().flat_map(u16::to_le_bytes).collect();
    ids.extend_from_slice(&[0, 0, 0, 0]);
    // SAFETY: the set and `device` are live; the buffer outlives the call.
    unsafe {
        SetupDiSetDeviceRegistryPropertyW(set.0, &raw mut device, SPDRP_HARDWAREID, Some(&ids))
    }
    .map_err(|e| format!("cannot give device {id} its hardware id: {}", e.message()))?;
    // SAFETY: the set and `device` are live.
    unsafe { SetupDiCallClassInstaller(DIF_REGISTERDEVICE, set.0, Some(&raw const device)) }
        .map_err(|e| format!("cannot register device {id}: {}", e.message()))?;
    let instance = instance_id(&set, &device)?;
    if let Err(e) = update(id, inf) {
        // SAFETY: the set and `device` are live.
        let _ = unsafe { SetupDiCallClassInstaller(DIF_REMOVE, set.0, Some(&raw const device)) };
        return Err(e);
    }
    Ok(instance)
}

/// removes the device `instance`. gone already is fine.
pub fn remove_device(instance: &str) -> Result<(), String> {
    // SAFETY: the set is destroyed by `Set`.
    let set = unsafe { SetupDiCreateDeviceInfoList(None, None) }
        .map(Set)
        .map_err(|e| format!("cannot open device {instance}: {}", e.message()))?;
    let mut device = SP_DEVINFO_DATA {
        cbSize: u32::try_from(size_of::<SP_DEVINFO_DATA>()).unwrap_or(u32::MAX),
        ..Default::default()
    };
    // SAFETY: the set, string and `device` outlive the call.
    let opened = unsafe {
        SetupDiOpenDeviceInfoW(
            set.0,
            &HSTRING::from(instance),
            None,
            0,
            Some(&raw mut device),
        )
    };
    if opened.is_err() {
        // no such device any more.
        return Ok(());
    }
    // SAFETY: the set and `device` are live.
    unsafe { SetupDiCallClassInstaller(DIF_REMOVE, set.0, Some(&raw const device)) }
        .map_err(|e| format!("cannot remove device {instance}: {}", e.message()))
}

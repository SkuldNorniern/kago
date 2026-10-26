//! Windows services through the Service Control Manager. each needs
//! administrator rights; the program must speak the service protocol itself.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{ERROR_SERVICE_ALREADY_RUNNING, ERROR_SERVICE_DOES_NOT_EXIST};
use windows::Win32::System::Services::{
    ChangeServiceConfig2W, ChangeServiceConfigW, CloseServiceHandle, ControlService,
    CreateServiceW, DeleteService, OpenSCManagerW, OpenServiceW, QUERY_SERVICE_CONFIGW,
    QueryServiceConfigW, QueryServiceStatus, SC_ACTION, SC_ACTION_NONE, SC_ACTION_RESTART,
    SC_HANDLE, SC_MANAGER_ALL_ACCESS, SC_MANAGER_CONNECT, SERVICE_ALL_ACCESS, SERVICE_AUTO_START,
    SERVICE_CONFIG_FAILURE_ACTIONS, SERVICE_CONTROL_STOP, SERVICE_DEMAND_START,
    SERVICE_ERROR_NORMAL, SERVICE_FAILURE_ACTIONSW, SERVICE_QUERY_CONFIG, SERVICE_QUERY_STATUS,
    SERVICE_STATUS, SERVICE_STOP, SERVICE_STOPPED, SERVICE_WIN32_OWN_PROCESS, StartServiceW,
};
use windows::core::{HSTRING, PCWSTR};

use crate::platform::WindowsService;

/// a service manager or service handle, closed on drop.
struct Handle(SC_HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from the service manager and is closed once.
        let _ = unsafe { CloseServiceHandle(self.0) };
    }
}

fn manager(access: u32) -> Result<Handle, String> {
    // SAFETY: the local machine's active database.
    unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), access) }
        .map(Handle)
        .map_err(|e| format!("cannot open the service manager: {}", e.message()))
}

/// the service `name`, or `None` when it does not exist.
fn open(manager: &Handle, name: &str, access: u32) -> Result<Option<Handle>, String> {
    // SAFETY: the name lives until the call returns.
    match unsafe { OpenServiceW(manager.0, &HSTRING::from(name), access) } {
        Ok(handle) => Ok(Some(Handle(handle))),
        Err(e) if e.code() == ERROR_SERVICE_DOES_NOT_EXIST.to_hresult() => Ok(None),
        Err(e) => Err(format!("service {name}: {}", e.message())),
    }
}

/// the service called `name` as it is set up now.
pub fn service(name: &str) -> Result<Option<WindowsService>, String> {
    let manager = manager(SC_MANAGER_CONNECT)?;
    let Some(service) = open(&manager, name, SERVICE_QUERY_CONFIG)? else {
        return Ok(None);
    };
    let mut needed = 0u32;
    // SAFETY: asks for the size first, then fills a buffer of that size,
    // aligned for the structure it holds.
    unsafe {
        let _ = QueryServiceConfigW(service.0, None, 0, &raw mut needed);
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
        let config = buffer.as_mut_ptr().cast::<QUERY_SERVICE_CONFIGW>();
        QueryServiceConfigW(service.0, Some(config), needed, &raw mut needed)
            .map_err(|e| format!("service {name}: {}", e.message()))?;
        let config = &*config;
        Ok(Some(WindowsService {
            display_name: config.lpDisplayName.to_string().unwrap_or_default(),
            command_line: config.lpBinaryPathName.to_string().unwrap_or_default(),
            automatic: config.dwStartType == SERVICE_AUTO_START,
            restart_on_failure: false,
        }))
    }
}

/// makes the service `name` run as `wanted` says, creating it or changing
/// it, and starts an automatic one; `None` stops and deletes it.
pub fn set_service(name: &str, wanted: Option<&WindowsService>) -> Result<(), String> {
    let manager = manager(SC_MANAGER_ALL_ACCESS)?;
    let existing = open(&manager, name, SERVICE_ALL_ACCESS)?;
    let Some(wanted) = wanted else {
        if let Some(service) = existing {
            stop(&service, name)?;
            // SAFETY: a live handle with delete access.
            unsafe { DeleteService(service.0) }
                .map_err(|e| format!("cannot delete service {name}: {}", e.message()))?;
        }
        return Ok(());
    };
    let start = if wanted.automatic {
        SERVICE_AUTO_START
    } else {
        SERVICE_DEMAND_START
    };
    let command = HSTRING::from(wanted.command_line.as_str());
    let display = HSTRING::from(wanted.display_name.as_str());
    let service = match existing {
        Some(service) => {
            stop(&service, name)?;
            // SAFETY: the strings live until the call returns; null
            // arguments keep what is there.
            unsafe {
                ChangeServiceConfigW(
                    service.0,
                    SERVICE_WIN32_OWN_PROCESS,
                    start,
                    SERVICE_ERROR_NORMAL,
                    &command,
                    PCWSTR::null(),
                    None,
                    PCWSTR::null(),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    &display,
                )
            }
            .map_err(|e| format!("cannot change service {name}: {}", e.message()))?;
            service
        }
        // SAFETY: as above; no account means LocalSystem.
        None => unsafe {
            CreateServiceW(
                manager.0,
                &HSTRING::from(name),
                &display,
                SERVICE_ALL_ACCESS,
                SERVICE_WIN32_OWN_PROCESS,
                start,
                SERVICE_ERROR_NORMAL,
                &command,
                PCWSTR::null(),
                None,
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
            )
        }
        .map(Handle)
        .map_err(|e| format!("cannot create service {name}: {}", e.message()))?,
    };
    restart_policy(&service, wanted.restart_on_failure);
    if wanted.automatic {
        // SAFETY: a live handle with start access.
        match unsafe { StartServiceW(service.0, None) } {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_SERVICE_ALREADY_RUNNING.to_hresult() => {}
            Err(e) => return Err(format!("service {name} did not start: {}", e.message())),
        }
    }
    Ok(())
}

/// stops the service `name` when there is one.
pub fn stop_service(name: &str) -> Result<(), String> {
    let manager = manager(SC_MANAGER_CONNECT)?;
    match open(&manager, name, SERVICE_STOP | SERVICE_QUERY_STATUS)? {
        Some(service) => stop(&service, name),
        None => Ok(()),
    }
}

/// restarts the service after a crash, three times a day at most, or
/// leaves it stopped.
fn restart_policy(service: &Handle, restart: bool) {
    let action = if restart {
        SC_ACTION_RESTART
    } else {
        SC_ACTION_NONE
    };
    let mut actions = [SC_ACTION {
        Type: action,
        Delay: 5_000,
    }; 3];
    let config = SERVICE_FAILURE_ACTIONSW {
        dwResetPeriod: 24 * 60 * 60,
        cActions: 3,
        lpsaActions: actions.as_mut_ptr(),
        ..Default::default()
    };
    // SAFETY: `config` and the actions it points to live through the call.
    let _ = unsafe {
        ChangeServiceConfig2W(
            service.0,
            SERVICE_CONFIG_FAILURE_ACTIONS,
            Some((&raw const config).cast()),
        )
    };
}

/// asks the service to stop and waits for it, up to 30 seconds.
fn stop(service: &Handle, name: &str) -> Result<(), String> {
    let mut status = SERVICE_STATUS::default();
    // SAFETY: a live handle; `status` is filled in.
    let _ = unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &raw mut status) };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        // SAFETY: as above.
        unsafe { QueryServiceStatus(service.0, &raw mut status) }
            .map_err(|e| format!("service {name}: {}", e.message()))?;
        if status.dwCurrentState == SERVICE_STOPPED {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("service {name} did not stop"));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

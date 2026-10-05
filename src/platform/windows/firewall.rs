//! Windows Firewall rules that let a program accept connections, through
//! the firewall's COM API. each needs administrator rights.

use std::path::PathBuf;

use windows::Win32::Foundation::{VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::NetworkManagement::WindowsFirewall::{
    INetFwPolicy2, INetFwRule, NET_FW_ACTION_ALLOW, NET_FW_IP_PROTOCOL_ANY, NET_FW_PROFILE2_DOMAIN,
    NET_FW_PROFILE2_PRIVATE, NET_FW_PROFILE2_PUBLIC, NET_FW_RULE_DIR_IN, NetFwPolicy2, NetFwRule,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::core::BSTR;

use crate::platform::Firewall;

/// runs `f` with the firewall policy, COM set up around it.
fn with_policy<T>(f: impl FnOnce(&INetFwPolicy2) -> windows::core::Result<T>) -> Result<T, String> {
    // SAFETY: COM is initialised for this call and uninitialised after
    // when this call initialised it.
    unsafe {
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER)
            .and_then(|policy: INetFwPolicy2| f(&policy));
        if init.is_ok() {
            CoUninitialize();
        }
        result.map_err(|e| format!("Windows Firewall: {}", e.message()))
    }
}

/// the inbound rule called `name`, if there is one.
pub fn rule(name: &str) -> Result<Option<Firewall>, String> {
    with_policy(|policy| {
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            let Ok(rule) = policy.Rules()?.Item(&BSTR::from(name)) else {
                return Ok(None);
            };
            let program = PathBuf::from(rule.ApplicationName()?.to_string());
            let public = rule.Profiles()? & NET_FW_PROFILE2_PUBLIC.0 != 0;
            Ok(Some(Firewall { program, public }))
        }
    })
}

/// lets `rule.program` accept connections on private and work networks,
/// and on public ones when asked, replacing a rule of the same name.
/// `None` removes the rule.
pub fn set_rule(name: &str, group: &str, rule: Option<&Firewall>) -> Result<(), String> {
    with_policy(|policy| {
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            let rules = policy.Rules()?;
            let key = BSTR::from(name);
            while rules.Item(&key).is_ok() {
                rules.Remove(&key)?;
            }
            let Some(wanted) = rule else {
                return Ok(());
            };
            let new: INetFwRule = CoCreateInstance(&NetFwRule, None, CLSCTX_INPROC_SERVER)?;
            new.SetName(&key)?;
            new.SetGrouping(&BSTR::from(group))?;
            new.SetApplicationName(&BSTR::from(
                wanted.program.as_os_str().to_string_lossy().as_ref(),
            ))?;
            new.SetProtocol(NET_FW_IP_PROTOCOL_ANY.0)?;
            new.SetDirection(NET_FW_RULE_DIR_IN)?;
            new.SetAction(NET_FW_ACTION_ALLOW)?;
            let mut profiles = NET_FW_PROFILE2_PRIVATE.0 | NET_FW_PROFILE2_DOMAIN.0;
            if wanted.public {
                profiles |= NET_FW_PROFILE2_PUBLIC.0;
            }
            new.SetProfiles(profiles)?;
            new.SetEnabled(VARIANT_TRUE)?;
            new.SetEdgeTraversal(VARIANT_FALSE)?;
            rules.Add(&new)
        }
    })
}

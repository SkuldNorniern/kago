//! Windows Firewall rules through its COM API; each needs administrator
//! rights. kago only touches rules in its own group, even when another
//! rule has the same name.

use std::path::PathBuf;

use windows::Win32::Foundation::{VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::NetworkManagement::WindowsFirewall::{
    INetFwPolicy2, INetFwRule, INetFwRules, NET_FW_ACTION_ALLOW, NET_FW_IP_PROTOCOL_ANY,
    NET_FW_IP_PROTOCOL_TCP, NET_FW_IP_PROTOCOL_UDP, NET_FW_PROFILE2_DOMAIN,
    NET_FW_PROFILE2_PRIVATE, NET_FW_PROFILE2_PUBLIC, NET_FW_RULE_DIR_IN, NetFwPolicy2, NetFwRule,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::System::Ole::IEnumVARIANT;
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, Interface};

use crate::manifest::Protocol;
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
        result.map_err(|e| {
            format!(
                "Windows Firewall: {}; is the Windows Defender Firewall service running?",
                e.message()
            )
        })
    }
}

/// every rule called `name` in `group`.
///
/// # Safety
///
/// COM must be initialised on this thread.
unsafe fn ours(
    rules: &INetFwRules,
    name: &str,
    group: &str,
) -> windows::core::Result<Vec<INetFwRule>> {
    let mut out = Vec::new();
    // SAFETY: the enumerator and each item are live COM objects.
    unsafe {
        let all: IEnumVARIANT = rules._NewEnum()?.cast()?;
        loop {
            let mut item = [VARIANT::default()];
            let mut fetched = 0;
            if all.Next(&mut item, &raw mut fetched).is_err() || fetched == 0 {
                break;
            }
            let Ok(dispatch) = windows::Win32::System::Com::IDispatch::try_from(&item[0]) else {
                continue;
            };
            let Ok(rule) = dispatch.cast::<INetFwRule>() else {
                continue;
            };
            let same_name = rule.Name().is_ok_and(|n| n == name);
            let same_group = rule.Grouping().is_ok_and(|g| g == group);
            if same_name && same_group {
                out.push(rule);
            }
        }
    }
    Ok(out)
}

/// the inbound rule called `name` in `group`, if there is one.
pub fn rule(name: &str, group: &str) -> Result<Option<Firewall>, String> {
    with_policy(|policy| {
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            let Some(rule) = ours(&policy.Rules()?, name, group)?.into_iter().next() else {
                return Ok(None);
            };
            let protocol = match rule.Protocol()? {
                p if p == NET_FW_IP_PROTOCOL_TCP.0 => Some(Protocol::Tcp),
                p if p == NET_FW_IP_PROTOCOL_UDP.0 => Some(Protocol::Udp),
                _ => None,
            };
            let ports = if protocol.is_some() {
                rule.LocalPorts()?.to_string()
            } else {
                String::new()
            };
            Ok(Some(Firewall {
                program: PathBuf::from(rule.ApplicationName()?.to_string()),
                public: rule.Profiles()? & NET_FW_PROFILE2_PUBLIC.0 != 0,
                protocol,
                ports: if ports == "*" { String::new() } else { ports },
            }))
        }
    })
}

/// makes the rule `name` in `group` say what `rule` says; `None` removes it.
pub fn set_rule(name: &str, group: &str, rule: Option<&Firewall>) -> Result<(), String> {
    with_policy(|policy| {
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            let rules = policy.Rules()?;
            // the firewall removes by name only, so each of ours gets a
            // name no other rule has first.
            for (i, old) in ours(&rules, name, group)?.into_iter().enumerate() {
                let unique =
                    BSTR::from(format!("{name} (kago removing {} {i})", std::process::id()));
                old.SetName(&unique)?;
                if let Err(e) = rules.Remove(&unique) {
                    // left under its own name, so it can be found again.
                    let _ = old.SetName(&BSTR::from(name));
                    return Err(e);
                }
            }
            let Some(wanted) = rule else {
                return Ok(());
            };
            let new: INetFwRule = CoCreateInstance(&NetFwRule, None, CLSCTX_INPROC_SERVER)?;
            new.SetName(&BSTR::from(name))?;
            new.SetGrouping(&BSTR::from(group))?;
            new.SetApplicationName(&BSTR::from(
                wanted.program.as_os_str().to_string_lossy().as_ref(),
            ))?;
            match wanted.protocol {
                Some(protocol) => {
                    let number = match protocol {
                        Protocol::Tcp => NET_FW_IP_PROTOCOL_TCP.0,
                        Protocol::Udp => NET_FW_IP_PROTOCOL_UDP.0,
                    };
                    new.SetProtocol(number)?;
                    if !wanted.ports.is_empty() {
                        new.SetLocalPorts(&BSTR::from(wanted.ports.as_str()))?;
                    }
                }
                None => new.SetProtocol(NET_FW_IP_PROTOCOL_ANY.0)?,
            }
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

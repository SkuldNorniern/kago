//! `firewall`: lets one of the package's programs accept connections
//! through Windows Firewall, like a game or a device link listening on
//! the network.

use super::policy::keywords;
use crate::diagnostic::Location;

keywords! {
    /// the one protocol a rule narrows to; every protocol without one.
    Protocol { Tcp => "tcp", Udp => "udp" }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirewallRule {
    /// the `app` id whose program may accept connections.
    pub app: String,
    /// the rule's name in the firewall; the package's name by default.
    pub name: Option<String>,
    /// also on public networks, like cafe Wi-Fi.
    pub public: bool,
    /// only this protocol, on `ports` when given.
    pub protocol: Option<Protocol>,
    /// the local ports, like `7000,9000-9010`; every port when empty.
    pub ports: String,
    pub component: Option<String>,
    pub at: Location,
}

/// checks a port list like `7000,7001,9000-9010`.
///
/// # Errors
///
/// on anything but ports from 1 to 65535 and ranges of them, low to high.
pub fn check_ports(text: &str) -> Result<(), String> {
    let port = |p: &str| {
        p.parse::<u16>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| format!("`{p}` is not a port from 1 to 65535"))
    };
    for part in text.split(',') {
        match part.split_once('-') {
            Some((low, high)) => {
                if port(low)? > port(high)? {
                    return Err(format!("`{part}` goes from high to low"));
                }
            }
            None => {
                port(part)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_lists() {
        assert!(check_ports("7000").is_ok());
        assert!(check_ports("7000,7001,9000-9010").is_ok());
        for bad in ["", "0", "70000", "a", "9010-9000", "7000,", "1-2-3"] {
            assert!(check_ports(bad).is_err(), "{bad}");
        }
    }
}

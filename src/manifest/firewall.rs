//! `firewall`: lets one of the package's programs accept connections
//! through Windows Firewall, like a game or a device link listening on
//! the network.

use crate::diagnostic::Location;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirewallRule {
    /// the `app` id whose program may accept connections.
    pub app: String,
    /// the rule's name in the firewall; the package's name by default.
    pub name: Option<String>,
    /// also on public networks, like cafe Wi-Fi.
    pub public: bool,
    pub component: Option<String>,
    pub at: Location,
}

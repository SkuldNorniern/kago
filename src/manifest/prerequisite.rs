//! `prerequisite`: another vendor's installer the package needs on the
//! machine first, like the Visual C++ runtime. it ships inside the setup
//! and runs only when its check says it is missing. the ones kago knows,
//! like `vcredist`, are actions in its library.

use super::Check;
use crate::diagnostic::Location;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prerequisite {
    pub id: String,
    /// shown while it installs.
    pub name: String,
    /// build-side installer, like `vcredist:vc_redist.x64.exe`.
    pub file: String,
    pub arguments: Vec<String>,
    pub check: Check,
    /// its installer needs administrator rights.
    pub admin: bool,
    pub at: Location,
}

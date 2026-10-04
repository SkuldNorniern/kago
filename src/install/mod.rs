//! install model. resolves variants for a target into the typed install plan.

mod operation;
mod plan;
mod resolve;
mod roots;
mod scope;

pub use operation::{Digest, Operation};
pub use plan::{Choice, InstallPlan, Sources, hash};
pub use resolve::{
    Resolved, ResolvedHook, ResolvedPath, ResolvedService, ResolvedShortcut, Selection, resolve,
};
pub use roots::{Roots, folder_name};
pub use scope::{Scope, choose};

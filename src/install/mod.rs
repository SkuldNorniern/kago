//! install model. resolves variants for a target into the typed install plan.

mod operation;
mod plan;
mod resolve;
mod scope;

pub use operation::{Digest, Operation};
pub use plan::{InstallPlan, hash};
pub use resolve::{
    Resolved, ResolvedHook, ResolvedPath, ResolvedService, ResolvedShortcut, Selection, resolve,
};
pub use scope::{Scope, choose};

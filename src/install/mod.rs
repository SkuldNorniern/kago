//! install model. resolves variants for a target into the typed install plan.

mod resolve;

pub use resolve::{
    Resolved, ResolvedHook, ResolvedPath, ResolvedService, ResolvedShortcut, Selection, resolve,
};

//! `driver`: puts a Windows driver package the install carries into the
//! driver store, like a virtual sound card or display, and makes the
//! software devices it drives.

use super::path::SemanticPath;
use super::selector::Selector;
use crate::diagnostic::Location;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Driver {
    /// the package's `.inf`, where the install puts it, like
    /// `{app}/drivers/audio/audio.inf`. the `.sys` and `.cat` files it
    /// names sit next to it.
    pub inf: SemanticPath,
    /// hardware ids of devices to make for it, like `Root\BifrostAudio`.
    /// a virtual device has no hardware to find, so without one the
    /// driver waits in the store for a device that never comes.
    pub devices: Vec<String>,
    pub component: Option<String>,
    pub when: Selector,
    pub at: Location,
}

/// checks a hardware id like `Root\BifrostAudio`.
///
/// # Errors
///
/// on an empty id, or one with spaces, commas or control characters,
/// which Windows does not take in one.
pub fn check_hardware_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("a hardware id cannot be empty".to_owned());
    }
    if let Some(c) = id
        .chars()
        .find(|c| c.is_whitespace() || c.is_control() || *c == ',')
    {
        return Err(format!("`{id}` is not a hardware id; it cannot hold {c:?}"));
    }
    if id.len() > 200 {
        return Err(format!(
            "`{id}` is longer than the 200 characters Windows takes"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_ids() {
        assert!(check_hardware_id(r"Root\BifrostAudio").is_ok());
        assert!(check_hardware_id(r"SWD\Bifrost&Display").is_ok());
        for bad in ["", "a b", "a,b", "a\tb"] {
            assert!(check_hardware_id(bad).is_err(), "{bad:?}");
        }
    }
}

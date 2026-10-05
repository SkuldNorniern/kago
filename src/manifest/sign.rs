//! the `sign` block: how `kago build` signs the setup and the package's
//! own programs with Authenticode. secrets never sit in the Kagofile;
//! they come from environment variables named there.

/// a value written in the Kagofile, or read from an environment variable
/// at build time, like a password or a CI's certificate thumbprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setting {
    Text(String),
    Env(String),
}

impl Setting {
    /// the value now.
    ///
    /// # Errors
    ///
    /// when the environment variable is not set.
    pub fn value(&self) -> Result<String, String> {
        match self {
            Self::Text(text) => Ok(text.clone()),
            Self::Env(name) => {
                std::env::var(name).map_err(|_| format!("environment variable {name} is not set"))
            }
        }
    }
}

/// where the signing key is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignKey {
    /// a certificate in the Windows store by its SHA-1 thumbprint: a USB
    /// token or HSM shows up there too.
    Certificate { thumbprint: Setting, machine: bool },
    /// a `.pfx` file, build-side, and the environment variable holding
    /// its password, if it has one.
    Pfx {
        file: String,
        password: Option<String>,
    },
    /// Azure Trusted Signing: its metadata JSON and the client DLL.
    Azure { metadata: String, dlib: String },
}

/// `sign` after defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sign {
    pub key: SignKey,
    /// RFC 3161 timestamp server.
    pub timestamp: String,
    /// also sign the package's own `.exe` and `.dll` files that are not
    /// signed yet.
    pub programs: bool,
}

/// the public RFC 3161 timestamp server of `DigiCert`.
pub const TIMESTAMP: &str = "http://timestamp.digicert.com";

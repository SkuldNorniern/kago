use std::fmt;
use std::str::FromStr;

/// what one artifact is built for, like `windows-x86_64` or
/// `linux-aarch64-musl`. libc is set for linux only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Target {
    os: Os,
    arch: Arch,
    libc: Option<Libc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Os {
    Windows,
    Macos,
    Linux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Arch {
    X86_64,
    Aarch64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Libc {
    Gnu,
    Musl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetError {
    UnknownOs(String),
    UnknownArch(String),
    UnknownLibc(String),
    /// libc given for windows or macos.
    LibcNotLinux(Os),
    /// not `os-arch` or `linux-arch-libc`.
    Shape(String),
}

impl fmt::Display for TargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOs(s) => write!(f, "unknown os `{s}`, expected windows, macos or linux"),
            Self::UnknownArch(s) => write!(f, "unknown arch `{s}`, expected x86_64 or aarch64"),
            Self::UnknownLibc(s) => write!(f, "unknown libc `{s}`, expected gnu or musl"),
            Self::LibcNotLinux(os) => write!(f, "libc only applies to linux, not {os}"),
            Self::Shape(s) => write!(f, "target `{s}` is not `os-arch` or `linux-arch-libc`"),
        }
    }
}

impl std::error::Error for TargetError {}

impl Target {
    /// linux without a libc gets gnu.
    ///
    /// # Errors
    ///
    /// when `libc` is given for windows or macos.
    pub fn new(os: Os, arch: Arch, libc: Option<Libc>) -> Result<Self, TargetError> {
        let libc = match (os, libc) {
            (Os::Linux, libc) => Some(libc.unwrap_or(Libc::Gnu)),
            (_, None) => None,
            (os, Some(_)) => return Err(TargetError::LibcNotLinux(os)),
        };
        Ok(Self { os, arch, libc })
    }

    /// the machine kago is running on. `None` on anything kago does not target.
    #[must_use]
    pub fn host() -> Option<Self> {
        let os = if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else {
            return None;
        };
        let arch = if cfg!(target_arch = "x86_64") {
            Arch::X86_64
        } else if cfg!(target_arch = "aarch64") {
            Arch::Aarch64
        } else {
            return None;
        };
        let libc = if cfg!(target_env = "musl") {
            Some(Libc::Musl)
        } else {
            None
        };
        Self::new(os, arch, libc).ok()
    }

    /// the Rust target triple, like `x86_64-pc-windows-msvc`.
    #[must_use]
    pub const fn triple(&self) -> &'static str {
        match (self.os, self.arch, self.libc) {
            (Os::Windows, Arch::X86_64, _) => "x86_64-pc-windows-msvc",
            (Os::Windows, Arch::Aarch64, _) => "aarch64-pc-windows-msvc",
            (Os::Macos, Arch::X86_64, _) => "x86_64-apple-darwin",
            (Os::Macos, Arch::Aarch64, _) => "aarch64-apple-darwin",
            (Os::Linux, Arch::X86_64, Some(Libc::Musl)) => "x86_64-unknown-linux-musl",
            (Os::Linux, Arch::X86_64, _) => "x86_64-unknown-linux-gnu",
            (Os::Linux, Arch::Aarch64, Some(Libc::Musl)) => "aarch64-unknown-linux-musl",
            (Os::Linux, Arch::Aarch64, _) => "aarch64-unknown-linux-gnu",
        }
    }

    /// executable suffix: `.exe` on Windows, empty elsewhere.
    #[must_use]
    pub const fn exe_suffix(&self) -> &'static str {
        match self.os {
            Os::Windows => ".exe",
            _ => "",
        }
    }

    #[must_use]
    pub const fn os(&self) -> Os {
        self.os
    }

    #[must_use]
    pub const fn arch(&self) -> Arch {
        self.arch
    }

    #[must_use]
    pub const fn libc(&self) -> Option<Libc> {
        self.libc
    }
}

impl FromStr for Target {
    type Err = TargetError;

    /// `windows-x86_64`, `macos-aarch64`, `linux-x86_64`, `linux-x86_64-musl`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('-');
        let (Some(os), Some(arch), libc, None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(TargetError::Shape(s.to_owned()));
        };
        Self::new(
            os.parse()?,
            arch.parse()?,
            libc.map(str::parse).transpose()?,
        )
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.os, self.arch)?;
        if let Some(libc) = self.libc {
            write!(f, "-{libc}")?;
        }
        Ok(())
    }
}

macro_rules! names {
    ($ty:ident, $err:ident, $($variant:ident => $name:literal),+) => {
        impl $ty {
            /// name used in targets and manifest selectors.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)+
                }
            }
        }

        impl FromStr for $ty {
            type Err = TargetError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($name => Ok(Self::$variant),)+
                    _ => Err(TargetError::$err(s.to_owned())),
                }
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

names!(Os, UnknownOs, Windows => "windows", Macos => "macos", Linux => "linux");
names!(Arch, UnknownArch, X86_64 => "x86_64", Aarch64 => "aarch64");
names!(Libc, UnknownLibc, Gnu => "gnu", Musl => "musl");

crate::string_serde!(Target);

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Target {
        s.parse().unwrap()
    }

    #[test]
    fn round_trips() {
        for s in [
            "windows-x86_64",
            "windows-aarch64",
            "macos-aarch64",
            "macos-x86_64",
            "linux-x86_64-gnu",
            "linux-aarch64-musl",
        ] {
            assert_eq!(t(s).to_string(), s);
        }
    }

    #[test]
    fn linux_defaults_to_gnu() {
        assert_eq!(t("linux-x86_64"), t("linux-x86_64-gnu"));
        assert_eq!(t("linux-x86_64").libc(), Some(Libc::Gnu));
    }

    #[test]
    fn rejects_bad_targets() {
        assert_eq!(
            "windows-x86_64-gnu".parse::<Target>(),
            Err(TargetError::LibcNotLinux(Os::Windows))
        );
        assert_eq!(
            "freebsd-x86_64".parse::<Target>(),
            Err(TargetError::UnknownOs("freebsd".into()))
        );
        assert_eq!(
            "linux-arm".parse::<Target>(),
            Err(TargetError::UnknownArch("arm".into()))
        );
        assert_eq!(
            "linux-x86_64-glibc".parse::<Target>(),
            Err(TargetError::UnknownLibc("glibc".into()))
        );
        assert_eq!(
            "windows".parse::<Target>(),
            Err(TargetError::Shape("windows".into()))
        );
        assert_eq!(
            "linux-x86_64-gnu-extra".parse::<Target>(),
            Err(TargetError::Shape("linux-x86_64-gnu-extra".into()))
        );
    }

    #[test]
    fn host_is_known_here() {
        assert!(Target::host().is_some());
    }
}

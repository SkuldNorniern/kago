//! Kago. one manifest lowered into an install plan, run by the kago runtime or emitted as native packages.

/// serde as the `Display` / `FromStr` string, so ids, versions, targets and
/// paths read the same in receipts as in `kago.toml`.
macro_rules! string_serde {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl serde::Serialize for $ty {
                fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                    s.collect_str(self)
                }
            }

            impl<'de> serde::Deserialize<'de> for $ty {
                fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                    let s = <String as serde::Deserialize>::deserialize(d)?;
                    s.parse().map_err(serde::de::Error::custom)
                }
            }
        )+
    };
}

pub(crate) use string_serde;

pub mod diagnostic;
pub mod install;
pub mod manifest;
pub mod package;
pub mod receipt;

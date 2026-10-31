//! `page`: setup pages of the package's own, with fields the person
//! installing fills in. their answers go into the install as
//! `{input.<id>}`, like `set server.host {input.host}`.

/// what a field takes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FieldKind {
    /// a line of text. `secret` shows dots instead of what is typed, and
    /// the answer is not remembered for updates.
    Text {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        secret: bool,
    },
    /// a whole number, between `min` and `max` when given.
    Number {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    /// one of the options.
    Choice { options: Vec<String> },
    /// a tick box: `true` or `false`.
    Check,
}

/// one field on a page.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub kind: FieldKind,
    /// the answer to begin with; empty for none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default: String,
    /// the setup does not go on while it is empty.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
}

/// why an answer does not fit its field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    Empty,
    NotNumber,
    TooSmall(i64),
    TooBig(i64),
    NotAnOption,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("it is required"),
            Self::NotNumber => f.write_str("it is not a whole number"),
            Self::TooSmall(min) => write!(f, "it is below {min}"),
            Self::TooBig(max) => write!(f, "it is above {max}"),
            Self::NotAnOption => f.write_str("it is not one of the options"),
        }
    }
}

impl Field {
    /// whether `answer` fits.
    ///
    /// # Errors
    ///
    /// the reason it does not.
    pub fn check(&self, answer: &str) -> Result<(), Problem> {
        if answer.is_empty() {
            return if self.required {
                Err(Problem::Empty)
            } else {
                Ok(())
            };
        }
        match &self.kind {
            FieldKind::Text { .. } => Ok(()),
            FieldKind::Number { min, max } => {
                let n: i64 = answer.trim().parse().map_err(|_| Problem::NotNumber)?;
                match (min, max) {
                    (Some(min), _) if n < *min => Err(Problem::TooSmall(*min)),
                    (_, Some(max)) if n > *max => Err(Problem::TooBig(*max)),
                    _ => Ok(()),
                }
            }
            FieldKind::Choice { options } => {
                if options.iter().any(|o| o == answer) {
                    Ok(())
                } else {
                    Err(Problem::NotAnOption)
                }
            }
            FieldKind::Check => match answer {
                "true" | "false" => Ok(()),
                _ => Err(Problem::NotAnOption),
            },
        }
    }

    /// the answer as a setting's value: a number field gives a number, a
    /// tick box true or false, anything else text.
    #[must_use]
    pub fn value(&self, answer: &str) -> super::Value {
        match &self.kind {
            FieldKind::Number { .. } => answer
                .trim()
                .parse::<i64>()
                .map_or_else(|_| super::Value::text(answer), |n| super::Value(n.into())),
            FieldKind::Check => super::Value((answer == "true").into()),
            _ => super::Value::text(answer),
        }
    }

    /// whether the answer stays private: not remembered for updates.
    #[must_use]
    pub const fn secret(&self) -> bool {
        matches!(self.kind, FieldKind::Text { secret: true })
    }
}

/// a page of fields, shown in the order of the Kagofile.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InputPage {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    pub fields: Vec<Field>,
    /// not shown when this finds what it looks for; its fields keep their
    /// last answer or default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip: Option<super::check::Detect>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(kind: FieldKind, required: bool) -> Field {
        Field {
            id: "x".into(),
            label: "X".into(),
            kind,
            default: String::new(),
            required,
        }
    }

    #[test]
    fn answers_are_checked_by_kind() {
        let port = field(
            FieldKind::Number {
                min: Some(1),
                max: Some(65535),
            },
            true,
        );
        assert_eq!(port.check(""), Err(Problem::Empty));
        assert_eq!(port.check("abc"), Err(Problem::NotNumber));
        assert_eq!(port.check("0"), Err(Problem::TooSmall(1)));
        assert_eq!(port.check("70000"), Err(Problem::TooBig(65535)));
        assert_eq!(port.check("9000"), Ok(()));
        assert_eq!(port.value("9000"), super::super::Value(9000.into()));
        let mode = field(
            FieldKind::Choice {
                options: vec!["local".into(), "cloud".into()],
            },
            false,
        );
        assert_eq!(mode.check(""), Ok(()), "optional");
        assert_eq!(mode.check("other"), Err(Problem::NotAnOption));
        let tick = field(FieldKind::Check, false);
        assert_eq!(tick.value("true"), super::super::Value(true.into()));
    }
}

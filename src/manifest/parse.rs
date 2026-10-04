use super::schema::RawManifest;
use crate::diagnostic::{self, Diagnostic, Location};

/// TOML text to the raw schema. one diagnostic when it does not fit.
pub(super) fn parse(file: &str, source: &str) -> Result<RawManifest, Diagnostic> {
    toml::from_str(source).map_err(|e| {
        let diagnostic = Diagnostic::error(diagnostic::MANIFEST_SYNTAX, e.message().trim_end());
        match e.span() {
            Some(span) => diagnostic.at(Location::at(file, source, span.start)),
            None => diagnostic,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::schema::OneOrMany;

    const MIN: &str = r#"
schema = 1

[package]
id = "dev.eira.aurea"
name = "Aurea"
version = "1.4.0"
"#;

    #[test]
    fn parses_minimal() {
        let raw = parse("kago.toml", MIN).unwrap();
        assert_eq!(raw.package.id.get_ref(), "dev.eira.aurea");
        assert_eq!(*raw.schema.unwrap().get_ref(), 1);
    }

    #[test]
    fn dotted_and_table_when_are_the_same() {
        let src = format!(
            "{MIN}\n[[files]]\nsource = \"a\"\ntarget = \"{{app}}/a\"\nwhen.os = \"windows\"\nwhen.arch = [\"x86_64\", \"aarch64\"]\n"
        );
        let raw = parse("kago.toml", &src).unwrap();
        let when = raw.files[0].when.as_ref().unwrap();
        assert_eq!(
            when.os.as_ref().unwrap().get_ref(),
            &OneOrMany::One("windows".into())
        );
        assert_eq!(when.arch.as_ref().unwrap().get_ref().values().len(), 2);
    }

    #[test]
    fn unknown_field_points_at_it() {
        let src = format!("{MIN}\n[[shortcut]]\nname = \"x\"\n");
        let d = parse("kago.toml", &src).unwrap_err();
        assert_eq!(d.code, diagnostic::MANIFEST_SYNTAX);
        assert!(d.message.contains("shortcut"), "{}", d.message);
        assert_eq!(d.location.unwrap().line, 9);
    }

    #[test]
    fn target_settings_stay_raw() {
        let src = format!(
            "{MIN}\n[targets.windows]\nartifacts = [\"setup\"]\n\n[targets.windows.msi]\nproduct_version = \"1.4.0\"\n"
        );
        let raw = parse("kago.toml", &src).unwrap();
        let windows = raw.targets.unwrap().windows.unwrap();
        assert_eq!(windows.artifacts[0].get_ref(), "setup");
        assert!(windows.settings.contains_key("msi"));
    }
}

//! how a setup looks beyond the defaults: the `brand` block.

/// all optional; a setup without a `brand` block uses kago's defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Brand {
    /// small text at the bottom left.
    pub text: Option<String>,
    /// main color of the modern look, as RGB.
    pub accent: Option<[u8; 3]>,
    /// build-side PNG shown as the app's mark. the app icon otherwise.
    pub logo: Option<String>,
    /// build-side PNG at the top right of the classic look's pages.
    pub header: Option<String>,
    /// build-side PNG along the left of the welcome and finish pages.
    pub side: Option<String>,
    /// build-side PNG across the top of the modern look, behind the name.
    pub banner: Option<String>,
    /// replaces the welcome text.
    pub welcome: Option<String>,
    /// replaces the finish text.
    pub finish: Option<String>,
    /// a link on the finish page: label, then URL.
    pub link: Option<(String, String)>,
}

/// `#6667ab` or `6667AB` as RGB.
///
/// # Errors
///
/// when it is not six hex digits.
pub fn parse_color(text: &str) -> Result<[u8; 3], String> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    let channel = |i: usize| {
        hex.get(i..i + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
    };
    match (hex.len(), channel(0), channel(2), channel(4)) {
        (6, Some(r), Some(g), Some(b)) => Ok([r, g, b]),
        _ => Err(format!("`{text}` is not a color like #6667ab")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_read_with_or_without_hash() {
        assert_eq!(parse_color("#6667ab"), Ok([0x66, 0x67, 0xab]));
        assert_eq!(parse_color("6667AB"), Ok([0x66, 0x67, 0xab]));
        assert!(parse_color("#666").is_err());
        assert!(parse_color("#66zz00").is_err());
    }
}

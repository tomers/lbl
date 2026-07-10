//! QR code rendering options (error correction, quiet zone, colors).

use once_cell::sync::Lazy;
use regex::Regex;

/// QR error-correction level (redundancy).
///
/// Maps to the [`node-qrcode`](https://www.npmjs.com/package/qrcode) /
/// ISO 18004 levels: L (~7%), M (~15%), Q (~25%), H (~30%).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QrErrorCorrection {
    /// Low (~7% recovery).
    L,
    /// Medium (~15% recovery); default.
    #[default]
    M,
    /// Quartile (~25% recovery).
    Q,
    /// High (~30% recovery).
    H,
}

impl QrErrorCorrection {
    /// The value accepted by the browser QR library.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::L => "L",
            Self::M => "M",
            Self::Q => "Q",
            Self::H => "H",
        }
    }

    /// Parse a config/CLI/HTML attribute value.
    ///
    /// Accepts single-letter codes (`L`/`M`/`Q`/`H`), names (`low`, `medium`,
    /// `quartile`, `high`), and approximate percentages (`7%`, `15%`, …).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "7" | "7%" | "l" | "low" => Some(Self::L),
            "15" | "15%" | "m" | "medium" => Some(Self::M),
            "25" | "25%" | "q" | "quartile" => Some(Self::Q),
            "30" | "30%" | "h" | "high" => Some(Self::H),
            _ => None,
        }
    }
}

static QR_EC_ATTR_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)\b(?:ec|error-correction|errorcorrectionlevel)\s*=\s*"([^"]*)""#)
        .expect("qr ec attr regex")
});
static QR_MARGIN_ATTR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)\bmargin\s*=\s*"([^"]*)""#).expect("qr margin attr regex"));
static QR_DARK_ATTR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)\bdark\s*=\s*"([^"]*)""#).expect("qr dark attr regex"));
static QR_LIGHT_ATTR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)\blight\s*=\s*"([^"]*)""#).expect("qr light attr regex"));
static QR_SIZE_MM_ATTR_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)\b(?:size_mm|size-mm|size)\s*=\s*"([^"]*)""#).expect("qr size attr regex")
});

/// Per-element overrides parsed from a `<qr …>` opening tag.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QrElementOverrides {
    pub error_correction: Option<QrErrorCorrection>,
    pub margin: Option<u32>,
    pub size_mm: Option<f64>,
    pub dark: Option<String>,
    pub light: Option<String>,
}

impl QrElementOverrides {
    /// Parse optional attributes on an authoring `<qr>` element.
    pub fn from_tag_attrs(attrs: &str) -> Self {
        Self {
            error_correction: QR_EC_ATTR_RE
                .captures(attrs)
                .and_then(|c| QrErrorCorrection::parse(&c[1])),
            margin: QR_MARGIN_ATTR_RE
                .captures(attrs)
                .and_then(|c| c[1].trim().parse().ok()),
            size_mm: QR_SIZE_MM_ATTR_RE
                .captures(attrs)
                .and_then(|c| c[1].trim().parse().ok())
                .filter(|v| *v > 0.0),
            dark: QR_DARK_ATTR_RE
                .captures(attrs)
                .map(|c| c[1].trim().to_string())
                .filter(|s| !s.is_empty()),
            light: QR_LIGHT_ATTR_RE
                .captures(attrs)
                .map(|c| c[1].trim().to_string())
                .filter(|s| !s.is_empty()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_error_correction_aliases() {
        assert_eq!(QrErrorCorrection::parse("H"), Some(QrErrorCorrection::H));
        assert_eq!(QrErrorCorrection::parse("high"), Some(QrErrorCorrection::H));
        assert_eq!(QrErrorCorrection::parse("25%"), Some(QrErrorCorrection::Q));
        assert_eq!(QrErrorCorrection::parse("nope"), None);
    }

    #[test]
    fn parses_element_attrs() {
        let o = QrElementOverrides::from_tag_attrs(r#" ec="H" margin="2" "#);
        assert_eq!(o.error_correction, Some(QrErrorCorrection::H));
        assert_eq!(o.margin, Some(2));
    }
}

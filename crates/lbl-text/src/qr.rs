//! QR directive options shared with authoring `<qr>` elements.

/// QR error-correction level (redundancy).
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

    /// Parse a directive or HTML attribute value.
    ///
    /// Accepts single-letter codes (`L`/`M`/`Q`/`H`), names (`low`, `medium`,
    /// `quartile`, `high`), and approximate percentages (`7%`, `15%`, …).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "l" | "low" | "7" | "7%" => Some(Self::L),
            "m" | "medium" | "15" | "15%" => Some(Self::M),
            "q" | "quartile" | "25" | "25%" => Some(Self::Q),
            "h" | "high" | "30" | "30%" => Some(Self::H),
            _ => None,
        }
    }
}

/// Optional QR settings from a directive or flag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QrOptions {
    pub error_correction: Option<QrErrorCorrection>,
    pub margin: Option<u32>,
    pub dark: Option<String>,
    pub light: Option<String>,
}

impl QrOptions {
    /// Serialize non-default options as `<qr …>` attributes.
    pub fn to_attrs(&self) -> String {
        let mut out = String::new();
        if let Some(ec) = self.error_correction {
            out.push_str(&format!(" ec=\"{}\"", ec.as_str()));
        }
        if let Some(m) = self.margin {
            out.push_str(&format!(" margin=\"{m}\""));
        }
        if let Some(d) = &self.dark {
            out.push_str(&format!(" dark=\"{}\"", escape_attr(d)));
        }
        if let Some(l) = &self.light {
            out.push_str(&format!(" light=\"{}\"", escape_attr(l)));
        }
        out
    }
}

/// Parse attributes on a block directive opening tag (`{{qr ec=low margin=2}}`).
pub fn parse_qr_attrs(attrs: &str) -> QrOptions {
    let mut options = QrOptions::default();
    for token in tokenize_attrs(attrs) {
        apply_option(&mut options, &token);
    }
    options
}

fn tokenize_attrs(attrs: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in attrs.chars() {
        match (quote, ch) {
            (None, ' ' | '\t' | '\n' | '\r') if cur.is_empty() => {}
            (None, ' ' | '\t' | '\n' | '\r') => {
                tokens.push(cur.clone());
                cur.clear();
            }
            (Some(q), c) if c == q => {
                quote = None;
                cur.push(c);
            }
            (None, '"' | '\'') => {
                quote = Some(ch);
                cur.push(ch);
            }
            (_, c) => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn apply_option(options: &mut QrOptions, token: &str) {
    let Some((key, val)) = token.split_once('=') else {
        return;
    };
    let key = key.trim().to_ascii_lowercase();
    let val = unquote(val.trim());
    match key.as_str() {
        "ec" | "error-correction" | "errorcorrectionlevel" => {
            options.error_correction = QrErrorCorrection::parse(val);
        }
        "margin" => {
            options.margin = val.parse().ok();
        }
        "dark" => {
            options.dark = Some(val.to_string());
        }
        "light" => {
            options.light = Some(val.to_string());
        }
        _ => {}
    }
}

fn unquote(s: &str) -> &str {
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        return &s[1..s.len() - 1];
    }
    s
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_opening_tag_attrs() {
        let opts = parse_qr_attrs("ec=low margin=2 dark=\"#000\"");
        assert_eq!(opts.error_correction, Some(QrErrorCorrection::L));
        assert_eq!(opts.margin, Some(2));
        assert_eq!(opts.dark.as_deref(), Some("#000"));
    }
}

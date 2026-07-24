//! System font stacks and helpers for `[[font:slug]]` / `data-lbl-font`.
//!
//! Web-font catalogs and face fetching are **not** part of this crate. Callers
//! that want named web fonts supply self-describing [`FontFaceRule`] values
//! through `lbl-transpile-html::FontDelivery`.

use std::collections::HashMap;

/// A single `@font-face` rule ready for injection (no external catalog needed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFaceRule {
    /// Directive / `data-lbl-font` slug (e.g. `roboto`).
    pub slug: String,
    /// CSS `font-family` stack for this slug (e.g. `'Roboto',sans-serif`).
    pub css_family: String,
    pub weight: u16,
    pub unicode_range: Option<String>,
    pub source: FontFaceSource,
}

/// Where a face binary is loaded from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontFaceSource {
    /// Absolute URL for `src:url(...)`.
    Url(String),
    /// Raw `woff2` bytes (emitted as a `data:` URI).
    Bytes(Vec<u8>),
}

/// Hardcoded system stacks (no `@font-face` files).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemFont {
    pub slug: &'static str,
    pub label: &'static str,
    pub css: &'static str,
}

/// System stacks available without a web-font provider.
pub const SYSTEM_FONTS: &[SystemFont] = &[
    SystemFont {
        slug: "sans",
        label: "Sans (system)",
        css: "system-ui,-apple-system,'Segoe UI',Roboto,sans-serif",
    },
    SystemFont {
        slug: "serif",
        label: "Serif (system)",
        css: "Georgia,'Times New Roman',serif",
    },
    SystemFont {
        slug: "mono",
        label: "Monospace (system)",
        css: "ui-monospace,'Cascadia Code','Roboto Mono',monospace",
    },
];

/// Look up a system stack by slug (case-insensitive).
pub fn resolve_system_slug(slug: &str) -> Option<&'static SystemFont> {
    let key = slug.trim().to_ascii_lowercase();
    SYSTEM_FONTS.iter().find(|f| f.slug == key)
}

/// True when `slug` is a non-empty font directive identifier.
pub fn is_font_slug(slug: &str) -> bool {
    !slug.trim().is_empty()
}

/// CSS `font-family` for a system slug, if known.
pub fn system_font_css(slug: &str) -> Option<&'static str> {
    resolve_system_slug(slug).map(|f| f.css)
}

/// Build `@font-face` CSS for one rule.
pub fn font_face_css_rule(rule: &FontFaceRule) -> String {
    let family = css_family_name(&rule.css_family).replace('\'', "\\'");
    let ur = rule
        .unicode_range
        .as_deref()
        .map(|r| format!("unicode-range:{r};"))
        .unwrap_or_default();
    let src = match &rule.source {
        FontFaceSource::Url(url) => {
            format!("url('{}') format('woff2')", url.replace('\'', "%27"))
        }
        FontFaceSource::Bytes(bytes) => {
            format!(
                "url(data:font/woff2;base64,{}) format('woff2')",
                base64_encode(bytes)
            )
        }
    };
    format!(
        "@font-face{{font-family:'{family}';font-style:normal;font-weight:{w};font-display:swap;\
{ur}src:{src};}}\n",
        family = family,
        w = rule.weight,
        ur = ur,
        src = src,
    )
}

/// Prefer the quoted family name from a CSS stack for `@font-face font-family`.
fn css_family_name(css_stack: &str) -> String {
    let t = css_stack.trim();
    if let Some(rest) = t.strip_prefix('\'') {
        if let Some(end) = rest.find('\'') {
            return rest[..end].to_string();
        }
    }
    if let Some(rest) = t.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return rest[..end].to_string();
        }
    }
    t.split(',')
        .next()
        .unwrap_or(t)
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .to_string()
}

/// Group rules by slug (first `css_family` wins for the element rule).
pub fn rules_by_slug(rules: &[FontFaceRule]) -> HashMap<String, Vec<&FontFaceRule>> {
    let mut map: HashMap<String, Vec<&FontFaceRule>> = HashMap::new();
    for rule in rules {
        map.entry(rule.slug.to_ascii_lowercase())
            .or_default()
            .push(rule);
    }
    map
}

fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (a << 16) | (b << 8) | c;
        out.push(T[((n >> 18) & 63) as usize]);
        out.push(T[((n >> 12) & 63) as usize]);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize]
        } else {
            b'='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize]
        } else {
            b'='
        });
    }
    String::from_utf8(out).expect("base64 alphabet is utf8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_system_slugs() {
        assert!(resolve_system_slug("sans").is_some());
        assert!(resolve_system_slug("Mono").is_some());
        assert!(resolve_system_slug("roboto").is_none());
    }

    #[test]
    fn rule_css_url_and_bytes() {
        let url_rule = FontFaceRule {
            slug: "roboto".into(),
            css_family: "'Roboto',sans-serif".into(),
            weight: 400,
            unicode_range: Some("U+0000-00FF".into()),
            source: FontFaceSource::Url("https://example/v1/files/roboto/400-latin.woff2".into()),
        };
        let css = font_face_css_rule(&url_rule);
        assert!(css.contains("@font-face"));
        assert!(css.contains("https://example/v1/files/roboto/"));
        assert!(css.contains("font-family:'Roboto'"));

        let bytes_rule = FontFaceRule {
            slug: "x".into(),
            css_family: "X".into(),
            weight: 400,
            unicode_range: None,
            source: FontFaceSource::Bytes(b"woff".to_vec()),
        };
        let css = font_face_css_rule(&bytes_rule);
        assert!(css.contains("data:font/woff2;base64,"));
    }
}

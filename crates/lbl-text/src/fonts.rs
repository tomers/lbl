//! Supported label fonts and helpers for CSS / web-font loading.

/// A font available for inline `[[font:…]]` directives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontDef {
    /// Slug used in directives (e.g. `roboto`, `bebas-neue`).
    pub slug: &'static str,
    /// CSS `font-family` value.
    pub css: &'static str,
    /// Google Fonts family query fragment, if a web font must be loaded.
    pub google: Option<&'static str>,
}

/// Curated fonts for label printing. System stacks need no web-font load.
pub const FONTS: &[FontDef] = &[
    FontDef {
        slug: "sans",
        css: "system-ui,-apple-system,'Segoe UI',Roboto,sans-serif",
        google: None,
    },
    FontDef {
        slug: "serif",
        css: "Georgia,'Times New Roman',serif",
        google: None,
    },
    FontDef {
        slug: "mono",
        css: "ui-monospace,'Cascadia Code','Roboto Mono',monospace",
        google: None,
    },
    FontDef {
        slug: "roboto",
        css: "'Roboto',sans-serif",
        google: Some("Roboto"),
    },
    FontDef {
        slug: "roboto-mono",
        css: "'Roboto Mono',monospace",
        google: Some("Roboto+Mono"),
    },
    FontDef {
        slug: "open-sans",
        css: "'Open Sans',sans-serif",
        google: Some("Open+Sans"),
    },
    FontDef {
        slug: "lato",
        css: "'Lato',sans-serif",
        google: Some("Lato"),
    },
    FontDef {
        slug: "oswald",
        css: "'Oswald',sans-serif",
        google: Some("Oswald"),
    },
    FontDef {
        slug: "barlow-condensed",
        css: "'Barlow Condensed',sans-serif",
        google: Some("Barlow+Condensed"),
    },
    FontDef {
        slug: "bebas-neue",
        css: "'Bebas Neue',sans-serif",
        google: Some("Bebas+Neue"),
    },
];

/// Look up a font by its directive slug (case-insensitive).
pub fn resolve_slug(slug: &str) -> Option<&'static FontDef> {
    let slug = slug.trim();
    FONTS.iter().find(|f| f.slug.eq_ignore_ascii_case(slug))
}

/// Build a Google Fonts stylesheet URL for the given web-font slugs.
pub fn google_fonts_link(slugs: &[&str]) -> Option<String> {
    let mut families: Vec<&str> = Vec::new();
    for slug in slugs {
        if let Some(def) = resolve_slug(slug) {
            if let Some(g) = def.google {
                if !families.contains(&g) {
                    families.push(g);
                }
            }
        }
    }
    if families.is_empty() {
        return None;
    }
    let query = families
        .iter()
        .map(|f| format!("family={f}"))
        .collect::<Vec<_>>()
        .join("&");
    Some(format!(
        "https://fonts.googleapis.com/css2?{query}&display=swap"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_known_slugs() {
        assert!(resolve_slug("roboto").is_some());
        assert!(resolve_slug("Roboto-Mono").is_some());
        assert!(resolve_slug("unknown").is_none());
    }

    #[test]
    fn google_link_deduplicates() {
        let url = google_fonts_link(&["roboto", "roboto"]).expect("url");
        assert!(url.contains("family=Roboto"));
        assert!(!url.contains("family=Roboto&family=Roboto"));
    }
}

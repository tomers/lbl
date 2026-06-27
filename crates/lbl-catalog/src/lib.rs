//! Curated media catalog: known label/tape SKUs and printer compatibility.
//!
//! The catalog lets users refer to media by a stable key (e.g. `11352` or its
//! alias `S0722520`) instead of raw dimensions. A bundled catalog ships with
//! the crate; users can overlay additional catalog files (TOML/JSON) that take
//! precedence.
//!
//! ```
//! use lbl_catalog::Catalog;
//! use lbl_core::units::Dpi;
//!
//! let catalog = Catalog::bundled().unwrap();
//! let entry = catalog.lookup("S0722520").unwrap();
//! assert_eq!(entry.canonical_key(), "11352");
//! let media = entry.media.to_media(Dpi(300.0));
//! assert_eq!(media.width_mm, 25.0);
//! ```

mod model;

pub use model::{CatalogEntry, ImageInfo, MediaSpec};

use model::CatalogFile;

/// Errors produced when loading the catalog.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    /// A catalog file could not be read.
    #[error("failed to read catalog at {path}: {source}")]
    Io {
        /// Path involved.
        path: String,
        /// Underlying io error.
        source: std::io::Error,
    },
    /// A catalog file could not be parsed.
    #[error("failed to parse catalog: {0}")]
    Parse(String),
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, CatalogError>;

const BUNDLED: &str = include_str!("../data/catalog.toml");

/// An in-memory media catalog: a set of [`CatalogEntry`] resolvable by key,
/// printer compatibility, or free-text search.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    entries: Vec<CatalogEntry>,
}

impl Catalog {
    /// Load only the catalog bundled with the crate.
    pub fn bundled() -> Result<Self> {
        let mut catalog = Catalog::default();
        catalog.merge_toml(BUNDLED)?;
        Ok(catalog)
    }

    /// Load the bundled catalog, then overlay user-supplied files (later files
    /// take precedence; entries sharing any key replace earlier ones).
    pub fn load_with_overlays<P: AsRef<std::path::Path>>(paths: &[P]) -> Result<Self> {
        let mut catalog = Catalog::bundled()?;
        for path in paths {
            let path = path.as_ref();
            let text = std::fs::read_to_string(path).map_err(|source| CatalogError::Io {
                path: path.display().to_string(),
                source,
            })?;
            match path.extension().and_then(|e| e.to_str()) {
                Some("json") => catalog.merge_json(&text)?,
                _ => catalog.merge_toml(&text)?,
            }
        }
        Ok(catalog)
    }

    fn merge_toml(&mut self, text: &str) -> Result<()> {
        let file: CatalogFile = toml::from_str(text).map_err(|e| CatalogError::Parse(e.to_string()))?;
        self.merge_entries(file.entries);
        Ok(())
    }

    fn merge_json(&mut self, text: &str) -> Result<()> {
        let file: CatalogFile =
            serde_json::from_str(text).map_err(|e| CatalogError::Parse(e.to_string()))?;
        self.merge_entries(file.entries);
        Ok(())
    }

    fn merge_entries(&mut self, entries: Vec<CatalogEntry>) {
        for entry in entries {
            // Replace any existing entry that shares a key.
            self.entries
                .retain(|e| !entry.keys.iter().any(|k| e.matches_key(k)));
            self.entries.push(entry);
        }
    }

    /// All entries, in insertion order.
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    /// Resolve an entry by any of its keys/aliases (case-insensitive).
    pub fn lookup(&self, key: &str) -> Option<&CatalogEntry> {
        self.entries.iter().find(|e| e.matches_key(key))
    }

    /// All entries compatible with the given printer model string.
    pub fn compatible_with(&self, printer_model: &str) -> Vec<&CatalogEntry> {
        self.entries
            .iter()
            .filter(|e| e.is_compatible_with(printer_model))
            .collect()
    }

    /// Free-text search over keys, name, and brand (case-insensitive).
    pub fn search(&self, query: &str) -> Vec<&CatalogEntry> {
        let q = query.to_ascii_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                e.name.to_ascii_lowercase().contains(&q)
                    || e.brand.to_ascii_lowercase().contains(&q)
                    || e.keys.iter().any(|k| k.to_ascii_lowercase().contains(&q))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_loads_and_resolves_aliases() {
        let catalog = Catalog::bundled().unwrap();
        assert!(!catalog.entries().is_empty());
        let by_sku = catalog.lookup("11352").unwrap();
        let by_alias = catalog.lookup("s0722520").unwrap();
        assert_eq!(by_sku.canonical_key(), by_alias.canonical_key());
        assert_eq!(by_sku.media.width_mm, 25.0);
    }

    #[test]
    fn compatibility_filter_works() {
        let catalog = Catalog::bundled().unwrap();
        let lw = catalog.compatible_with("DYMO LabelWriter 550");
        assert!(lw.iter().any(|e| e.matches_key("2191636")));
        assert!(lw.iter().any(|e| e.matches_key("11352")));
    }

    #[test]
    fn overlay_replaces_by_key() {
        let mut catalog = Catalog::bundled().unwrap();
        catalog
            .merge_toml(
                r#"
                [[entries]]
                brand = "DYMO"
                keys = ["11352"]
                name = "Custom override"
                media = { width_mm = 25.0, length = { kind = "fixed", mm = 54.0 } }
                "#,
            )
            .unwrap();
        assert_eq!(catalog.lookup("11352").unwrap().name, "Custom override");
    }
}

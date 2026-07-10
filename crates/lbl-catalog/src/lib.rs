//! Curated catalog of known printers and media.
//!
//! The catalog lets users refer to media by a stable key (e.g. `11352` or its
//! alias `S0722520`) and printers by model key (e.g. `LabelWriter 550`). Each
//! printer entry lists supported media keys and carries native capabilities such
//! as DPI. A bundled catalog ships with the crate; users can overlay additional
//! catalog files (TOML/JSON) that take precedence.
//!
//! ```
//! use lbl_catalog::Catalog;
//!
//! let catalog = Catalog::bundled().unwrap();
//! let printer = catalog.lookup_printer("LabelWriter 550").unwrap();
//! assert_eq!(printer.dpi, 300.0);
//! let media = catalog.compatible_with("LabelWriter 550");
//! assert!(media.iter().any(|e| e.matches_key("11352")));
//! ```

mod model;

pub use model::{
    encode_capabilities_for, CatalogEntry, ConnectionHint, ImageInfo, MediaSpec, PrinterEntry,
    ResolvedTransport,
};

use lbl_core::printer::Protocol;
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

/// Default DPI assumed by the CLI when the user does not pass `--dpi`.
pub const CLI_DEFAULT_DPI: f64 = 300.0;

/// Result of resolving a free-form printer query against the catalog.
#[derive(Debug, Clone, PartialEq)]
pub enum PrinterLookup<'a> {
    /// Exactly one catalog entry matches.
    Found(&'a PrinterEntry),
    /// No catalog entry matches.
    NotFound,
    /// More than one entry matches the query.
    Ambiguous(Vec<&'a PrinterEntry>),
}

const BUNDLED: &str = include_str!("../data/catalog.toml");

fn split_printer_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            current.push(c);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// An in-memory catalog of known media and printers.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    entries: Vec<CatalogEntry>,
    printers: Vec<PrinterEntry>,
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
        let file: CatalogFile =
            toml::from_str(text).map_err(|e| CatalogError::Parse(e.to_string()))?;
        self.merge_entries(file.entries);
        self.merge_printers(file.printers);
        Ok(())
    }

    fn merge_json(&mut self, text: &str) -> Result<()> {
        let file: CatalogFile =
            serde_json::from_str(text).map_err(|e| CatalogError::Parse(e.to_string()))?;
        self.merge_entries(file.entries);
        self.merge_printers(file.printers);
        Ok(())
    }

    fn merge_entries(&mut self, entries: Vec<CatalogEntry>) {
        for entry in entries {
            self.entries
                .retain(|e| !entry.keys.iter().any(|k| e.matches_key(k)));
            self.entries.push(entry);
        }
    }

    fn merge_printers(&mut self, printers: Vec<PrinterEntry>) {
        for printer in printers {
            self.printers
                .retain(|p| !printer.keys.iter().any(|k| p.matches_key(k)));
            self.printers.push(printer);
        }
    }

    /// All media entries, in insertion order.
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    /// All printer entries, in insertion order.
    pub fn printers(&self) -> &[PrinterEntry] {
        &self.printers
    }

    /// Resolve a media entry by any of its keys/aliases (case-insensitive).
    pub fn lookup(&self, key: &str) -> Option<&CatalogEntry> {
        self.entries.iter().find(|e| e.matches_key(key))
    }

    /// Resolve a media entry by manufacturer product id (e.g. NIIMBOT RFID code).
    pub fn lookup_by_product_id(&self, id: &str) -> Option<&CatalogEntry> {
        self.entries.iter().find(|e| e.matches_product_id(id))
    }

    /// Resolve a printer entry by any of its keys/aliases (case-insensitive).
    pub fn lookup_printer(&self, key: &str) -> Option<&PrinterEntry> {
        self.printers.iter().find(|p| p.matches_key(key))
    }

    /// Resolve a printer entry by key/alias, falling back to a free-form
    /// model-string match when it identifies exactly one catalog entry.
    pub fn resolve_printer(&self, key: &str) -> Option<&PrinterEntry> {
        match self.lookup_printer_query(key) {
            PrinterLookup::Found(printer) => Some(printer),
            PrinterLookup::NotFound | PrinterLookup::Ambiguous(_) => None,
        }
    }

    /// Resolve a printer query, distinguishing not-found from ambiguous matches.
    pub fn lookup_printer_query(&self, query: &str) -> PrinterLookup<'_> {
        if let Some(printer) = self.lookup_printer(query) {
            return PrinterLookup::Found(printer);
        }
        let matches = self.matching_printers(query);
        match matches.len() {
            0 => PrinterLookup::NotFound,
            1 => PrinterLookup::Found(matches[0]),
            _ => {
                let max_score = matches
                    .iter()
                    .filter_map(|p| p.match_score(query))
                    .max()
                    .unwrap_or(0);
                let best: Vec<_> = matches
                    .iter()
                    .filter(|p| p.match_score(query) == Some(max_score))
                    .copied()
                    .collect();
                if best.len() == 1 {
                    PrinterLookup::Found(best[0])
                } else {
                    PrinterLookup::Ambiguous(matches)
                }
            }
        }
    }

    /// All catalog printers matching a free-form model string.
    pub fn matching_printers(&self, printer_model: &str) -> Vec<&PrinterEntry> {
        self.printers
            .iter()
            .filter(|p| p.matches_model(printer_model))
            .collect()
    }

    /// Shortest `--printer` term that matches only this entry in the catalog.
    pub fn suggest_unique_printer_term(&self, printer: &PrinterEntry) -> String {
        let mut candidates: Vec<(u8, String)> = Vec::new();

        let mut keys: Vec<String> = printer.keys.clone();
        keys.sort_by_key(|k| k.len());
        for key in keys {
            candidates.push((0, key));
        }

        for source in printer.keys.iter().chain(std::iter::once(&printer.name)) {
            let words = split_printer_tokens(source);
            for token in &words {
                if token.len() >= 2 {
                    candidates.push((1, token.clone()));
                }
            }
            for pair in words.windows(2) {
                candidates.push((2, format!("{} {}", pair[0], pair[1])));
            }
        }

        for key in &printer.keys {
            let chars: Vec<char> = key.chars().collect();
            for len in 4..=chars.len() {
                for start in 0..=chars.len() - len {
                    let sub: String = chars[start..start + len].iter().collect();
                    if sub.trim().is_empty() {
                        continue;
                    }
                    candidates.push((3, sub));
                }
            }
        }

        candidates.sort_by(|(ta, a), (tb, b)| {
            ta.cmp(tb)
                .then_with(|| a.len().cmp(&b.len()))
                .then_with(|| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()))
        });

        let mut seen = std::collections::HashSet::new();
        for (_, term) in candidates {
            let norm = term.to_ascii_lowercase();
            if !seen.insert(norm) {
                continue;
            }
            if self.is_unique_printer_term(printer, &term) {
                return term;
            }
        }

        printer.canonical_key().to_string()
    }

    fn is_unique_printer_term(&self, printer: &PrinterEntry, term: &str) -> bool {
        let matches = self.matching_printers(term);
        matches.len() == 1 && matches[0].canonical_key() == printer.canonical_key()
    }

    /// Human-readable error for an ambiguous printer query.
    pub fn ambiguous_printer_message(&self, query: &str, matches: &[&PrinterEntry]) -> String {
        let mut lines = vec![format!(
            "ambiguous printer '{query}': multiple catalog entries match:"
        )];
        for printer in matches {
            let suggest = self.suggest_unique_printer_term(printer);
            lines.push(format!(
                "  {} ({}) — try: --printer {suggest}",
                printer.name,
                printer.canonical_key()
            ));
        }
        lines.join("\n")
    }

    /// Resolve a printer query or return a user-facing error message.
    pub fn require_printer(&self, query: &str) -> std::result::Result<&PrinterEntry, String> {
        match self.lookup_printer_query(query) {
            PrinterLookup::Found(printer) => Ok(printer),
            PrinterLookup::NotFound => Err(format!("unknown printer '{query}'")),
            PrinterLookup::Ambiguous(matches) => {
                Err(self.ambiguous_printer_message(query, &matches))
            }
        }
    }

    /// Find a printer entry matching a free-form model string when the match
    /// is unambiguous.
    pub fn match_printer(&self, printer_model: &str) -> Option<&PrinterEntry> {
        self.resolve_printer(printer_model)
    }

    /// Find a printer entry that matches a USB vendor/product id.
    ///
    /// Prefers an exact product match, then a vendor-wildcard match.
    pub fn match_usb(&self, vendor_id: u16, product_id: u16) -> Option<&PrinterEntry> {
        self.printers
            .iter()
            .find(|p| {
                p.connections
                    .iter()
                    .any(|c| c.is_exact_usb_match(vendor_id, product_id))
            })
            .or_else(|| {
                self.printers.iter().find(|p| {
                    p.connections.iter().any(|c| {
                        matches!(
                            c,
                            ConnectionHint::Usb {
                                vendor_id: vid,
                                product_id: None,
                            } if *vid == vendor_id
                        )
                    })
                })
            })
    }

    /// Media entries supported by the given printer model string.
    pub fn compatible_with(&self, printer_model: &str) -> Vec<&CatalogEntry> {
        let Some(printer) = self.match_printer(printer_model) else {
            return Vec::new();
        };
        self.media_for_printer(printer)
    }

    /// Media entries listed in a printer's `supported_media`.
    pub fn media_for_printer(&self, printer: &PrinterEntry) -> Vec<&CatalogEntry> {
        printer
            .supported_media
            .iter()
            .filter_map(|key| self.lookup(key))
            .collect()
    }

    /// Whether a media key is supported by the given printer model.
    ///
    /// Accepts any catalog alias for a supported SKU (e.g. `30252` when the
    /// printer lists canonical key `99010`).
    pub fn supports_media(&self, printer_model: &str, media_key: &str) -> bool {
        let Some(printer) = self.match_printer(printer_model) else {
            return false;
        };
        printer.supported_media.iter().any(|supported_key| {
            supported_key.eq_ignore_ascii_case(media_key)
                || self
                    .lookup(supported_key)
                    .zip(self.lookup(media_key))
                    .is_some_and(|(supported, requested)| {
                        supported.canonical_key() == requested.canonical_key()
                    })
        })
    }

    /// Resolve the effective DPI for a print run.
    ///
    /// An explicit non-default `cli_dpi` wins. Otherwise the catalog supplies
    /// the printer's native DPI when `printer_key` or `protocol` identifies a
    /// known model.
    pub fn resolve_dpi(&self, printer_key: Option<&str>, protocol: Protocol, cli_dpi: f64) -> f64 {
        if (cli_dpi - CLI_DEFAULT_DPI).abs() > f64::EPSILON {
            return cli_dpi;
        }
        if let Some(key) = printer_key {
            if let Some(printer) = self.resolve_printer(key) {
                return printer.dpi;
            }
        }
        self.dpi_for_protocol(protocol).unwrap_or(cli_dpi)
    }

    fn dpi_for_protocol(&self, protocol: Protocol) -> Option<f64> {
        let dpis: Vec<f64> = self
            .printers
            .iter()
            .filter(|p| p.protocol == protocol)
            .map(|p| p.dpi)
            .collect();
        if dpis.is_empty() {
            None
        } else if dpis.iter().all(|d| (*d - dpis[0]).abs() < f64::EPSILON) {
            Some(dpis[0])
        } else {
            None
        }
    }

    /// Free-text search over media keys, name, and brand (case-insensitive).
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

    /// Free-text search over printer keys, name, and brand (case-insensitive).
    pub fn search_printers(&self, query: &str) -> Vec<&PrinterEntry> {
        let q = query.to_ascii_lowercase();
        self.printers
            .iter()
            .filter(|p| {
                p.name.to_ascii_lowercase().contains(&q)
                    || p.brand.to_ascii_lowercase().contains(&q)
                    || p.keys.iter().any(|k| k.to_ascii_lowercase().contains(&q))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::units::Dpi;

    #[test]
    fn bundled_loads_and_resolves_aliases() {
        let catalog = Catalog::bundled().unwrap();
        assert!(!catalog.entries().is_empty());
        assert!(!catalog.printers().is_empty());
        let by_sku = catalog.lookup("11352").unwrap();
        let by_alias = catalog.lookup("s0722520").unwrap();
        assert_eq!(by_sku.canonical_key(), by_alias.canonical_key());
        assert_eq!(by_sku.media.width_mm, 25.0);
        let us_address = catalog.lookup("30252").unwrap();
        let eu_address = catalog.lookup("99010").unwrap();
        assert_eq!(us_address.canonical_key(), eu_address.canonical_key());
        assert_eq!(us_address.media.width_mm, 28.0);
    }

    #[test]
    fn brother_ql820_is_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        let printer = catalog.lookup_printer("QL-820NWBc").unwrap();
        assert_eq!(printer.protocol, Protocol::BrotherQl);
        assert_eq!(printer.dpi, 300.0);
        assert!(printer.supports_cut);
        assert!(catalog.supports_media("QL-820NWBc", "DK-11201"));
        assert!(catalog.supports_media("QL820NWB", "DK-22205"));
        let matched = catalog.match_usb(0x04f9, 0x209d).unwrap();
        assert_eq!(matched.canonical_key(), "QL-820NWBc");
    }

    #[test]
    fn compatibility_filter_works() {
        let catalog = Catalog::bundled().unwrap();
        let lw = catalog.compatible_with("DYMO LabelWriter 550");
        assert!(lw.iter().any(|e| e.matches_key("2191636")));
        assert!(lw.iter().any(|e| e.matches_key("11352")));
        assert!(!lw.iter().any(|e| e.matches_key("2166659")));
    }

    #[test]
    fn supports_media_resolves_aliases() {
        let catalog = Catalog::bundled().unwrap();
        assert!(catalog.supports_media("LabelWriter 550", "99010"));
        assert!(catalog.supports_media("LabelWriter 550", "30252"));
        assert!(!catalog.supports_media("LabelWriter 550", "2166659"));
    }

    #[test]
    fn labelmanager_d1_tape_is_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        let tape = catalog.lookup("45013S").unwrap();
        assert_eq!(tape.canonical_key(), "45013");
        assert_eq!(tape.media.width_mm, 12.0);
        assert!(matches!(
            tape.media.length,
            lbl_core::media::MediaLength::Continuous
        ));
        let lm280 = catalog.compatible_with("LabelManager 280");
        assert!(lm280.iter().any(|e| e.matches_key("45013")));
        assert!(lm280.iter().any(|e| e.matches_key("45013S")));
        assert!(lm280.iter().any(|e| e.matches_key("S0720530S")));
        let printer = catalog.lookup_printer("LM280").unwrap();
        assert_eq!(printer.feed_trail_mm, Some(8.1));
        assert_eq!(printer.head_printable_height_mm, Some(8.2));
        assert!(printer.feed_reverse);
        assert_eq!(printer.default_media.as_deref(), Some("45013"));
    }

    #[test]
    fn niimbot_d110_media_is_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        let roll = catalog.lookup("12x40").unwrap();
        assert_eq!(roll.brand, "NIIMBOT");
        assert_eq!(roll.media.width_mm, 12.0);
        let d110 = catalog.compatible_with("NIIMBOT D110");
        assert!(d110.iter().any(|e| e.matches_key("12x40")));
        let printer = catalog.lookup_printer("D110").unwrap();
        assert_eq!(printer.dpi, 203.0);
    }

    #[test]
    fn resolve_dpi_from_printer() {
        let catalog = Catalog::bundled().unwrap();
        assert_eq!(
            catalog.resolve_dpi(Some("D110"), Protocol::Niimbot, CLI_DEFAULT_DPI),
            203.0
        );
        assert_eq!(
            catalog.resolve_dpi(None, Protocol::Niimbot, CLI_DEFAULT_DPI),
            203.0
        );
        assert_eq!(
            catalog.resolve_dpi(Some("LabelWriter 550"), Protocol::DymoLw, CLI_DEFAULT_DPI),
            300.0
        );
        assert_eq!(catalog.resolve_dpi(None, Protocol::DymoLw, 600.0), 600.0);
    }

    #[test]
    fn usb_match_prefers_exact_product() {
        let catalog = Catalog::bundled().unwrap();
        let lw550 = catalog.match_usb(0x0922, 0x0028).unwrap();
        assert!(lw550.matches_key("LabelWriter 550"));
        assert!(!lw550.matches_key("LabelWriter"));
        let turbo = catalog.match_usb(0x0922, 0x0029).unwrap();
        assert!(turbo.matches_key("LabelWriter 550 Turbo"));
        let lm280 = catalog.match_usb(0x0922, 0x1005).unwrap();
        assert!(lm280.matches_key("LabelManager 280"));
        let lm280_printer = catalog.match_usb(0x0922, 0x1006).unwrap();
        assert!(lm280_printer.matches_key("LabelManager 280"));
    }

    #[test]
    fn usb_vendor_wildcard_only_when_no_exact_product_match() {
        let catalog = Catalog::bundled().unwrap();
        // Known 550 PID → specific entry, not the legacy vendor wildcard.
        assert!(catalog
            .match_usb(0x0922, 0x0028)
            .unwrap()
            .matches_key("LabelWriter 550"));
        // Unlisted DYMO PID → legacy fallback.
        let legacy = catalog.match_usb(0x0922, 0x0042).unwrap();
        assert!(legacy.matches_key("LabelWriter"));
        assert!(!legacy.matches_key("LabelWriter 550"));
    }

    #[test]
    fn d110_defaults_to_bluetooth() {
        let catalog = Catalog::bundled().unwrap();
        let d110 = catalog.lookup_printer("D110").unwrap();
        let transport = d110.default_transport();
        assert_eq!(transport.bluetooth.as_deref(), Some("D110"));
        assert!(transport.serial.is_none());
    }

    #[test]
    fn resolve_printer_falls_back_to_match() {
        let catalog = Catalog::bundled().unwrap();
        assert!(catalog.lookup_printer("LW550").is_some());
        let printer = catalog.resolve_printer("LW550").unwrap();
        assert!(printer.matches_key("LabelWriter 550"));
    }

    #[test]
    fn resolve_printer_prefers_specific_labelwriter_key() {
        let catalog = Catalog::bundled().unwrap();
        let printer = catalog.resolve_printer("DYMO LabelWriter 550").unwrap();
        assert!(printer.matches_key("LabelWriter 550"));
        assert!(!printer.matches_key("LabelWriter"));
    }

    #[test]
    fn ambiguous_printer_query_fails() {
        let catalog = Catalog::bundled().unwrap();
        assert!(matches!(
            catalog.lookup_printer_query("550"),
            PrinterLookup::Ambiguous(_)
        ));
        assert!(catalog.resolve_printer("550").is_none());
        let err = catalog.require_printer("550").unwrap_err();
        assert!(err.contains("ambiguous printer '550'"));
        assert!(err.contains("DYMO LabelWriter 550"));
        assert!(err.contains("DYMO LabelWriter 550 Turbo"));
        assert!(err.contains("--printer Turbo") || err.contains("--printer turbo"));
    }

    #[test]
    fn suggest_unique_printer_terms_for_550_family() {
        let catalog = Catalog::bundled().unwrap();
        let base = catalog.lookup_printer("LabelWriter 550").unwrap();
        let turbo = catalog.lookup_printer("LabelWriter 550 Turbo").unwrap();
        let base_term = catalog.suggest_unique_printer_term(base);
        let turbo_term = catalog.suggest_unique_printer_term(turbo);
        assert_eq!(catalog.matching_printers(&base_term).len(), 1);
        assert_eq!(catalog.matching_printers(&turbo_term).len(), 1);
        assert!(base.matches_model(&base_term));
        assert!(turbo.matches_model(&turbo_term));
        assert_ne!(
            base_term.to_ascii_lowercase(),
            turbo_term.to_ascii_lowercase()
        );
    }

    #[test]
    fn lookup_by_product_id_resolves_niimbot_roll() {
        let catalog = Catalog::bundled().unwrap();
        let entry = catalog.lookup_by_product_id("10262260").unwrap();
        assert!(entry.matches_key("50x30"));
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

                [[printers]]
                brand = "DYMO"
                keys = ["LabelWriter 550"]
                name = "Custom printer"
                protocol = "dymolw"
                dpi = 300.0
                max_width_mm = 57.0
                supported_media = ["11352"]
                "#,
            )
            .unwrap();
        assert_eq!(catalog.lookup("11352").unwrap().name, "Custom override");
        assert_eq!(
            catalog.lookup_printer("LabelWriter 550").unwrap().name,
            "Custom printer"
        );
    }

    #[test]
    fn media_spec_to_device_media() {
        let catalog = Catalog::bundled().unwrap();
        let entry = catalog.lookup("11352").unwrap();
        let media = entry.media.to_media(Dpi(300.0));
        assert_eq!(media.width_mm, 25.0);
    }
}

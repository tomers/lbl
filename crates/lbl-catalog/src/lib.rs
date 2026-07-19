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
//! let printer = catalog.lookup_device("LabelWriter 550").unwrap();
//! assert_eq!(printer.dpi, 300.0);
//! let media = catalog.compatible_with("LabelWriter 550");
//! assert!(media.iter().any(|e| e.matches_key("11352")));
//! ```

mod model;
mod validate;

pub use model::{
    encode_capabilities_for, CatalogEntry, ConnectionHint, DeviceEntry, DeviceRole, DeviceSupport,
    ImageInfo, Maturity, MediaSpec, ResolvedTransport,
};
pub use validate::{validate_catalog_geometry, CatalogGeometryError};

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
pub enum DeviceLookup<'a> {
    /// Exactly one catalog entry matches.
    Found(&'a DeviceEntry),
    /// No catalog entry matches.
    NotFound,
    /// More than one entry matches the query.
    Ambiguous(Vec<&'a DeviceEntry>),
}

const BUNDLED: &str = include_str!("../data/catalog.toml");

fn split_device_tokens(s: &str) -> Vec<String> {
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
    devices: Vec<DeviceEntry>,
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
        if text.contains("[[printers]]") {
            return Err(CatalogError::Parse(
                "catalog uses obsolete [[printers]] tables; rename them to [[devices]]".into(),
            ));
        }
        let file: CatalogFile =
            toml::from_str(text).map_err(|e| CatalogError::Parse(e.to_string()))?;
        self.merge_entries(file.entries);
        self.merge_devices(file.devices);
        Ok(())
    }

    fn merge_json(&mut self, text: &str) -> Result<()> {
        let file: CatalogFile =
            serde_json::from_str(text).map_err(|e| CatalogError::Parse(e.to_string()))?;
        self.merge_entries(file.entries);
        self.merge_devices(file.devices);
        Ok(())
    }

    fn merge_entries(&mut self, entries: Vec<CatalogEntry>) {
        for entry in entries {
            self.entries
                .retain(|e| !entry.keys.iter().any(|k| e.matches_key(k)));
            self.entries.push(entry);
        }
    }

    fn merge_devices(&mut self, devices: Vec<DeviceEntry>) {
        for device in devices {
            self.devices
                .retain(|p| !device.keys.iter().any(|k| p.matches_key(k)));
            self.devices.push(device);
        }
    }

    /// All media entries, in insertion order.
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    /// All device entries, in insertion order.
    pub fn devices(&self) -> &[DeviceEntry] {
        &self.devices
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
    pub fn lookup_device(&self, key: &str) -> Option<&DeviceEntry> {
        self.devices.iter().find(|p| p.matches_key(key))
    }

    /// Resolve a printer entry by key/alias, falling back to a free-form
    /// model-string match when it identifies exactly one catalog entry.
    pub fn resolve_device(&self, key: &str) -> Option<&DeviceEntry> {
        match self.lookup_device_query(key) {
            DeviceLookup::Found(printer) => Some(printer),
            DeviceLookup::NotFound | DeviceLookup::Ambiguous(_) => None,
        }
    }

    /// Resolve a printer query, distinguishing not-found from ambiguous matches.
    pub fn lookup_device_query(&self, query: &str) -> DeviceLookup<'_> {
        if let Some(printer) = self.lookup_device(query) {
            return DeviceLookup::Found(printer);
        }
        let matches = self.matching_devices(query);
        match matches.len() {
            0 => DeviceLookup::NotFound,
            1 => DeviceLookup::Found(matches[0]),
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
                    DeviceLookup::Found(best[0])
                } else {
                    DeviceLookup::Ambiguous(matches)
                }
            }
        }
    }

    /// All catalog printers matching a free-form model string.
    pub fn matching_devices(&self, printer_model: &str) -> Vec<&DeviceEntry> {
        self.devices
            .iter()
            .filter(|p| p.matches_model(printer_model))
            .collect()
    }

    /// Shortest `--printer` term that matches only this entry in the catalog.
    pub fn suggest_unique_device_term(&self, printer: &DeviceEntry) -> String {
        let mut candidates: Vec<(u8, String)> = Vec::new();

        let mut keys: Vec<String> = printer.keys.clone();
        keys.sort_by_key(|k| k.len());
        for key in keys {
            candidates.push((0, key));
        }

        for source in printer.keys.iter().chain(std::iter::once(&printer.name)) {
            let words = split_device_tokens(source);
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
            if self.is_unique_device_term(printer, &term) {
                return term;
            }
        }

        printer.canonical_key().to_string()
    }

    fn is_unique_device_term(&self, printer: &DeviceEntry, term: &str) -> bool {
        let matches = self.matching_devices(term);
        matches.len() == 1 && matches[0].canonical_key() == printer.canonical_key()
    }

    /// Human-readable error for an ambiguous printer query.
    pub fn ambiguous_device_message(&self, query: &str, matches: &[&DeviceEntry]) -> String {
        let mut lines = vec![format!(
            "ambiguous printer '{query}': multiple catalog entries match:"
        )];
        for printer in matches {
            let suggest = self.suggest_unique_device_term(printer);
            lines.push(format!(
                "  {} ({}) — try: --printer {suggest}",
                printer.name,
                printer.canonical_key()
            ));
        }
        lines.join("\n")
    }

    /// Resolve a printer query or return a user-facing error message.
    pub fn require_device(&self, query: &str) -> std::result::Result<&DeviceEntry, String> {
        match self.lookup_device_query(query) {
            DeviceLookup::Found(printer) => Ok(printer),
            DeviceLookup::NotFound => Err(format!("unknown printer '{query}'")),
            DeviceLookup::Ambiguous(matches) => Err(self.ambiguous_device_message(query, &matches)),
        }
    }

    /// Find a printer entry matching a free-form model string when the match
    /// is unambiguous.
    pub fn match_printer(&self, printer_model: &str) -> Option<&DeviceEntry> {
        self.resolve_device(printer_model)
    }

    /// Find a printer entry that matches a USB vendor/product id.
    ///
    /// Prefers an exact product match, then a vendor-wildcard match.
    pub fn match_usb(&self, vendor_id: u16, product_id: u16) -> Option<&DeviceEntry> {
        self.devices
            .iter()
            .find(|p| {
                p.connections
                    .iter()
                    .any(|c| c.is_exact_usb_match(vendor_id, product_id))
            })
            .or_else(|| {
                self.devices.iter().find(|p| {
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
        self.media_for_device(printer)
    }

    /// Media entries listed in a printer's `supported_media`.
    pub fn media_for_device(&self, printer: &DeviceEntry) -> Vec<&CatalogEntry> {
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
            if let Some(printer) = self.resolve_device(key) {
                return printer.dpi;
            }
        }
        self.dpi_for_protocol(protocol).unwrap_or(cli_dpi)
    }

    fn dpi_for_protocol(&self, protocol: Protocol) -> Option<f64> {
        let dpis: Vec<f64> = self
            .devices
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
    pub fn search_devices(&self, query: &str) -> Vec<&DeviceEntry> {
        let q = query.to_ascii_lowercase();
        self.devices
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
        assert!(!catalog.devices().is_empty());
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
        let printer = catalog.lookup_device("QL-820NWBc").unwrap();
        assert_eq!(printer.protocol, Protocol::BrotherQl);
        assert_eq!(printer.dpi, 300.0);
        assert!(printer.supports_cut);
        assert!(catalog.supports_media("QL-820NWBc", "DK-11201"));
        assert!(catalog.supports_media("QL820NWB", "DK-22205"));
        let matched = catalog.match_usb(0x04f9, 0x209d).unwrap();
        assert_eq!(matched.canonical_key(), "QL-820NWBc");
    }

    #[test]
    fn brother_ql500_family_dialect_caps() {
        let catalog = Catalog::bundled().unwrap();

        let ql500 = catalog.lookup_device("QL-500").unwrap();
        assert_eq!(ql500.protocol, Protocol::BrotherQl);
        assert_eq!(ql500.maturity, Maturity::Experimental);
        assert!(!ql500.supports_cut);
        assert!(!ql500.supports_expanded_mode);
        assert!(!ql500.supports_cut_every);
        assert!(!ql500.emit_raster_mode_switch);
        assert_eq!(ql500.invalidate_bytes, Some(200));
        assert_eq!(
            catalog.match_usb(0x04f9, 0x2015).unwrap().canonical_key(),
            "QL-500"
        );

        let ql550 = catalog.lookup_device("QL-550").unwrap();
        assert!(ql550.supports_cut);
        assert!(!ql550.supports_expanded_mode);
        assert!(!ql550.supports_cut_every);
        assert!(!ql550.emit_raster_mode_switch);

        let ql560 = catalog.lookup_device("QL-560").unwrap();
        assert!(ql560.supports_cut_every);
        assert!(ql560.supports_expanded_mode);
        assert!(!ql560.emit_raster_mode_switch);

        let ql580 = catalog.lookup_device("QL-580N").unwrap();
        assert!(ql580.emit_raster_mode_switch);
        assert!(ql580.supports_cut_every);

        let ql650 = catalog.lookup_device("QL-650TD").unwrap();
        assert!(ql650.supports_expanded_mode);
        assert!(!ql650.supports_cut_every);
        assert!(ql650.emit_raster_mode_switch);
        assert_eq!(
            catalog.match_usb(0x04f9, 0x201b).unwrap().canonical_key(),
            "QL-650TD"
        );
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
        let printer = catalog.lookup_device("LM280").unwrap();
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
        let wide = catalog.lookup("15x30").unwrap();
        assert_eq!(wide.media.width_mm, 15.0);
        assert!(matches!(
            wide.media.length,
            lbl_core::media::MediaLength::Fixed(30.0)
        ));
        let d110 = catalog.compatible_with("NIIMBOT D110");
        assert!(d110.iter().any(|e| e.matches_key("12x40")));
        assert!(d110.iter().any(|e| e.matches_key("15x30")));
        let printer = catalog.lookup_device("D110").unwrap();
        assert_eq!(printer.dpi, 203.0);
        assert_eq!(printer.max_width_mm, 12.0);
        assert!(printer.reports_media);
    }

    #[test]
    fn niimbot_b1_stores_physical_50mm_stock() {
        let catalog = Catalog::bundled().unwrap();
        let label = catalog.lookup("50x30").unwrap();
        assert_eq!(label.media.width_mm, 50.0);
        let printer = catalog.lookup_device("B1").unwrap();
        assert_eq!(printer.max_width_mm, 48.0);
    }

    #[test]
    fn bundled_catalog_geometry_is_consistent() {
        let catalog = Catalog::bundled().unwrap();
        validate_catalog_geometry(&catalog).unwrap_or_else(|errors| {
            let joined = errors
                .iter()
                .map(|e| e.message.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            panic!("catalog geometry errors:\n{joined}");
        });
    }

    #[test]
    fn niimbot_printers_report_media() {
        let catalog = Catalog::bundled().unwrap();
        let missing: Vec<_> = catalog
            .devices()
            .iter()
            .filter(|p| p.protocol == Protocol::Niimbot && !p.reports_media)
            .map(|p| p.canonical_key())
            .collect();
        assert!(
            missing.is_empty(),
            "NIIMBOT RFID printers must set reports_media: {missing:?}"
        );
    }

    #[test]
    fn letratag_lt200b_is_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        let tape = catalog.lookup("lt-12-white-paper").unwrap();
        assert_eq!(tape.brand, "DYMO");
        assert_eq!(tape.media.width_mm, 12.0);
        let printer = catalog.lookup_device("LT-200B").unwrap();
        assert_eq!(printer.protocol, Protocol::LetraTag);
        assert_eq!(printer.maturity, Maturity::Experimental);
        assert!(printer.supports_cut);
        assert!(printer
            .supported_media
            .iter()
            .any(|k| k == "lt-12-white-paper"));
        assert!(printer.connections.iter().any(|c| matches!(
            c,
            ConnectionHint::Ble { name } if name.contains("Letratag") || name.contains("LT-200")
        )));
    }

    #[test]
    fn resolve_dpi_from_printer() {
        let catalog = Catalog::bundled().unwrap();
        assert_eq!(
            catalog.resolve_dpi(Some("D110"), Protocol::Niimbot, CLI_DEFAULT_DPI),
            203.0
        );
        // Niimbot catalog includes both 203 dpi and 300 dpi models (Pro/H series),
        // so protocol-only DPI lookup falls back to the CLI default.
        assert_eq!(
            catalog.resolve_dpi(None, Protocol::Niimbot, CLI_DEFAULT_DPI),
            CLI_DEFAULT_DPI
        );
        assert_eq!(
            catalog.resolve_dpi(Some("LabelWriter 550"), Protocol::DymoLw, CLI_DEFAULT_DPI),
            300.0
        );
        assert_eq!(catalog.resolve_dpi(None, Protocol::DymoLw, 600.0), 600.0);
    }

    #[test]
    fn dymo_lw5_supports_soft_reboot() {
        let catalog = Catalog::bundled().unwrap();
        for key in [
            "LabelWriter 550",
            "LabelWriter 550 Turbo",
            "LabelWriter 5XL",
        ] {
            let printer = catalog.lookup_device(key).unwrap();
            assert!(
                printer.supports_soft_reboot,
                "{key} should advertise soft reboot"
            );
        }
        let classic = catalog.lookup_device("LabelWriter 450").unwrap();
        assert!(!classic.supports_soft_reboot);
    }

    #[test]
    fn printer_support_urls_prefer_product_over_brand() {
        let catalog = Catalog::bundled().unwrap();
        let dymo = catalog.lookup_device("LabelWriter 550").unwrap();
        assert_eq!(
            dymo.support.brand_url.as_deref(),
            Some("https://www.dymo.com/support")
        );
        assert!(dymo.support.product_url.is_none());
        assert_eq!(
            dymo.support.primary_url(),
            Some("https://www.dymo.com/support")
        );

        let brother = catalog.lookup_device("QL-820NWBc").unwrap();
        assert!(brother
            .support
            .product_url
            .as_deref()
            .is_some_and(|u| u.contains("lpql820nwbeus")));
        assert_eq!(
            brother.support.primary_url(),
            brother.support.product_url.as_deref()
        );

        for printer in catalog.devices() {
            assert!(
                !printer.support.is_empty(),
                "{} missing support links",
                printer.canonical_key()
            );
        }
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
        let lm420p = catalog.match_usb(0x0922, 0x1004).unwrap();
        assert!(lm420p.matches_key("LabelManager 420P"));
        assert_eq!(lm420p.max_width_mm, 19.0);
        let lm_wireless = catalog.match_usb(0x0922, 0x1008).unwrap();
        assert!(lm_wireless.matches_key("LabelManager Wireless PnP"));
        let lm_pc = catalog.match_usb(0x0922, 0x0011).unwrap();
        assert!(lm_pc.matches_key("LabelManager PC"));
        let lm400 = catalog.match_usb(0x0922, 0x0013).unwrap();
        assert!(lm400.matches_key("LabelManager 400"));
        let lp350 = catalog.match_usb(0x0922, 0x0015).unwrap();
        assert!(lp350.matches_key("LabelPoint 350"));
        assert_eq!(lp350.max_width_mm, 19.0);
        let ql600 = catalog.match_usb(0x04f9, 0x20c0).unwrap();
        assert!(ql600.matches_key("QL-600"));
        assert_eq!(ql600.max_width_mm, 62.0);
        let ql1050 = catalog.match_usb(0x04f9, 0x2020).unwrap();
        assert!(ql1050.matches_key("QL-1050"));
        assert_eq!(ql1050.max_width_mm, 103.0);
        let ql1060n = catalog.match_usb(0x04f9, 0x202a).unwrap();
        assert!(ql1060n.matches_key("QL-1060N"));
        let zd421 = catalog.match_usb(0x0a5f, 0x0185).unwrap();
        assert!(zd421.matches_key("ZD421"));
        let zd420 = catalog.match_usb(0x0a5f, 0x0120).unwrap();
        assert!(zd420.matches_key("ZD420"));
        let zp450 = catalog.match_usb(0x0a5f, 0x008c).unwrap();
        assert!(zp450.matches_key("ZP450"));
        let tsc = catalog.match_usb(0x1203, 0x0160).unwrap();
        assert_eq!(tsc.protocol, Protocol::Tspl);
        assert!(tsc.matches_key("TE200"));
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
    fn niimbot_wide_heads_and_d110m_are_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        let b31 = catalog.lookup_device("B31").unwrap();
        assert_eq!(b31.max_width_mm, 75.0);
        assert_eq!(b31.dpi, 203.0);
        assert!(catalog.supports_media("B31", "70x40"));
        let b4 = catalog.lookup_device("B4").unwrap();
        assert_eq!(b4.max_width_mm, 104.0);
        assert!(catalog.supports_media("B4", "102x152"));
        let k3 = catalog.lookup_device("K3").unwrap();
        assert_eq!(k3.max_width_mm, 80.0);
        assert!(k3.matches_key("K3_W"));
        let k4 = catalog.lookup_device("K4").unwrap();
        assert_eq!(k4.max_width_mm, 104.0);
        let b1 = catalog.lookup_device("B1_SE").unwrap();
        assert!(b1.matches_key("B1"));
        let b21 = catalog.lookup_device("B21_C2B").unwrap();
        assert!(b21.matches_key("B21"));
        let d110m = catalog.lookup_device("D110_M").unwrap();
        assert_eq!(d110m.max_width_mm, 12.0);
        assert!(d110m.matches_key("D110M"));
        let d11 = catalog.lookup_device("D11").unwrap();
        assert_eq!(d11.name, "NIIMBOT D11");
        assert!(!d11.matches_key("D110"));
        let d11s = catalog.lookup_device("D11S").unwrap();
        assert_eq!(d11s.name, "NIIMBOT D11S");
        assert!(!d11s.matches_key("D110"));
        let d110 = catalog.lookup_device("D110").unwrap();
        assert_eq!(d110.name, "NIIMBOT D110");
        assert!(!d110.matches_key("D11"));
    }

    #[test]
    fn printer_maturity_is_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        // Hardware-exercised on hand: LW 550, LM 280, B1, D110.
        assert_eq!(
            catalog.lookup_device("LabelWriter 550").unwrap().maturity,
            Maturity::Verified
        );
        assert_eq!(
            catalog.lookup_device("LabelManager 280").unwrap().maturity,
            Maturity::Verified
        );
        assert_eq!(
            catalog.lookup_device("B1").unwrap().maturity,
            Maturity::Verified
        );
        assert_eq!(
            catalog.lookup_device("D110").unwrap().maturity,
            Maturity::Verified
        );
        // Same protocol as on-hand hardware → supported.
        assert_eq!(
            catalog
                .lookup_device("LabelWriter 550 Turbo")
                .unwrap()
                .maturity,
            Maturity::Supported
        );
        assert_eq!(
            catalog.lookup_device("B1 Pro").unwrap().maturity,
            Maturity::Supported
        );
        assert_eq!(
            catalog.lookup_device("LM500TS").unwrap().maturity,
            Maturity::Supported
        );
        // GPGL cut path ready; hardware checklist tracked separately.
        assert_eq!(
            catalog.lookup_device("cameo4").unwrap().maturity,
            Maturity::Supported
        );
        // Protocols never exercised on hand → experimental.
        assert_eq!(
            catalog.lookup_device("LabelWriter 450").unwrap().maturity,
            Maturity::Experimental
        );
        assert_eq!(
            catalog.lookup_device("QL-800").unwrap().maturity,
            Maturity::Experimental
        );
        assert_eq!(
            catalog.lookup_device("ZT231").unwrap().maturity,
            Maturity::Experimental
        );
        assert_eq!(
            catalog.lookup_device("M110").unwrap().maturity,
            Maturity::Experimental
        );
        assert_eq!(
            catalog.lookup_device("D30").unwrap().maturity,
            Maturity::Experimental
        );
        assert_eq!(
            catalog.lookup_device("Q30").unwrap().maturity,
            Maturity::Experimental
        );
        assert_eq!(
            catalog.lookup_device("X1038").unwrap().maturity,
            Maturity::Experimental
        );
        assert_eq!(
            catalog.lookup_device("ITPP941").unwrap().maturity,
            Maturity::Experimental
        );
        let verified: Vec<_> = catalog
            .devices()
            .iter()
            .filter(|p| p.maturity == Maturity::Verified)
            .map(|p| p.keys[0].as_str())
            .collect();
        assert_eq!(
            verified,
            vec!["LabelWriter 550", "LabelManager 280", "D110", "D11", "B1",]
        );
        assert!(catalog
            .devices()
            .iter()
            .filter(|p| p.maturity == Maturity::Supported)
            .all(|p| matches!(
                p.protocol,
                Protocol::DymoLw | Protocol::Dymo | Protocol::Niimbot | Protocol::Gpgl
            )));
        assert!(catalog.devices().iter().any(|p| {
            p.maturity == Maturity::Experimental
                && !matches!(
                    p.protocol,
                    Protocol::DymoLw | Protocol::Dymo | Protocol::Niimbot | Protocol::Gpgl
                )
        }));
    }

    #[test]
    fn silhouette_cameo4_cutter_is_catalogued() {
        use crate::DeviceRole;
        let catalog = Catalog::bundled().unwrap();
        let cameo = catalog.lookup_device("cameo4").unwrap();
        assert_eq!(cameo.role, DeviceRole::Cutter);
        assert_eq!(cameo.protocol, Protocol::Gpgl);
        assert_eq!(cameo.maturity, Maturity::Supported);
        assert!(cameo
            .connections
            .iter()
            .any(|c| c.is_exact_usb_match(0x0b4d, 0x1137)));
        assert!(catalog.lookup("silhouette-mat-12x12").is_some());
        let cutters = catalog
            .devices()
            .iter()
            .filter(|d| d.role == DeviceRole::Cutter)
            .count();
        assert!(
            cutters >= 19,
            "expected full Silhouette matrix, got {cutters}"
        );
    }

    #[test]
    fn tsc_da_series_and_d1_19mm_are_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        let da210 = catalog.lookup_device("DA210").unwrap();
        assert_eq!(da210.protocol, Protocol::Tspl);
        assert_eq!(da210.max_width_mm, 108.0);
        assert!(da210
            .connections
            .iter()
            .any(|c| c.is_exact_usb_match(0x1203, 0x0160)));
        assert!(!da210.connections.iter().any(|c| {
            matches!(
                c,
                ConnectionHint::Usb {
                    vendor_id: 0x1203,
                    product_id: None,
                }
            )
        }));
        let tape19 = catalog.lookup("45803").unwrap();
        assert_eq!(tape19.media.width_mm, 19.0);
        let tape19_yellow = catalog.lookup("45808").unwrap();
        assert_eq!(
            tape19_yellow.media.color,
            lbl_core::media::MediaColor::Yellow
        );
        let tape24 = catalog.lookup("53713").unwrap();
        assert_eq!(tape24.media.width_mm, 24.0);
        let lm420p = catalog.compatible_with("LabelManager 420P");
        assert!(lm420p.iter().any(|e| e.matches_key("45803")));
        assert!(lm420p.iter().any(|e| e.matches_key("45808")));
        let lm500ts = catalog.lookup_device("LM500TS").unwrap();
        assert_eq!(lm500ts.max_width_mm, 24.0);
        assert!(catalog.supports_media("LM500TS", "53713"));
        assert!(lm500ts.connections.is_empty());
        let b18 = catalog.lookup_device("B18").unwrap();
        assert_eq!(b18.protocol, Protocol::Niimbot);
        assert_eq!(b18.max_width_mm, 12.0);
        assert_eq!(b18.maturity, Maturity::Experimental);
        let n1 = catalog.lookup_device("N1").unwrap();
        assert_eq!(n1.max_width_mm, 12.0);
        let m2h = catalog.lookup_device("M2_H").unwrap();
        assert_eq!(m2h.dpi, 300.0);
        assert_eq!(m2h.max_width_mm, 48.0);
        let zt231 = catalog.lookup_device("ZT231").unwrap();
        assert_eq!(zt231.protocol, Protocol::Zpl);
        assert_eq!(zt231.max_width_mm, 104.0);
    }

    #[test]
    fn phomemo_m02x_and_branded_media_are_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        let m02x = catalog.lookup_device("M02X").unwrap();
        assert_eq!(m02x.protocol, Protocol::PhomemoM02x);
        assert_eq!(m02x.dpi, 203.0);
        assert_eq!(m02x.max_width_mm, 53.0);
        assert!(catalog.supports_media("M02X", "phomemo-53-cont"));
        assert!(catalog.supports_media("M02X", "phomemo-53-sticker"));
        let m02s = catalog.lookup_device("M02S").unwrap();
        assert_eq!(m02s.protocol, Protocol::Phomemo);
        assert_eq!(m02s.dpi, 300.0);
        assert!(catalog.supports_media("M02S", "phomemo-15-cont"));
        let t02 = catalog.lookup_device("T02").unwrap();
        assert_eq!(t02.protocol, Protocol::Phomemo);
        let roll = catalog.lookup("phomemo-53-cont").unwrap();
        assert_eq!(roll.media.width_mm, 53.0);
    }

    #[test]
    fn rollo_munbyn_and_phomemo_labelers_are_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        let rollo = catalog.lookup_device("X1038").unwrap();
        assert_eq!(rollo.protocol, Protocol::Tspl);
        assert_eq!(rollo.max_width_mm, 104.0);
        assert!(catalog.supports_media("X1038", "102x152"));
        let wireless = catalog.lookup_device("X1040").unwrap();
        assert_eq!(wireless.protocol, Protocol::Tspl);
        let munbyn = catalog.lookup_device("ITPP941").unwrap();
        assert_eq!(munbyn.protocol, Protocol::Tspl);
        assert!(munbyn
            .connections
            .iter()
            .any(|c| c.is_exact_usb_match(0x09c6, 0x0426)));
        let m110 = catalog.lookup_device("M110").unwrap();
        assert_eq!(m110.protocol, Protocol::PhomemoM110);
        assert_eq!(m110.max_width_mm, 50.0);
        assert!(catalog.supports_media("M110", "phomemo-50x30"));
        assert!(catalog.supports_media("M110", "phomemo-40x30"));
        let d30 = catalog.lookup_device("D30").unwrap();
        assert_eq!(d30.protocol, Protocol::PhomemoD30);
        assert_eq!(d30.max_width_mm, 15.0);
        assert!(catalog.supports_media("D30", "phomemo-12x40"));
        let q30 = catalog.lookup_device("Q30").unwrap();
        assert_eq!(q30.protocol, Protocol::PhomemoD30);
        assert!(catalog.supports_media("Q30", "phomemo-14x40"));
        let tape12 = catalog.lookup("phomemo-12x40").unwrap();
        assert_eq!(tape12.media.width_mm, 12.0);
    }

    #[test]
    fn epson_colorworks_esclabel_is_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        let c4000 = catalog.lookup_device("CW-C4000").unwrap();
        assert_eq!(c4000.protocol, Protocol::EscLabel);
        assert_eq!(c4000.dpi, 600.0);
        assert_eq!(c4000.max_width_mm, 108.0);
        assert!(c4000.supports_cut);
        assert!(c4000.supports_color);
        assert_eq!(c4000.maturity, Maturity::Experimental);
        // No VID-only Epson wildcard — would match any Seiko Epson USB device.
        assert!(c4000.connections.is_empty());
        assert!(catalog.match_usb(0x04b8, 0x0001).is_none());
        assert!(catalog.supports_media("CW-C4000", "epson-matte-4x6"));
        assert!(catalog.supports_media("CW-C4000", "epson-matte-4x3"));
        assert!(catalog.supports_media("CW-C4000", "epson-premium-matte-4x6"));
        assert!(catalog.supports_media("CW-C4000", "epson-cont-108"));
        let c6500 = catalog.lookup_device("CW-C6500A").unwrap();
        assert_eq!(c6500.max_width_mm, 215.9);
        assert!(c6500.connections.is_empty());
        assert!(catalog.supports_media("CW-C6500A", "epson-cont-215"));
        let matte = catalog.lookup("epson-matte-4x6").unwrap();
        assert_eq!(matte.media.width_mm, 102.0);
        assert_eq!(matte.brand, "Epson");
        assert!(matte.matches_product_id("C33S045714"));
        let pe43 = catalog.lookup("C33S045713").unwrap();
        assert_eq!(pe43.media.width_mm, 102.0);
        assert_eq!(pe43.media.length, lbl_core::media::MediaLength::Fixed(76.0));
    }

    #[test]
    fn bixolon_slcs_is_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        let dx420 = catalog.lookup_device("SLP-DX420").unwrap();
        assert_eq!(dx420.protocol, Protocol::Slcs);
        assert_eq!(dx420.dpi, 203.0);
        assert!(dx420.supports_cut);
        assert_eq!(dx420.maturity, Maturity::Experimental);
        assert!(catalog.supports_media("SLP-DX420", "102x152"));
        let t400 = catalog.lookup_device("SLP-T400").unwrap();
        assert_eq!(t400.protocol, Protocol::Slcs);
        assert!(t400.connections.iter().any(|c| matches!(
            c,
            ConnectionHint::Usb {
                vendor_id: 0x1504,
                product_id: Some(0x0008)
            }
        )));
    }

    #[test]
    fn godex_ezpl_is_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        let dt4x = catalog.lookup_device("DT4x").unwrap();
        assert_eq!(dt4x.protocol, Protocol::Ezpl);
        assert_eq!(dt4x.dpi, 203.0);
        assert_eq!(dt4x.max_width_mm, 108.0);
        assert!(dt4x.supports_cut);
        assert!(catalog.supports_media("DT4x", "102x152"));
        let g530 = catalog.lookup_device("G530").unwrap();
        assert_eq!(g530.protocol, Protocol::Ezpl);
        assert_eq!(g530.dpi, 300.0);
    }

    #[test]
    fn sato_sbpl_is_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        let cl4 = catalog.lookup_device("CL4NX").unwrap();
        assert_eq!(cl4.protocol, Protocol::Sbpl);
        assert_eq!(cl4.dpi, 203.0);
        assert!(cl4.supports_cut);
        assert_eq!(cl4.maturity, Maturity::Experimental);
        assert!(catalog.supports_media("CL4NX", "102x152"));
        assert!(cl4.connections.iter().any(|c| matches!(
            c,
            ConnectionHint::Usb {
                vendor_id: 0x0828,
                product_id: None
            }
        )));
    }

    #[test]
    fn honeywell_dpl_is_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        let pc42 = catalog.lookup_device("PC42d").unwrap();
        assert_eq!(pc42.protocol, Protocol::Dpl);
        assert_eq!(pc42.dpi, 203.0);
        assert!(pc42.supports_cut);
        assert!(catalog.supports_media("PC42d", "102x152"));
        let px65 = catalog.lookup_device("PX65").unwrap();
        assert_eq!(px65.protocol, Protocol::Dpl);
        assert_eq!(px65.dpi, 300.0);
        assert!(px65.connections.iter().any(|c| matches!(
            c,
            ConnectionHint::Usb {
                vendor_id: 0x0b0b,
                product_id: None
            }
        )));
    }

    #[test]
    fn citizen_zpl_and_dpl_are_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        let e321 = catalog.lookup_device("CL-E321").unwrap();
        assert_eq!(e321.protocol, Protocol::Zpl);
        assert_eq!(e321.maturity, Maturity::Experimental);
        assert!(e321.connections.is_empty());
        assert!(catalog.supports_media("CL-E321", "102x152"));

        let s521 = catalog.lookup_device("CL-S521").unwrap();
        assert_eq!(s521.protocol, Protocol::Dpl);
        assert!(s521.connections.iter().any(|c| matches!(
            c,
            ConnectionHint::Usb {
                vendor_id: 0x08bd,
                product_id: Some(0x0208)
            }
        )));
        let s631 = catalog.lookup_device("CL-S631").unwrap();
        assert_eq!(s631.protocol, Protocol::Dpl);
        assert_eq!(s631.dpi, 300.0);
        assert!(s631.connections.iter().any(|c| matches!(
            c,
            ConnectionHint::Usb {
                vendor_id: 0x1d90,
                product_id: Some(0x2037)
            }
        )));
        let s700 = catalog.lookup_device("CL-S700").unwrap();
        assert!(s700.connections.iter().any(|c| matches!(
            c,
            ConnectionHint::Usb {
                vendor_id: 0x2730,
                product_id: Some(0x0fff)
            }
        )));
        let s621 = catalog.lookup_device("CL-S621").unwrap();
        assert_eq!(s621.protocol, Protocol::Dpl);
        assert!(s621.connections.iter().any(|c| matches!(
            c,
            ConnectionHint::Usb {
                vendor_id: 0x2730,
                product_id: Some(0x0fff)
            }
        )));
        let e720 = catalog.lookup_device("CL-E720").unwrap();
        assert_eq!(e720.protocol, Protocol::Zpl);
        assert!(e720.connections.is_empty());
        let s700dt = catalog.lookup_device("CL-S700DT").unwrap();
        assert_eq!(s700dt.protocol, Protocol::Dpl);
    }

    #[test]
    fn toshiba_tpcl_is_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        let ev4 = catalog.lookup_device("B-EV4D").unwrap();
        assert_eq!(ev4.protocol, Protocol::Tpcl);
        assert_eq!(ev4.dpi, 203.0);
        assert!(ev4.supports_cut);
        assert_eq!(ev4.maturity, Maturity::Experimental);
        assert!(catalog.supports_media("B-EV4D", "102x152"));
        assert!(ev4.connections.is_empty());
        let sx5 = catalog.lookup_device("B-SX5").unwrap();
        assert_eq!(sx5.protocol, Protocol::Tpcl);
        assert_eq!(sx5.max_width_mm, 128.0);
        let bv = catalog.lookup_device("BV420D").unwrap();
        assert_eq!(bv.protocol, Protocol::Tpcl);
        assert!(bv.connections.is_empty());
        let sv4 = catalog.lookup_device("B-SV4").unwrap();
        assert!(sv4.connections.iter().any(|c| matches!(
            c,
            ConnectionHint::Usb {
                vendor_id: 0x08a6,
                product_id: Some(0x0051)
            }
        )));
        // Shared encode: desktop + industrial families on `tpcl`.
        for key in ["B-FV4D", "B-SV4", "B-SX4", "B-SA4TM", "B-EV4T"] {
            assert_eq!(
                catalog.lookup_device(key).unwrap().protocol,
                Protocol::Tpcl,
                "{key}"
            );
        }
    }

    #[test]
    fn catalog_depth_clones_and_usb_are_present() {
        let catalog = Catalog::bundled().unwrap();
        let d35 = catalog.lookup_device("D35").unwrap();
        assert_eq!(d35.protocol, Protocol::PhomemoD30);
        assert_eq!(d35.max_width_mm, 15.0);
        let q30s = catalog.lookup_device("Q30S").unwrap();
        assert_eq!(q30s.protocol, Protocol::PhomemoD30);
        let ttp = catalog.lookup_device("TTP-244 Pro").unwrap();
        assert_eq!(ttp.protocol, Protocol::Tspl);
        assert_eq!(ttp.max_width_mm, 108.0);
        assert!(ttp.connections.is_empty());
        let xp = catalog.lookup_device("XP-420B").unwrap();
        assert_eq!(xp.protocol, Protocol::Tspl);
        assert_eq!(xp.brand, "Xprinter");
        let e550 = catalog.lookup_device("PT-E550W").unwrap();
        assert_eq!(e550.protocol, Protocol::BrotherPt);
        assert!(e550.connections.iter().any(|c| matches!(
            c,
            ConnectionHint::Usb {
                vendor_id: 0x04f9,
                product_id: Some(0x2060)
            }
        )));
        // Documented unknowns — still empty (no guessed PIDs).
        assert!(catalog
            .lookup_device("ZT231")
            .unwrap()
            .connections
            .is_empty());
        assert!(catalog
            .lookup_device("LM500TS")
            .unwrap()
            .connections
            .is_empty());
        assert!(catalog
            .lookup_device("X1038")
            .unwrap()
            .connections
            .is_empty());
        assert!(catalog
            .lookup_device("CW-C4000")
            .unwrap()
            .connections
            .is_empty());
    }

    #[test]
    fn d110_defaults_to_bluetooth() {
        let catalog = Catalog::bundled().unwrap();
        let d110 = catalog.lookup_device("D110").unwrap();
        let transport = d110.default_transport();
        assert_eq!(transport.bluetooth.as_deref(), Some("D110"));
        assert!(transport.serial.is_none());
    }

    #[test]
    fn resolve_device_falls_back_to_match() {
        let catalog = Catalog::bundled().unwrap();
        assert!(catalog.lookup_device("LW550").is_some());
        let printer = catalog.resolve_device("LW550").unwrap();
        assert!(printer.matches_key("LabelWriter 550"));
    }

    #[test]
    fn resolve_device_prefers_specific_labelwriter_key() {
        let catalog = Catalog::bundled().unwrap();
        let printer = catalog.resolve_device("DYMO LabelWriter 550").unwrap();
        assert!(printer.matches_key("LabelWriter 550"));
        assert!(!printer.matches_key("LabelWriter"));
    }

    #[test]
    fn ambiguous_printer_query_fails() {
        let catalog = Catalog::bundled().unwrap();
        assert!(matches!(
            catalog.lookup_device_query("550"),
            DeviceLookup::Ambiguous(_)
        ));
        assert!(catalog.resolve_device("550").is_none());
        let err = catalog.require_device("550").unwrap_err();
        assert!(err.contains("ambiguous printer '550'"));
        assert!(err.contains("DYMO LabelWriter 550"));
        assert!(err.contains("DYMO LabelWriter 550 Turbo"));
        // "Turbo" alone also matches LabelWriter 450 Turbo.
        assert!(
            err.contains("--printer 550 Turbo") || err.contains("--printer 550 turbo"),
            "expected '550 Turbo' suggestion, got:\n{err}"
        );
    }

    #[test]
    fn suggest_unique_device_terms_for_550_family() {
        let catalog = Catalog::bundled().unwrap();
        let base = catalog.lookup_device("LabelWriter 550").unwrap();
        let turbo = catalog.lookup_device("LabelWriter 550 Turbo").unwrap();
        let base_term = catalog.suggest_unique_device_term(base);
        let turbo_term = catalog.suggest_unique_device_term(turbo);
        assert_eq!(catalog.matching_devices(&base_term).len(), 1);
        assert_eq!(catalog.matching_devices(&turbo_term).len(), 1);
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
        assert!(catalog
            .lookup_by_product_id("6972842748577")
            .unwrap()
            .matches_key("50x30"));
        assert!(catalog
            .lookup_by_product_id("T40X30-230")
            .unwrap()
            .matches_key("40x30"));
        // Numeric code observed on a physical D110 15x30 RFID tag (the tag does
        // not report the printed "T15X30-200" pack code).
        assert!(catalog
            .lookup_by_product_id("02282280")
            .unwrap()
            .matches_key("15x30"));
        // EAN from a physical D11 15x50 roll (retail NB107 / NIIMBOT.UK).
        assert!(catalog
            .lookup_by_product_id("6972842743596")
            .unwrap()
            .matches_key("15x50"));
        assert!(catalog.supports_media("D11", "15x50"));
        // Official white D-series EAN block (NIIMBOT.UK / getCloudTemplateByOneCode).
        assert!(catalog
            .lookup_by_product_id("6972842743558")
            .unwrap()
            .matches_key("12x22"));
        assert!(catalog
            .lookup_by_product_id("6972842743572")
            .unwrap()
            .matches_key("12x40"));
        assert!(catalog
            .lookup_by_product_id("6972842743589")
            .unwrap()
            .matches_key("15x30"));
        assert!(catalog.supports_media("D11", "15x26"));
        assert!(catalog.supports_media("D11", "14x40"));
        assert!(catalog.supports_media("D101", "25x50"));
    }

    #[test]
    fn remaining_media_gaps_are_catalogued() {
        let catalog = Catalog::bundled().unwrap();
        assert!(catalog.lookup("DK-11218").unwrap().name.contains("Round"));
        assert!(catalog.lookup("DK-11219").unwrap().name.contains("Round"));
        assert!(catalog
            .lookup("TZe-FX231")
            .unwrap()
            .name
            .contains("Flexible"));
        assert!(catalog.lookup("TZe-SE4").unwrap().name.contains("Security"));
        assert!(catalog.lookup("18051").unwrap().name.contains("Rhino"));
        assert!(catalog.lookup("30346").unwrap().name.contains("US"));
        assert_eq!(
            catalog.lookup("z-perform-1000d-4x6").unwrap().brand,
            "Zebra"
        );
        assert!(matches!(
            catalog.lookup("zebra-4x6-blackmark").unwrap().media.sense,
            Some(lbl_core::media::MediaSense::BlackMark { .. })
        ));
        assert!(catalog
            .lookup("50x30-pro")
            .unwrap()
            .matches_key("50x30-pro"));
        assert_eq!(catalog.lookup("phomemo-60x40").unwrap().brand, "Phomemo");
        let duo = catalog.lookup_device("LabelWriter Duo").unwrap();
        assert_eq!(duo.protocol, Protocol::DymoLwClassic);
        assert!(catalog.supports_media("LabelWriter Duo", "99014-duo"));
        let m220 = catalog.lookup_device("M220").unwrap();
        assert_eq!(m220.protocol, Protocol::PhomemoM110);
        assert!(catalog.supports_media("M220", "phomemo-80x60"));
        assert!(catalog.supports_media("QL-820NWB", "DK-11218"));
    }

    #[test]
    fn obsolete_printers_table_is_rejected() {
        let mut catalog = Catalog::default();
        let err = catalog
            .merge_toml(
                "[[printers]]\nbrand = \"X\"\nkeys = [\"x\"]\nname = \"X\"\nprotocol = \"virtual\"\ndpi = 300.0\nmax_width_mm = 10.0\n",
            )
            .unwrap_err();
        assert!(
            err.to_string().contains("obsolete [[printers]]"),
            "unexpected error: {err}"
        );
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

                [[devices]]
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
            catalog.lookup_device("LabelWriter 550").unwrap().name,
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

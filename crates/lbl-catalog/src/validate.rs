//! Semantic checks for catalog geometry.
//!
//! `MediaSpec::width_mm` is physical stock width. Dimension keys such as
//! `15x30` must therefore store 15 mm — never the printable head width. Printer
//! clamps (`max_width_mm`, `head_printable_height_mm`) narrow the inkable band
//! at encode/preview time.

use crate::{Catalog, CatalogEntry, DeviceEntry};
use lbl_core::media::MediaLength;

const DIM_TOLERANCE_MM: f64 = 0.05;

/// A catalog geometry rule violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogGeometryError {
    /// Human-readable description.
    pub message: String,
}

impl std::fmt::Display for CatalogGeometryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CatalogGeometryError {}

/// Validate physical/printable geometry invariants for a loaded catalog.
pub fn validate_catalog_geometry(catalog: &Catalog) -> Result<(), Vec<CatalogGeometryError>> {
    let mut errors = Vec::new();
    for entry in catalog.entries() {
        validate_entry_dimensions(entry, &mut errors);
    }
    for printer in catalog.devices() {
        validate_printer_printable_band(printer, &mut errors);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_entry_dimensions(entry: &CatalogEntry, errors: &mut Vec<CatalogGeometryError>) {
    let Some((width_mm, length_mm)) = dimension_hint_from_keys(&entry.keys) else {
        return;
    };
    // Inch marketing names (`4x6`, `rollo-4x6`) store millimeters (~101.6×152.4).
    if looks_like_inch_dimensions(width_mm, length_mm, entry) {
        return;
    }
    if (entry.media.width_mm - width_mm).abs() > DIM_TOLERANCE_MM {
        errors.push(CatalogGeometryError {
            message: format!(
                "media `{}`: key implies physical width {width_mm} mm but media.width_mm is {}. \
                 Store physical stock width; printable clamps belong on the printer \
                 (max_width_mm / head_printable_height_mm).",
                entry.canonical_key(),
                entry.media.width_mm
            ),
        });
    }
    if let MediaLength::Fixed(mm) = entry.media.length {
        if (mm - length_mm).abs() > DIM_TOLERANCE_MM {
            errors.push(CatalogGeometryError {
                message: format!(
                    "media `{}`: key implies fixed length {length_mm} mm but media.length is {mm} mm",
                    entry.canonical_key()
                ),
            });
        }
    }
}

fn looks_like_inch_dimensions(key_w: f64, key_l: f64, entry: &CatalogEntry) -> bool {
    const INCH_MM: f64 = 25.4;
    const TOL_MM: f64 = 3.0;
    let width_ok = (entry.media.width_mm - key_w * INCH_MM).abs() <= TOL_MM;
    match entry.media.length {
        MediaLength::Fixed(mm) => width_ok && (mm - key_l * INCH_MM).abs() <= TOL_MM,
        MediaLength::Continuous => width_ok,
    }
}

fn validate_printer_printable_band(printer: &DeviceEntry, errors: &mut Vec<CatalogGeometryError>) {
    if !(printer.capabilities.max_width_mm.is_finite() && printer.capabilities.max_width_mm > 0.0) {
        errors.push(CatalogGeometryError {
            message: format!(
                "printer `{}`: max_width_mm must be a positive finite value",
                printer.canonical_key()
            ),
        });
    }
    if printer.capabilities.supports_precut {
        let dx_ok = printer
            .capabilities
            .feed_trail_mm
            .is_some_and(|d| d.is_finite() && d > 0.0);
        if !dx_ok {
            errors.push(CatalogGeometryError {
                message: format!(
                    "printer `{}`: supports_precut requires feed_trail_mm > 0 (head-to-cutter gap)",
                    printer.canonical_key()
                ),
            });
        }
    }
    if printer.capabilities.supports_half_cut && !printer.capabilities.supports_cut {
        errors.push(CatalogGeometryError {
            message: format!(
                "printer `{}`: supports_half_cut requires supports_cut",
                printer.canonical_key()
            ),
        });
    }
    if let Some(band) = printer.capabilities.head_printable_height_mm {
        if !(band.is_finite() && band > 0.0) {
            errors.push(CatalogGeometryError {
                message: format!(
                    "printer `{}`: head_printable_height_mm must be a positive finite value",
                    printer.canonical_key()
                ),
            });
        } else if band > printer.capabilities.max_width_mm + DIM_TOLERANCE_MM {
            errors.push(CatalogGeometryError {
                message: format!(
                    "printer `{}`: head_printable_height_mm ({band}) exceeds max_width_mm ({})",
                    printer.canonical_key(),
                    printer.capabilities.max_width_mm
                ),
            });
        }
    }
}

/// Parse `WxL` from catalog keys such as `15x30`, `12x40-clear`, `niimbot-15x30`,
/// or `phomemo-50x30`.
fn dimension_hint_from_keys(keys: &[String]) -> Option<(f64, f64)> {
    for key in keys {
        if let Some(dims) = parse_dimension_key(key) {
            return Some(dims);
        }
    }
    None
}

fn parse_dimension_key(key: &str) -> Option<(f64, f64)> {
    let lower = key.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'x' {
                let width: f64 = lower[start..i].parse().ok()?;
                i += 1;
                let len_start = i;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                if i > len_start {
                    let length: f64 = lower[len_start..i].parse().ok()?;
                    // Reject accidental parses inside product codes without a
                    // clear WxL token boundary (digit/x must not be mid-word).
                    let before_ok = start == 0
                        || !bytes[start - 1].is_ascii_alphanumeric()
                        || bytes[start - 1] == b'-';
                    let after_ok =
                        i == bytes.len() || !bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-';
                    if before_ok && after_ok && width > 0.0 && length > 0.0 {
                        return Some((width, length));
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_dimension_keys() {
        assert_eq!(parse_dimension_key("15x30"), Some((15.0, 30.0)));
        assert_eq!(parse_dimension_key("12x40-clear"), Some((12.0, 40.0)));
        assert_eq!(parse_dimension_key("niimbot-15x30"), Some((15.0, 30.0)));
        assert_eq!(parse_dimension_key("50x30-pro"), Some((50.0, 30.0)));
        assert_eq!(parse_dimension_key("phomemo-50x30"), Some((50.0, 30.0)));
        assert_eq!(parse_dimension_key("TZe-251"), None);
        assert_eq!(parse_dimension_key("11352"), None);
    }
}

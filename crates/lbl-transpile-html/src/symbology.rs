//! Barcode symbology names and which JS renderer draws them.
//!
//! Authoring HTML uses JsBarcode-style names (`CODE128`, `EAN13`, …) plus
//! industrial/postal names (`PDF417`, `DATAMATRIX`, `AZTEC`, …). Classic 1D
//! codes stay on JsBarcode (stable goldens / small asset). Extended symbologies
//! render via bwip-js (`data-bcid` = BWIPP encoder id).

/// Which browser library draws this symbology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarcodeRenderer {
    /// JsBarcode (`format` option = authoring name, e.g. `CODE128`).
    JsBarcode,
    /// bwip-js (`bcid` = BWIPP id, e.g. `pdf417`).
    Bwip,
}

/// Resolved symbology for transpile / layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbologyInfo {
    /// Canonical authoring name written to `data-symbology`.
    pub name: String,
    pub renderer: BarcodeRenderer,
    /// BWIPP `bcid` when `renderer == Bwip`.
    pub bcid: Option<&'static str>,
    /// Matrix / stacked codes: no human-readable caption under the bars.
    pub is_2d: bool,
}

const JSBARCODE_1D: &[&str] = &[
    "CODE128",
    "CODE128A",
    "CODE128B",
    "CODE128C",
    "EAN13",
    "EAN8",
    "EAN5",
    "EAN2",
    "UPC",
    "UPCE",
    "CODE39",
    "ITF14",
    "ITF",
    "MSI",
    "MSI10",
    "MSI11",
    "MSI1010",
    "MSI1110",
    "pharmacode",
    "codabar",
];

/// Map authoring alias (uppercase, except `pharmacode` / `codabar`) → BWIPP bcid.
fn bwip_alias(name: &str) -> Option<(&'static str, bool)> {
    match name {
        "PDF417" => Some(("pdf417", true)),
        "PDF417COMPACT" => Some(("pdf417compact", true)),
        "MICROPDF417" => Some(("micropdf417", true)),
        "DATAMATRIX" | "DATA_MATRIX" | "DM" => Some(("datamatrix", true)),
        "DATAMATRIXRECTANGULAR" | "DATAMATRIX_RECT" => Some(("datamatrixrectangular", true)),
        "AZTEC" | "AZTECCODE" => Some(("azteccode", true)),
        "AZTECCOMPACT" => Some(("azteccodecompact", true)),
        "MAXICODE" | "MAXI" => Some(("maxicode", true)),
        "DATABAR" | "GS1DATABAR" | "DATABAROMNI" => Some(("databaromni", false)),
        "DATABARSTACKED" => Some(("databarstacked", false)),
        "DATABARLIMITED" => Some(("databarlimited", false)),
        "DATABAREXPANDED" => Some(("databarexpanded", false)),
        "POSTNET" => Some(("postnet", false)),
        "PLANET" => Some(("planet", false)),
        "GS1128" | "GS1-128" | "EAN128" | "UCC128" => Some(("gs1-128", false)),
        "ISBN" => Some(("isbn", false)),
        "CODE93" => Some(("code93", false)),
        "CODE11" => Some(("code11", false)),
        // UPCE via bwip when not using JsBarcode's limited UPCE support path.
        _ => None,
    }
}

fn normalize_name(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "CODE128".to_string();
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower == "pharmacode" || lower == "codabar" {
        return lower;
    }
    trimmed.to_ascii_uppercase()
}

/// Resolve an authoring `type` / `data-symbology` value.
pub fn resolve_symbology(raw: &str) -> SymbologyInfo {
    let name = normalize_name(raw);
    if JSBARCODE_1D.iter().any(|s| *s == name) {
        return SymbologyInfo {
            name,
            renderer: BarcodeRenderer::JsBarcode,
            bcid: None,
            is_2d: false,
        };
    }
    if let Some((bcid, is_2d)) = bwip_alias(&name) {
        return SymbologyInfo {
            name,
            renderer: BarcodeRenderer::Bwip,
            bcid: Some(bcid),
            is_2d,
        };
    }
    // Unknown names: try JsBarcode first (historical default).
    SymbologyInfo {
        name,
        renderer: BarcodeRenderer::JsBarcode,
        bcid: None,
        is_2d: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code128_stays_on_jsbarcode() {
        let s = resolve_symbology("code128");
        assert_eq!(s.name, "CODE128");
        assert_eq!(s.renderer, BarcodeRenderer::JsBarcode);
        assert!(!s.is_2d);
    }

    #[test]
    fn pharmacode_keeps_lowercase() {
        let s = resolve_symbology("Pharmacode");
        assert_eq!(s.name, "pharmacode");
        assert_eq!(s.renderer, BarcodeRenderer::JsBarcode);
    }

    #[test]
    fn datamatrix_and_aliases() {
        for alias in ["DATAMATRIX", "DataMatrix", "DM"] {
            let s = resolve_symbology(alias);
            assert_eq!(s.renderer, BarcodeRenderer::Bwip);
            assert_eq!(s.bcid, Some("datamatrix"));
            assert!(s.is_2d);
        }
    }

    #[test]
    fn gs1_128_aliases() {
        for alias in ["GS1128", "EAN128", "GS1-128"] {
            let s = resolve_symbology(alias);
            assert_eq!(s.bcid, Some("gs1-128"));
            assert!(!s.is_2d);
        }
    }
}

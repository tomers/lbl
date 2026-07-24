//! Optional web-font face resolution for preview/print.
//!
//! The engine accepts opaque `data-lbl-font` slugs. Callers that can supply
//! face bytes or URLs implement [`FontProvider`]; the default is a no-op so
//! pure `lbl-server` needs no font catalog or network.

use anyhow::Result;
use lbl_transpile_html::FontDelivery;

/// Resolve web-font faces for authoring HTML that contains `data-lbl-font`.
pub trait FontProvider: Send + Sync {
    /// Build delivery rules for the combined HTML of one or more labels.
    fn delivery_for_html(&self, html: &str) -> Result<FontDelivery>;
}

/// No web faces — system stacks only.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopFontProvider;

impl FontProvider for NoopFontProvider {
    fn delivery_for_html(&self, _html: &str) -> Result<FontDelivery> {
        Ok(FontDelivery::None)
    }
}

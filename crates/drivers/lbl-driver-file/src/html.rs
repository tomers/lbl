//! Browser-gallery preview sink ([`Protocol::Html`](lbl_driver_api::Protocol::Html)).
//!
//! Encoding yields an empty byte stream; the pipeline writes PNG pages and an
//! `index.html` outside the driver. Registered so protocol id resolution stays
//! driver-owned like every other protocol.

use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

/// HTML preview driver (`Protocol::Html`).
#[derive(Debug, Default, Clone, Copy)]
pub struct HtmlDriver;

impl HtmlDriver {
    /// Create a new HTML preview driver.
    pub fn new() -> Self {
        Self
    }
}

impl Driver for HtmlDriver {
    fn protocol(&self) -> Protocol {
        Protocol::Html
    }

    fn name(&self) -> &'static str {
        "html-preview"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["html"]
    }

    fn encode(&self, bitmap: &MonoBitmap, _ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.width == 0 || bitmap.height == 0 {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }
        Ok(Vec::new())
    }
}

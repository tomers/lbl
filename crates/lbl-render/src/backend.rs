//! The render backend abstraction and the Node/Playwright sidecar backend.

use std::io::Write;
use std::process::{Command, Stdio};

use image::RgbaImage;

use crate::{RenderError, Result};

/// A backend that rasterizes HTML into an `RgbaImage` at the requested pixel
/// dimensions. Implementations include in-process Chromium and an external
/// sidecar process.
pub trait RenderBackend {
    /// Rasterize `html` at the given pixel `width` and `height`. A `None` axis
    /// lets the content determine that dimension (continuous media). At most
    /// one axis is normally `None`.
    fn rasterize(
        &self,
        html: &str,
        width: Option<u32>,
        height: Option<u32>,
    ) -> Result<RgbaImage>;
}

/// Drives an external renderer process (e.g. a Node + Playwright script) behind
/// the [`RenderBackend`] trait.
///
/// The process receives the HTML on stdin and the target size via the
/// `LBL_RENDER_WIDTH` / `LBL_RENDER_HEIGHT` environment variables, and must
/// write a PNG to stdout.
#[derive(Debug, Clone)]
pub struct SidecarBackend {
    program: String,
    args: Vec<String>,
}

impl SidecarBackend {
    /// Create a sidecar backend that runs `program` with `args`.
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }

    /// The conventional default: `node sidecar/render.js`.
    pub fn node_default() -> Self {
        Self::new("node", vec!["sidecar/render.js".to_string()])
    }
}

impl RenderBackend for SidecarBackend {
    fn rasterize(
        &self,
        html: &str,
        width: Option<u32>,
        height: Option<u32>,
    ) -> Result<RgbaImage> {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Each axis is passed only when fixed; an omitted axis tells the sidecar
        // to let the content determine that dimension.
        if let Some(w) = width {
            cmd.env("LBL_RENDER_WIDTH", w.to_string());
        }
        if let Some(h) = height {
            cmd.env("LBL_RENDER_HEIGHT", h.to_string());
        }

        let mut child = cmd.spawn().map_err(|e| {
            RenderError::Backend(format!("spawning sidecar '{}': {e}", self.program))
        })?;

        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(html.as_bytes())?;

        let output = child
            .wait_with_output()
            .map_err(|e| RenderError::Backend(format!("sidecar wait: {e}")))?;
        if !output.status.success() {
            return Err(RenderError::Backend(format!(
                "sidecar exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let img = image::load_from_memory(&output.stdout)?;
        Ok(img.to_rgba8())
    }
}

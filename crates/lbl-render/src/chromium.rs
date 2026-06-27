//! In-process headless Chromium backend via chromiumoxide.

use std::time::Duration;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;
use chromiumoxide::page::ScreenshotParams;
use futures::StreamExt;
use image::RgbaImage;
use tokio::runtime::Runtime;

use crate::backend::RenderBackend;
use crate::{RenderError, Result};

/// A render backend that drives a headless Chromium instance in-process.
///
/// A single browser is launched for the lifetime of the backend and reused for
/// every label, which makes batch rendering fast.
pub struct ChromiumBackend {
    rt: Runtime,
    browser: Browser,
    _handler: tokio::task::JoinHandle<()>,
}

impl ChromiumBackend {
    /// Launch a headless Chromium instance.
    pub fn launch() -> Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| RenderError::Backend(format!("tokio runtime: {e}")))?;

        let (browser, handler) = rt.block_on(async {
            let config = BrowserConfig::builder()
                .arg("--no-sandbox")
                .arg("--disable-gpu")
                .arg("--hide-scrollbars")
                .build()
                .map_err(RenderError::Backend)?;
            Browser::launch(config)
                .await
                .map_err(|e| RenderError::Backend(e.to_string()))
        })?;

        let handler_task = {
            let mut handler = handler;
            rt.spawn(async move { while handler.next().await.is_some() {} })
        };

        Ok(Self {
            rt,
            browser,
            _handler: handler_task,
        })
    }
}

impl RenderBackend for ChromiumBackend {
    fn rasterize(&self, html: &str, width: u32, height: Option<u32>) -> Result<RgbaImage> {
        let bytes = self.rt.block_on(async {
            let data_url = format!(
                "data:text/html;charset=utf-8,{}",
                urlencoding::encode(html)
            );
            let page = self
                .browser
                .new_page(data_url.as_str())
                .await
                .map_err(|e| RenderError::Backend(e.to_string()))?;

            // Pin the viewport to the requested device pixels (supersampling is
            // already folded into `width`/`height` by the caller).
            let metrics = SetDeviceMetricsOverrideParams::builder()
                .width(width as i64)
                .height(height.unwrap_or(1).max(1) as i64)
                .device_scale_factor(1.0)
                .mobile(false)
                .build()
                .map_err(RenderError::Backend)?;
            page.execute(metrics)
                .await
                .map_err(|e| RenderError::Backend(e.to_string()))?;

            page.wait_for_navigation()
                .await
                .map_err(|e| RenderError::Backend(e.to_string()))?;

            // Allow webfonts and the QR/barcode scripts to settle.
            let _ = page.evaluate("document.fonts && document.fonts.ready").await;
            tokio::time::sleep(Duration::from_millis(200)).await;

            let params = ScreenshotParams::builder()
                .full_page(height.is_none())
                .omit_background(false)
                .build();
            let bytes = page
                .screenshot(params)
                .await
                .map_err(|e| RenderError::Backend(e.to_string()))?;
            let _ = page.close().await;
            Ok::<Vec<u8>, RenderError>(bytes)
        })?;

        Ok(image::load_from_memory(&bytes)?.to_rgba8())
    }
}

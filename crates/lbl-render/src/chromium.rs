//! In-process headless Chromium backend via chromiumoxide.

use std::time::Duration;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide::Page;
use futures::StreamExt;
use image::RgbaImage;
use tempfile::{Builder, TempDir};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

use crate::backend::RenderBackend;
use crate::{RenderError, Result};

/// Upper bound on how long a single page is allowed to load and render before
/// the rasterize call gives up. Renders are self-contained `data:` URLs, so
/// this only ever trips on a genuinely stuck page.
const NAV_TIMEOUT: Duration = Duration::from_secs(20);

/// Poll `document.readyState` until the page reports `complete`.
async fn wait_for_load(page: &Page) -> Result<()> {
    loop {
        let ready = page
            .evaluate("document.readyState")
            .await
            .ok()
            .and_then(|v| v.into_value::<String>().ok());
        if ready.as_deref() == Some("complete") {
            return Ok(());
        }
        sleep(Duration::from_millis(25)).await;
    }
}

/// A render backend that drives a headless Chromium instance in-process.
///
/// A single browser is launched for the lifetime of the backend and reused for
/// every label, which makes batch rendering fast.
pub struct ChromiumBackend {
    rt: Runtime,
    browser: Browser,
    _handler: JoinHandle<()>,
    // Kept alive for the lifetime of the backend so the unique Chromium profile
    // directory is removed on drop.
    _profile_dir: TempDir,
}

impl ChromiumBackend {
    /// Launch a headless Chromium instance.
    pub fn launch() -> Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| RenderError::Backend(format!("tokio runtime: {e}")))?;

        // Use a unique, per-process profile directory. Without this,
        // chromiumoxide falls back to a fixed `<tmp>/chromiumoxide-runner`
        // path, whose stale `SingletonLock` (left by a previously crashed or
        // killed Chromium) makes every subsequent launch abort.
        let profile_dir = Builder::new()
            .prefix("lbl-chromium-")
            .tempdir()
            .map_err(RenderError::Io)?;

        let (browser, handler) = rt.block_on(async {
            let config = BrowserConfig::builder()
                .arg("--no-sandbox")
                .arg("--disable-gpu")
                .arg("--hide-scrollbars")
                .user_data_dir(profile_dir.path())
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
            _profile_dir: profile_dir,
        })
    }
}

impl RenderBackend for ChromiumBackend {
    fn rasterize(&self, html: &str, width: u32, height: Option<u32>) -> Result<RgbaImage> {
        let bytes = self.rt.block_on(async {
            // Guard the whole interaction so a misbehaving page can never wedge
            // the render indefinitely.
            timeout(NAV_TIMEOUT, async {
                let data_url =
                    format!("data:text/html;charset=utf-8,{}", urlencoding::encode(html));
                let page = self
                    .browser
                    .new_page(data_url.as_str())
                    .await
                    .map_err(|e| RenderError::Backend(e.to_string()))?;

                // Pin the viewport to the requested device pixels (supersampling
                // is already folded into `width`/`height` by the caller).
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

                // `new_page` already navigates to the data URL, so we must NOT
                // call `wait_for_navigation` (it would block forever waiting for
                // a *subsequent* navigation that never happens for a static
                // data: URL). Instead poll until the document has fully loaded.
                wait_for_load(&page).await?;

                // Allow webfonts and the QR/barcode scripts to settle.
                let _ = page.evaluate("document.fonts && document.fonts.ready").await;
                sleep(Duration::from_millis(200)).await;

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
            })
            .await
            .map_err(|_| {
                RenderError::Backend(format!(
                    "page did not finish rendering within {}s",
                    NAV_TIMEOUT.as_secs()
                ))
            })?
        })?;

        Ok(image::load_from_memory(&bytes)?.to_rgba8())
    }
}

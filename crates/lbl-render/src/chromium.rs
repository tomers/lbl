//! In-process headless Chromium backend via chromiumoxide.

use std::fs;
use std::thread;
use std::time::Duration;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;
use chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams;
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide::Page;
use futures::StreamExt;
use image::RgbaImage;
use lbl_core::CSS_LAYOUT_REFERENCE_DPI;
use tempfile::{Builder, TempDir};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

use crate::backend::{PdfExportRequest, RenderBackend};
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
    // `None` only transiently during drop, after the live resources have been
    // handed to the shutdown thread. All access goes through [`Self::inner`].
    inner: Option<BackendInner>,
}

/// The live resources backing a [`ChromiumBackend`]. Bundled so [`Drop`] can
/// move them, as a unit, onto a thread that has no ambient runtime.
struct BackendInner {
    rt: Runtime,
    browser: Browser,
    _handler: JoinHandle<()>,
    // Kept alive for the lifetime of the backend so the unique Chromium profile
    // directory is removed on drop.
    _profile_dir: TempDir,
}

impl ChromiumBackend {
    fn inner(&self) -> &BackendInner {
        self.inner
            .as_ref()
            .expect("ChromiumBackend used after drop")
    }

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

        // Recent Chromium requires a writable Crashpad database. Container
        // users without a home dir (e.g. ECS `app` with `--no-create-home`)
        // otherwise die on launch with `chrome_crashpad_handler: --database
        // is required`. Pin XDG/HOME and crash dumps into the profile dir.
        let xdg_home = profile_dir.path().join("xdg");
        fs::create_dir_all(&xdg_home)?;
        let xdg_home = xdg_home
            .to_str()
            .ok_or_else(|| RenderError::Backend("chromium profile path is not UTF-8".into()))?
            .to_owned();

        let (browser, handler) = rt.block_on(async {
            // chromiumoxide `.arg()` expects bare flag names (e.g. `disable-gpu`),
            // not `--`-prefixed strings; use `.no_sandbox()` for Docker/root.
            let mut builder = BrowserConfig::builder()
                .no_sandbox()
                .arg("disable-gpu")
                // Small Fargate /dev/shm makes Chromium crash without this.
                .arg("disable-dev-shm-usage")
                .arg(("crash-dumps-dir", xdg_home.as_str()))
                .env("HOME", &xdg_home)
                .env("XDG_CONFIG_HOME", &xdg_home)
                .env("XDG_CACHE_HOME", &xdg_home);
            if let Ok(chrome) = std::env::var("CHROME") {
                builder = builder.chrome_executable(chrome);
            }
            let config = builder
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
            inner: Some(BackendInner {
                rt,
                browser,
                _handler: handler_task,
                _profile_dir: profile_dir,
            }),
        })
    }

    /// Report whether the underlying browser still answers CDP requests.
    ///
    /// A reused backend can outlive its Chromium process (a crash, an OOM kill,
    /// or a lost websocket). Callers use this to tell a genuine render error
    /// (bad input, timeout under load) apart from a dead browser that must be
    /// relaunched, so a slow render never triggers a needless relaunch.
    pub fn healthy(&self) -> bool {
        let inner = self.inner();
        inner.rt.block_on(async {
            matches!(
                timeout(Duration::from_secs(2), inner.browser.version()).await,
                Ok(Ok(_))
            )
        })
    }
}

impl Drop for ChromiumBackend {
    fn drop(&mut self) {
        // The backend is meant to be long-lived and reused, so it may be dropped
        // from within an async runtime (owning state torn down on an executor
        // thread, or at server shutdown). Both calling `Runtime::block_on` and
        // dropping a `Runtime` panic when a runtime is entered on the current
        // thread, so the whole teardown — graceful browser close, then dropping
        // the runtime, handler task and profile dir — is handed to a dedicated
        // thread that has no ambient runtime. Closing explicitly also avoids
        // chromiumoxide's "Browser was not closed manually" warning and reaps the
        // child process instead of leaking it.
        let Some(inner) = self.inner.take() else {
            return;
        };
        thread::spawn(move || {
            let BackendInner {
                rt,
                mut browser,
                _handler,
                _profile_dir,
            } = inner;
            rt.block_on(async {
                let _ = browser.close().await;
                let _ = browser.wait().await;
            });
        });
    }
}

impl RenderBackend for ChromiumBackend {
    fn rasterize(&self, html: &str, width: Option<u32>, height: Option<u32>) -> Result<RgbaImage> {
        // A `None` axis is content-determined: pin the other axis and let a
        // full-page screenshot capture the laid-out extent of the free one.
        let auto = width.is_none() || height.is_none();
        let inner = self.inner();
        let bytes = inner.rt.block_on(async {
            // Guard the whole interaction so a misbehaving page can never wedge
            // the render indefinitely.
            timeout(NAV_TIMEOUT, async {
                let data_url =
                    format!("data:text/html;charset=utf-8,{}", urlencoding::encode(html));
                let page = inner
                    .browser
                    .new_page(data_url.as_str())
                    .await
                    .map_err(|e| RenderError::Backend(e.to_string()))?;

                // Pin the viewport to the requested device pixels (supersampling
                // is already folded into `width`/`height` by the caller). A
                // content-determined axis gets a minimal placeholder; the
                // full-page screenshot below captures its real extent.
                let metrics = SetDeviceMetricsOverrideParams::builder()
                    .width(width.unwrap_or(1).max(1) as i64)
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
                let _ = page
                    .evaluate("document.fonts && document.fonts.ready")
                    .await;
                sleep(Duration::from_millis(200)).await;

                let params = ScreenshotParams::builder()
                    .full_page(auto)
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

        let mut img = image::load_from_memory(&bytes)?.to_rgba8();
        // Full-page capture follows content bounds; expand to the pinned viewport
        // axes so fixed-width media shows the full tape even when the ink is narrow.
        if auto {
            img = pad_to_min_size(img, width.unwrap_or(1), height.unwrap_or(1));
        }
        Ok(img)
    }

    fn export_pdf(&self, html: &str, req: &PdfExportRequest) -> Result<Vec<u8>> {
        let inner = self.inner();
        inner.rt.block_on(async {
            timeout(NAV_TIMEOUT, async {
                let data_url =
                    format!("data:text/html;charset=utf-8,{}", urlencoding::encode(html));
                let page = inner
                    .browser
                    .new_page(data_url.as_str())
                    .await
                    .map_err(|e| RenderError::Backend(e.to_string()))?;

                let width_px = css_px_for_mm(req.width_mm);
                let height_px = req.height_mm.map(css_px_for_mm).unwrap_or(800).max(1);
                let metrics = SetDeviceMetricsOverrideParams::builder()
                    .width(width_px.max(1) as i64)
                    .height(height_px.max(1) as i64)
                    .device_scale_factor(1.0)
                    .mobile(false)
                    .build()
                    .map_err(RenderError::Backend)?;
                page.execute(metrics)
                    .await
                    .map_err(|e| RenderError::Backend(e.to_string()))?;

                wait_for_load(&page).await?;
                let _ = page
                    .evaluate("document.fonts && document.fonts.ready")
                    .await;
                sleep(Duration::from_millis(200)).await;

                let params = PrintToPdfParams::builder()
                    .print_background(true)
                    .prefer_css_page_size(true)
                    .margin_top(0.0)
                    .margin_bottom(0.0)
                    .margin_left(0.0)
                    .margin_right(0.0)
                    .build();
                let pdf = page
                    .pdf(params)
                    .await
                    .map_err(|e| RenderError::Backend(e.to_string()))?;
                let _ = page.close().await;
                Ok::<Vec<u8>, RenderError>(pdf)
            })
            .await
            .map_err(|_| {
                RenderError::Backend(format!(
                    "page did not finish PDF export within {}s",
                    NAV_TIMEOUT.as_secs()
                ))
            })?
        })
    }
}

/// CSS pixels at [`CSS_LAYOUT_REFERENCE_DPI`] for vector PDF viewport sizing.
fn css_px_for_mm(mm: f64) -> u32 {
    (mm * CSS_LAYOUT_REFERENCE_DPI / 25.4).round().max(1.0) as u32
}

/// Pad `img` to at least `min_w` x `min_h` with white, keeping existing pixels
/// in the top-left corner.
fn pad_to_min_size(img: RgbaImage, min_w: u32, min_h: u32) -> RgbaImage {
    let w = img.width().max(min_w.max(1));
    let h = img.height().max(min_h.max(1));
    if w == img.width() && h == img.height() {
        return img;
    }
    let mut out = RgbaImage::from_pixel(w, h, image::Rgba([255, 255, 255, 255]));
    image::imageops::overlay(&mut out, &img, 0, 0);
    out
}

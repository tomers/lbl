//! Shared application state.

use std::env;
use std::sync::Arc;

use lbl_catalog::Catalog;
use lbl_config::{Loader, ProfileStore};
use lbl_text::font_assets_base_url_from_env;

use crate::font_cache::FontFileCache;
use crate::render_pool::RenderPool;

/// Application state shared across handlers (cheaply cloneable).
#[derive(Clone)]
pub struct AppState {
    /// The media catalog.
    pub catalog: Arc<Catalog>,
    /// Persisted printer profiles.
    pub profiles: Arc<ProfileStore>,
    /// Configuration loader (paths + figment).
    pub loader: Arc<Loader>,
    /// When false, local device enumeration is skipped and host profile routes
    /// (`/api/devices/profiles*`) are not mounted.
    pub host_discovery_enabled: bool,
    /// Shared headless-Chromium renderer reused across all preview/print
    /// requests, with a bound on how many renders run concurrently.
    pub renderer: Arc<RenderPool>,
    /// Base URL for label font binaries (prod CDN or local fixture dir).
    pub font_assets_base_url: String,
    /// Disk cache for fetched font faces (raster inlining).
    pub font_cache: Arc<FontFileCache>,
}

impl AppState {
    /// Build state using discovered config paths and the bundled catalog.
    pub fn discover() -> anyhow::Result<Self> {
        let loader = Loader::new();
        let profiles = ProfileStore::new(loader.paths().profiles.clone());
        let catalog = Catalog::load_with_overlays(&loader.load()?.catalog.extra_paths)
            .or_else(|_| Catalog::bundled())?;
        Ok(Self {
            catalog: Arc::new(catalog),
            profiles: Arc::new(profiles),
            loader: Arc::new(loader),
            host_discovery_enabled: host_discovery_enabled_from_env(),
            renderer: Arc::new(RenderPool::new(render_concurrency_from_env())),
            font_assets_base_url: font_assets_base_url_from_env(),
            font_cache: Arc::new(FontFileCache::from_env_or_default()),
        })
    }

    /// Launch the shared browser ahead of the first request so an interactive
    /// preview does not pay Chromium's cold-start cost. Failures are logged and
    /// left for the first render to retry; a missing browser must not stop the
    /// server from starting (e.g. deployments that only use the sidecar).
    pub fn warm_renderer(&self) {
        let renderer = self.renderer.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = renderer.warm() {
                tracing::warn!(error = ?e, "browser warm-up failed; first render will retry");
            }
        });
    }
}

/// Maximum concurrent renders, from `LBL_RENDER_CONCURRENCY` (default 2).
///
/// The right value tracks the CPU available to the process; deployments set it
/// to match their allotted cores.
fn render_concurrency_from_env() -> usize {
    env::var("LBL_RENDER_CONCURRENCY")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(2)
}

/// Read `LBL_HOST_DISCOVERY` (default: enabled).
///
/// Set to `0`, `false`, `off`, `no`, or `disabled` to skip USB / serial / BLE enumeration.
fn host_discovery_enabled_from_env() -> bool {
    match env::var("LBL_HOST_DISCOVERY") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "disabled" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    // Process environment is shared across the threads the test harness runs in
    // parallel, so tests that mutate it must not overlap. Poison-tolerant so one
    // failing test does not cascade into spurious failures in the others.
    static ENV_GUARD: Mutex<()> = Mutex::new(());

    #[test]
    fn host_discovery_defaults_to_enabled() {
        let _env = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        env::remove_var("LBL_HOST_DISCOVERY");
        assert!(host_discovery_enabled_from_env());
    }

    #[test]
    fn host_discovery_respects_disable_values() {
        let _env = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        for v in ["0", "disabled", "false", "no", "off"] {
            env::set_var("LBL_HOST_DISCOVERY", v);
            assert!(
                !host_discovery_enabled_from_env(),
                "expected disabled for LBL_HOST_DISCOVERY={v}"
            );
        }
        env::remove_var("LBL_HOST_DISCOVERY");
    }

    #[test]
    fn render_concurrency_defaults_and_overrides() {
        let _env = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        env::remove_var("LBL_RENDER_CONCURRENCY");
        assert_eq!(render_concurrency_from_env(), 2);

        env::set_var("LBL_RENDER_CONCURRENCY", "4");
        assert_eq!(render_concurrency_from_env(), 4);

        // Non-positive / unparseable values fall back to the default.
        for v in ["0", "-1", "abc", ""] {
            env::set_var("LBL_RENDER_CONCURRENCY", v);
            assert_eq!(
                render_concurrency_from_env(),
                2,
                "LBL_RENDER_CONCURRENCY={v}"
            );
        }
        env::remove_var("LBL_RENDER_CONCURRENCY");
    }
}

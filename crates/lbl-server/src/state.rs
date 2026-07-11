//! Shared application state.

use std::sync::{Arc, Mutex};

use lbl_catalog::Catalog;
use lbl_config::{Loader, ProfileStore};

/// Application state shared across handlers (cheaply cloneable).
#[derive(Clone)]
pub struct AppState {
    /// The media catalog.
    pub catalog: Arc<Catalog>,
    /// Persisted printer profiles.
    pub profiles: Arc<ProfileStore>,
    /// Configuration loader (paths + figment).
    pub loader: Arc<Loader>,
    /// When false, local device enumeration is skipped (`GET /api/printers`, etc.).
    pub host_discovery_enabled: bool,
    /// Serializes in-process Chromium launches.
    ///
    /// Each `ChromiumBackend::launch` starts a browser plus a nested Tokio
    /// runtime. Overlapping launches (common when Studio fires several
    /// `/api/preview` requests on refresh) reset the upstream connection and
    /// surface as gateway `500 Internal Server Error`.
    pub chromium_lock: Arc<Mutex<()>>,
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
            chromium_lock: Arc::new(Mutex::new(())),
        })
    }
}

/// Read `LBL_HOST_DISCOVERY` (default: enabled).
///
/// Set to `0`, `false`, `off`, `no`, or `disabled` to skip USB / serial / BLE enumeration.
fn host_discovery_enabled_from_env() -> bool {
    match std::env::var("LBL_HOST_DISCOVERY") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "disabled" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_discovery_defaults_to_enabled() {
        std::env::remove_var("LBL_HOST_DISCOVERY");
        assert!(host_discovery_enabled_from_env());
    }

    #[test]
    fn host_discovery_respects_disable_values() {
        for v in ["0", "disabled", "false", "no", "off"] {
            std::env::set_var("LBL_HOST_DISCOVERY", v);
            assert!(
                !host_discovery_enabled_from_env(),
                "expected disabled for LBL_HOST_DISCOVERY={v}"
            );
        }
        std::env::remove_var("LBL_HOST_DISCOVERY");
    }
}

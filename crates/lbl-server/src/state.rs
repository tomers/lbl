//! Shared application state.

use std::sync::Arc;

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
        })
    }
}

//! Persistence of user-owned printer profiles.
//!
//! Profiles are stored separately from the main config (in `printers.toml`)
//! so that a disconnected printer retains its desired configuration and can be
//! restored on reconnect.

use std::path::{Path, PathBuf};

use lbl_core::printer::{DeviceId, DeviceProfile};
use serde::{Deserialize, Serialize};

use crate::{ConfigError, Result};

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    printers: Vec<DeviceProfile>,
}

/// A read/write store of persisted [`DeviceProfile`]s backed by a TOML file.
#[derive(Debug, Clone)]
pub struct ProfileStore {
    path: PathBuf,
}

impl ProfileStore {
    /// Open (lazily) the store at the given path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The backing file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load all persisted profiles. Returns an empty list if the file does not
    /// exist yet.
    pub fn load(&self) -> Result<Vec<DeviceProfile>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&self.path).map_err(|source| ConfigError::Io {
            path: self.path.display().to_string(),
            source,
        })?;
        let parsed: ProfilesFile =
            toml::from_str(&text).map_err(|e| ConfigError::Load(e.to_string()))?;
        Ok(parsed.printers)
    }

    /// Persist the full list of profiles, creating parent directories.
    pub fn save_all(&self, profiles: &[DeviceProfile]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
        let file = ProfilesFile {
            printers: profiles.to_vec(),
        };
        let text = toml::to_string_pretty(&file)?;
        std::fs::write(&self.path, text).map_err(|source| ConfigError::Io {
            path: self.path.display().to_string(),
            source,
        })
    }

    /// Insert or update a profile (matched by id), then persist.
    pub fn upsert(&self, profile: DeviceProfile) -> Result<()> {
        let mut profiles = self.load()?;
        match profiles.iter_mut().find(|p| p.id == profile.id) {
            Some(existing) => *existing = profile,
            None => profiles.push(profile),
        }
        self.save_all(&profiles)
    }

    /// Remove a profile by id (no-op if absent), then persist.
    pub fn remove(&self, id: &DeviceId) -> Result<()> {
        let mut profiles = self.load()?;
        profiles.retain(|p| &p.id != id);
        self.save_all(&profiles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::printer::{DeviceCapabilities, DeviceModel, Protocol, Transport};
    use lbl_core::units::Dpi;

    fn sample(id: &str) -> DeviceProfile {
        DeviceProfile {
            id: DeviceId(id.to_string()),
            name: format!("Printer {id}"),
            model: DeviceModel {
                brand: "DYMO".into(),
                model: "LabelWriter 550".into(),
                protocol: Protocol::Dymo,
                capabilities: DeviceCapabilities {
                    dpi: Dpi(300.0),
                    max_width_mm: 56.0,
                    supports_cut: false,
                    supports_color: false,
                    reports_media: true,
                    ..Default::default()
                },
            },
            transport: Transport::Usb {
                vendor_id: 0x0922,
                product_id: 0x1001,
                serial: Some(id.to_string()),
            },
            default: false,
            default_media: None,
        }
    }

    #[test]
    fn upsert_and_remove_roundtrip() {
        let dir = std::env::temp_dir().join(format!("lbl-test-{}", std::process::id()));
        let store = ProfileStore::new(dir.join("printers.toml"));

        store.upsert(sample("a")).unwrap();
        store.upsert(sample("b")).unwrap();
        assert_eq!(store.load().unwrap().len(), 2);

        // Update existing id should not duplicate.
        let mut updated = sample("a");
        updated.name = "Renamed".into();
        store.upsert(updated).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded.iter().find(|p| p.id.0 == "a").unwrap().name,
            "Renamed"
        );

        store.remove(&DeviceId("a".into())).unwrap();
        assert_eq!(store.load().unwrap().len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

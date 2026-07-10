//! Pluggable storage for saved templates.
//!
//! This open crate ships a local-filesystem store ([`LocalFsStore`]) that
//! satisfies a single-user "save my templates" workflow. Account-scoped cloud
//! stores, shared/curated packs, versioning, and a marketplace are provided by
//! downstream crates that implement [`TemplateStore`]; the trait is the seam
//! between the open engine and those hosted features.

use std::path::PathBuf;

use serde_json::Value;

/// Errors produced by a [`TemplateStore`].
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// No template exists for the given id.
    #[error("template not found: {0}")]
    NotFound(String),

    /// The id is not usable as a storage key (e.g. contains path separators).
    #[error("invalid template id: {0}")]
    InvalidId(String),

    /// An I/O error occurred.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Any other store-specific failure.
    #[error("{0}")]
    Other(String),
}

/// A stored template plus minimal metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRecord {
    /// Stable identifier, unique within a store.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Template source (may contain frontmatter).
    pub source: String,
}

/// A place templates can be listed, fetched, and saved.
pub trait TemplateStore {
    /// List all stored templates.
    fn list(&self) -> Result<Vec<TemplateRecord>, StoreError>;
    /// Fetch a single template by id.
    fn get(&self, id: &str) -> Result<TemplateRecord, StoreError>;
    /// Create or replace a template.
    fn save(&self, record: &TemplateRecord) -> Result<(), StoreError>;
}

/// A [`TemplateStore`] backed by a local directory, one JSON file per template.
#[derive(Debug, Clone)]
pub struct LocalFsStore {
    dir: PathBuf,
}

impl LocalFsStore {
    /// Create a store rooted at `dir` (created lazily on first save).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path_for(&self, id: &str) -> Result<PathBuf, StoreError> {
        // Reject ids that could escape the store directory.
        if id.is_empty() || id.contains(['/', '\\']) || id.contains("..") {
            return Err(StoreError::InvalidId(id.to_string()));
        }
        Ok(self.dir.join(format!("{id}.json")))
    }

    fn read_record(path: &std::path::Path) -> Result<TemplateRecord, StoreError> {
        let text = std::fs::read_to_string(path)?;
        let value: Value =
            serde_json::from_str(&text).map_err(|e| StoreError::Other(e.to_string()))?;
        let field = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        Ok(TemplateRecord {
            id: field("id"),
            name: field("name"),
            source: field("source"),
        })
    }
}

impl TemplateStore for LocalFsStore {
    fn list(&self) -> Result<Vec<TemplateRecord>, StoreError> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                out.push(Self::read_record(&path)?);
            }
        }
        Ok(out)
    }

    fn get(&self, id: &str) -> Result<TemplateRecord, StoreError> {
        let path = self.path_for(id)?;
        match std::fs::exists(&path) {
            Ok(true) => Self::read_record(&path),
            Ok(false) => Err(StoreError::NotFound(id.to_string())),
            Err(e) => Err(e.into()),
        }
    }

    fn save(&self, record: &TemplateRecord) -> Result<(), StoreError> {
        let path = self.path_for(&record.id)?;
        std::fs::create_dir_all(&self.dir)?;
        let value = serde_json::json!({
            "id": record.id,
            "name": record.name,
            "source": record.source,
        });
        let text =
            serde_json::to_string_pretty(&value).map_err(|e| StoreError::Other(e.to_string()))?;
        std::fs::write(&path, text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str) -> TemplateRecord {
        TemplateRecord {
            id: id.to_string(),
            name: format!("Template {id}"),
            source: "<div>{{ name }}</div>".to_string(),
        }
    }

    #[test]
    fn save_get_list_roundtrip() {
        let dir = std::env::temp_dir().join(format!("lbl-store-test-{}", std::process::id()));
        let store = LocalFsStore::new(&dir);

        assert!(store.list().unwrap().is_empty());
        store.save(&rec("a")).unwrap();
        store.save(&rec("b")).unwrap();

        assert_eq!(store.get("a").unwrap(), rec("a"));
        assert_eq!(store.list().unwrap().len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_unsafe_ids() {
        let store = LocalFsStore::new(std::env::temp_dir());
        assert!(matches!(
            store.get("../etc/passwd"),
            Err(StoreError::InvalidId(_))
        ));
        assert!(matches!(store.get(""), Err(StoreError::InvalidId(_))));
    }

    #[test]
    fn missing_is_not_found() {
        let dir = std::env::temp_dir().join(format!("lbl-store-missing-{}", std::process::id()));
        let store = LocalFsStore::new(&dir);
        assert!(matches!(store.get("nope"), Err(StoreError::NotFound(_))));
    }
}

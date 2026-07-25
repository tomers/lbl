//! Layered configuration for the `lbl` toolchain.
//!
//! Configuration is merged from multiple sources in an idiomatic precedence
//! order (lowest to highest priority):
//!
//! 1. Built-in defaults
//! 2. System config file (`/etc/lbl/config.toml`)
//! 3. User config file (`~/.config/lbl/config.toml`)
//! 4. Project/local config file (`./lbl.toml`)
//! 5. Environment variables (`LBL_*`)
//! 6. Explicit CLI overrides (highest priority)
//!
//! The merge is implemented with [`figment`], which also records provenance so
//! tools can show *which layer* supplied each effective
//! value (see [`Loader::figment`] and [`describe_sources`]).
//!
//! User-owned printers are persisted separately (see [`ProfileStore`]) so that
//! a disconnected printer keeps its desired configuration across runs.

mod loader;
mod model;
mod paths;
mod profiles;

pub use loader::{describe_sources, format_effective, Loader};
pub use model::{
    CatalogConfig, Config, DriverPrintConfig, DymoPrintConfig, GeneralConfig, PrintConfig,
    RenderConfig, StyleBarcode, StyleChrome, StyleConfig, StyleFit, StyleMediaInset, StylePadding,
    StyleQr, StyleTypography,
};
pub use paths::{describe_paths, format_paths_report, stdout_color, ConfigPaths, PathLine};
pub use profiles::ProfileStore;

/// Errors produced while loading or persisting configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A configuration source could not be parsed/merged.
    #[error("failed to load configuration: {0}")]
    Load(String),

    /// Reading or writing a config/profile file failed.
    #[error("config io error at {path}: {source}")]
    Io {
        /// The path involved.
        path: String,
        /// The underlying io error.
        source: std::io::Error,
    },

    /// Serializing profiles to TOML failed.
    #[error("failed to serialize profiles: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, ConfigError>;

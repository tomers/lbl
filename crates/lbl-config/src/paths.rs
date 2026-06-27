//! Resolution of the standard configuration file locations.

use std::path::PathBuf;

use directories::{BaseDirs, ProjectDirs};

/// The set of paths the loader consults, in precedence order.
#[derive(Debug, Clone)]
pub struct ConfigPaths {
    /// System-wide config file (lowest precedence among files).
    pub system: PathBuf,
    /// Per-user config file.
    pub user: PathBuf,
    /// Project/local config file (highest precedence among files).
    pub project: PathBuf,
    /// Directory user-owned printer profiles are persisted in.
    pub profiles: PathBuf,
    /// Cache directory for catalog images and render scratch.
    pub cache: PathBuf,
}

impl ConfigPaths {
    /// Resolve the standard paths for this platform, using `cwd` as the base
    /// for the project-local config.
    pub fn resolve(cwd: &std::path::Path) -> Self {
        let project_dirs = ProjectDirs::from("org", "labelle", "lbl");
        let user_config_dir = project_dirs
            .as_ref()
            .map(|p| p.config_dir().to_path_buf())
            .or_else(|| BaseDirs::new().map(|b| b.config_dir().join("lbl")))
            .unwrap_or_else(|| PathBuf::from(".lbl"));
        let cache_dir = project_dirs
            .as_ref()
            .map(|p| p.cache_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".lbl-cache"));

        Self {
            system: PathBuf::from("/etc/lbl/config.toml"),
            user: user_config_dir.join("config.toml"),
            project: cwd.join("lbl.toml"),
            profiles: user_config_dir.join("printers.toml"),
            cache: cache_dir,
        }
    }

    /// Resolve using the current working directory.
    pub fn discover() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::resolve(&cwd)
    }
}

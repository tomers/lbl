//! Builds the layered [`figment::Figment`] and resolves the effective config.

use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};

use crate::model::Config;
use crate::paths::ConfigPaths;
use crate::{ConfigError, Result};

/// Builds a layered configuration following idiomatic precedence.
///
/// Sources are merged lowest-to-highest:
/// defaults < system file < user file < project file < env (`LBL_*`) < CLI.
#[derive(Debug, Clone)]
pub struct Loader {
    paths: ConfigPaths,
    figment: Figment,
}

impl Loader {
    /// Start a loader with the standard discovered paths and the base layers
    /// (defaults + config files + environment) applied.
    pub fn new() -> Self {
        Self::with_paths(ConfigPaths::discover())
    }

    /// Start a loader with explicit paths (useful for tests).
    pub fn with_paths(paths: ConfigPaths) -> Self {
        let figment = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file(&paths.system))
            .merge(Toml::file(&paths.user))
            .merge(Toml::file(&paths.project))
            .merge(Env::prefixed("LBL_").split("__"));
        Self { paths, figment }
    }

    /// Layer CLI overrides on top (highest priority). `overrides` is any
    /// serializable value (typically a flattened struct of `Option` fields).
    pub fn with_cli_overrides<T: serde::Serialize>(mut self, overrides: T) -> Self {
        self.figment = self.figment.merge(Serialized::defaults(overrides));
        self
    }

    /// The resolved paths used by this loader.
    pub fn paths(&self) -> &ConfigPaths {
        &self.paths
    }

    /// Access the underlying figment (e.g. for provenance introspection).
    pub fn figment(&self) -> &Figment {
        &self.figment
    }

    /// Extract the fully-merged [`Config`].
    pub fn load(&self) -> Result<Config> {
        self.figment
            .extract()
            .map_err(|e| ConfigError::Load(e.to_string()))
    }
}

/// Format the effective configuration for display. When `include_sources` is
/// true, append a provenance table from figment metadata (file path, `LBL_*`
/// env var, built-in default, etc.).
pub fn format_effective(loader: &Loader, include_sources: bool) -> Result<String> {
    let cfg = loader.load()?;
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| ConfigError::Load(e.to_string()))?;
    if !include_sources {
        return Ok(json);
    }
    let mut out = json;
    out.push_str("\n\nSources\n");
    for (key, source) in describe_sources(loader.figment()) {
        out.push_str(&format!("{key}\t{source}\n"));
    }
    Ok(out)
}

impl Default for Loader {
    fn default() -> Self {
        Self::new()
    }
}

/// Describe where each effective configuration value came from, for display in
/// CLIs and HTTP clients.
///
/// Returns `(dotted.key, source description)` pairs sorted by key.
pub fn describe_sources(figment: &Figment) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (path, _value) in flatten(figment) {
        let source = figment
            .find_metadata(&path)
            .map(|m| m.name.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        out.push((path, source));
    }
    out.sort();
    out
}

fn flatten(figment: &Figment) -> Vec<(String, figment::value::Value)> {
    let mut out = Vec::new();
    if let Ok(map) = figment.extract::<figment::value::Dict>() {
        for (k, v) in map {
            flatten_value(&k, &v, &mut out);
        }
    }
    out
}

fn flatten_value(
    prefix: &str,
    value: &figment::value::Value,
    out: &mut Vec<(String, figment::value::Value)>,
) {
    match value {
        figment::value::Value::Dict(_, dict) => {
            for (k, v) in dict {
                let key = format!("{prefix}.{k}");
                flatten_value(&key, v, out);
            }
        }
        other => out.push((prefix.to_string(), other.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn empty_paths() -> ConfigPaths {
        ConfigPaths {
            system: PathBuf::from("/nonexistent/system.toml"),
            user: PathBuf::from("/nonexistent/user.toml"),
            project: PathBuf::from("/nonexistent/project.toml"),
            profiles: PathBuf::from("/nonexistent/printers.toml"),
            cache: PathBuf::from("/nonexistent/cache"),
        }
    }

    #[test]
    fn defaults_load_when_no_files_present() {
        let cfg = Loader::with_paths(empty_paths()).load().unwrap();
        assert_eq!(cfg.render.supersample, 4);
        assert_eq!(cfg.render.dither, "floyd-steinberg");
        assert!(cfg.catalog.affiliate_enabled);
    }

    #[test]
    #[allow(clippy::result_large_err)]
    fn print_settings_from_env() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LBL_PRINT__CONFIRM", "true");
            jail.set_env("LBL_PRINT__BLUETOOTH", "D110");
            jail.set_env("LBL_PRINT__PROTOCOL", "niimbot");
            let cfg = Loader::with_paths(empty_paths()).load().unwrap();
            assert!(cfg.print.confirm);
            assert_eq!(cfg.print.bluetooth.as_deref(), Some("D110"));
            assert_eq!(cfg.print.protocol.as_deref(), Some("niimbot"));
            Ok(())
        });
    }

    #[test]
    fn cli_overrides_win() {
        #[derive(serde::Serialize)]
        struct Over {
            render: RenderOver,
        }
        #[derive(serde::Serialize)]
        struct RenderOver {
            supersample: u32,
        }
        let cfg = Loader::with_paths(empty_paths())
            .with_cli_overrides(Over {
                render: RenderOver { supersample: 8 },
            })
            .load()
            .unwrap();
        assert_eq!(cfg.render.supersample, 8);
    }

    #[test]
    fn format_effective_includes_sources() {
        let loader = Loader::with_paths(empty_paths());
        let out = format_effective(&loader, true).unwrap();
        assert!(out.contains("\"supersample\": 4"));
        assert!(out.contains("Sources\n"));
        assert!(out.contains("render.supersample"));
    }
}

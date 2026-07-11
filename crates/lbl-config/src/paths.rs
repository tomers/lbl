//! Resolution of the standard configuration file locations.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use directories::{BaseDirs, ProjectDirs};

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
const OK: &str = "\x1b[38;5;114m";
const MISSING: &str = "\x1b[31m";

/// Whether stdout should be colorized. Honors `NO_COLOR`.
pub fn stdout_color() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

/// One line in a `config paths` report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathLine {
    /// Short label (`system`, `user`, `catalog[0]`, …).
    pub label: String,
    /// Resolved path on disk.
    pub path: PathBuf,
    /// Whether [`Self::path`] exists on disk.
    pub exists: bool,
    /// Human-readable status (`missing`, `4 sections`, `2 printers`, …).
    pub status: String,
}

/// Describe the standard config locations and any extra catalog files from the
/// merged config.
pub fn describe_paths(paths: &ConfigPaths, catalog_extra: &[String]) -> Vec<PathLine> {
    let mut lines = vec![
        path_line("system", &paths.system, describe_config_file),
        path_line("user", &paths.user, describe_config_file),
        path_line("project", &paths.project, describe_config_file),
        path_line("profiles", &paths.profiles, describe_profiles_file),
        path_line("cache", &paths.cache, describe_directory),
    ];
    for (i, extra) in catalog_extra.iter().enumerate() {
        lines.push(path_line(
            &format!("catalog[{i}]"),
            Path::new(extra),
            describe_catalog_file,
        ));
    }
    lines
}

/// Format [`describe_paths`] for terminal output (label, path, status columns).
///
/// When `color` is true, existing paths are shown in green and missing paths in
/// red; missing path columns are dimmed.
pub fn format_paths_report(paths: &ConfigPaths, catalog_extra: &[String], color: bool) -> String {
    let lines = describe_paths(paths, catalog_extra);
    if lines.is_empty() {
        return String::new();
    }
    let label_w = lines.iter().map(|l| l.label.len()).max().unwrap_or(0);
    let path_w = lines
        .iter()
        .map(|l| l.path.display().to_string().len())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for line in lines {
        let path_plain = line.path.display().to_string();
        let pad = path_w - path_plain.len();
        let path_col = if color && !line.exists {
            format!("{DIM}{path_plain}{RESET}{}", " ".repeat(pad))
        } else {
            format!("{path_plain}{}", " ".repeat(pad))
        };
        let status_col = if color {
            let code = if line.exists { OK } else { MISSING };
            format!("{code}{}{RESET}", line.status)
        } else {
            line.status.clone()
        };
        out.push_str(&format!(
            "{:<label_w$}  {path_col}  {status_col}\n",
            line.label,
            label_w = label_w,
        ));
    }
    out
}

fn path_line(label: &str, path: &Path, describe: fn(&Path) -> String) -> PathLine {
    PathLine {
        label: label.to_string(),
        path: path.to_path_buf(),
        exists: path.exists(),
        status: describe(path),
    }
}

fn describe_config_file(path: &Path) -> String {
    if !path.exists() {
        return "missing".into();
    }
    if !path.is_file() {
        return "not a file".into();
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return format!("unreadable: {e}"),
    };
    if text.trim().is_empty() {
        return "empty".into();
    }
    match toml::from_str::<toml::Value>(&text) {
        Ok(toml::Value::Table(table)) => {
            let n = table.len();
            if n == 1 {
                "1 section".into()
            } else {
                format!("{n} sections")
            }
        }
        Ok(_) => "present".into(),
        Err(e) => format!("invalid: {e}"),
    }
}

fn describe_profiles_file(path: &Path) -> String {
    if !path.exists() {
        return "missing".into();
    }
    if !path.is_file() {
        return "not a file".into();
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return format!("unreadable: {e}"),
    };
    if text.trim().is_empty() {
        return "empty".into();
    }
    match toml::from_str::<toml::Value>(&text) {
        Ok(toml::Value::Table(table)) => {
            let n = table
                .get("printers")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            if n == 1 {
                "1 printer".into()
            } else {
                format!("{n} printers")
            }
        }
        Ok(_) => "present".into(),
        Err(e) => format!("invalid: {e}"),
    }
}

fn describe_catalog_file(path: &Path) -> String {
    if !path.exists() {
        return "missing".into();
    }
    if !path.is_file() {
        return "not a file".into();
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return format!("unreadable: {e}"),
    };
    if text.trim().is_empty() {
        return "empty".into();
    }
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "json" => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(serde_json::Value::Array(items)) => format!("{} entries", items.len()),
            Ok(serde_json::Value::Object(map)) => count_catalog_sections(&map),
            Ok(_) => "present".into(),
            Err(e) => format!("invalid: {e}"),
        },
        _ => match toml::from_str::<toml::Value>(&text) {
            Ok(toml::Value::Table(table)) => count_catalog_sections_toml(&table),
            Ok(_) => "present".into(),
            Err(e) => format!("invalid: {e}"),
        },
    }
}

fn count_catalog_sections(map: &serde_json::Map<String, serde_json::Value>) -> String {
    let entries = map
        .get("entries")
        .and_then(|v| v.as_array())
        .map(|a| a.len());
    let printers = map
        .get("printers")
        .and_then(|v| v.as_array())
        .map(|a| a.len());
    match (entries, printers) {
        (Some(e), Some(p)) if e > 0 || p > 0 => format!("{e} entries, {p} printers"),
        (Some(e), _) if e > 0 => format!("{e} entries"),
        (_, Some(p)) if p > 0 => format!("{p} printers"),
        _ => {
            let n = map.len();
            if n == 1 {
                "1 section".into()
            } else {
                format!("{n} sections")
            }
        }
    }
}

fn count_catalog_sections_toml(table: &toml::map::Map<String, toml::Value>) -> String {
    let entries = table
        .get("entries")
        .and_then(|v| v.as_array())
        .map(|a| a.len());
    let printers = table
        .get("printers")
        .and_then(|v| v.as_array())
        .map(|a| a.len());
    match (entries, printers) {
        (Some(e), Some(p)) if e > 0 || p > 0 => format!("{e} entries, {p} printers"),
        (Some(e), _) if e > 0 => format!("{e} entries"),
        (_, Some(p)) if p > 0 => format!("{p} printers"),
        _ => {
            let n = table.len();
            if n == 1 {
                "1 section".into()
            } else {
                format!("{n} sections")
            }
        }
    }
}

fn describe_directory(path: &Path) -> String {
    if !path.exists() {
        return "missing".into();
    }
    if !path.is_dir() {
        return "not a directory".into();
    }
    match std::fs::read_dir(path) {
        Ok(entries) => {
            let n = entries.count();
            if n == 1 {
                "1 item".into()
            } else {
                format!("{n} items")
            }
        }
        Err(e) => format!("unreadable: {e}"),
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn empty_paths(root: &Path) -> ConfigPaths {
        ConfigPaths {
            system: root.join("system.toml"),
            user: root.join("user.toml"),
            project: root.join("lbl.toml"),
            profiles: root.join("printers.toml"),
            cache: root.join("cache"),
        }
    }

    #[test]
    fn describe_paths_marks_missing_and_counts_sections() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("user.toml"),
            "[style]\nfont_size_mm = 3.0\n[print]\nconfirm = true\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("printers.toml"),
            "[[printers]]\nid = \"a\"\n[[printers]]\nid = \"b\"\n",
        )
        .unwrap();
        fs::create_dir(dir.path().join("cache")).unwrap();
        fs::write(dir.path().join("cache").join("x.png"), b"x").unwrap();

        let paths = empty_paths(dir.path());
        let lines = describe_paths(&paths, &[]);
        let by_label: std::collections::HashMap<_, _> = lines
            .iter()
            .map(|l| (l.label.as_str(), l.status.as_str()))
            .collect();

        assert_eq!(by_label["system"], "missing");
        assert_eq!(by_label["user"], "2 sections");
        assert_eq!(by_label["project"], "missing");
        assert_eq!(by_label["profiles"], "2 printers");
        assert_eq!(by_label["cache"], "1 item");
    }

    #[test]
    fn format_paths_report_aligns_columns() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("lbl.toml"), "[style]\n").unwrap();
        let paths = empty_paths(dir.path());
        let out = format_paths_report(&paths, &["./extra-catalog.toml".into()], false);
        assert!(out.contains("missing"));
        assert!(out.contains("1 section"));
        assert!(out.lines().count() >= 6);
    }

    #[test]
    fn format_paths_report_colorizes_by_existence() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("user.toml"), "[style]\n").unwrap();
        let paths = empty_paths(dir.path());
        let out = format_paths_report(&paths, &[], true);
        assert!(out.contains(MISSING));
        assert!(out.contains(OK));
        assert!(out.contains("missing"));
        assert!(out.contains("1 section"));
    }
}

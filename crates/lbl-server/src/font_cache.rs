//! Disk cache + fetch for label-font binaries used at raster time.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use lbl_text::{face_paths_for_slugs, font_asset_url, resolve_slug};
use once_cell::sync::Lazy;
use regex::Regex;

static LBL_FONT_ATTR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"data-lbl-font="([^"]+)""#).expect("lbl font attr regex"));

/// On-disk cache under the process cache dir for fetched `woff2` faces.
#[derive(Debug, Clone)]
pub struct FontFileCache {
    root: PathBuf,
}

impl FontFileCache {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Default cache rooted at `LBL_FONT_CACHE_DIR` or a temp directory.
    pub fn from_env_or_default() -> Self {
        let root = std::env::var("LBL_FONT_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("lbl-font-cache"));
        Self { root }
    }

    fn cache_path(&self, rel_path: &str) -> PathBuf {
        self.root.join(rel_path)
    }

    /// Load bytes for `rel_path`, using disk cache or fetching from `base_url`.
    pub fn get_or_fetch(&self, base_url: &str, rel_path: &str) -> Result<Vec<u8>> {
        let path = self.cache_path(rel_path);
        if path.is_file() {
            return fs::read(&path).with_context(|| format!("read font cache {}", path.display()));
        }

        // Offline fixtures / local trees via file:// or a filesystem directory base.
        if let Some(local) = local_file_from_base(base_url, rel_path) {
            let bytes =
                fs::read(&local).with_context(|| format!("read local font {}", local.display()))?;
            self.write_cache(&path, &bytes)?;
            return Ok(bytes);
        }

        let url = font_asset_url(base_url, rel_path);
        let bytes = http_get_bytes(&url).with_context(|| format!("fetch font face {url}"))?;
        if bytes.len() < 100 {
            return Err(anyhow!("font face {url} is too small ({})", bytes.len()));
        }
        self.write_cache(&path, &bytes)?;
        Ok(bytes)
    }

    fn write_cache(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)?;
        Ok(())
    }
}

fn local_file_from_base(base_url: &str, rel_path: &str) -> Option<PathBuf> {
    let base = base_url.trim();
    if let Some(rest) = base.strip_prefix("file://") {
        return Some(Path::new(rest).join(rel_path));
    }
    let p = Path::new(base);
    if p.is_dir() {
        return Some(p.join(rel_path));
    }
    None
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("lbl-server-font-cache/1.0")
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .context("build font HTTP client")?;
    let response = client
        .get(url)
        .send()
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("status for {url}"))?;
    let bytes = response.bytes().context("read font HTTP body")?;
    Ok(bytes.to_vec())
}

/// Collect `data-lbl-font` slugs from authoring or transpiled HTML.
pub fn font_slugs_in_html(html: &str) -> Vec<String> {
    let mut slugs = Vec::new();
    for caps in LBL_FONT_ATTR_RE.captures_iter(html) {
        let slug = caps[1].to_string();
        if resolve_slug(&slug).is_some() && !slugs.iter().any(|s| s == &slug) {
            slugs.push(slug);
        }
    }
    slugs
}

/// Fetch every face file required by web-font slugs in `html`.
pub fn load_font_files_for_html(
    html: &str,
    base_url: &str,
    cache: &FontFileCache,
) -> Result<HashMap<String, Vec<u8>>> {
    let slugs = font_slugs_in_html(html);
    let paths = face_paths_for_slugs(slugs.iter().map(String::as_str));
    let mut files = HashMap::new();
    for path in paths {
        let bytes = cache.get_or_fetch(base_url, path)?;
        files.insert(path.to_string(), bytes);
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/label-fonts-fixtures")
    }

    #[test]
    fn loads_heebo_faces_from_fixtures() {
        let cache = FontFileCache::new(std::env::temp_dir().join("lbl-font-cache-test-heebo"));
        let html = r#"<span data-lbl-font="heebo">שלום</span>"#;
        let files = load_font_files_for_html(html, fixtures_dir().to_str().unwrap(), &cache)
            .expect("fixture fonts");
        assert!(files
            .keys()
            .any(|k| k.contains("heebo") && k.contains("hebrew")));
        assert!(files.values().all(|b| b.len() > 100));
    }
}

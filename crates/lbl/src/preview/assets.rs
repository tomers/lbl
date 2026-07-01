//! Embedded static assets for the Nuxt UI preview bundle.

use std::path::Path;

use anyhow::{Context, Result};
use include_dir::{include_dir, Dir};

static PREVIEW_ASSETS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets/preview");

/// Extract the embedded preview UI into `dest`.
pub fn extract_preview_assets(dest: &Path) -> Result<()> {
    PREVIEW_ASSETS
        .extract(dest)
        .with_context(|| format!("extract preview UI assets to {}", dest.display()))
}

/// The built `index.html` shell from the Nuxt bundle (before payload injection).
pub fn index_shell() -> Result<String> {
    PREVIEW_ASSETS
        .get_file("index.html")
        .and_then(|file| file.contents_utf8())
        .map(str::to_string)
        .with_context(|| "embedded preview UI is missing index.html; run `just preview-ui-build`")
}

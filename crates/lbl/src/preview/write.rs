//! Write the HTML preview bundle (static UI + PNGs + injected payload).

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use image::ImageFormat;
use lbl_core::bitmap::MonoBitmap;

use super::assets::{extract_preview_assets, index_shell};
use super::context::HtmlPreviewContext;
use crate::debug::LabelTrace;

/// Resolved output locations for an HTML preview bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlPreviewPaths {
    /// Directory that contains `index.html` and the `images/` subfolder.
    pub bundle_dir: PathBuf,
    /// Path to the gallery page (typically `<bundle_dir>/index.html`).
    pub index_html: PathBuf,
}

/// Decide where an HTML preview bundle should be written.
pub fn resolve_html_preview_paths(
    out_dir: Option<&Path>,
    file: Option<&Path>,
) -> Result<HtmlPreviewPaths> {
    if out_dir.is_some() && file.is_some() {
        bail!("--protocol html: pass --out-dir or --file, not both");
    }
    if let Some(dir) = out_dir {
        let bundle_dir = dir.to_path_buf();
        return Ok(HtmlPreviewPaths {
            index_html: bundle_dir.join("index.html"),
            bundle_dir,
        });
    }
    if let Some(file) = file {
        let index_html = file.to_path_buf();
        if index_html.extension().is_none_or(|ext| ext != "html") {
            bail!("--file with --protocol html must end in .html");
        }
        let bundle_dir = index_html
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_path_buf();
        return Ok(HtmlPreviewPaths {
            bundle_dir,
            index_html,
        });
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let bundle_dir = std::env::temp_dir().join(format!(
        "lbl-preview-{}-{stamp}",
        std::process::id()
    ));
    Ok(HtmlPreviewPaths {
        index_html: bundle_dir.join("index.html"),
        bundle_dir,
    })
}

/// Write PNGs, extract the UI bundle, and inject the preview payload into
/// `index.html`.
pub fn write_html_preview(
    context: &HtmlPreviewContext,
    traces: &[LabelTrace],
    paths: &HtmlPreviewPaths,
) -> Result<()> {
    std::fs::create_dir_all(&paths.bundle_dir).with_context(|| {
        format!(
            "create preview directory {}",
            paths.bundle_dir.display()
        )
    })?;

    extract_preview_assets(&paths.bundle_dir)?;

    let images_dir = paths.bundle_dir.join("images");
    std::fs::create_dir_all(&images_dir).with_context(|| {
        format!("create preview images directory {}", images_dir.display())
    })?;

    for trace in traces {
        let name = format!("label-{:04}.png", trace.index);
        let png = png_bytes_from_mono(&trace.dithered)?;
        std::fs::write(images_dir.join(&name), &png).with_context(|| {
            format!("write preview image {name}")
        })?;
    }

    let payload =
        serde_json::to_string(context).context("serialize html preview payload")?;
    let html = inject_preview_payload(&normalize_static_paths(&index_shell()?), &payload)?;

    if let Some(parent) = paths.index_html.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(&paths.index_html, html).with_context(|| {
        format!("write preview page {}", paths.index_html.display())
    })?;
    Ok(())
}

fn inject_preview_payload(shell: &str, payload_json: &str) -> Result<String> {
    const MARKER: &str = "<!--LBL_PREVIEW_PAYLOAD-->";
    if shell.contains(MARKER) {
        let script = format!(
            "<script>window.__LBL_PREVIEW__={payload_json};</script>\n{MARKER}"
        );
        return Ok(shell.replace(MARKER, &script));
    }

    let script = format!("<script>window.__LBL_PREVIEW__={payload_json};</script>");
    if let Some(pos) = shell.find("<script") {
        return Ok(format!(
            "{}\n{}\n{}",
            &shell[..pos],
            script,
            &shell[pos..]
        ));
    }
    if let Some(pos) = shell.find("</body>") {
        return Ok(format!(
            "{}\n{}\n{}",
            &shell[..pos],
            script,
            &shell[pos..]
        ));
    }
    Ok(format!("{shell}\n{script}\n"))
}

/// Fix absolute `/_nuxt/` asset paths for static hosting at arbitrary URLs.
fn normalize_static_paths(html: &str) -> String {
    html.replace("href=\"/_nuxt/", "href=\"./_nuxt/")
        .replace("src=\"/_nuxt/", "src=\"./_nuxt/")
        .replace("baseURL:\"/\"", "baseURL:\"./\"")
        .replace("buildAssetsDir:\"/_nuxt/\"", "buildAssetsDir:\"./_nuxt/\"")
}

fn png_bytes_from_mono(bitmap: &MonoBitmap) -> Result<Vec<u8>> {
    let img = lbl_driver_file::mono_to_luma(bitmap);
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png)
        .context("encode preview png")?;
    Ok(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::context::{
        HtmlPreviewInput, HtmlPreviewMedia, HtmlPreviewPrinter, HtmlPreviewTemplate,
    };
    use image::Rgba;
    use image::RgbaImage;
    use lbl_core::Rotation;
    use lbl_core::printer::Protocol;
    use lbl_dither::Algorithm;
    use lbl_transpile_html::AssetsBase;

    fn sample_trace(index: usize) -> LabelTrace {
        let mut dithered = MonoBitmap::new(4, 2);
        dithered.set(0, 0, true);
        dithered.set(2, 1, true);
        LabelTrace {
            index,
            authoring_html: String::new(),
            transpiled_html: String::new(),
            assets_base: AssetsBase::Cdn,
            width_dots: Some(4),
            height_dots: Some(2),
            rotation: Rotation::None,
            supersample: 1,
            rendered: RgbaImage::from_pixel(4, 2, Rgba([255, 255, 255, 255])),
            dither: Algorithm::Auto,
            dithered,
            protocol: Protocol::Html,
            driver_name: "html-preview".into(),
            media_type: None,
            encoded: Vec::new(),
        }
    }

    fn sample_context(count: usize) -> HtmlPreviewContext {
        HtmlPreviewContext::build(
            HtmlPreviewInput {
                printer: HtmlPreviewPrinter::from_run(
                    None,
                    None,
                    None,
                    Protocol::Html,
                    300.0,
                    None,
                    None,
                ),
                media: HtmlPreviewMedia::from_resolved(
                    &lbl_core::media::Media::fixed(25.0, 54.0, lbl_core::units::Dpi(300.0)),
                    Some("11352"),
                    Some("Test media"),
                ),
                template: HtmlPreviewTemplate {
                    kind: "text".into(),
                    path: None,
                    each: None,
                    body: "<div>hello</div>".into(),
                },
                data: serde_json::json!({"hello": "world"}),
                records: vec![serde_json::json!({"hello": "world"}); count],
            },
            &(0..count).map(sample_trace).collect::<Vec<_>>(),
        )
    }

    #[test]
    fn writes_index_payload_and_images() {
        let dir = std::env::temp_dir().join(format!(
            "lbl-preview-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = HtmlPreviewPaths {
            bundle_dir: dir.clone(),
            index_html: dir.join("index.html"),
        };
        let traces = vec![sample_trace(0), sample_trace(1)];
        write_html_preview(&sample_context(2), &traces, &paths).unwrap();
        let html = std::fs::read_to_string(dir.join("index.html")).unwrap();
        assert!(html.contains("window.__LBL_PREVIEW__="));
        assert!(html.contains("\"count\":2"));
        assert!(std::fs::metadata(dir.join("images/label-0000.png")).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_rejects_both_targets() {
        assert!(resolve_html_preview_paths(
            Some(Path::new("/tmp/out")),
            Some(Path::new("/tmp/out/index.html"))
        )
        .is_err());
    }

    #[test]
    fn normalize_static_paths_rewrites_nuxt_config() {
        let html = r#"<link href="/_nuxt/x.css"><script>baseURL:"/",buildAssetsDir:"/_nuxt/"</script>"#;
        let out = super::normalize_static_paths(html);
        assert!(out.contains("./_nuxt/x.css"));
        assert!(out.contains("baseURL:\"./\""));
        assert!(out.contains("buildAssetsDir:\"./_nuxt/\""));
    }
}

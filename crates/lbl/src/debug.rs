//! Pipeline tracing and a self-contained HTML debug report.
//!
//! When the `print` flow is run with a debug knob enabled, each label is
//! processed through [`crate::pipeline::encode_label_traced`], which captures
//! the input and output of every stage into a [`LabelTrace`]. [`render_report`]
//! turns those traces into a single standalone HTML page that documents the
//! whole pipeline: each stage's purpose, the equivalent `lbl-*` command line,
//! and a before/after view (e.g. the ditherer's grayscale input next to its
//! 1-bit output).

use base64::Engine as _;
use image::RgbaImage;
use lbl_core::bitmap::MonoBitmap;
use lbl_core::printer::Protocol;
use lbl_core::Rotation;
use lbl_dither::Algorithm;
use lbl_driver_file::MediaType;
use lbl_transpile_html::AssetsBase;

/// Everything captured while encoding a single label, used to build the report.
pub struct LabelTrace {
    /// Zero-based index of the label within the batch.
    pub index: usize,
    /// The authoring HTML fed into the pipeline.
    pub authoring_html: String,
    /// The browser-ready HTML produced by transpilation.
    pub transpiled_html: String,
    /// Where transpilation loaded JS libraries from.
    pub assets_base: AssetsBase,
    /// Width of the render canvas in device dots (the *logical* reading frame;
    /// `None` = content-determined). For landscape this is the media's feed
    /// length, since the canvas is transposed before rotation.
    pub width_dots: Option<u32>,
    /// Height of the render canvas in device dots (`None` = content-determined).
    pub height_dots: Option<u32>,
    /// Rotation applied to the rendered raster (after layout, before dither).
    pub rotation: Rotation,
    /// Supersample factor used by the renderer.
    pub supersample: u32,
    /// The rendered (post-rotation, pre-dither) raster.
    pub rendered: RgbaImage,
    /// The dithering algorithm used.
    pub dither: Algorithm,
    /// The 1-bit bitmap produced by the ditherer.
    pub dithered: MonoBitmap,
    /// Target protocol.
    pub protocol: Protocol,
    /// The driver that produced the encoded bytes.
    pub driver_name: String,
    /// For the virtual printer, the selected output format.
    pub media_type: Option<MediaType>,
    /// Final encoded bytes (protocol bytes, or image-file bytes for virtual).
    pub encoded: Vec<u8>,
}

/// The CLI-facing name of a protocol (matches the `--protocol` value).
pub fn protocol_cli_name(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Dymo => "dymo",
        Protocol::DymoLw => "dymo-lw",
        Protocol::EscPos => "escpos",
        Protocol::Zpl => "zpl",
        Protocol::Tspl => "tspl",
        Protocol::Niimbot => "niimbot",
        Protocol::Virtual => "virtual",
        Protocol::Console => "console",
    }
}

/// The CLI-facing name of a dither algorithm (matches `--algorithm`/`--dither`).
fn dither_cli(alg: Algorithm) -> String {
    match alg {
        Algorithm::Auto => "--algorithm auto".to_string(),
        Algorithm::FloydSteinberg => "--algorithm floyd-steinberg".to_string(),
        Algorithm::Ordered => "--algorithm ordered".to_string(),
        Algorithm::Threshold(t) => format!("--algorithm none --threshold {t}"),
    }
}

/// A human-readable label for a non-identity rotation, e.g. `90° clockwise`.
fn rotation_label(rotation: Rotation) -> Option<&'static str> {
    match rotation {
        Rotation::None => None,
        Rotation::Cw90 => Some("90° clockwise"),
        Rotation::Cw180 => Some("180°"),
        Rotation::Cw270 => Some("90° counter-clockwise"),
    }
}

fn assets_base_flag(base: &AssetsBase) -> String {
    match base {
        AssetsBase::Cdn => String::new(),
        AssetsBase::Local(p) => format!(" --assets-base {p}"),
    }
}

fn png_data_uri_from_rgba(img: &RgbaImage) -> String {
    let mut buf = std::io::Cursor::new(Vec::new());
    // PNG encoding of an in-memory raster does not fail in practice.
    let _ = img.write_to(&mut buf, image::ImageFormat::Png);
    data_uri("image/png", &buf.into_inner())
}

fn png_data_uri_from_mono(bmp: &MonoBitmap) -> String {
    let img = lbl_driver_file::mono_to_luma(bmp);
    let mut buf = std::io::Cursor::new(Vec::new());
    let _ = img.write_to(&mut buf, image::ImageFormat::Png);
    data_uri("image/png", &buf.into_inner())
}

fn data_uri(mime: &str, bytes: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{b64}")
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// A short hex dump of the first `max` bytes of `data`.
fn hex_preview(data: &[u8], max: usize) -> String {
    let mut out = String::new();
    for (i, byte) in data.iter().take(max).enumerate() {
        if i > 0 && i % 16 == 0 {
            out.push('\n');
        } else if i > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{byte:02x}"));
    }
    if data.len() > max {
        out.push_str(&format!("\n… ({} more bytes)", data.len() - max));
    }
    out
}

/// Render a complete HTML debug report for a batch of label traces.
pub fn render_report(traces: &[LabelTrace]) -> String {
    let mut body = String::new();
    for trace in traces {
        body.push_str(&render_label(trace));
    }

    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>lbl pipeline debug</title>\n<style>{CSS}</style>\n</head>\n<body>\n\
<header><h1>lbl pipeline debug</h1>\n\
<p>{n} label(s). Each stage shows its command-line equivalent and its input/output.</p>\n\
</header>\n{body}</body>\n</html>\n",
        n = traces.len(),
    )
}

fn render_label(t: &LabelTrace) -> String {
    let mut stages = String::new();

    // 1. Authoring HTML (input artifact).
    stages.push_str(&stage_card(
        "1",
        "Authoring HTML",
        "Source (text / template / HTML) turned into authoring HTML.",
        "lbl text … | lbl-template …",
        &code_panel("Authoring HTML", &t.authoring_html),
    ));

    // 2. Transpile.
    let transpile_cmd = format!(
        "lbl-transpile-html --mode print{}",
        assets_base_flag(&t.assets_base)
    );
    stages.push_str(&stage_card(
        "2",
        "Transpile",
        "Custom elements (&lt;qr&gt;/&lt;barcode&gt;) and flex layout become browser-ready HTML.",
        &transpile_cmd,
        &two_col(
            &code_panel("Input — authoring HTML", &t.authoring_html),
            &code_panel("Output — browser-ready HTML", &t.transpiled_html),
        ),
    ));

    // 3. Render.
    let dims = match (t.width_dots, t.height_dots) {
        (Some(w), Some(h)) => format!(" --width-dots {w} --height-dots {h}"),
        (Some(w), None) => format!(" --width-dots {w}"),
        (None, Some(h)) => format!(" --height-dots {h}"),
        (None, None) => String::new(),
    };
    let render_cmd = format!("lbl-render{dims} --supersample {}", t.supersample);
    let render_desc = match rotation_label(t.rotation) {
        Some(turn) => format!(
            "Headless Chromium rasterizes the HTML (two-pass: hi-res then Lanczos \
downscale), then the raster is rotated {turn} onto the print head."
        ),
        None => "Headless Chromium rasterizes the HTML (two-pass: hi-res then Lanczos downscale)."
            .to_string(),
    };
    let (rw, rh) = t.rendered.dimensions();
    stages.push_str(&stage_card(
        "3",
        "Render",
        &render_desc,
        &render_cmd,
        &two_col(
            &code_panel("Input — browser-ready HTML", &t.transpiled_html),
            &image_panel(
                &format!("Output — raster ({rw}×{rh})"),
                &png_data_uri_from_rgba(&t.rendered),
            ),
        ),
    ));

    // 4. Dither.
    let dither_cmd = format!("lbl-dither {}", dither_cli(t.dither));
    stages.push_str(&stage_card(
        "4",
        "Dither",
        "The grayscale raster is reduced to the printer's 1-bit depth (photo-aware error diffusion).",
        &dither_cmd,
        &two_col(
            &image_panel("Input — grayscale raster", &png_data_uri_from_rgba(&t.rendered)),
            &image_panel(
                &format!("Output — 1-bit bitmap ({}×{})", t.dithered.width, t.dithered.height),
                &png_data_uri_from_mono(&t.dithered),
            ),
        ),
    ));

    // 5. Encode.
    let encode_cmd = match t.media_type {
        Some(mt) => format!(
            "lbl-encode --protocol {} --media-type {}",
            protocol_cli_name(t.protocol),
            mt.name()
        ),
        None => format!("lbl-encode --protocol {}", protocol_cli_name(t.protocol)),
    };
    let output_panel = match t.media_type {
        // Virtual printer: the encoded bytes *are* a viewable image file.
        Some(mt) if !matches!(mt, MediaType::Pbm) => image_panel(
            &format!("Output — {} file ({} bytes)", mt.name(), t.encoded.len()),
            &data_uri(mt.mime(), &t.encoded),
        ),
        _ => code_panel(
            &format!("Output — {} bytes via {}", t.encoded.len(), t.driver_name),
            &hex_preview(&t.encoded, 256),
        ),
    };
    stages.push_str(&stage_card(
        "5",
        "Encode",
        "The 1-bit bitmap becomes protocol bytes (or, for the virtual printer, an image file).",
        &encode_cmd,
        &two_col(
            &image_panel("Input — 1-bit bitmap", &png_data_uri_from_mono(&t.dithered)),
            &output_panel,
        ),
    ));

    format!(
        "<section class=\"label\">\n<h2>Label #{idx}</h2>\n{stages}</section>\n",
        idx = t.index,
    )
}

fn stage_card(num: &str, title: &str, desc: &str, command: &str, content: &str) -> String {
    format!(
        "<article class=\"stage\">\n\
<div class=\"stage-head\"><span class=\"num\">{num}</span><h3>{title}</h3></div>\n\
<p class=\"desc\">{desc}</p>\n\
<pre class=\"cmd\"><code>$ {cmd}</code></pre>\n\
{content}\n</article>\n",
        cmd = escape_html(command),
    )
}

fn two_col(a: &str, b: &str) -> String {
    format!("<div class=\"cols\">{a}{b}</div>")
}

fn code_panel(label: &str, content: &str) -> String {
    format!(
        "<div class=\"panel\"><div class=\"panel-label\">{label}</div>\
<pre class=\"code\"><code>{}</code></pre></div>",
        escape_html(content)
    )
}

fn image_panel(label: &str, data_uri: &str) -> String {
    format!(
        "<div class=\"panel\"><div class=\"panel-label\">{label}</div>\
<div class=\"img-wrap\"><img alt=\"{label}\" src=\"{data_uri}\"></div></div>"
    )
}

const CSS: &str = "\
:root{color-scheme:light dark;--bg:#0f1115;--card:#1a1d24;--muted:#8b93a7;--fg:#e7eaf0;--accent:#5b9dff;--border:#2a2f3a}\
*{box-sizing:border-box}\
body{margin:0;font:14px/1.5 ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,sans-serif;background:var(--bg);color:var(--fg)}\
header{padding:24px 32px;border-bottom:1px solid var(--border)}\
header h1{margin:0 0 4px;font-size:20px}\
header p{margin:0;color:var(--muted)}\
.label{padding:16px 32px 32px}\
.label>h2{font-size:16px;color:var(--accent);border-bottom:1px solid var(--border);padding-bottom:8px}\
.stage{background:var(--card);border:1px solid var(--border);border-radius:10px;padding:16px;margin:16px 0}\
.stage-head{display:flex;align-items:center;gap:10px}\
.stage-head h3{margin:0;font-size:15px}\
.num{display:inline-grid;place-items:center;width:24px;height:24px;border-radius:50%;background:var(--accent);color:#06122b;font-weight:700;font-size:12px}\
.desc{color:var(--muted);margin:8px 0}\
.cmd{background:#06080d;border:1px solid var(--border);border-radius:8px;padding:10px 12px;overflow:auto;color:#9ee493}\
.cmd code{white-space:pre}\
.cols{display:grid;grid-template-columns:1fr 1fr;gap:12px;margin-top:12px}\
@media(max-width:820px){.cols{grid-template-columns:1fr}}\
.panel{background:#06080d;border:1px solid var(--border);border-radius:8px;overflow:hidden}\
.panel-label{padding:6px 10px;font-size:12px;color:var(--muted);border-bottom:1px solid var(--border);background:#0b0e14}\
.code{margin:0;padding:10px 12px;max-height:340px;overflow:auto;white-space:pre-wrap;word-break:break-word;font:12px/1.45 ui-monospace,SFMono-Regular,Menlo,monospace}\
.img-wrap{padding:12px;display:grid;place-items:center;background:repeating-conic-gradient(#1b1e25 0% 25%,#15171d 0% 50%) 50%/16px 16px}\
.img-wrap img{max-width:100%;height:auto;image-rendering:pixelated;background:#fff}\
";

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn sample_trace() -> LabelTrace {
        let mut dithered = MonoBitmap::new(8, 2);
        dithered.set(0, 0, true);
        LabelTrace {
            index: 0,
            authoring_html: "<div>hello</div>".into(),
            transpiled_html: "<html><body>hello</body></html>".into(),
            assets_base: AssetsBase::Cdn,
            width_dots: Some(8),
            height_dots: Some(2),
            rotation: Rotation::None,
            supersample: 3,
            rendered: RgbaImage::from_pixel(8, 2, Rgba([255, 255, 255, 255])),
            dither: Algorithm::Auto,
            dithered,
            protocol: Protocol::Virtual,
            driver_name: "virtual-file".into(),
            media_type: Some(MediaType::Png),
            encoded: vec![0x89, b'P', b'N', b'G'],
        }
    }

    #[test]
    fn report_contains_stage_commands_and_images() {
        let html = render_report(&[sample_trace()]);
        assert!(html.contains("lbl pipeline debug"));
        assert!(html.contains("lbl-transpile-html --mode print"));
        assert!(html.contains("lbl-render --width-dots 8 --height-dots 2 --supersample 3"));
        assert!(html.contains("lbl-dither --algorithm auto"));
        assert!(html.contains("lbl-encode --protocol virtual --media-type png"));
        // Stage visuals are embedded as inline data URIs.
        assert!(html.contains("data:image/png;base64,"));
        // Authoring HTML is shown escaped, not as live markup.
        assert!(html.contains("&lt;div&gt;hello&lt;/div&gt;"));
    }

    #[test]
    fn landscape_render_notes_rotation_and_logical_dims() {
        let mut t = sample_trace();
        // Landscape transposes the canvas (feed length becomes the render width)
        // and rotates a quarter-turn clockwise onto the head.
        t.width_dots = Some(2);
        t.height_dots = Some(8);
        t.rotation = Rotation::Cw90;
        let html = render_report(&[t]);
        assert!(html.contains("lbl-render --width-dots 2 --height-dots 8 --supersample 3"));
        assert!(html.contains("rotated 90° clockwise"));
    }

    #[test]
    fn auto_width_render_omits_width_flag() {
        let mut t = sample_trace();
        t.width_dots = None;
        t.height_dots = Some(8);
        let html = render_report(&[t]);
        assert!(html.contains("lbl-render --height-dots 8 --supersample 3"));
    }

    #[test]
    fn threshold_algorithm_renders_threshold_flag() {
        let mut t = sample_trace();
        t.dither = Algorithm::Threshold(200);
        t.media_type = None;
        t.protocol = Protocol::Zpl;
        let html = render_report(&[t]);
        assert!(html.contains("lbl-dither --algorithm none --threshold 200"));
        assert!(html.contains("lbl-encode --protocol zpl"));
    }
}

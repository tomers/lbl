//! End-to-end golden-image tests for the label pipeline.
//!
//! Each case authors a label (via `lbl-text`, `lbl-markdown`, `lbl-template`, or
//! raw HTML), runs it through transpile -> render -> dither -> encode, and
//! compares the produced image bytes against a checked-in reference under
//! `tests/golden/`. Together the cases cover the project's user-visible
//! functionality: every authoring front-end, the QR/barcode/image/sizing/flex
//! directives, configurable styling, all dithering algorithms, fixed and
//! continuous media, catalog-resolved media, every output image format, label
//! fit/scale/alignment, and a batched portrait identity-card showcase on DYMO
//! 99014 (LabelWriter 550).
//! Identity-card photos are fetched from Wikimedia Commons at test time (network
//! required for that case).
//!
//! The whole suite shares a single headless-Chromium instance. If Chromium
//! cannot be launched (e.g. no browser installed in CI), the test logs a notice
//! and passes rather than failing — the pipeline below it is independently unit
//! tested.
//!
//! Regenerate references with `LBL_BLESS=1 cargo test -p lbl --test golden_labels`.

mod common;

use common::*;
use lbl::{authoring_labels, resolve_media, Source};
use lbl_catalog::Catalog;
use lbl_core::media::Media;
use lbl_core::units::Dpi;
use lbl_dither::Algorithm;
use lbl_driver_file::{encode_image, MediaType};
use lbl_render::ChromiumBackend;
use lbl_transpile_html::LabelStyle;
use lbl_transpile_html::{LabelAlign, LabelFit, LabelValign, MediaInset, MediaInsetPx};

/// Render resolution for the suite. Kept modest so references stay small and
/// rendering is fast, while still leaving room for legible QR/barcodes.
const DPI: f64 = 150.0;
const SUPERSAMPLE: u32 = 2;

/// Print resolution for the identity-card showcase (DYMO 99014 portrait badges).
const ID_CARDS_DPI: f64 = 300.0;
const ID_CARDS_SUPERSAMPLE: u32 = 3;

#[test]
fn golden_labels() {
    let backend = match ChromiumBackend::launch() {
        Ok(backend) => backend,
        Err(err) => {
            eprintln!("skipping golden_labels: could not launch Chromium: {err}");
            return;
        }
    };

    let style = default_style(DPI, SUPERSAMPLE);
    let small = Media::fixed(40.0, 30.0, Dpi(DPI));
    let wide = Media::fixed(60.0, 28.0, Dpi(DPI));
    let strip = Media::continuous(50.0, Dpi(DPI));
    // Tall fixed label so fit-box scale and axis alignment leave visible margins.
    let layout_media = Media::fixed(40.0, 60.0, Dpi(DPI));
    let layout_style = bordered_style(DPI, SUPERSAMPLE);

    let mut failures: Vec<String> = Vec::new();

    // --- Authoring front-ends ------------------------------------------------

    // Plain text (lbl-text).
    failures.extend(run_case(
        &backend,
        "text_plain",
        Source::Text {
            text: "Hello, lbl!".into(),
            raw: false,
        },
        &small,
        &style,
        Algorithm::Auto,
        SUPERSAMPLE,
    ));

    // Raw text: inline directives are kept literal.
    failures.extend(run_case(
        &backend,
        "text_raw",
        Source::Text {
            text: "Keep {{qr:x}} literal".into(),
            raw: true,
        },
        &small,
        &style,
        Algorithm::Auto,
        SUPERSAMPLE,
    ));

    // Multi-line text (newlines -> <br>).
    failures.extend(run_case(
        &backend,
        "text_multiline",
        Source::Text {
            text: "Aisle 4\nBin 12\nQty 60".into(),
            raw: false,
        },
        &strip,
        &style,
        Algorithm::Auto,
        SUPERSAMPLE,
    ));

    // Inline size directive (relative font scaling).
    failures.extend(run_case(
        &backend,
        "text_size",
        Source::Text {
            text: "Order {{size:2:#44}} now".into(),
            raw: false,
        },
        &small,
        &style,
        Algorithm::Auto,
        SUPERSAMPLE,
    ));

    // Markdown front-end (lbl-markdown) with an inline QR directive. Continuous
    // media auto-sizes the height so the heading, paragraph, and QR all fit.
    failures.extend(run_case(
        &backend,
        "markdown",
        Source::Markdown("# Order 44\n\nShip **fast** to dock 4\n\n{{qr:https://track/42}}".into()),
        &strip,
        &style,
        Algorithm::Auto,
        SUPERSAMPLE,
    ));

    // Template front-end (lbl-template) batched over a data array -> 2 labels.
    failures.extend(run_case(
        &backend,
        "template_batch",
        Source::Template {
            template:
                "<div class=\"lbl-label\"><div class=\"lbl-text\">{{ name }} - #{{ id }}</div></div>"
                    .into(),
            data: Some(serde_json::json!([
                {"name": "Alpha", "id": 1},
                {"name": "Beta", "id": 2}
            ])),
            each: None,
        },
        &small,
        &style,
        Algorithm::Auto,
        SUPERSAMPLE,
    ));

    // Portrait identity badges on DYMO 99014 (54×101 mm), batched from a single
    // `.lbl` fixture (Sopranos character IDs with Wikimedia Commons photos), as with:
    //   lbl-template --template identity_cards.lbl --inline-resources
    let id_card_style = identity_card_style(ID_CARDS_DPI, ID_CARDS_SUPERSAMPLE);
    match resolve_media(
        &Catalog::bundled().expect("bundled catalog"),
        Some("99014"),
        None,
        None,
        ID_CARDS_DPI,
    ) {
        Ok(id_media) => failures.extend(run_lbl_fixture_case(
            &backend,
            "identity_cards",
            &identity_cards_lbl_fixture(),
            &id_media,
            &id_card_style,
            Algorithm::Auto,
            ID_CARDS_SUPERSAMPLE,
        )),
        Err(err) => failures.push(format!("identity_cards: resolve_media failed: {err}")),
    }

    // --- Directives ----------------------------------------------------------

    // QR code.
    failures.extend(run_case(
        &backend,
        "qr",
        Source::Text {
            text: "{{qr:https://lbl.example/42}}".into(),
            raw: false,
        },
        &small,
        &style,
        Algorithm::Auto,
        SUPERSAMPLE,
    ));

    // Barcode, default symbology (CODE128).
    failures.extend(run_case(
        &backend,
        "barcode_code128",
        Source::Text {
            text: "{{barcode:LBL-128}}".into(),
            raw: false,
        },
        &wide,
        &style,
        Algorithm::Auto,
        SUPERSAMPLE,
    ));

    // Barcode, explicit EAN-13 symbology.
    failures.extend(run_case(
        &backend,
        "barcode_ean13",
        Source::Text {
            text: "{{barcode:EAN13:0123456789012}}".into(),
            raw: false,
        },
        &wide,
        &style,
        Algorithm::Auto,
        SUPERSAMPLE,
    ));

    // Image directive (embedded data URI, kept hermetic). Continuous media so
    // the stretched image is fully contained.
    failures.extend(run_case(
        &backend,
        "image",
        Source::Text {
            text: format!("Logo {{{{image:{}}}}}", checkerboard_data_uri()),
            raw: false,
        },
        &strip,
        &style,
        Algorithm::Auto,
        SUPERSAMPLE,
    ));

    // --- Layout & styling ----------------------------------------------------

    // Flex layout utilities (row + space-between) combining text and a QR.
    failures.extend(run_case(
        &backend,
        "flex_layout",
        Source::Html(
            "<div class=\"lbl-label\"><div class=\"lbl-row lbl-between lbl-center\">\
             <div class=\"lbl-text\">SKU 7788</div><qr>https://lbl.example/7788</qr>\
             </div></div>"
                .into(),
        ),
        &wide,
        &style,
        Algorithm::Auto,
        SUPERSAMPLE,
    ));

    // Border + padding styling.
    failures.extend(run_case(
        &backend,
        "style_border",
        Source::Text {
            text: "Fragile".into(),
            raw: false,
        },
        &small,
        &bordered_style(DPI, SUPERSAMPLE),
        Algorithm::Auto,
        SUPERSAMPLE,
    ));

    // Fit-box scale and axis alignment (`tests/fixtures/layout/short_label.html`).
    failures.extend(run_layout_case(
        &backend,
        "layout_fit_scale_80_center",
        &layout_media,
        &layout_style,
        LayoutOptions {
            label_fit_scale: 0.8,
            ..LayoutOptions::fill_defaults()
        },
    ));
    failures.extend(run_layout_case(
        &backend,
        "layout_fit_scale_80_start",
        &layout_media,
        &layout_style,
        LayoutOptions {
            label_fit_scale: 0.8,
            label_align: LabelAlign::Start,
            label_valign: LabelValign::Start,
            ..LayoutOptions::fill_defaults()
        },
    ));
    failures.extend(run_layout_case(
        &backend,
        "layout_fit_scale_80_end",
        &layout_media,
        &layout_style,
        LayoutOptions {
            label_fit_scale: 0.8,
            label_align: LabelAlign::End,
            label_valign: LabelValign::End,
            ..LayoutOptions::fill_defaults()
        },
    ));
    failures.extend(run_layout_case(
        &backend,
        "layout_align_start",
        &layout_media,
        &layout_style,
        LayoutOptions {
            label_align: LabelAlign::Start,
            ..LayoutOptions::fill_defaults()
        },
    ));
    failures.extend(run_layout_case(
        &backend,
        "layout_valign_start",
        &layout_media,
        &layout_style,
        LayoutOptions {
            label_valign: LabelValign::Start,
            ..LayoutOptions::fill_defaults()
        },
    ));
    failures.extend(run_layout_case(
        &backend,
        "layout_align_end_valign_end",
        &layout_media,
        &layout_style,
        LayoutOptions {
            label_align: LabelAlign::End,
            label_valign: LabelValign::End,
            ..LayoutOptions::fill_defaults()
        },
    ));
    failures.extend(run_layout_case(
        &backend,
        "layout_continuous_align_end",
        &strip,
        &layout_style,
        LayoutOptions {
            label_fit: LabelFit::Content,
            label_align: LabelAlign::End,
            label_valign: LabelValign::Center,
            label_fit_scale: 1.0,
            media_inset: MediaInsetPx::default(),
        },
    ));
    failures.extend(run_layout_case(
        &backend,
        "layout_media_inset",
        &layout_media,
        &layout_style,
        LayoutOptions {
            label_fit_scale: 0.8,
            media_inset: MediaInset {
                all_mm: 3.0,
                start_mm: Some(5.0),
                cross_end_mm: Some(2.0),
                ..Default::default()
            }
            .to_px(DPI, SUPERSAMPLE),
            ..LayoutOptions::fill_defaults()
        },
    ));

    // --- Media ---------------------------------------------------------------

    // Catalog-resolved media (DYMO 11352 address label).
    match resolve_media(
        &Catalog::bundled().expect("bundled catalog"),
        Some("11352"),
        None,
        None,
        DPI,
    ) {
        Ok(catalog_media) => failures.extend(run_case(
            &backend,
            "catalog_sku",
            Source::Text {
                text: "From the catalog".into(),
                raw: false,
            },
            &catalog_media,
            &style,
            Algorithm::Auto,
            SUPERSAMPLE,
        )),
        Err(err) => failures.push(format!("catalog_sku: resolve_media failed: {err}")),
    }

    // --- Dithering algorithms ------------------------------------------------
    // A gradient makes the differences between algorithms visible (and stable).
    let gradient_html = format!(
        "<div class=\"lbl-label\"><img src=\"{}\" style=\"width:100%\" /></div>",
        gradient_data_uri(240, 80)
    );
    for (name, algorithm) in [
        ("dither_auto", Algorithm::Auto),
        ("dither_floyd", Algorithm::FloydSteinberg),
        ("dither_ordered", Algorithm::Ordered),
        ("dither_threshold", Algorithm::Threshold(128)),
    ] {
        let bitmap = render_bitmap(
            &backend,
            &gradient_html,
            &small,
            &style,
            SUPERSAMPLE,
            algorithm,
        );
        match encode_image(&bitmap, MediaType::Png) {
            Ok(png) => {
                if let Err(err) = check_png(name, &png) {
                    failures.push(err);
                }
            }
            Err(err) => failures.push(format!("{name}: encode failed: {err}")),
        }
    }

    // --- Output image formats ------------------------------------------------
    // Render once, then encode the same bitmap to every supported media type.
    let formats_bitmap = render_bitmap(
        &backend,
        "<div class=\"lbl-label\"><div class=\"lbl-text\">Formats</div></div>",
        &small,
        &style,
        SUPERSAMPLE,
        Algorithm::Auto,
    );
    for media_type in MediaType::ALL {
        match encode_image(&formats_bitmap, media_type) {
            Ok(bytes) => {
                let name = format!("formats_{}", media_type.name());
                // The 1-bit PBM is exact; raster formats get the usual tolerance.
                let result = if media_type == MediaType::Pbm {
                    check_image(&name, media_type.extension(), &bytes, 0.0)
                } else {
                    check_image(&name, media_type.extension(), &bytes, 0.02)
                };
                if let Err(err) = result {
                    failures.push(err);
                }
            }
            Err(err) => failures.push(format!("formats_{}: {err}", media_type.name())),
        }
    }

    assert!(
        failures.is_empty(),
        "golden image mismatches:\n  - {}",
        failures.join("\n  - ")
    );
}

/// Author `source` into one or more labels, render each, and compare the PNG
/// output against `tests/golden/<name>.png` (suffixing `-<index>` when the
/// source expands into a batch). Returns a list of failure messages.
fn run_case(
    backend: &ChromiumBackend,
    name: &str,
    source: Source,
    media: &Media,
    style: &LabelStyle,
    algorithm: Algorithm,
    supersample: u32,
) -> Vec<String> {
    let labels = match authoring_labels(source, &lbl_template::BatchSelection::default()) {
        Ok(labels) => labels,
        Err(err) => return vec![format!("{name}: authoring failed: {err}")],
    };

    let mut failures = Vec::new();
    let batched = labels.len() > 1;
    for label in &labels {
        let bitmap = render_bitmap(backend, &label.html, media, style, supersample, algorithm);
        let png = match encode_image(&bitmap, MediaType::Png) {
            Ok(png) => png,
            Err(err) => {
                failures.push(format!("{name}: encode failed: {err}"));
                continue;
            }
        };
        let golden = if batched {
            format!("{name}-{}", label.index)
        } else {
            name.to_string()
        };
        if let Err(err) = check_png(&golden, &png) {
            failures.push(err);
        }
    }
    failures
}

/// Render the shared layout fixture with explicit fit/alignment and compare PNG
/// output against `tests/golden/<name>.png`.
fn run_layout_case(
    backend: &ChromiumBackend,
    name: &str,
    media: &Media,
    style: &LabelStyle,
    layout: LayoutOptions,
) -> Vec<String> {
    let bitmap = render_bitmap_with_layout(
        backend,
        LAYOUT_FIXTURE_HTML,
        media,
        style,
        SUPERSAMPLE,
        Algorithm::Auto,
        layout,
    );
    match encode_image(&bitmap, MediaType::Png) {
        Ok(png) => match check_png(name, &png) {
            Ok(()) => Vec::new(),
            Err(err) => vec![err],
        },
        Err(err) => vec![format!("{name}: encode failed: {err}")],
    }
}

/// Like [`run_case`], but authors labels from a `.lbl` single-file fixture
/// (mirrors `lbl-template --template PATH --inline-resources`).
fn run_lbl_fixture_case(
    backend: &ChromiumBackend,
    name: &str,
    lbl_path: &std::path::Path,
    media: &Media,
    style: &LabelStyle,
    algorithm: Algorithm,
    supersample: u32,
) -> Vec<String> {
    let labels = match authoring_labels_from_lbl(lbl_path) {
        Ok(labels) => labels,
        Err(err) => return vec![format!("{name}: {err}")],
    };

    let mut failures = Vec::new();
    let batched = labels.len() > 1;
    for label in &labels {
        let bitmap = render_bitmap(backend, &label.html, media, style, supersample, algorithm);
        let png = match encode_image(&bitmap, MediaType::Png) {
            Ok(png) => png,
            Err(err) => {
                failures.push(format!("{name}: encode failed: {err}"));
                continue;
            }
        };
        let golden = if batched {
            format!("{name}-{}", label.index)
        } else {
            name.to_string()
        };
        if let Err(err) = check_png(&golden, &png) {
            failures.push(err);
        }
    }
    failures
}

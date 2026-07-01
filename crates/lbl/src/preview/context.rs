//! Serialized preview payload consumed by the Nuxt UI bundle.

use lbl_core::media::{Material, Media, MediaColor, MediaLength};
use lbl_core::printer::Protocol;
use serde::Serialize;
use serde_json::Value;

use crate::debug::{protocol_cli_name, LabelTrace};

/// Inputs gathered by the orchestrator before writing a preview bundle.
#[derive(Debug, Clone)]
pub struct HtmlPreviewInput {
    /// Resolved printer metadata, if any.
    pub printer: HtmlPreviewPrinter,
    /// Resolved media metadata.
    pub media: HtmlPreviewMedia,
    /// Template / source description.
    pub template: HtmlPreviewTemplate,
    /// Full data root passed to the template (or source summary).
    pub data: Value,
    /// Per-label record values aligned with pipeline traces.
    pub records: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HtmlPreviewContext {
    pub count: usize,
    pub printer: HtmlPreviewPrinter,
    pub media: HtmlPreviewMedia,
    pub template: HtmlPreviewTemplate,
    pub data: Value,
    pub labels: Vec<HtmlPreviewLabel>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HtmlPreviewPrinter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    pub protocol: String,
    pub dpi: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_width_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HtmlPreviewMedia {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sku: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub width_mm: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length_mm: Option<f64>,
    pub continuous: bool,
    pub dpi: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HtmlPreviewTemplate {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub each: Option<String>,
    pub body: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HtmlPreviewLabel {
    pub index: usize,
    pub image: String,
    pub width: u32,
    pub height: u32,
    pub record: Value,
}

impl HtmlPreviewContext {
    pub fn build(input: HtmlPreviewInput, traces: &[LabelTrace]) -> Self {
        let labels = traces
            .iter()
            .map(|trace| {
                let record = input
                    .records
                    .get(trace.index)
                    .cloned()
                    .unwrap_or(Value::Null);
                HtmlPreviewLabel {
                    index: trace.index,
                    image: format!("images/label-{:04}.png", trace.index),
                    width: trace.dithered.width,
                    height: trace.dithered.height,
                    record,
                }
            })
            .collect();
        Self {
            count: traces.len(),
            printer: input.printer,
            media: input.media,
            template: input.template,
            data: input.data,
            labels,
        }
    }
}

impl HtmlPreviewPrinter {
    pub fn from_run(
        printer_key: Option<&str>,
        printer_name: Option<&str>,
        printer_brand: Option<&str>,
        protocol: Protocol,
        dpi: f64,
        max_width_mm: Option<f64>,
        transport: Option<String>,
    ) -> Self {
        Self {
            key: printer_key.map(str::to_string),
            name: printer_name.map(str::to_string),
            brand: printer_brand.map(str::to_string),
            protocol: protocol_cli_name(protocol).to_string(),
            dpi,
            max_width_mm,
            transport,
        }
    }
}

impl HtmlPreviewMedia {
    pub fn from_resolved(media: &Media, sku: Option<&str>, catalog_name: Option<&str>) -> Self {
        let (length_mm, continuous) = match media.length {
            MediaLength::Fixed(mm) => (Some(mm), false),
            MediaLength::Continuous => (None, true),
        };
        Self {
            sku: sku.map(str::to_string),
            name: catalog_name.map(str::to_string),
            width_mm: media.width_mm,
            length_mm,
            continuous,
            dpi: media.dpi.0,
            material: Some(material_label(media.material)),
            color: Some(color_label(media.color)),
        }
    }
}

fn material_label(material: Material) -> String {
    match material {
        Material::Paper => "paper",
        Material::Polypropylene => "polypropylene",
        Material::Vinyl => "vinyl",
        Material::Nylon => "nylon",
        Material::Other => "other",
    }
    .to_string()
}

fn color_label(color: MediaColor) -> String {
    match color {
        MediaColor::White => "white",
        MediaColor::Transparent => "transparent",
        MediaColor::Yellow => "yellow",
        MediaColor::Red => "red",
        MediaColor::Other => "other",
    }
    .to_string()
}

//! Build [`HtmlPreviewInput`] from a print run.

use anyhow::Result;
use lbl_catalog::{Catalog, PrinterEntry};
use lbl_core::media::Media;
use lbl_core::printer::Protocol;
use lbl_template::resolve_batch;
use serde_json::{json, Value};

use super::context::{
    HtmlPreviewInput, HtmlPreviewMedia, HtmlPreviewPrinter, HtmlPreviewTemplate,
};
use crate::pipeline::{Source, TemplateFormat};

/// Source flags needed to recover template paths for the preview UI.
pub struct PreviewSourceArgs<'a> {
    pub template_path: Option<&'a str>,
}

pub fn input_from_run(
    source: &Source,
    source_args: PreviewSourceArgs<'_>,
    catalog: &Catalog,
    printer_entry: Option<&PrinterEntry>,
    printer_key: Option<&str>,
    protocol: Protocol,
    dpi: f64,
    media: &Media,
    media_sku: Option<&str>,
    network: &Option<String>,
    usb: &Option<String>,
    serial: &Option<String>,
    bluetooth: &Option<String>,
) -> Result<HtmlPreviewInput> {
    let catalog_media = media_sku.and_then(|sku| catalog.lookup(sku));
    let preview_media = HtmlPreviewMedia::from_resolved(
        media,
        media_sku,
        catalog_media.map(|entry| entry.name.as_str()),
    );

    let preview_printer = HtmlPreviewPrinter::from_run(
        printer_key.or_else(|| printer_entry.map(|p| p.canonical_key())),
        printer_entry.map(|p| p.name.as_str()),
        printer_entry.map(|p| p.brand.as_str()),
        protocol,
        dpi,
        printer_entry.map(|p| p.max_width_mm),
        transport_summary(network, usb, serial, bluetooth),
    );

    let (template, data, records) = preview_template_and_data(source, source_args)?;

    Ok(HtmlPreviewInput {
        printer: preview_printer,
        media: preview_media,
        template,
        data,
        records,
    })
}

fn preview_template_and_data(
    source: &Source,
    source_args: PreviewSourceArgs<'_>,
) -> Result<(HtmlPreviewTemplate, Value, Vec<Value>)> {
    Ok(match source {
        Source::Template {
            template,
            data,
            each,
            format,
        } => {
            let batch = resolve_batch(template, data.clone(), each.as_deref())?;
            let kind = match format {
                TemplateFormat::Text => "template (text)",
                TemplateFormat::Markdown => "template (markdown)",
                TemplateFormat::Html => "template",
            };
            (
                HtmlPreviewTemplate {
                    kind: kind.into(),
                    path: source_args.template_path.map(str::to_string),
                    each: each.clone(),
                    body: batch.template_body,
                },
                batch.data_root,
                batch.records,
            )
        }
        Source::Text { text, raw } => {
            let body = if *raw {
                text.clone()
            } else {
                lbl_text::Document::parse(text, false).to_authoring_document()
            };
            (
                HtmlPreviewTemplate {
                    kind: if *raw { "text (raw)".into() } else { "text".into() },
                    path: None,
                    each: None,
                    body,
                },
                json!({ "text": text, "raw": raw }),
                vec![json!({ "text": text, "raw": raw })],
            )
        }
        Source::Markdown(markdown) => (
            HtmlPreviewTemplate {
                kind: "markdown".into(),
                path: None,
                each: None,
                body: markdown.clone(),
            },
            json!({ "markdown": markdown }),
            vec![json!({ "markdown": markdown })],
        ),
        Source::Html(html) => (
            HtmlPreviewTemplate {
                kind: "html".into(),
                path: None,
                each: None,
                body: html.clone(),
            },
            json!({ "html": html }),
            vec![json!({ "html": html })],
        ),
    })
}

fn transport_summary(
    network: &Option<String>,
    usb: &Option<String>,
    serial: &Option<String>,
    bluetooth: &Option<String>,
) -> Option<String> {
    if let Some(host) = network {
        return Some(format!("network {host}"));
    }
    if let Some(vidpid) = usb {
        return Some(format!("usb {vidpid}"));
    }
    if let Some(path) = serial {
        return Some(format!("serial {path}"));
    }
    bluetooth
        .as_ref()
        .map(|name| format!("bluetooth {name}"))
}

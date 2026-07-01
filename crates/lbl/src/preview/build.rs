//! Build [`HtmlPreviewInput`] from a print run.

use anyhow::Result;
use lbl_catalog::{Catalog, PrinterEntry};
use lbl_core::media::Media;
use lbl_core::printer::Protocol;
use lbl_template::resolve_batch;
use serde_json::{json, Value};

use super::context::{HtmlPreviewInput, HtmlPreviewMedia, HtmlPreviewPrinter, HtmlPreviewTemplate};
use crate::pipeline::{Source, TemplateFormat};

/// Source flags needed to recover template paths for the preview UI.
pub struct PreviewSourceArgs<'a> {
    pub template_path: Option<&'a str>,
}

/// Transport targets selected for the print run.
pub struct PreviewTransport<'a> {
    pub network: &'a Option<String>,
    pub usb: &'a Option<String>,
    pub serial: &'a Option<String>,
    pub bluetooth: &'a Option<String>,
}

/// Resolved printer and media context for a preview run.
pub struct PreviewRunContext<'a> {
    pub catalog: &'a Catalog,
    pub printer_entry: Option<&'a PrinterEntry>,
    pub printer_key: Option<&'a str>,
    pub protocol: Protocol,
    pub dpi: f64,
    pub media: &'a Media,
    pub media_sku: Option<&'a str>,
    pub transport: PreviewTransport<'a>,
}

pub fn input_from_run(
    source: &Source,
    source_args: PreviewSourceArgs<'_>,
    ctx: PreviewRunContext<'_>,
) -> Result<HtmlPreviewInput> {
    let catalog_media = ctx.media_sku.and_then(|sku| ctx.catalog.lookup(sku));
    let preview_media = HtmlPreviewMedia::from_resolved(
        ctx.media,
        ctx.media_sku,
        catalog_media.map(|entry| entry.name.as_str()),
    );

    let preview_printer = HtmlPreviewPrinter::from_run(
        ctx.printer_key
            .or_else(|| ctx.printer_entry.map(|p| p.canonical_key())),
        ctx.printer_entry.map(|p| p.name.as_str()),
        ctx.printer_entry.map(|p| p.brand.as_str()),
        ctx.protocol,
        ctx.dpi,
        ctx.printer_entry.map(|p| p.max_width_mm),
        transport_summary(&ctx.transport),
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
                    kind: if *raw {
                        "text (raw)".into()
                    } else {
                        "text".into()
                    },
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

fn transport_summary(transport: &PreviewTransport<'_>) -> Option<String> {
    if let Some(host) = transport.network {
        return Some(format!("network {host}"));
    }
    if let Some(vidpid) = transport.usb {
        return Some(format!("usb {vidpid}"));
    }
    if let Some(path) = transport.serial {
        return Some(format!("serial {path}"));
    }
    transport
        .bluetooth
        .as_ref()
        .map(|name| format!("bluetooth {name}"))
}

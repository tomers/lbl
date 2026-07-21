//! The templating engine: render data through a template into N labels.

use minijinja::Environment;
use serde_json::{Map, Value};

use crate::frontmatter::{self, SplitSource};
use crate::resources::{inline_images, ResourceResolver};
use crate::selection::select_batch_indices;
use crate::{data, selection::BatchSelection, TemplateError};

/// A single rendered label.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedLabel {
    /// Zero-based index within the batch.
    pub index: usize,
    /// The rendered authoring HTML.
    pub html: String,
}

/// Resolved template source, data root, and per-label records for a batch.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchSource {
    /// Full template source (including frontmatter, if any).
    pub source: String,
    /// Template body after frontmatter is stripped.
    pub template_body: String,
    /// Parsed data root (external data wins over frontmatter).
    pub data_root: Value,
    /// One value per label in the batch.
    pub records: Vec<Value>,
}

/// Resolve the data root and batch records without rendering.
pub fn resolve_batch(
    source: &str,
    external_data: Option<Value>,
    each: Option<&str>,
) -> Result<BatchSource, TemplateError> {
    let SplitSource {
        data_text,
        data_format,
        template,
    } = frontmatter::split(source);

    let frontmatter_data = match data_text {
        Some(text) => Some(match data_format {
            Some(fmt) => data::parse(&text, fmt)?,
            None => data::parse_auto(&text)?,
        }),
        None => None,
    };

    let data_root = external_data.or(frontmatter_data).unwrap_or(Value::Null);
    let records = select_records(&data_root, each)?;
    Ok(BatchSource {
        source: source.to_string(),
        template_body: template,
        data_root,
        records,
    })
}

/// Options controlling a render.
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    /// JSON-pointer to an array within the data to iterate over for batch
    /// rendering (e.g. `/items`). If unset, a top-level array is used as the
    /// batch, otherwise a single label is produced.
    pub each: Option<String>,
    /// Which batch records to render.
    pub selection: BatchSelection,
}

/// The templating engine.
#[derive(Debug, Default)]
pub struct Engine;

impl Engine {
    /// Create a new engine.
    pub fn new() -> Self {
        Self
    }

    /// Render `source` (which may contain frontmatter) with optional external
    /// `data`, producing one or more labels. External data takes precedence
    /// over frontmatter data.
    pub fn render(
        &self,
        source: &str,
        external_data: Option<Value>,
        opts: &RenderOptions,
    ) -> Result<Vec<RenderedLabel>, TemplateError> {
        let batch = resolve_batch(source, external_data, opts.each.as_deref())?;
        let template = batch.template_body;
        let root = batch.data_root;
        let records = batch.records;
        let selected = select_batch_indices(&records, &opts.selection)?;

        let mut env = Environment::new();
        env.add_template("label", &template)
            .map_err(|e| TemplateError::Render(e.to_string()))?;
        let tmpl = env
            .get_template("label")
            .map_err(|e| TemplateError::Render(e.to_string()))?;

        let total = records.len();
        let mut out = Vec::with_capacity(selected.len());
        for index in selected {
            let record = records[index].clone();
            let ctx = build_context(record, index, total, &root);
            let html = tmpl
                .render(minijinja::Value::from_serialize(&ctx))
                .map_err(|e| TemplateError::Render(e.to_string()))?;
            out.push(RenderedLabel { index, html });
        }
        Ok(out)
    }

    /// Render, then inline image resources into each label so the documents are
    /// self-contained for the renderer.
    pub fn render_with_resources<R: ResourceResolver>(
        &self,
        source: &str,
        external_data: Option<Value>,
        opts: &RenderOptions,
        resolver: &R,
    ) -> Result<Vec<RenderedLabel>, TemplateError> {
        let mut labels = self.render(source, external_data, opts)?;
        for label in &mut labels {
            label.html = inline_images(&label.html, resolver)?;
        }
        Ok(labels)
    }
}

/// Determine the batch records from the data root.
fn select_records(root: &Value, each: Option<&str>) -> Result<Vec<Value>, TemplateError> {
    if let Some(pointer) = each {
        let target = root
            .pointer(pointer)
            .ok_or_else(|| TemplateError::Data(format!("pointer '{pointer}' not found in data")))?;
        return match target {
            Value::Array(items) => Ok(items.clone()),
            other => Err(TemplateError::Data(format!(
                "pointer '{pointer}' is not an array (found {})",
                kind_of(other)
            ))),
        };
    }
    match root {
        Value::Array(items) => Ok(items.clone()),
        Value::Null => Ok(vec![Value::Object(Map::new())]),
        other => Ok(vec![other.clone()]),
    }
}

/// Build the per-record template context. `it`, `index`, `count`, and `data`
/// are always bound; when the record is an object, its fields are exposed at
/// the top level and take precedence over those bindings, so user data is
/// never silently clobbered by a same-named convenience binding.
fn build_context(record: Value, index: usize, total: usize, root: &Value) -> Value {
    let mut ctx = Map::new();
    ctx.insert("it".to_string(), record.clone());
    ctx.insert("index".to_string(), Value::from(index));
    ctx.insert("count".to_string(), Value::from(total));
    ctx.insert("data".to_string(), root.clone());
    if let Value::Object(fields) = record {
        for (k, v) in fields {
            ctx.insert(k, v);
        }
    }
    Value::Object(ctx)
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn single_object_renders_once() {
        let labels = Engine::new()
            .render(
                "<div>{{ name }}</div>",
                Some(json!({"name": "Alice"})),
                &RenderOptions::default(),
            )
            .unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].html, "<div>Alice</div>");
    }

    #[test]
    fn array_expands_to_batch() {
        let labels = Engine::new()
            .render(
                "<div>{{ index }}:{{ name }}</div>",
                Some(json!([{"name":"A"},{"name":"B"}])),
                &RenderOptions::default(),
            )
            .unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].html, "<div>0:A</div>");
        assert_eq!(labels[1].html, "<div>1:B</div>");
    }

    #[test]
    fn record_fields_shadow_convenience_bindings() {
        let labels = Engine::new()
            .render(
                "<div>{{ index }}/{{ count }}</div>",
                Some(json!([
                    {"index": "7", "count": "9"},
                    {"index": "8", "count": "9"}
                ])),
                &RenderOptions::default(),
            )
            .unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].html, "<div>7/9</div>");
        assert_eq!(labels[1].html, "<div>8/9</div>");
    }

    #[test]
    fn each_pointer_selects_array() {
        let labels = Engine::new()
            .render(
                "<div>{{ name }} of {{ count }}</div>",
                Some(json!({"items":[{"name":"X"},{"name":"Y"}]})),
                &RenderOptions {
                    each: Some("/items".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[1].html, "<div>Y of 2</div>");
    }

    #[test]
    fn frontmatter_provides_data() {
        let src = "---toml\nname = \"Frodo\"\n---\n<div>{{ name }}</div>";
        let labels = Engine::new()
            .render(src, None, &RenderOptions::default())
            .unwrap();
        assert_eq!(labels[0].html, "<div>Frodo</div>");
    }

    #[test]
    fn render_respects_batch_selection() {
        let labels = Engine::new()
            .render(
                "<div>{{ name }}</div>",
                Some(json!([
                    {"name": "Tony Soprano"},
                    {"name": "Carmela Soprano"}
                ])),
                &RenderOptions {
                    each: None,
                    selection: crate::selection::BatchSelection {
                        filter: Some("car".into()),
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].index, 1);
        assert_eq!(labels[0].html, "<div>Carmela Soprano</div>");
    }

    #[test]
    fn frontmatter_after_html_comment_renders_batch() {
        let src = "<!-- markdownlint-disable-file -->\n---json\n[{\"name\":\"A\"},{\"name\":\"B\"}]\n---\n{{ name }}";
        let labels = Engine::new()
            .render(src, None, &RenderOptions::default())
            .unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].html, "A");
        assert_eq!(labels[1].html, "B");
    }

    #[test]
    fn resolve_batch_without_rendering() {
        let batch = resolve_batch(
            "---json\n[{\"name\":\"A\"},{\"name\":\"B\"}]\n---\n<div>{{ name }}</div>",
            None,
            None,
        )
        .unwrap();
        assert_eq!(batch.records.len(), 2);
        assert_eq!(batch.template_body, "<div>{{ name }}</div>");
    }
}

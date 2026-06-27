//! The templating engine: render data through a template into N labels.

use minijinja::Environment;
use serde_json::{Map, Value};

use crate::frontmatter::{self, SplitSource};
use crate::resources::{inline_images, ResourceResolver};
use crate::{data, TemplateError};

/// A single rendered label.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedLabel {
    /// Zero-based index within the batch.
    pub index: usize,
    /// The rendered authoring HTML.
    pub html: String,
}

/// Options controlling a render.
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    /// JSON-pointer to an array within the data to iterate over for batch
    /// rendering (e.g. `/items`). If unset, a top-level array is used as the
    /// batch, otherwise a single label is produced.
    pub each: Option<String>,
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

        let root = external_data.or(frontmatter_data).unwrap_or(Value::Null);
        let records = select_records(&root, opts.each.as_deref())?;
        let total = records.len();

        let mut env = Environment::new();
        env.add_template("label", &template)
            .map_err(|e| TemplateError::Render(e.to_string()))?;
        let tmpl = env
            .get_template("label")
            .map_err(|e| TemplateError::Render(e.to_string()))?;

        let mut out = Vec::with_capacity(total);
        for (index, record) in records.into_iter().enumerate() {
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

/// Build the per-record template context. When the record is an object, its
/// fields are exposed at the top level; `it`, `index`, `count`, and `data` are
/// always available.
fn build_context(record: Value, index: usize, total: usize, root: &Value) -> Value {
    let mut ctx = Map::new();
    if let Value::Object(fields) = &record {
        for (k, v) in fields {
            ctx.insert(k.clone(), v.clone());
        }
    }
    ctx.insert("it".to_string(), record);
    ctx.insert("index".to_string(), Value::from(index));
    ctx.insert("count".to_string(), Value::from(total));
    ctx.insert("data".to_string(), root.clone());
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
    fn each_pointer_selects_array() {
        let labels = Engine::new()
            .render(
                "<div>{{ name }} of {{ count }}</div>",
                Some(json!({"items":[{"name":"X"},{"name":"Y"}]})),
                &RenderOptions {
                    each: Some("/items".into()),
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
}

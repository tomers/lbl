//! Pluggable data sources feeding the batch renderer.
//!
//! The engine renders a JSON *data root* (see [`crate::Engine::render`] and
//! [`crate::resolve_batch`]); a [`DataSource`] is anything that can *produce*
//! that root. This open crate ships inline-text and local-file sources. Richer
//! connectors — spreadsheets, databases, HTTP APIs, contact books — are provided
//! by downstream crates that implement this trait. The engine never learns to
//! read those formats itself; it only consumes the JSON a source yields, which
//! keeps the data-integration surface pluggable and out of the core.

use serde_json::Value;

/// Errors produced while loading data from a [`DataSource`].
#[derive(Debug, thiserror::Error)]
pub enum DataError {
    /// The source data could not be parsed into JSON.
    #[error("data parse error: {0}")]
    Parse(String),

    /// An I/O error occurred while reading the source.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// The requested source or capability is not implemented by this build.
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// Any other source-specific failure.
    #[error("{0}")]
    Other(String),
}

/// Parameters for a data load. Connectors interpret these as needed (a sheet
/// name, table, SQL query, endpoint, etc.).
#[derive(Debug, Clone, Default)]
pub struct DataRequest {
    /// Optional selector into the source (sheet/table/query/endpoint …).
    pub selector: Option<String>,
    /// Connector-specific options as free-form JSON.
    pub options: Value,
}

/// A single described field/column, for building field-mapping UIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataField {
    /// Field/column name as it appears in the source.
    pub name: String,
    /// An example value, if the source can cheaply provide one.
    pub example: Option<String>,
}

/// The shape of the records a source produces, used to drive mapping UIs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DataSchema {
    /// The fields/columns available on each record.
    pub fields: Vec<DataField>,
}

/// A source of label data.
///
/// Implementations return a JSON data root suitable for
/// [`crate::Engine::render`] — typically an array of row objects, but any value
/// the engine accepts is valid.
pub trait DataSource {
    /// Load the data root for `req`.
    fn load(&self, req: &DataRequest) -> Result<Value, DataError>;

    /// Optionally describe the columns/fields for a mapping UI. Sources that
    /// cannot introspect cheaply may return `None` (the default).
    fn schema(&self) -> Option<DataSchema> {
        None
    }
}

/// Inline text in a known (or auto-detected) serialization format.
#[derive(Debug, Clone)]
pub struct InlineData {
    /// The raw text to parse.
    pub text: String,
    /// The format, or `None` to auto-detect (JSON, then YAML, then TOML).
    pub format: Option<crate::DataFormat>,
}

impl DataSource for InlineData {
    fn load(&self, _req: &DataRequest) -> Result<Value, DataError> {
        match self.format {
            Some(fmt) => crate::data::parse(&self.text, fmt),
            None => crate::data::parse_auto(&self.text),
        }
        .map_err(|e| DataError::Parse(e.to_string()))
    }
}

/// A local JSON / TOML / YAML file, with the format inferred from its extension
/// (falling back to auto-detection).
#[derive(Debug, Clone)]
pub struct LocalFileSource {
    /// Path to the data file.
    pub path: std::path::PathBuf,
}

impl DataSource for LocalFileSource {
    fn load(&self, req: &DataRequest) -> Result<Value, DataError> {
        let text = std::fs::read_to_string(&self.path)?;
        let format = self
            .path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(crate::DataFormat::from_extension);
        InlineData { text, format }.load(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inline_auto_detects_json() {
        let src = InlineData {
            text: r#"[{"name":"A"}]"#.to_string(),
            format: None,
        };
        let value = src.load(&DataRequest::default()).unwrap();
        assert_eq!(value, json!([{"name": "A"}]));
    }

    #[test]
    fn inline_respects_explicit_format() {
        let src = InlineData {
            text: "name = \"A\"".to_string(),
            format: Some(crate::DataFormat::Toml),
        };
        assert_eq!(src.load(&DataRequest::default()).unwrap()["name"], "A");
    }
}

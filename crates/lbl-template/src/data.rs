//! Loading and parsing of data in JSON, TOML, or YAML.

use crate::TemplateError;

/// Supported data serialization formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataFormat {
    /// JSON.
    Json,
    /// TOML.
    Toml,
    /// YAML.
    Yaml,
}

impl DataFormat {
    /// Guess the format from a file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "json" => Some(Self::Json),
            "toml" => Some(Self::Toml),
            "yaml" | "yml" => Some(Self::Yaml),
            _ => None,
        }
    }
}

/// Parse `text` in the explicitly given format into a JSON value (the common
/// context representation).
pub fn parse(text: &str, format: DataFormat) -> Result<serde_json::Value, TemplateError> {
    let value = match format {
        DataFormat::Json => serde_json::from_str(text).map_err(de)?,
        DataFormat::Toml => toml::from_str(text).map_err(|e| TemplateError::Data(e.to_string()))?,
        DataFormat::Yaml => {
            serde_yaml::from_str(text).map_err(|e| TemplateError::Data(e.to_string()))?
        }
    };
    Ok(value)
}

/// Parse `text`, trying to auto-detect the format (JSON, then YAML, then TOML).
pub fn parse_auto(text: &str) -> Result<serde_json::Value, TemplateError> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
        return Ok(v);
    }
    if let Ok(v) = serde_yaml::from_str::<serde_json::Value>(text) {
        return Ok(v);
    }
    if let Ok(v) = toml::from_str::<serde_json::Value>(text) {
        return Ok(v);
    }
    Err(TemplateError::Data(
        "could not parse data as JSON, YAML, or TOML".to_string(),
    ))
}

fn de(e: serde_json::Error) -> TemplateError {
    TemplateError::Data(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_format() {
        assert!(parse(r#"{"a":1}"#, DataFormat::Json).is_ok());
        assert!(parse("a = 1", DataFormat::Toml).is_ok());
        assert!(parse("a: 1", DataFormat::Yaml).is_ok());
    }

    #[test]
    fn auto_detects() {
        assert_eq!(parse_auto(r#"{"a":1}"#).unwrap()["a"], serde_json::json!(1));
        assert_eq!(parse_auto("a: 2").unwrap()["a"], serde_json::json!(2));
    }
}

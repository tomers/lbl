//! Single-file (JSX-like) sources: optional data frontmatter + template body.
//!
//! A source may co-locate its data and template, e.g.:
//!
//! ```text
//! ---toml
//! [[items]]
//! name = "Alice"
//! ---
//! <div class="lbl-label">{{ name }}</div>
//! ```
//!
//! The opening fence may carry a format tag (`---toml`, `---yaml`, `---json`);
//! without one the format is auto-detected.

use crate::data::DataFormat;

/// The result of splitting a single-file source.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitSource {
    /// The raw frontmatter data text, if present.
    pub data_text: Option<String>,
    /// The explicit format tag on the frontmatter fence, if any.
    pub data_format: Option<DataFormat>,
    /// The template body.
    pub template: String,
}

/// Split a source into optional frontmatter and the template body.
pub fn split(source: &str) -> SplitSource {
    let trimmed_start = source.trim_start_matches(['\u{feff}']);
    let mut lines = trimmed_start.lines();

    let first = match lines.next() {
        Some(l) => l,
        None => {
            return SplitSource {
                data_text: None,
                data_format: None,
                template: String::new(),
            }
        }
    };

    if let Some(tag) = first.strip_prefix("---") {
        let format = match tag.trim() {
            "" => None,
            "toml" => Some(DataFormat::Toml),
            "yaml" | "yml" => Some(DataFormat::Yaml),
            "json" => Some(DataFormat::Json),
            _ => None,
        };
        let mut data_lines = Vec::new();
        let mut body_lines = Vec::new();
        let mut in_body = false;
        for line in lines {
            if !in_body && line.trim_end() == "---" {
                in_body = true;
                continue;
            }
            if in_body {
                body_lines.push(line);
            } else {
                data_lines.push(line);
            }
        }
        if in_body {
            return SplitSource {
                data_text: Some(data_lines.join("\n")),
                data_format: format,
                template: body_lines.join("\n"),
            };
        }
    }

    SplitSource {
        data_text: None,
        data_format: None,
        template: source.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_frontmatter() {
        let s = split("<div>{{ x }}</div>");
        assert!(s.data_text.is_none());
        assert_eq!(s.template, "<div>{{ x }}</div>");
    }

    #[test]
    fn toml_frontmatter() {
        let src = "---toml\nname = \"Alice\"\n---\n<div>{{ name }}</div>";
        let s = split(src);
        assert_eq!(s.data_format, Some(DataFormat::Toml));
        assert_eq!(s.data_text.as_deref(), Some("name = \"Alice\""));
        assert_eq!(s.template, "<div>{{ name }}</div>");
    }

    #[test]
    fn untagged_frontmatter_autodetect() {
        let src = "---\n{\"name\":\"Bob\"}\n---\nbody";
        let s = split(src);
        assert_eq!(s.data_format, None);
        assert_eq!(s.template, "body");
    }
}

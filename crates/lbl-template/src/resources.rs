//! Fetching and inlining of external resources referenced by labels.
//!
//! When printing (especially batches such as ID cards), images are referenced
//! by local path or remote URL. The templating engine is responsible for
//! fetching these and integrating them into the final HTML so the renderer
//! receives a self-contained document.

use std::collections::HashMap;

use base64::Engine;
use regex::Regex;

use crate::TemplateError;

/// Resolves a resource reference (path or URL) to its raw bytes and MIME type.
pub trait ResourceResolver {
    /// Fetch the resource at `reference`, returning `(bytes, mime_type)`.
    fn fetch(&self, reference: &str) -> Result<(Vec<u8>, String), TemplateError>;
}

/// The default resolver: local filesystem for paths, HTTP(S) for URLs.
#[derive(Debug, Default, Clone)]
pub struct DefaultResolver;

impl ResourceResolver for DefaultResolver {
    fn fetch(&self, reference: &str) -> Result<(Vec<u8>, String), TemplateError> {
        if reference.starts_with("http://") || reference.starts_with("https://") {
            let resp = reqwest::blocking::get(reference)
                .and_then(|r| r.error_for_status())
                .map_err(|e| TemplateError::Resource(format!("{reference}: {e}")))?;
            let mime = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
                .unwrap_or_else(|| guess_mime(reference));
            let bytes = resp
                .bytes()
                .map_err(|e| TemplateError::Resource(format!("{reference}: {e}")))?
                .to_vec();
            Ok((bytes, mime))
        } else {
            let bytes = std::fs::read(reference)
                .map_err(|e| TemplateError::Resource(format!("{reference}: {e}")))?;
            Ok((bytes, guess_mime(reference)))
        }
    }
}

fn guess_mime(reference: &str) -> String {
    mime_guess::from_path(reference)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

/// In-memory resolver for tests: maps references to `(bytes, mime)`.
#[derive(Debug, Default, Clone)]
pub struct MapResolver {
    /// Reference -> (bytes, mime).
    pub map: HashMap<String, (Vec<u8>, String)>,
}

impl ResourceResolver for MapResolver {
    fn fetch(&self, reference: &str) -> Result<(Vec<u8>, String), TemplateError> {
        self.map
            .get(reference)
            .cloned()
            .ok_or_else(|| TemplateError::Resource(format!("not found: {reference}")))
    }
}

fn img_src_regex() -> Regex {
    Regex::new(r#"(<img\b[^>]*\bsrc=")([^"]+)(")"#).expect("valid regex")
}

/// Replace `<img src="...">` references with self-contained `data:` URIs by
/// fetching each resource through `resolver`. Existing `data:` URIs are left
/// untouched. Failures for individual resources are returned as an error.
pub fn inline_images<R: ResourceResolver>(
    html: &str,
    resolver: &R,
) -> Result<String, TemplateError> {
    let re = img_src_regex();
    let mut error: Option<TemplateError> = None;

    let result = re.replace_all(html, |caps: &regex::Captures| {
        let prefix = &caps[1];
        let src = &caps[2];
        let suffix = &caps[3];
        if src.starts_with("data:") {
            return format!("{prefix}{src}{suffix}");
        }
        match resolver.fetch(src) {
            Ok((bytes, mime)) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                format!("{prefix}data:{mime};base64,{b64}{suffix}")
            }
            Err(e) => {
                if error.is_none() {
                    error = Some(e);
                }
                format!("{prefix}{src}{suffix}")
            }
        }
    });

    if let Some(e) = error {
        return Err(e);
    }
    Ok(result.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inlines_from_map_resolver() {
        let mut map = HashMap::new();
        map.insert(
            "photo.png".to_string(),
            (vec![1, 2, 3], "image/png".to_string()),
        );
        let resolver = MapResolver { map };
        let html = r#"<div><img class="x" src="photo.png" /></div>"#;
        let out = inline_images(html, &resolver).unwrap();
        assert!(out.contains("src=\"data:image/png;base64,AQID\""));
    }

    #[test]
    fn leaves_data_uris_untouched() {
        let resolver = MapResolver::default();
        let html = r#"<img src="data:image/png;base64,AAAA">"#;
        let out = inline_images(html, &resolver).unwrap();
        assert_eq!(out, html);
    }
}

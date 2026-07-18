//! Date/time stamp authoring elements and resolution.
//!
//! Mustache `{{date|time|datetime:FORMAT}}` and HTML `<stamp kind="…" format="…">`
//! become wall-clock text once per job via [`resolve_stamps_at`].

use chrono::{DateTime, Local};

/// Scope of a stamp field (filters Studio presets; format string drives output).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampKind {
    Date,
    Time,
    DateTime,
}

impl StampKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "date" => Some(Self::Date),
            "time" => Some(Self::Time),
            "datetime" => Some(Self::DateTime),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Date => "date",
            Self::Time => "time",
            Self::DateTime => "datetime",
        }
    }
}

/// Format `now` with a chrono strftime pattern.
pub fn format_stamp(now: DateTime<Local>, format: &str) -> String {
    now.format(format).to_string()
}

/// Replace every `<stamp …>` in authoring HTML with escaped formatted local time.
///
/// Call once per preview/print job with a single `now` so batch labels share
/// the same instant. Unresolved or malformed stamps return an error (fail fast).
pub fn resolve_stamps_at(html: &str, now: DateTime<Local>) -> Result<String, String> {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = find_stamp_start(rest) {
        out.push_str(&rest[..start]);
        let after_lt = &rest[start..];
        let (attrs, after_open) = parse_open_tag(after_lt)?;
        let kind = attrs
            .kind
            .as_deref()
            .and_then(StampKind::parse)
            .ok_or_else(|| "stamp element missing or invalid kind attribute".to_string())?;
        let format = attrs
            .format
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "stamp element missing format attribute".to_string())?;
        // Kind is retained for authoring round-trip; output is format-driven.
        let _ = kind;
        let formatted = format_stamp(now, format);
        out.push_str(&escape_text(&formatted));
        rest = after_open;
    }
    out.push_str(rest);
    Ok(out)
}

/// Resolve stamps using the host local clock.
pub fn resolve_stamps(html: &str) -> Result<String, String> {
    resolve_stamps_at(html, Local::now())
}

struct StampAttrs {
    kind: Option<String>,
    format: Option<String>,
}

fn find_stamp_start(html: &str) -> Option<usize> {
    let mut search = html;
    let mut offset = 0;
    while let Some(rel) = search.find("<stamp") {
        let abs = offset + rel;
        let after = &html[abs + "<stamp".len()..];
        let next = after.chars().next();
        if next.is_none_or(|c| c.is_whitespace() || c == '/' || c == '>') {
            return Some(abs);
        }
        offset = abs + 1;
        search = &html[offset..];
    }
    None
}

/// Parse `<stamp …>` or `<stamp …/>` and optional `</stamp>`; return attrs and
/// the slice after the full element.
fn parse_open_tag(after_lt: &str) -> Result<(StampAttrs, &str), String> {
    debug_assert!(after_lt.starts_with("<stamp"));
    let after_name = &after_lt["<stamp".len()..];
    let close_rel = find_tag_close(after_name).ok_or_else(|| "unclosed stamp tag".to_string())?;
    let inside = &after_name[..close_rel];
    let self_closing = inside.trim_end().ends_with('/');
    let attr_src = if self_closing {
        inside.trim_end().trim_end_matches('/').trim_end()
    } else {
        inside
    };
    let attrs = parse_attrs(attr_src)?;
    let after_open = &after_name[close_rel + 1..];
    if self_closing {
        return Ok((attrs, after_open));
    }
    let end = after_open
        .find("</stamp>")
        .ok_or_else(|| "stamp element missing </stamp>".to_string())?;
    Ok((attrs, &after_open[end + "</stamp>".len()..]))
}

/// Index of `>` that closes the open tag, ignoring `>` inside double-quoted attrs.
fn find_tag_close(after_name: &str) -> Option<usize> {
    let bytes = after_name.as_bytes();
    let mut i = 0;
    let mut in_quote = false;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_quote = !in_quote,
            b'>' if !in_quote => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

fn parse_attrs(src: &str) -> Result<StampAttrs, String> {
    let mut kind = None;
    let mut format = None;
    let mut rest = src.trim();
    while !rest.is_empty() {
        let eq = rest
            .find('=')
            .ok_or_else(|| format!("invalid stamp attribute near '{rest}'"))?;
        let name = rest[..eq].trim();
        let after_eq = rest[eq + 1..].trim_start();
        let (value, next) = parse_attr_value(after_eq)?;
        match name {
            "kind" => kind = Some(value),
            "format" => format = Some(value),
            other => {
                return Err(format!("unknown stamp attribute '{other}'"));
            }
        }
        rest = next.trim_start();
    }
    Ok(StampAttrs { kind, format })
}

fn parse_attr_value(src: &str) -> Result<(String, &str), String> {
    let bytes = src.as_bytes();
    if bytes.first() != Some(&b'"') {
        return Err("stamp attributes must use double-quoted values".into());
    }
    let mut out = String::new();
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let rest = &src[i + 1..];
                return Ok((out, rest));
            }
            b'&' => {
                if src[i..].starts_with("&quot;") {
                    out.push('"');
                    i += 6;
                } else if src[i..].starts_with("&amp;") {
                    out.push('&');
                    i += 5;
                } else if src[i..].starts_with("&lt;") {
                    out.push('<');
                    i += 4;
                } else if src[i..].starts_with("&gt;") {
                    out.push('>');
                    i += 4;
                } else {
                    out.push('&');
                    i += 1;
                }
            }
            c => {
                out.push(c as char);
                i += 1;
            }
        }
    }
    Err("unclosed stamp attribute value".into())
}

fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_now() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 7, 18, 14, 30, 0)
            .single()
            .expect("valid local datetime")
    }

    #[test]
    fn resolves_date_stamp() {
        let html = r#"<div class="lbl-text"><stamp kind="date" format="%Y-%m-%d"></stamp></div>"#;
        let out = resolve_stamps_at(html, fixed_now()).unwrap();
        assert_eq!(out, r#"<div class="lbl-text">2026-07-18</div>"#);
    }

    #[test]
    fn resolves_self_closing_and_time() {
        let html = r#"at <stamp kind="time" format="%H:%M" />"#;
        let out = resolve_stamps_at(html, fixed_now()).unwrap();
        assert_eq!(out, "at 14:30");
    }

    #[test]
    fn resolves_datetime_with_colon_in_format() {
        let html = r#"<stamp kind="datetime" format="%Y-%m-%d %H:%M"></stamp>"#;
        let out = resolve_stamps_at(html, fixed_now()).unwrap();
        assert_eq!(out, "2026-07-18 14:30");
    }

    #[test]
    fn rejects_missing_format() {
        let err = resolve_stamps_at(r#"<stamp kind="date"></stamp>"#, fixed_now()).unwrap_err();
        assert!(err.contains("format"), "{err}");
    }

    #[test]
    fn rejects_bad_kind() {
        let err = resolve_stamps_at(r#"<stamp kind="weekday" format="%Y"></stamp>"#, fixed_now())
            .unwrap_err();
        assert!(err.contains("kind"), "{err}");
    }

    #[test]
    fn escapes_formatted_output() {
        // Pathological format that embeds HTML-ish chars via literal text.
        let html = r#"<stamp kind="date" format="A<B>&C"></stamp>"#;
        let out = resolve_stamps_at(html, fixed_now()).unwrap();
        assert_eq!(out, "A&lt;B&gt;&amp;C");
    }
}

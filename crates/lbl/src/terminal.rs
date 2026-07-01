//! Terminal-facing helpers for the `lbl print` flow.
//!
//! Three user-facing features share this module so they stay consistent:
//!
//! * `--protocol console` — dump the dithered raster to stdout as text.
//! * `--confirm` — preview each label and ask before printing to a device/file.
//! * `--debug` — print the effective configuration (with provenance), then a
//!   per-stage dump (syntax-highlighted HTML, the dithered raster as art, an
//!   encoded-byte preview) to stderr.
//!
//! Raster art is produced by [`lbl_driver_console::render_terminal`] so the
//! preview matches `--protocol console` exactly. Color (ANSI) is used only when
//! the destination stream is a TTY and `NO_COLOR` is unset.

use std::io::{self, IsTerminal, Write};

use console::{Key, Term};
use lbl_driver_console::{render_terminal, TerminalOptions};

use std::time::Duration;

use crate::debug::{protocol_cli_name, LabelTrace};
use crate::preprocess::{JobPreprocessInput, PreprocessEstimate};
use crate::print_stats::PrintSummaryInput;

// ANSI styling. Kept local and minimal so the crate stays dependency-light.
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const WARN: &str = "\x1b[38;5;220m"; // yellow
const HEADER: &str = "\x1b[1;38;5;75m"; // bold blue
const TAG: &str = "\x1b[38;5;75m"; // blue
const ATTR: &str = "\x1b[38;5;180m"; // tan
const VAL: &str = "\x1b[38;5;114m"; // green
const PUNCT: &str = "\x1b[38;5;245m"; // gray
const COMMENT: &str = "\x1b[38;5;102m"; // dim gray

/// Whether to emit ANSI color, given a stream's TTY status. Honors `NO_COLOR`.
pub fn color_for(is_tty: bool) -> bool {
    is_tty && std::env::var_os("NO_COLOR").is_none()
}

/// Whether stdout should be colorized.
pub fn stdout_color() -> bool {
    color_for(io::stdout().is_terminal())
}

/// Whether stderr should be colorized.
pub fn stderr_color() -> bool {
    color_for(io::stderr().is_terminal())
}

/// Warn before a heavy preprocessing job starts (render + dither + encode).
pub fn warn_preprocess_before(
    input: &JobPreprocessInput,
    estimate: &PreprocessEstimate,
) -> io::Result<()> {
    let color = stderr_color();
    let (w, r, d) = if color {
        (WARN, RESET, DIM)
    } else {
        ("", "", "")
    };
    let mut err = io::stderr();
    let ss = input.supersample.max(1);
    let hires_w = input.width_dots.saturating_mul(ss);
    let hires_h = input.height_dots.saturating_mul(ss);
    let eta = format_seconds(estimate.estimated_seconds);
    let w_dots = input.width_dots;
    let h_dots = input.height_dots;

    writeln!(
        err,
        "\n{w}⚠ {BOLD}Heavy label preprocessing expected{RESET}{r} (~{eta} for {} label(s))",
        input.label_count
    )?;
    writeln!(
        err,
        "{d}  {w_dots}×{h_dots} device dots · supersample {ss} · high-res pass {hires_w}×{hires_h} px per label{r}"
    )?;
    write_preprocess_mitigations(&mut err, input, color)?;
    writeln!(err)?;
    err.flush()
}

/// Periodic batch guidance while preprocessing is still running.
pub fn warn_preprocess_batch_progress(
    elapsed: Duration,
    completed: usize,
    total: usize,
    input: &JobPreprocessInput,
    estimate: &PreprocessEstimate,
) -> io::Result<()> {
    let color = stderr_color();
    let (w, r, d) = if color {
        (WARN, RESET, DIM)
    } else {
        ("", "", "")
    };
    let mut err = io::stderr();
    writeln!(
        err,
        "\n{w}⚠ {BOLD}Preprocessing still running{RESET}{r} — {completed}/{total} labels · {:.0}s elapsed · ~{} remaining",
        elapsed.as_secs_f64(),
        format_seconds(estimate.estimated_seconds * (1.0 - completed as f64 / total as f64))
    )?;
    write_preprocess_mitigations(&mut err, input, color)?;
    writeln!(err, "{d}  (Cancel with Ctrl+C; labels already prepared are not sent until spooling completes.){r}")?;
    writeln!(err)?;
    err.flush()
}

fn write_preprocess_mitigations(
    err: &mut impl Write,
    input: &JobPreprocessInput,
    color: bool,
) -> io::Result<()> {
    let (d, r, v) = if color {
        (DIM, RESET, VAL)
    } else {
        ("", "", "")
    };
    writeln!(err, "{d}  Mitigations:{r}")?;
    if let Some(suggested) = crate::preprocess::suggest_supersample(input.supersample) {
        writeln!(
            err,
            "{d}    • lower supersample: {v}--supersample {suggested}{r}  (now {})",
            input.supersample
        )?;
        writeln!(
            err,
            "{d}      or set {v}[render] supersample = {suggested}{r} in lbl.toml"
        )?;
    }
    if input.label_count > 1 {
        writeln!(
            err,
            "{d}    • print fewer labels: {v}--one <index>{r}  or  {v}--indices 0,1,…{r}"
        )?;
    }
    Ok(())
}

/// Summary after a successful hardware print.
pub fn print_run_summary(input: &PrintSummaryInput<'_>) -> io::Result<()> {
    use crate::print_stats::{
        format_duration, format_efficiency, format_throughput, total_feed_mm,
    };

    let color = stderr_color();
    let (h, r, d, v) = if color {
        (HEADER, RESET, DIM, VAL)
    } else {
        ("", "", "", "")
    };
    let mut err = io::stderr();

    let timings = input.timings;
    let total = timings.total();
    let efficiency = timings.efficiency();
    let feed_mm = total_feed_mm(
        input.feed_dots,
        input.label_count,
        input.media,
        input.rotation,
        input.copies,
        input.media.dpi.0,
    );
    let throughput = format_throughput(feed_mm, timings.print);

    let labels = input.label_count;
    let copies = input.copies.max(1);
    let label_word = if labels == 1 { "label" } else { "labels" };
    let copy_note = if copies > 1 {
        format!(" × {copies} copies")
    } else {
        String::new()
    };

    writeln!(
        err,
        "\n{h}Print complete{r} — {labels} {label_word}{copy_note} · {feed_mm:.1} mm feed"
    )?;
    writeln!(
        err,
        "{d}  total {v}{}{r}  ·  preprocess {v}{}{r}  ·  printing {v}{}{r}  ·  {v}{throughput}{r}  ·  efficiency {v}{}{r}",
        format_duration(total),
        format_duration(timings.preprocess),
        format_duration(timings.print),
        format_efficiency(efficiency),
    )?;

    if input.efficiency_warn_below > 0.0 && efficiency < input.efficiency_warn_below {
        let (w, _) = if color { (WARN, RESET) } else { ("", "") };
        writeln!(
            err,
            "\n{w}⚠ Preprocessing took most of the run (efficiency {} < {:.0}% threshold).{r}",
            format_efficiency(efficiency),
            input.efficiency_warn_below * 100.0
        )?;
        write_preprocess_mitigations(&mut err, input.preprocess, color)?;
    }

    writeln!(err)?;
    err.flush()
}

fn format_seconds(secs: f64) -> String {
    if secs < 1.0 {
        "under 1s".to_string()
    } else if secs < 60.0 {
        format!("{:.0}s", secs.round())
    } else {
        let mins = (secs / 60.0).floor() as u32;
        let rem = secs - f64::from(mins * 60);
        if rem < 1.0 {
            format!("{mins}m")
        } else {
            format!("{mins}m {:.0}s", rem.round())
        }
    }
}

/// Best-effort terminal width in columns (from `$COLUMNS`), clamped to a sane
/// range, defaulting to 120 when unknown.
fn term_cols() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(120)
        .clamp(40, 200)
}

/// [`TerminalOptions`] for a framed raster that fits the current terminal.
pub fn raster_options(color: bool) -> TerminalOptions {
    TerminalOptions {
        // Leave room for the frame's two border columns.
        max_width: term_cols().saturating_sub(2).max(16),
        frame: true,
        color,
    }
}

/// Render a dithered bitmap as terminal art sized to the current terminal.
pub fn render_raster(bitmap: &lbl_core::bitmap::MonoBitmap, color: bool) -> String {
    render_terminal(bitmap, &raster_options(color))
}

/// Preview every label, then prompt for confirmation with a single keypress
/// (`y` to print, `n`/`q` or anything else to cancel). Returns whether the
/// user approved the print. The preview and prompt go to stderr so stdout
/// stays clean for any piped output.
pub fn confirm_print(traces: &[LabelTrace]) -> io::Result<bool> {
    let color = stderr_color();
    let mut err = io::stderr();
    for t in traces {
        writeln!(
            err,
            "Label #{} — {}×{} raster:",
            t.index, t.dithered.width, t.dithered.height
        )?;
        write!(err, "{}", render_raster(&t.dithered, color))?;
    }
    let n = traces.len();
    let plural = if n == 1 { "label" } else { "labels" };
    write!(err, "\nPrint {n} {plural}? [y/N] ")?;
    err.flush()?;

    let approved = match read_prompt_key()? {
        Some(key) => prompt_key_confirms(key),
        None => false,
    };
    writeln!(err)?;
    Ok(approved)
}

/// Whether a single prompt key means "yes".
fn prompt_key_confirms(key: char) -> bool {
    matches!(key, 'y' | 'Y')
}

/// Read one key for an interactive prompt. On a TTY this is a single keypress
/// (no Enter). Falls back to a line read when no TTY is available.
fn read_prompt_key() -> io::Result<Option<char>> {
    let term = Term::stderr();
    if term.is_term() {
        return match term.read_key()? {
            Key::Char(c) => Ok(Some(c)),
            _ => Ok(None),
        };
    }

    let mut line = String::new();
    let read = if io::stdin().is_terminal() {
        io::stdin().read_line(&mut line)?
    } else {
        #[cfg(unix)]
        {
            use std::io::BufRead;
            if let Ok(tty) = std::fs::File::open("/dev/tty") {
                io::BufReader::new(tty).read_line(&mut line)?
            } else {
                io::stdin().read_line(&mut line)?
            }
        }
        #[cfg(not(unix))]
        {
            io::stdin().read_line(&mut line)?
        }
    };
    if read == 0 {
        return Ok(None);
    }
    Ok(line.trim().chars().next())
}

/// Write the dithered raster of every label to stdout (the `--protocol console`
/// output). Color is used when stdout is a TTY.
pub fn dump_rasters(traces: &[LabelTrace]) -> io::Result<()> {
    let color = stdout_color();
    let mut out = io::stdout().lock();
    let many = traces.len() > 1;
    for t in traces {
        if many {
            let (a, z) = if color { (HEADER, RESET) } else { ("", "") };
            writeln!(out, "{a}── Label #{} ──{z}", t.index)?;
        }
        write!(out, "{}", render_raster(&t.dithered, color))?;
    }
    out.flush()
}

/// Print a per-stage debug dump for every label to stderr.
pub fn dump_debug(traces: &[LabelTrace]) -> io::Result<()> {
    let color = stderr_color();
    let mut err = io::stderr();
    for t in traces {
        write!(err, "{}", render_label_debug(t, color))?;
    }
    err.flush()
}

/// Print the effective layered configuration (and figment provenance) to stderr.
pub fn dump_config_report(loader: &lbl_config::Loader) -> io::Result<()> {
    let color = stderr_color();
    let mut err = io::stderr();
    writeln!(err, "{}", heading("═══ Configuration ═══", color))?;
    write!(err, "{}", render_config_report(loader, color)?)?;
    writeln!(err)?;
    writeln!(
        err,
        "{}",
        dimmed(
            "Per-run flags (--protocol, --padding-mm, …) override config but are not shown here.",
            color
        )
    )?;
    err.flush()
}

/// Effective configuration JSON, with optional syntax coloring.
pub fn render_config_json(loader: &lbl_config::Loader, color: bool) -> io::Result<String> {
    let cfg = loader.load().map_err(|e| io::Error::other(e.to_string()))?;
    let json = serde_json::to_string_pretty(&cfg).map_err(io::Error::other)?;
    Ok(highlight_json(&json, color))
}

/// Effective configuration as text, with optional JSON and provenance coloring.
pub fn render_config_report(loader: &lbl_config::Loader, color: bool) -> io::Result<String> {
    let mut out = render_config_json(loader, color)?;
    out.push_str("\n\n");
    out.push_str(&sources_block(loader, color));
    Ok(out)
}

fn sources_block(loader: &lbl_config::Loader, color: bool) -> String {
    let mut out = String::new();
    if color {
        out.push_str(BOLD);
    }
    out.push_str("Sources");
    if color {
        out.push_str(RESET);
    }
    out.push('\n');
    for (key, source) in lbl_config::describe_sources(loader.figment()) {
        if color {
            out.push_str(ATTR);
            out.push_str(&key);
            out.push_str(RESET);
            out.push('\t');
            out.push_str(DIM);
            out.push_str(&source);
            out.push_str(RESET);
        } else {
            out.push_str(&key);
            out.push('\t');
            out.push_str(&source);
        }
        out.push('\n');
    }
    out
}

/// Build the per-stage debug report for a single label.
pub fn render_label_debug(t: &LabelTrace, color: bool) -> String {
    let mut out = String::new();
    out.push_str(&heading(&format!("═══ Label #{} ═══", t.index), color));
    out.push('\n');

    out.push_str(&stage_title(1, "Authoring HTML", color));
    out.push_str(&highlight_html(&t.authoring_html, color));
    out.push_str("\n\n");

    out.push_str(&stage_title(2, "Transpile → browser-ready HTML", color));
    out.push_str(&highlight_html(&t.transpiled_html, color));
    out.push_str("\n\n");

    let (rw, rh) = t.rendered.dimensions();
    out.push_str(&stage_title(3, "Render", color));
    out.push_str(&dimmed(&format!("{rw}×{rh} grayscale raster\n"), color));
    out.push('\n');

    out.push_str(&stage_title(
        4,
        &format!(
            "Dither ({}) → {}×{} 1-bit raster",
            dither_name(t.dither),
            t.dithered.width,
            t.dithered.height
        ),
        color,
    ));
    out.push_str(&render_raster(&t.dithered, color));
    out.push('\n');

    out.push_str(&stage_title(
        5,
        &format!(
            "Encode → {} via {}: {} bytes",
            protocol_cli_name(t.protocol),
            t.driver_name,
            t.encoded.len()
        ),
        color,
    ));
    out.push_str(&dimmed(&hex_preview(&t.encoded, 128), color));
    out.push_str("\n\n");
    out
}

fn dither_name(alg: lbl_dither::Algorithm) -> &'static str {
    use lbl_dither::Algorithm::*;
    match alg {
        Auto => "auto",
        FloydSteinberg => "floyd-steinberg",
        Ordered => "ordered",
        Threshold(_) => "threshold",
    }
}

fn heading(text: &str, color: bool) -> String {
    if color {
        format!("{HEADER}{text}{RESET}\n")
    } else {
        format!("{text}\n")
    }
}

fn stage_title(num: u8, title: &str, color: bool) -> String {
    if color {
        format!("{BOLD}[{num}] {title}{RESET}\n")
    } else {
        format!("[{num}] {title}\n")
    }
}

fn dimmed(text: &str, color: bool) -> String {
    if color {
        format!("{DIM}{text}{RESET}")
    } else {
        text.to_string()
    }
}

/// A short hex dump of the first `max` bytes of `data`.
fn hex_preview(data: &[u8], max: usize) -> String {
    let mut out = String::new();
    for (i, byte) in data.iter().take(max).enumerate() {
        if i > 0 && i % 16 == 0 {
            out.push('\n');
        } else if i > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{byte:02x}"));
    }
    if data.len() > max {
        out.push_str(&format!("\n… ({} more bytes)", data.len() - max));
    }
    out.push('\n');
    out
}

/// Colorize HTML for the terminal with a tiny tag-aware tokenizer. When `color`
/// is false the source is returned unchanged.
pub fn highlight_html(src: &str, color: bool) -> String {
    if !color {
        return src.to_string();
    }
    let mut out = String::new();
    let mut rest = src;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let after = &rest[lt..];
        if let Some(stripped) = after.strip_prefix("<!--") {
            match stripped.find("-->") {
                Some(end) => {
                    let close = lt + 4 + end + 3;
                    out.push_str(COMMENT);
                    out.push_str(&rest[lt..close]);
                    out.push_str(RESET);
                    rest = &rest[close..];
                }
                None => {
                    out.push_str(COMMENT);
                    out.push_str(after);
                    out.push_str(RESET);
                    rest = "";
                }
            }
            continue;
        }
        match after.find('>') {
            Some(gt) => {
                out.push_str(&highlight_tag(&after[..=gt]));
                rest = &after[gt + 1..];
            }
            None => {
                out.push_str(after);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Colorize a single `<...>` tag.
fn highlight_tag(tag: &str) -> String {
    let mut out = String::new();
    out.push_str(PUNCT);
    out.push('<');
    out.push_str(RESET);

    // Strip the surrounding angle brackets (both ASCII, so byte slicing is safe).
    let inner: Vec<char> = tag[1..tag.len() - 1].chars().collect();
    let mut i = 0;
    let is_name = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':');

    out.push_str(TAG);
    while i < inner.len() && (is_name(inner[i]) || matches!(inner[i], '/' | '!')) {
        out.push(inner[i]);
        i += 1;
    }
    out.push_str(RESET);

    while i < inner.len() {
        let c = inner[i];
        if c.is_whitespace() {
            out.push(c);
            i += 1;
        } else if c == '"' || c == '\'' {
            out.push_str(VAL);
            out.push(c);
            i += 1;
            while i < inner.len() && inner[i] != c {
                out.push(inner[i]);
                i += 1;
            }
            if i < inner.len() {
                out.push(inner[i]);
                i += 1;
            }
            out.push_str(RESET);
        } else if is_name(c) {
            out.push_str(ATTR);
            while i < inner.len() && is_name(inner[i]) {
                out.push(inner[i]);
                i += 1;
            }
            out.push_str(RESET);
        } else {
            out.push_str(PUNCT);
            out.push(c);
            out.push_str(RESET);
            i += 1;
        }
    }

    out.push_str(PUNCT);
    out.push('>');
    out.push_str(RESET);
    out
}

/// Colorize JSON for the terminal. When `color` is false the source is returned
/// unchanged.
pub fn highlight_json(src: &str, color: bool) -> String {
    if !color {
        return src.to_string();
    }
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            out.push(c);
            i += 1;
            continue;
        }
        if c == '"' {
            let (end, is_key) = scan_json_string(&chars, i);
            out.push_str(if is_key { TAG } else { VAL });
            for ch in &chars[i..end] {
                out.push(*ch);
            }
            out.push_str(RESET);
            i = end;
            continue;
        }
        if matches!(c, '{' | '}' | '[' | ']' | ':' | ',') {
            out.push_str(PUNCT);
            out.push(c);
            out.push_str(RESET);
            i += 1;
            continue;
        }
        if c == 'n' && starts_with(&chars, i, "null") {
            out.push_str(TAG);
            out.push_str("null");
            out.push_str(RESET);
            i += 4;
            continue;
        }
        if c == 't' && starts_with(&chars, i, "true") {
            out.push_str(TAG);
            out.push_str("true");
            out.push_str(RESET);
            i += 4;
            continue;
        }
        if c == 'f' && starts_with(&chars, i, "false") {
            out.push_str(TAG);
            out.push_str("false");
            out.push_str(RESET);
            i += 5;
            continue;
        }
        if c == '-' || c.is_ascii_digit() {
            let end = scan_json_number(&chars, i);
            out.push_str(ATTR);
            for ch in &chars[i..end] {
                out.push(*ch);
            }
            out.push_str(RESET);
            i = end;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn starts_with(chars: &[char], i: usize, word: &str) -> bool {
    chars[i..]
        .iter()
        .copied()
        .zip(word.chars())
        .all(|(a, b)| a == b)
}

fn scan_json_string(chars: &[char], start: usize) -> (usize, bool) {
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == '"' {
            let end = i + 1;
            let mut j = end;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            let is_key = j < chars.len() && chars[j] == ':';
            return (end, is_key);
        }
        i += 1;
    }
    (chars.len(), false)
}

fn scan_json_number(chars: &[char], start: usize) -> usize {
    let mut i = start;
    if i < chars.len() && chars[i] == '-' {
        i += 1;
    }
    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
        i += 1;
    }
    if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
        i += 1;
        if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
            i += 1;
        }
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_honors_no_color() {
        // With a TTY but NO_COLOR set, color is disabled.
        temp_env_no_color(|| assert!(!color_for(true)));
    }

    fn temp_env_no_color(f: impl FnOnce()) {
        std::env::set_var("NO_COLOR", "1");
        f();
        std::env::remove_var("NO_COLOR");
    }

    #[test]
    fn highlight_plain_when_disabled() {
        let src = "<div class=\"x\">hi</div>";
        assert_eq!(highlight_html(src, false), src);
    }

    #[test]
    fn highlight_colorizes_tags_and_attrs() {
        let src = "<div class=\"x\">hi<!-- c --></div>";
        let out = highlight_html(src, true);
        assert!(out.contains('\x1b'));
        assert!(out.contains(TAG));
        assert!(out.contains(ATTR));
        assert!(out.contains(VAL));
        assert!(out.contains(COMMENT));
        // The visible text and tag names survive.
        assert!(out.contains("div"));
        assert!(out.contains("hi"));
    }

    #[test]
    fn highlight_handles_unterminated_tag() {
        let out = highlight_html("text <div ", true);
        assert!(out.contains("text "));
        assert!(out.contains("div"));
    }

    #[test]
    fn highlight_json_plain_when_disabled() {
        let src = r#"{"a": 1}"#;
        assert_eq!(highlight_json(src, false), src);
    }

    #[test]
    fn highlight_json_colorizes_keys_values_and_literals() {
        let src = "{\n  \"style\": {\n    \"padding_mm\": 2.0,\n    \"label_fit\": \"auto\",\n    \"confirm\": true,\n    \"printer\": null\n  }\n}";
        let out = highlight_json(src, true);
        assert!(out.contains('\x1b'));
        assert!(out.contains(TAG));
        assert!(out.contains(VAL));
        assert!(out.contains(ATTR));
        assert!(out.contains(PUNCT));
        assert!(out.contains("padding_mm"));
        assert!(out.contains("auto"));
    }

    #[test]
    fn prompt_key_confirms_y_only() {
        assert!(prompt_key_confirms('y'));
        assert!(prompt_key_confirms('Y'));
        assert!(!prompt_key_confirms('n'));
        assert!(!prompt_key_confirms('N'));
        assert!(!prompt_key_confirms('q'));
        assert!(!prompt_key_confirms('Q'));
    }
}

//! Generate README and mdBook examples for fixed-size label printing.
//!
//! Reads `docs/examples/manifest.toml`, runs `lbl print` for each case with the
//! virtual driver, writes PNG previews, and patches generated markdown.
//!
//! Usage:
//!   cargo run -q -p lbl --bin generate-doc-examples
//!   cargo run -q -p lbl --bin generate-doc-examples -- --check

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Deserialize;

const README_START: &str = "<!-- doc-examples:start -->";
const README_END: &str = "<!-- doc-examples:end -->";

const PIXEL_DELTA: i16 = 8;
const PNG_TOLERANCE: f64 = 0.02;

#[derive(Parser)]
#[command(name = "generate-doc-examples")]
struct Cli {
    /// Fail when generated output differs from committed files.
    #[arg(long)]
    check: bool,
    /// Repository root (defaults to the workspace root).
    #[arg(long)]
    root: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    defaults: Defaults,
    example: Vec<Example>,
}

#[derive(Debug, Deserialize)]
struct Defaults {
    protocol: String,
    supersample: u32,
}

#[derive(Debug, Deserialize)]
struct Example {
    id: String,
    caption: String,
    media: String,
    #[serde(default)]
    dpi: Option<f64>,
    #[serde(default)]
    width_mm: Option<f64>,
    #[serde(default)]
    length_mm: Option<f64>,
    #[serde(default)]
    dir: Option<String>,
    args: Vec<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("generate-doc-examples: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = cli
        .root
        .clone()
        .unwrap_or_else(|| workspace_root().expect("workspace root"));
    let examples_root = root.join("docs/examples");
    let manifest_path = examples_root.join("manifest.toml");
    let manifest_text = fs::read_to_string(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest: Manifest =
        toml::from_str(&manifest_text).context("parse docs/examples/manifest.toml")?;

    let images_dir = root.join("docs/src/generated/images");
    if !cli.check {
        fs::create_dir_all(&images_dir)?;
    }

    let lbl_bin = resolve_lbl_binary(&root)?;
    let mut rows = Vec::new();
    let mut stale_pngs = Vec::new();

    for example in &manifest.example {
        let png_path = images_dir.join(format!("{}.png", example.id));
        let render_target = if cli.check {
            std::env::temp_dir().join(format!("lbl-doc-example-{}.png", example.id))
        } else {
            png_path.clone()
        };
        let fresh = render_example(
            &lbl_bin,
            &examples_root,
            &manifest.defaults,
            example,
            &render_target,
        )?;
        if cli.check {
            if png_path.is_file() {
                if let Err(err) = compare_png(&png_path, &fresh) {
                    stale_pngs.push(format!("{}: {err}", png_path.display()));
                }
            } else {
                stale_pngs.push(format!("missing {}", png_path.display()));
            }
            let _ = fs::remove_file(&render_target);
        } else {
            if render_target != png_path {
                fs::rename(&render_target, &png_path).or_else(|_| {
                    fs::copy(&render_target, &png_path)?;
                    fs::remove_file(&render_target)?;
                    Ok::<(), std::io::Error>(())
                })?;
            }
        }
        rows.push(ExampleRow {
            caption: example.caption.clone(),
            command: format_display_command(&example.args),
            readme_image: format!("docs/src/generated/images/{}.png", example.id),
            book_image: format!("images/{}.png", example.id),
        });
    }

    if !stale_pngs.is_empty() {
        bail!(
            "preview image(s) stale:\n  - {}\nrun `just doc-examples` and commit",
            stale_pngs.join("\n  - ")
        );
    }

    let readme_section = render_readme_section(&rows);
    let book_page = render_book_page(&rows);

    let readme_path = root.join("README.md");
    let book_path = root.join("docs/src/generated/label-examples.md");

    if cli.check {
        check_readme_section(&readme_path, &readme_section)?;
        check_file_contents(&book_path, &book_page)?;
        eprintln!("doc examples are up to date");
    } else {
        patch_readme(&readme_path, &readme_section)?;
        fs::create_dir_all(book_path.parent().unwrap())?;
        fs::write(&book_path, &book_page)
            .with_context(|| format!("write {}", book_path.display()))?;
        eprintln!(
            "wrote {} preview(s), README section, and {}",
            manifest.example.len(),
            book_path.display()
        );
    }

    Ok(())
}

struct ExampleRow {
    caption: String,
    command: String,
    readme_image: String,
    book_image: String,
}

fn workspace_root() -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .context("resolve workspace root from CARGO_MANIFEST_DIR")
}

fn resolve_lbl_binary(root: &Path) -> Result<PathBuf> {
    let release = root.join("target/release/lbl");
    if release.is_file() {
        return Ok(release);
    }
    let debug = root.join("target/debug/lbl");
    if debug.is_file() {
        return Ok(debug);
    }

    eprintln!("building lbl …");
    let status = Command::new("cargo")
        .args(["build", "-q", "-p", "lbl", "--bin", "lbl"])
        .current_dir(root)
        .status()
        .context("cargo build -p lbl")?;
    if !status.success() {
        bail!("cargo build -p lbl failed");
    }
    Ok(root.join("target/debug/lbl"))
}

fn render_example(
    lbl_bin: &Path,
    examples_root: &Path,
    defaults: &Defaults,
    example: &Example,
    png_path: &Path,
) -> Result<Vec<u8>> {
    let work_dir = example
        .dir
        .as_ref()
        .map(|d| examples_root.join(d))
        .unwrap_or_else(|| examples_root.to_path_buf());

    let mut cmd = Command::new(lbl_bin);
    cmd.arg("print");
    cmd.args(&example.args);
    if !example.media.is_empty() {
        cmd.args(["--media", &example.media]);
    } else {
        let width = example
            .width_mm
            .context("example needs media or width_mm")?;
        cmd.args(["--width-mm", &format_num(width)]);
        if let Some(length) = example.length_mm {
            cmd.args(["--length-mm", &format_num(length)]);
        }
    }
    if let Some(dpi) = example.dpi {
        cmd.args(["--dpi", &format_num(dpi)]);
    }
    if !example.args.iter().any(|a| a == "--supersample") {
        cmd.args([
            "--supersample",
            &defaults.supersample.to_string(),
        ]);
    }
    cmd.args([
        "--protocol",
        &defaults.protocol,
        "--file",
        &png_path.display().to_string(),
    ]);
    cmd.current_dir(&work_dir);
    cmd.stdout(Stdio::null());

    let output = cmd
        .output()
        .with_context(|| format!("run lbl print for {}", example.id))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "lbl print failed for {} (cwd {}):\n{stderr}",
            example.id,
            work_dir.display()
        );
    }
    fs::read(png_path).with_context(|| format!("read generated {}", png_path.display()))
}

fn format_num(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        n.to_string()
    }
}

fn format_display_command(args: &[String]) -> String {
    format!("lbl print {}", shell_join(args))
}

fn shell_join(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.is_empty() {
                "''".to_string()
            } else if arg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
            {
                arg.clone()
            } else {
                format!("'{}'", arg.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_readme_section(rows: &[ExampleRow]) -> String {
    let mut out = String::new();
    out.push_str(README_START);
    out.push('\n');
    out.push_str("\n## Fixed-size label examples\n\n");
    out.push_str(
        "Commands show content and layout flags only. Media size, DPI, protocol, \
         and output path are supplied by project config (`lbl.toml`) or environment — \
         see [Configuration](docs/src/guides/configuration.md). Preview images are \
         generated from the manifest in [`docs/examples/manifest.toml`](docs/examples/manifest.toml) \
         via `just doc-examples`.\n\n",
    );
    for row in rows {
        out.push_str("<table>\n<tr>\n<td valign=\"top\">\n\n");
        out.push_str(&row.caption);
        out.push_str("\n\n```bash\n");
        out.push_str(&row.command);
        out.push_str("\n```\n\n</td>\n<td>\n\n");
        out.push_str(&format!(
            "<img src=\"{}\" alt=\"{}\" width=\"240\"/>\n\n",
            row.readme_image, row.caption
        ));
        out.push_str("</td>\n</tr>\n</table>\n\n");
    }
    out.push_str(README_END);
    out.push('\n');
    out
}

fn render_book_page(rows: &[ExampleRow]) -> String {
    let mut out = String::new();
    out.push_str("# Fixed-size label examples\n\n");
    out.push_str(
        "Commands show content and layout flags only. Media size, DPI, protocol, \
         and output path come from project config (`lbl.toml`) or environment.\n\n",
    );
    out.push_str(
        "The cases below mirror the guides: getting started, printing text, batch \
         printing, configuration, rendering quality, and printers & media. Regenerate \
         previews with `just doc-examples`.\n\n",
    );
    for row in rows {
        out.push_str(&format!("## {}\n\n", row.caption));
        out.push_str("```bash\n");
        out.push_str(&row.command);
        out.push_str("\n```\n\n");
        out.push_str(&format!(
            "<img src=\"{}\" alt=\"{}\" width=\"320\"/>\n\n",
            row.book_image, row.caption
        ));
    }
    out
}

fn patch_readme(readme_path: &Path, section: &str) -> Result<()> {
    let readme = fs::read_to_string(readme_path)
        .with_context(|| format!("read {}", readme_path.display()))?;
    if !readme.contains(README_START) || !readme.contains(README_END) {
        bail!(
            "README.md is missing {README_START} / {README_END} markers; add them first"
        );
    }
    let start = readme
        .find(README_START)
        .context("README start marker missing")?;
    let end = readme
        .find(README_END)
        .context("README end marker missing")?
        + README_END.len();
    let mut patched = String::new();
    patched.push_str(&readme[..start]);
    patched.push_str(section);
    patched.push_str(&readme[end..]);
    fs::write(readme_path, patched)
        .with_context(|| format!("write {}", readme_path.display()))?;
    Ok(())
}

fn check_readme_section(readme_path: &Path, expected: &str) -> Result<()> {
    let readme = fs::read_to_string(readme_path)
        .with_context(|| format!("read {}", readme_path.display()))?;
    let start = readme
        .find(README_START)
        .context("README start marker missing")?;
    let end = readme
        .find(README_END)
        .context("README end marker missing")?
        + README_END.len();
    let actual = &readme[start..end];
    if actual != expected.trim_end() {
        bail!(
            "README.md doc-examples section is stale; run `just doc-examples` and commit"
        );
    }
    Ok(())
}

fn check_file_contents(path: &Path, expected: &str) -> Result<()> {
    let existing = fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    if existing != expected {
        bail!(
            "{} is stale; run `just doc-examples` and commit",
            path.display()
        );
    }
    Ok(())
}

fn compare_png(expected_path: &Path, actual: &[u8]) -> Result<(), String> {
    let expected = fs::read(expected_path).map_err(|e| format!("read: {e}"))?;
    compare_png_bytes(&expected, actual)
}

fn compare_png_bytes(expected: &[u8], actual: &[u8]) -> Result<(), String> {
    let expected = image::load_from_memory(expected)
        .map_err(|e| format!("decode reference: {e}"))?
        .to_luma8();
    let actual = image::load_from_memory(actual)
        .map_err(|e| format!("decode output: {e}"))?
        .to_luma8();

    if expected.dimensions() != actual.dimensions() {
        return Err(format!(
            "dimension mismatch: reference {:?}, output {:?}",
            expected.dimensions(),
            actual.dimensions()
        ));
    }

    let total = (expected.width() as u64) * (expected.height() as u64);
    let mut differing = 0u64;
    for (pe, pa) in expected.pixels().zip(actual.pixels()) {
        if (pe[0] as i16 - pa[0] as i16).abs() > PIXEL_DELTA {
            differing += 1;
        }
    }

    let fraction = differing as f64 / total.max(1) as f64;
    if fraction > PNG_TOLERANCE {
        return Err(format!(
            "{differing}/{total} pixels differ ({:.2}%), exceeds allowed {:.2}%",
            fraction * 100.0,
            PNG_TOLERANCE * 100.0
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_join_quotes_spaces() {
        assert_eq!(
            shell_join(&["--template".into(), "User #{{ it }}".into()]),
            "--template 'User #{{ it }}'"
        );
    }
}

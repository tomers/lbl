//! Generate README and mdBook examples for fixed-size label printing.
//!
//! Reads `docs/examples/manifest.toml`, runs `lbl print` for each case with the
//! virtual driver, writes PNG previews, and patches generated markdown.
//!
//! Usage:
//!   cargo run -q -p lbl --bin generate-doc-examples
//!   cargo run -q -p lbl --bin generate-doc-examples -- --check

use std::collections::{HashMap, HashSet};
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
struct ExampleFile {
    path: String,
    lang: String,
}

#[derive(Debug, Deserialize)]
struct CompareVariant {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct Example {
    id: String,
    title: String,
    description: String,
    doc: String,
    #[serde(default)]
    doc_title: Option<String>,
    #[serde(default)]
    doc_section: Option<String>,
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
    #[serde(default)]
    dir_comment: Option<String>,
    #[serde(default)]
    files: Vec<ExampleFile>,
    #[serde(default)]
    composite: Option<String>,
    #[serde(default)]
    separate_images: bool,
    #[serde(default)]
    xargs: Option<String>,
    #[serde(default)]
    show_media: bool,
    #[serde(default)]
    render_args: Vec<String>,
    #[serde(default)]
    compare: Vec<CompareVariant>,
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
    let mut keep_images = HashSet::new();

    for example in &manifest.example {
        let png_path = images_dir.join(format!("{}.png", example.id));
        let render_target = if cli.check {
            std::env::temp_dir().join(format!("{}.png", example.id))
        } else {
            png_path.clone()
        };
        let rendered_paths = render_example(
            &lbl_bin,
            &examples_root,
            &manifest.defaults,
            example,
            &render_target,
        )?;
        if cli.check {
            for path in &rendered_paths {
                let committed = images_dir.join(path.file_name().unwrap());
                if committed.is_file() {
                    let fresh =
                        fs::read(path).with_context(|| format!("read {}", path.display()))?;
                    if let Err(err) = compare_png_bytes(
                        &fs::read(&committed)
                            .with_context(|| format!("read {}", committed.display()))?,
                        &fresh,
                    ) {
                        stale_pngs.push(format!("{}: {err}", committed.display()));
                    }
                } else {
                    stale_pngs.push(format!("missing {}", committed.display()));
                }
                let _ = fs::remove_file(path);
            }
        } else {
            for path in &rendered_paths {
                let name = path.file_name().unwrap();
                let committed = images_dir.join(name);
                keep_images.insert(name.to_string_lossy().into_owned());
                if path != &committed {
                    fs::rename(path, &committed).or_else(|_| {
                        fs::copy(path, &committed)?;
                        fs::remove_file(path)?;
                        Ok::<(), std::io::Error>(())
                    })?;
                }
            }
        }
        let readme_images: Vec<String> = rendered_paths
            .iter()
            .map(|path| {
                format!(
                    "docs/src/generated/images/{}",
                    path.file_name().unwrap().to_string_lossy()
                )
            })
            .collect();
        let book_images: Vec<String> = readme_images
            .iter()
            .map(|path| path.replacen("docs/src/generated/", "", 1))
            .collect();
        rows.push(ExampleRow {
            title: example.title.clone(),
            description: example.description.clone(),
            caption: example.caption.clone(),
            command: format_display_command(example),
            readme_doc: format_readme_doc_link(example),
            book_doc: format_book_doc_link(example),
            doc_title: example
                .doc_title
                .clone()
                .unwrap_or_else(|| default_doc_title(&example.doc)),
            files: load_example_files(&examples_root, example)?,
            readme_images,
            book_images,
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
        prune_orphan_images(&images_dir, &keep_images)?;
        eprintln!(
            "wrote {} preview(s), README section, and {}",
            manifest.example.len(),
            book_path.display()
        );
    }

    Ok(())
}

struct ExampleRow {
    title: String,
    description: String,
    caption: String,
    command: String,
    readme_doc: String,
    book_doc: String,
    doc_title: String,
    files: Vec<EmbeddedFile>,
    readme_images: Vec<String>,
    book_images: Vec<String>,
}

struct EmbeddedFile {
    path: String,
    lang: String,
    contents: String,
    readme_href: String,
    book_href: String,
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
) -> Result<Vec<PathBuf>> {
    let work_dir = example_work_dir(examples_root, example);

    if !example.compare.is_empty() {
        for (i, variant) in example.compare.iter().enumerate() {
            let target = numbered_output_path(png_path, i);
            let args = resolve_print_args(example, Some(&variant.args));
            run_lbl_print(
                lbl_bin,
                defaults,
                example,
                LblPrintRequest {
                    work_dir: &work_dir,
                    png_path: &target,
                    print_args: &args,
                    data: None,
                    env: &variant.env,
                },
            )?;
        }
    } else if let Some(xargs) = &example.xargs {
        let values = xargs_values(xargs)?;
        for (i, value) in values.iter().enumerate() {
            let target = numbered_output_path(png_path, i);
            let args = resolve_print_args(example, None);
            run_lbl_print(
                lbl_bin,
                defaults,
                example,
                LblPrintRequest {
                    work_dir: &work_dir,
                    png_path: &target,
                    print_args: &args,
                    data: Some(value),
                    env: &HashMap::new(),
                },
            )?;
        }
    } else {
        let args = resolve_print_args(example, None);
        run_lbl_print(
            lbl_bin,
            defaults,
            example,
            LblPrintRequest {
                work_dir: &work_dir,
                png_path,
                print_args: &args,
                data: None,
                env: &HashMap::new(),
            },
        )?;
    }

    let composite = example.composite.as_deref() == Some("side_by_side");
    if composite && !example.separate_images {
        composite_side_by_side(png_path)?;
    }
    let paths = collect_label_pngs(png_path)?;
    Ok(paths)
}

fn resolve_print_args(example: &Example, compare_extra: Option<&[String]>) -> Vec<String> {
    let mut args = example.args.clone();
    args.extend(example.render_args.clone());
    if let Some(extra) = compare_extra {
        args = merge_flag_args(args, extra);
    }
    args
}

fn merge_flag_args(base: Vec<String>, overlay: &[String]) -> Vec<String> {
    let mut pairs: Vec<(String, Option<String>)> = Vec::new();
    let mut i = 0;
    while i < base.len() {
        let flag = base[i].clone();
        if flag.starts_with("--") {
            if let Some(value) = base.get(i + 1).filter(|v| !v.starts_with("--")) {
                i += 2;
                pairs.push((flag, Some(value.clone())));
            } else {
                i += 1;
                pairs.push((flag, None));
            }
        } else {
            i += 1;
            pairs.push((flag, None));
        }
    }
    i = 0;
    while i < overlay.len() {
        let flag = overlay[i].clone();
        if flag.starts_with("--") {
            let value = overlay.get(i + 1).filter(|v| !v.starts_with("--")).cloned();
            let step = if value.is_some() { 2 } else { 1 };
            if let Some(pos) = pairs.iter().position(|(f, _)| f == &flag) {
                pairs[pos] = (flag, value);
            } else {
                pairs.push((flag, value));
            }
            i += step;
        } else {
            pairs.push((flag, None));
            i += 1;
        }
    }
    let mut out = Vec::new();
    for (flag, value) in pairs {
        out.push(flag);
        if let Some(value) = value {
            out.push(value);
        }
    }
    out
}

struct LblPrintRequest<'a> {
    work_dir: &'a Path,
    png_path: &'a Path,
    print_args: &'a [String],
    data: Option<&'a str>,
    env: &'a HashMap<String, String>,
}

fn run_lbl_print(
    lbl_bin: &Path,
    defaults: &Defaults,
    example: &Example,
    req: LblPrintRequest<'_>,
) -> Result<()> {
    let mut cmd = Command::new(lbl_bin);
    cmd.arg("print");
    cmd.args(req.print_args);
    for (key, value) in req.env {
        cmd.env(key, value);
    }
    if let Some(value) = req.data {
        cmd.args(["--data", value]);
    }
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
    if !req.print_args.iter().any(|a| a == "--supersample") {
        cmd.args(["--supersample", &defaults.supersample.to_string()]);
    }
    cmd.args([
        "--protocol",
        &defaults.protocol,
        "--file",
        &req.png_path.display().to_string(),
    ]);
    cmd.current_dir(req.work_dir);
    cmd.stdout(Stdio::null());

    let output = cmd
        .output()
        .with_context(|| format!("run lbl print for {}", example.id))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "lbl print failed for {} (cwd {}):\n{stderr}",
            example.id,
            req.work_dir.display()
        );
    }
    Ok(())
}

fn xargs_values(spec: &str) -> Result<Vec<String>> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(spec)
        .output()
        .with_context(|| format!("run xargs producer `{spec}`"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("xargs producer `{spec}` failed:\n{stderr}");
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn numbered_output_path(base: &Path, index: usize) -> PathBuf {
    if index == 0 {
        return base.to_path_buf();
    }
    let parent = base.parent().unwrap_or_else(|| Path::new("."));
    let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("label");
    let name = match base.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{stem}-{index:02}.{ext}"),
        None => format!("{stem}-{index:02}"),
    };
    parent.join(name)
}

fn format_num(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        n.to_string()
    }
}

const COMPOSITE_GAP_PX: u32 = 12;

/// Collect the base PNG and numbered batch siblings (`out.png`, `out-01.png`, …).
fn collect_label_pngs(base: &Path) -> Result<Vec<PathBuf>> {
    if !base.is_file() {
        bail!("missing batch label output {}", base.display());
    }
    let mut paths = vec![base.to_path_buf()];
    let parent = base.parent().context("batch label path has no parent")?;
    let stem = base
        .file_stem()
        .and_then(|s| s.to_str())
        .context("batch label path has no stem")?;
    let ext = base
        .extension()
        .and_then(|e| e.to_str())
        .context("batch label path has no extension")?;
    let mut n = 1usize;
    loop {
        let sibling = parent.join(format!("{stem}-{n:02}.{ext}"));
        if sibling.is_file() {
            paths.push(sibling);
            n += 1;
        } else {
            break;
        }
    }
    Ok(paths)
}

/// Stitch a multi-label batch into one preview image.
fn composite_side_by_side(base: &Path) -> Result<()> {
    let paths = collect_label_pngs(base)?;
    if paths.len() <= 1 {
        return Ok(());
    }

    let images: Vec<_> = paths
        .iter()
        .map(|path| {
            image::open(path)
                .with_context(|| format!("open {}", path.display()))
                .map(|img| img.to_rgba8())
        })
        .collect::<Result<_>>()?;

    let gap = COMPOSITE_GAP_PX;
    let total_w: u32 =
        images.iter().map(image::RgbaImage::width).sum::<u32>() + gap * (images.len() as u32 - 1);
    let max_h = images
        .iter()
        .map(image::RgbaImage::height)
        .max()
        .unwrap_or(0);

    let mut canvas =
        image::RgbaImage::from_pixel(total_w, max_h, image::Rgba([255, 255, 255, 255]));
    let mut x = 0u32;
    for img in &images {
        let y = (max_h.saturating_sub(img.height())) / 2;
        image::imageops::overlay(&mut canvas, img, i64::from(x), i64::from(y));
        x += img.width() + gap;
    }

    canvas
        .save(base)
        .with_context(|| format!("write composite {}", base.display()))?;
    for path in paths.iter().skip(1) {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(())
}

fn format_display_command(example: &Example) -> String {
    let mut out = String::new();
    if let Some(dir) = &example.dir {
        out.push_str("$ cd docs/examples/");
        out.push_str(dir);
        out.push('\n');
        if let Some(comment) = &example.dir_comment {
            out.push_str("# ");
            out.push_str(comment);
            out.push('\n');
        }
    }

    if let Some(xargs) = &example.xargs {
        out.push_str("$ ");
        out.push_str(xargs);
        out.push_str(" | xargs -n1 lbl print \\\n");
        let arg_lines = format_print_arg_lines(&display_args_for(example, None));
        for line in &arg_lines {
            out.push_str("  ");
            out.push_str(line);
            out.push_str(" \\");
            out.push('\n');
        }
        out.push_str("  --data");
        return out.trim_end().to_string();
    }

    if !example.compare.is_empty() {
        let mut blocks = Vec::new();
        for variant in &example.compare {
            let label = variant
                .label
                .clone()
                .unwrap_or_else(|| "variant".to_string());
            let prefix = format_compare_env_prefix(&variant.env);
            blocks.push(format!(
                "# {}\n{}{}",
                label,
                prefix,
                format_lbl_print_block(&display_args_for(example, Some(&variant.args)))
            ));
        }
        out.push_str(&blocks.join("\n\n"));
        return out.trim_end().to_string();
    }

    out.push_str(&format_lbl_print_block(&display_args_for(example, None)));
    ensure_command_block_has_context(&mut out);
    out.trim_end().to_string()
}

fn ensure_command_block_has_context(block: &mut String) {
    let has_non_command = block.lines().any(|line| {
        let t = line.trim();
        !t.is_empty() && !t.starts_with('$')
    });
    if !has_non_command {
        block.push_str("\n# (preview label written to file)");
    }
}

fn display_args_for(example: &Example, compare_extra: Option<&[String]>) -> Vec<String> {
    let mut args = example.args.clone();
    if let Some(extra) = compare_extra {
        args = merge_flag_args(args, extra);
    }
    let media = display_media_args(example);
    if example.show_media {
        let mut out = media;
        out.extend(args);
        out
    } else {
        args.extend(media);
        args
    }
}

fn display_media_args(example: &Example) -> Vec<String> {
    if example.media.is_empty() {
        let mut out = Vec::new();
        if let Some(width) = example.width_mm {
            out.push("--width-mm".to_string());
            out.push(format_num(width));
        }
        if let Some(length) = example.length_mm {
            out.push("--length-mm".to_string());
            out.push(format_num(length));
        }
        if let Some(dpi) = example.dpi {
            out.push("--dpi".to_string());
            out.push(format_num(dpi));
        }
        out
    } else if example.show_media {
        let mut out = vec!["--media".to_string(), example.media.clone()];
        if let Some(dpi) = example.dpi {
            out.push("--dpi".to_string());
            out.push(format_num(dpi));
        }
        out
    } else {
        Vec::new()
    }
}

fn format_compare_env_prefix(env: &HashMap<String, String>) -> String {
    if env.is_empty() {
        return String::new();
    }
    let mut parts: Vec<_> = env
        .iter()
        .map(|(key, value)| format!("{}={}", key, shell_quote(value)))
        .collect();
    parts.sort();
    format!("{} ", parts.join(" "))
}

fn format_lbl_print_block(args: &[String]) -> String {
    let arg_lines = format_print_arg_lines(args);
    if arg_lines.is_empty() {
        return "$ lbl print".to_string();
    }
    let mut out = String::from("$ lbl print \\\n");
    for (idx, line) in arg_lines.iter().enumerate() {
        out.push_str("  ");
        out.push_str(line);
        if idx < arg_lines.len() - 1 {
            out.push_str(" \\");
        }
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn format_print_arg_lines(args: &[String]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let flag = &args[i];
        if let Some(value) = args.get(i + 1).filter(|_| flag.starts_with("--")) {
            i += 2;
            lines.push(format!("{flag} {}", format_shell_value(value)));
        } else {
            i += 1;
            lines.push(flag.clone());
        }
    }
    lines
}

fn format_shell_value(s: &str) -> String {
    if s.contains('\n') {
        let mut out = String::from("$'");
        for ch in s.chars() {
            match ch {
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\\' => out.push_str("\\\\"),
                '\'' => out.push_str("\\'"),
                _ => out.push(ch),
            }
        }
        out.push('\'');
        return out;
    }
    shell_quote(s)
}

fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn default_doc_title(doc: &str) -> String {
    let stem = doc
        .rsplit('/')
        .next()
        .unwrap_or(doc)
        .trim_end_matches(".md");
    stem.split('-')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().chain(chars).collect(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_readme_doc_link(example: &Example) -> String {
    format_doc_href(&format!("docs/src/{}", example.doc), example)
}

fn format_book_doc_link(example: &Example) -> String {
    format_doc_href(&format!("../{}", example.doc), example)
}

fn format_doc_href(base: &str, example: &Example) -> String {
    match &example.doc_section {
        Some(section) => format!("{base}#{section}"),
        None => base.to_string(),
    }
}

fn example_work_dir(examples_root: &Path, example: &Example) -> PathBuf {
    example
        .dir
        .as_ref()
        .map(|d| examples_root.join(d))
        .unwrap_or_else(|| examples_root.to_path_buf())
}

fn load_example_files(examples_root: &Path, example: &Example) -> Result<Vec<EmbeddedFile>> {
    let work_dir = example_work_dir(examples_root, example);
    let repo_root = examples_root
        .parent()
        .and_then(|p| p.parent())
        .context("resolve repo root from docs/examples")?;
    example
        .files
        .iter()
        .map(|file| {
            let path = work_dir.join(&file.path);
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("read example file {}", path.display()))?;
            let readme_href = path
                .strip_prefix(repo_root)
                .with_context(|| format!("{} outside repo root", path.display()))?
                .display()
                .to_string();
            let book_href = readme_href.replacen("docs/", "../../", 1);
            Ok(EmbeddedFile {
                path: file.path.clone(),
                lang: file.lang.clone(),
                contents,
                readme_href,
                book_href,
            })
        })
        .collect()
}

fn render_embedded_files(files: &[EmbeddedFile], book: bool) -> String {
    let mut out = String::new();
    for file in files {
        let href = if book {
            &file.book_href
        } else {
            &file.readme_href
        };
        out.push('[');
        out.push_str(&file.path);
        out.push_str("](");
        out.push_str(href);
        out.push_str(")\n\n```");
        out.push_str(&file.lang);
        out.push('\n');
        out.push_str(file.contents.trim_end());
        out.push_str("\n```\n\n");
    }
    out
}

fn render_example_title(row: &ExampleRow) -> String {
    let mut out = String::new();
    out.push_str("### ");
    out.push_str(&row.title);
    out.push_str("\n\n");
    out.push_str(&row.description);
    out.push_str("\n\n");
    out
}

fn render_example_meta(row: &ExampleRow, doc_href: &str) -> String {
    let mut out = String::new();
    out.push('*');
    out.push_str(&row.caption);
    out.push_str("* · [");
    out.push_str(&row.doc_title);
    out.push_str(" →](");
    out.push_str(doc_href);
    out.push_str(")\n\n");
    out
}

fn append_examples_intro(out: &mut String, regenerate_line: &str) {
    out.push_str("Each preview highlights a different `lbl print` capability. Commands show\n");
    out.push_str(
        "the flags that matter for each example; protocol and output path come from project\n",
    );
    out.push_str("config (`lbl.toml`) or the doc generator defaults.\n\n");
    out.push_str(regenerate_line);
    out.push_str("\n\n");
}

fn render_readme_section(rows: &[ExampleRow]) -> String {
    let mut out = String::new();
    out.push_str(README_START);
    out.push('\n');
    out.push_str("\n## Examples\n\n");
    append_examples_intro(
        &mut out,
        "Regenerate from [`docs/examples/manifest.toml`](docs/examples/manifest.toml) with `just doc-examples`.",
    );
    for (idx, row) in rows.iter().enumerate() {
        if idx > 0 {
            out.push_str("---\n\n");
        }
        out.push_str(&render_example_title(row));
        for image in &row.readme_images {
            out.push_str(&format!(
                "<img src=\"{}\" alt=\"{}\" />\n\n",
                image, row.title
            ));
        }
        out.push_str(&render_example_meta(row, &row.readme_doc));
        if !row.files.is_empty() {
            out.push_str(&render_embedded_files(&row.files, false));
        }
        out.push_str("```console\n");
        out.push_str(&row.command);
        out.push_str("\n```\n\n");
    }
    out.push_str(README_END);
    out.push('\n');
    out
}

fn render_book_page(rows: &[ExampleRow]) -> String {
    let mut out = String::new();
    out.push_str(
        "# Examples

",
    );
    append_examples_intro(&mut out, "Regenerate with `just doc-examples`.");
    for (idx, row) in rows.iter().enumerate() {
        if idx > 0 {
            out.push_str("---\n\n");
        }
        out.push_str("## ");
        out.push_str(&row.title);
        out.push('\n');
        out.push('\n');
        out.push_str(&row.description);
        out.push('\n');
        out.push_str("\n*");
        out.push_str(&row.caption);
        out.push_str("* · [");
        out.push_str(&row.doc_title);
        out.push_str(" →](");
        out.push_str(&row.book_doc);
        out.push_str(")\n\n");
        if !row.files.is_empty() {
            out.push_str(&render_embedded_files(&row.files, true));
        }
        out.push_str("```console\n");
        out.push_str(&row.command);
        out.push_str("\n```\n\n");
        for image in &row.book_images {
            out.push_str(&format!(
                "<img src=\"{}\" alt=\"{}\" width=\"320\"/>\n",
                image, row.title
            ));
        }
    }
    out.trim_end_matches('\n').to_string() + "\n"
}

fn patch_readme(readme_path: &Path, section: &str) -> Result<()> {
    let readme = fs::read_to_string(readme_path)
        .with_context(|| format!("read {}", readme_path.display()))?;
    if !readme.contains(README_START) || !readme.contains(README_END) {
        bail!("README.md is missing {README_START} / {README_END} markers; add them first");
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
    patched.push_str(section.trim_end_matches('\n'));
    let suffix = readme[end..].trim_start_matches('\n');
    if !suffix.is_empty() {
        patched.push_str("\n\n");
        patched.push_str(suffix);
    } else {
        patched.push('\n');
    }
    fs::write(readme_path, patched).with_context(|| format!("write {}", readme_path.display()))?;
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
        bail!("README.md doc-examples section is stale; run `just doc-examples` and commit");
    }
    Ok(())
}

fn prune_orphan_images(images_dir: &Path, keep: &HashSet<String>) -> Result<()> {
    if !images_dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(images_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".png") && !keep.contains(&name) {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn check_file_contents(path: &Path, expected: &str) -> Result<()> {
    let existing = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    if existing != expected {
        bail!(
            "{} is stale; run `just doc-examples` and commit",
            path.display()
        );
    }
    Ok(())
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

    fn example_with(args: Vec<&str>) -> Example {
        Example {
            id: "test".into(),
            title: String::new(),
            description: String::new(),
            doc: String::new(),
            doc_title: None,
            doc_section: None,
            caption: String::new(),
            media: String::new(),
            dpi: None,
            width_mm: None,
            length_mm: None,
            dir: None,
            dir_comment: None,
            files: Vec::new(),
            composite: None,
            separate_images: false,
            xargs: None,
            show_media: false,
            render_args: Vec::new(),
            compare: Vec::new(),
            args: args.into_iter().map(str::to_string).collect(),
        }
    }

    #[test]
    fn shell_quote_spaces() {
        assert_eq!(shell_quote("User #{{ it }}"), "'User #{{ it }}'");
    }

    #[test]
    fn format_shell_value_uses_ansi_c_for_newlines() {
        assert_eq!(format_shell_value("Aisle 4\nBin 12"), "$'Aisle 4\\nBin 12'");
    }

    #[test]
    fn format_display_command_uses_continuations() {
        let cmd =
            format_display_command(&example_with(vec!["--text", "hello", "--padding-mm", "0"]));
        assert_eq!(cmd, "$ lbl print \\\n  --text hello \\\n  --padding-mm 0");
    }

    #[test]
    fn format_display_command_includes_cd_for_example_dir() {
        let mut ex = example_with(vec!["--template", "card.html"]);
        ex.dir = Some("batch-card".into());
        assert_eq!(
            format_display_command(&ex),
            "$ cd docs/examples/batch-card\n$ lbl print \\\n  --template card.html"
        );
    }

    #[test]
    fn format_display_command_formats_xargs_pipeline() {
        let mut ex = example_with(vec!["--template", "User #{{ it }}", "--padding-mm", "0"]);
        ex.xargs = Some("seq 1 3".into());
        assert_eq!(
            format_display_command(&ex),
            "$ seq 1 3 | xargs -n1 lbl print \\\n  \
             --template 'User #{{ it }}' \\\n  \
             --padding-mm 0 \\\n  \
             --data"
        );
    }

    #[test]
    fn numbered_output_path_inserts_suffix() {
        let base = PathBuf::from("/tmp/shell-template.png");
        assert_eq!(
            numbered_output_path(&base, 1),
            PathBuf::from("/tmp/shell-template-01.png")
        );
    }

    #[test]
    fn display_media_args_shows_explicit_dimensions() {
        let mut ex = example_with(vec!["--text", "Hi"]);
        ex.width_mm = Some(56.0);
        ex.length_mm = Some(89.0);
        ex.dpi = Some(300.0);
        assert_eq!(
            display_media_args(&ex),
            vec!["--width-mm", "56", "--length-mm", "89", "--dpi", "300"]
        );
    }

    #[test]
    fn display_media_args_shows_catalog_media_when_requested() {
        let mut ex = example_with(vec!["--text", "Hi"]);
        ex.media = "12x40".into();
        ex.dpi = Some(203.0);
        ex.show_media = true;
        assert_eq!(
            display_media_args(&ex),
            vec!["--media", "12x40", "--dpi", "203"]
        );
    }

    #[test]
    fn merge_flag_args_overrides_existing_flags() {
        let base = vec![
            "--text".into(),
            "hello".into(),
            "--supersample".into(),
            "4".into(),
        ];
        let overlay = vec!["--supersample".into(), "8".into()];
        assert_eq!(
            merge_flag_args(base, &overlay),
            vec!["--text", "hello", "--supersample", "8"]
        );
    }
}

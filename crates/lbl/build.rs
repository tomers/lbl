//! Build the Nuxt preview UI when embedded assets are missing.
//!
//! Output lands in `assets/preview/` (gitignored). `include_dir!` embeds that
//! tree at compile time.

use std::path::Path;
use std::process::Command;

fn main() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let preview_ui = manifest_dir.join("preview-ui");
    let assets = manifest_dir.join("assets/preview");
    let nuxt_dir = assets.join("_nuxt");

    rerun_if_changed();

    if nuxt_dir.is_dir() {
        return;
    }

    eprintln!(
        "lbl: building preview UI into {} (run `just preview-ui-build` to rebuild manually)",
        assets.display()
    );

    if !preview_ui.join("node_modules").is_dir() {
        run("npm install", &preview_ui);
    }
    run("npm run build", &preview_ui);

    if !nuxt_dir.is_dir() {
        panic!(
            "preview UI build did not produce {}; install Node.js and run `just preview-ui-build`",
            nuxt_dir.display()
        );
    }
}

fn rerun_if_changed() {
    println!("cargo:rerun-if-changed=preview-ui");
}

fn run(cmd: &str, cwd: &Path) {
    let status = Command::new("sh")
        .arg("-ec")
        .arg(cmd)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|err| panic!("failed to run `{cmd}` in {}: {err}", cwd.display()));
    if !status.success() {
        panic!("`{cmd}` failed in {}", cwd.display());
    }
}

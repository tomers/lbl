# lbl development commands. Run `just` for the full list.
#
# Tooling is managed by mise (see mise.toml); `mise run <recipe>` delegates here.

set allow-duplicate-recipes := true

import "justfiles/mise.just"
import "justfiles/rust_lint.just"
import "justfiles/rust_cargo.just"

default:
    @just --list

help:
    @just --list

alias pc := pre-commit
alias pca := pre-commit-all

# Repo-wide dependency maintenance (Cargo).
mod maintenance './justfiles/maintenance.just'

# Run the lbl-server API on the host.
serve *args: _ensure-mise
    cargo run -p lbl-server -- --bind 127.0.0.1:8787 {{ args }}

# Run the lbl CLI (e.g. `just lbl catalog show 11352`).
lbl *args: _ensure-mise
    cargo run -q -p lbl --bin lbl -- {{ args }}

# Build the Nuxt UI bundle embedded by `lbl print --protocol html`.
# Also runs automatically from `crates/lbl/build.rs` when `assets/preview/_nuxt`
# is missing (e.g. fresh clone).
preview-ui-build: _ensure-mise
    #!/usr/bin/env bash
    set -euo pipefail
    cd crates/lbl/preview-ui
    npm install
    npm run build

# Lint the Rust workspace (rustc warnings-as-errors + clippy + rustfmt check).
lint: _ensure-mise
    @just rust-lint

# Apply Rust autofixes (clippy --fix + rustfmt).
lint-fix: _ensure-mise
    @just rust-lint-fix

lint-fix-allow-dirty: _ensure-mise
    @just clippy-fix --allow-dirty
    @just format-fix

rust-lint-fix-allow-dirty: lint-fix-allow-dirty

# Run the Rust workspace test suite (cargo-nextest).
test *args: _ensure-mise
    @just rust-test {{ args }}

# Rust workspace lint (clippy + rustc warnings-as-errors + rustfmt check).
rust-lint: rustc-lint clippy format

# Rust workspace autofix (clippy --fix + rustfmt).
rust-lint-fix:
    @just clippy-fix
    @just format-fix

rust-pre-commit: _ensure-mise
    #!/usr/bin/env bash
    set -euo pipefail
    if just rust-lint; then
      exit 0
    fi
    echo "rust-lint failed; applying autofixes (clippy --fix + rustfmt). Review, 'git add', and commit again." 1>&2
    just clippy-fix --allow-dirty || true
    just format-fix
    exit 1

pre-commit: _ensure-mise
    @pre-commit run

pre-commit-all: _ensure-mise
    @pre-commit run --all-files

# Lint markdown files with markdownlint.
#
# Usage:
#   just markdownlint docs/src/guides/configuration.md
#   just markdownlint-fix docs/src/guides/configuration.md
markdownlint *files: _ensure-mise
    @pre-commit run markdownlint --hook-stage manual --files {{files}}

markdownlint-fix *files: _ensure-mise
    @pre-commit run markdownlint-fix --hook-stage manual --files {{files}} || \
      pre-commit run markdownlint --hook-stage manual --files {{files}}

alias mdlint-fix := markdownlint-fix
alias mdlint := markdownlint

# lbl development commands. Run `just` for the full list.
#
# Tooling is managed by mise (see mise.toml); `mise run <recipe>` delegates here.

set allow-duplicate-recipes := true

import "justfiles/rust_lint.just"
import "justfiles/rust_cargo.just"

default:
    @just --list

help:
    @just --list

# Run the lbl-server API on the host.
serve *args:
    cargo run -p lbl-server -- --bind 127.0.0.1:8787 {{ args }}

# Run the lbl CLI (e.g. `just lbl catalog show 11352`).
lbl *args:
    cargo run -q -p lbl --bin lbl -- {{ args }}

# Lint the Rust workspace.
lint:
    @just rust-lint

# Apply Rust autofixes.
lint-fix:
    @just rust-lint-fix

# Run the Rust workspace test suite (cargo-nextest).
test *args:
    @just rust-test {{ args }}

# Rust workspace lint (clippy + rustc warnings-as-errors + rustfmt check).
rust-lint: rustc-lint clippy format

# Rust workspace autofix (clippy --fix + rustfmt).
rust-lint-fix:
    @just clippy-fix
    @just format-fix

rust-pre-commit:
    #!/usr/bin/env bash
    set -euo pipefail
    if just rust-lint; then
      exit 0
    fi
    echo "rust-lint failed; applying autofixes (clippy --fix + rustfmt). Review, 'git add', and commit again." 1>&2
    just clippy-fix --allow-dirty || true
    just format-fix
    exit 1

pre-commit:
    @pre-commit run

pre-commit-all:
    @pre-commit run --all-files

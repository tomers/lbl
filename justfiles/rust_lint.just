alias l := lint

# Shared Rust lint helpers.
#
# By default, lint the Cargo project/workspace in the directory that contains the importing
# `justfile`. For lbl, the importing justfile is the repo root, so these lint the whole
# Cargo workspace (a virtual manifest lints every member by default).

_rust-lint-dir:
    @realpath "{{ justfile_directory() }}"

# Rust compiler linting (treat warnings as errors)
rustc-lint:
    @cd "{{ justfile_directory() }}" && \
    RUSTFLAGS="-D warnings" cargo check --all-targets

# Clippy linting (treat warnings as errors)
clippy *args:
    @cd "{{ justfile_directory() }}" && \
    cargo clippy --all-targets {{ args }} -- -D warnings

lint: rustc-lint clippy format

lint-fix *args:
    @just clippy-fix {{ args }}
    @just format-fix

lint-fix-allow-dirty:
    @just lint-fix --allow-dirty

clippy-fix *args:
    @cd "{{ justfile_directory() }}" && \
    cargo clippy --all-targets --fix {{ args }} -- -D warnings

# Stricter clippy for workspaces that need every feature surface (e.g. `required-features` bins).
# Use from a submodule (`mod rl '.../rust_lint.just'`) with `lint: rl::format rl::clippy-workspace-all-features`
# so the top-level `lint` name stays free—`import` would already define `lint` and cannot be redefined.
[private]
clippy-workspace-all-features *args:
    @cd "{{ justfile_directory() }}" && \
    cargo clippy --workspace --all-targets --all-features {{ args }} -- -D warnings

[private]
clippy-workspace-all-features-fix *args:
    @cd "{{ justfile_directory() }}" && \
    cargo clippy --workspace --all-targets --all-features --fix {{ args }} -- -D warnings

# Same as `lint-fix` but for `clippy-workspace-all-features*`. Dependencies, not
# `just clippy-…-fix`, so it also works when this file is only loaded as a `mod`
# (nested `just clippy-…-fix` would not resolve on the top-level justfile).
[private]
lint-fix-workspace-all-features *args: (clippy-workspace-all-features-fix args) format-fix

# Run Rust formatter check
format:
    @cd "{{ justfile_directory() }}" && \
    cargo fmt --all -- --check

# Run Rust formatter and fix formatting issues
format-fix:
    @cd "{{ justfile_directory() }}" && \
    cargo fmt --all

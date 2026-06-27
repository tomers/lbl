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

lint-fix *args:
    @just clippy-fix {{ args }}
    @just format-fix

lint-fix-allow-dirty:
    @just lint-fix --allow-dirty

clippy-fix *args:
    @cd "{{ justfile_directory() }}" && \
    cargo clippy --all-targets --fix {{ args }} -- -D warnings

# Run Rust formatter check
format:
    @cd "{{ justfile_directory() }}" && \
    cargo fmt --all -- --check

# Run Rust formatter and fix formatting issues
format-fix:
    @cd "{{ justfile_directory() }}" && \
    cargo fmt --all

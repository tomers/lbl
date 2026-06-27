## Shared Rust Cargo helpers.

_rust-cargo-dir:
    @realpath "{{ justfile_directory() }}"

fmt:
    @cd $(just _rust-cargo-dir) && \
    cargo fmt

check:
    @cd $(just _rust-cargo-dir) && \
    cargo check

test *args:
    @cd $(just _rust-cargo-dir) && \
    cargo nextest run {{ args }}

rust-test *args:
    @cd $(just _rust-cargo-dir) && \
    cargo nextest run {{ args }}

build:
    @cd $(just _rust-cargo-dir) && \
    cargo build

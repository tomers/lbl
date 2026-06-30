# Repo-wide dependency maintenance (Cargo).
#
# Invoke as:
#   just maintenance cargo-upgrade
#   just maintenance cargo-upgrade-all

_:
    @just --list

# Upgrade workspace Cargo.toml deps (cargo-edit) and refresh Cargo.lock.
# Requires: cargo install cargo-edit  (or: mise install)
cargo-upgrade:
    #!/usr/bin/env bash
    set -euo pipefail
    root="$(realpath "{{ source_directory() }}/..")"
    cd "$root"
    echo "==> cargo upgrade (workspace root)"
    cargo upgrade --incompatible allow
    cargo update

alias cargo-upgrade-all := cargo-upgrade
alias deps-upgrade-all := cargo-upgrade

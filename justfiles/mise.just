# Dev tooling entrypoint checks (see mise.toml).

# Warn when mise is missing. Non-fatal so ad-hoc PATH setups still work.
[private]
_ensure-mise:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v mise >/dev/null 2>&1; then
      exit 0
    fi
    yellow=$'\033[1;33m'
    cyan=$'\033[36m'
    bold=$'\033[1m'
    reset=$'\033[0m'
    printf '%b%bwarning:%b %bmise is not installed — lbl pins dev tools (just, pre-commit, …) in mise.toml.%b\n' \
      "$yellow" "$bold" "$reset" "$yellow" "$reset" >&2
    printf '  Install:  %bhttps://mise.jdx.dev/getting-started.html%b\n' "$cyan" "$reset" >&2
    printf '  Then run: %bmise install && pre-commit install --install-hooks%b\n' "$cyan" "$reset" >&2

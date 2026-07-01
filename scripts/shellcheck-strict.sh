#!/usr/bin/env bash
# Run shellcheck from each script's directory so `source=` / relative `.` paths resolve.
set -euo pipefail

resolve_shellcheck() {
  if [[ -n "${SHELLCHECK:-}" && -x "${SHELLCHECK}" ]]; then
    printf '%s\n' "${SHELLCHECK}"
    return 0
  fi
  if command -v shellcheck >/dev/null 2>&1; then
    command -v shellcheck
    return 0
  fi
  local candidate
  candidate="$(
    find "${PRE_COMMIT_HOME:-${XDG_CACHE_HOME:-$HOME/.cache}/pre-commit}" \
      -path '*/py_env-python*/bin/shellcheck' -type f 2>/dev/null | head -1
  )"
  if [[ -n "$candidate" && -x "$candidate" ]]; then
    printf '%s\n' "$candidate"
    return 0
  fi
  echo "shellcheck not found (install via mise: mise install)" >&2
  return 1
}

if [[ $# -eq 0 ]]; then
  echo "usage: $0 <shell-script>..." >&2
  exit 2
fi

SC="$(resolve_shellcheck)"
SC_ARGS=(-x --severity=style)
exit_code=0

for file in "$@"; do
  [[ -f "$file" ]] || continue
  dir="$(dirname "$file")"
  base="$(basename "$file")"
  if ! (cd "$dir" && "$SC" "${SC_ARGS[@]}" "$base"); then
    exit_code=1
  fi
done

exit "$exit_code"

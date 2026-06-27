# 0006 — Layered configuration with figment

- Status: Accepted

## Context

Users configure `lbl` from several places: defaults, system/user/project files,
environment variables, and CLI flags. We want idiomatic precedence and the
ability to show *where* each effective value came from.

## Decision

Use `figment` to merge sources in precedence order
(defaults < system < user < project < env `LBL_*` < CLI). Expose
`describe_sources()` for provenance. Persist user-owned printers in a separate
`printers.toml` via a `ProfileStore`, decoupled from the merged config, so a
disconnected printer keeps its configuration.

## Consequences

- Familiar, predictable configuration behavior.
- Provenance is exposed via `GET /api/config/sources`.
- Two concerns (settings vs. owned devices) are stored separately and evolve
  independently.

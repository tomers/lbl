# 0001 — Modular pipeline of composable tools

- Status: Accepted

## Context

Label printing involves several distinct concerns: content authoring,
templating, layout/transpilation, rasterization, dithering, protocol encoding,
spooling, and transport. We want each to be independently testable, reusable,
and replaceable, and we want power users to script the flow with Unix pipes.

## Decision

Split the toolchain into a pipeline of small, single-purpose stages driven by a
top-level `lbl` orchestrator. Each stage is **both** a library crate (`lbl-*`)
and a standalone binary (`lbl-*`) that reads stdin and writes stdout. The
orchestrator composes them into `print`/`preview` flows and also exposes them as
subcommands.

## Consequences

- Clear contracts between stages (see Data Formats & Contracts).
- Easy unit testing per stage; the binaries are thin shells over libraries.
- Users can start/stop at any stage and pipe between tools.
- Slightly more boilerplate (a crate + a bin per stage) — accepted for the
  modularity gained.

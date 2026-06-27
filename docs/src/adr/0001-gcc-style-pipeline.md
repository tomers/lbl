# 0001 — GCC-style modular pipeline

- Status: Accepted

## Context

Label printing involves several distinct concerns: content authoring,
templating, layout/transpilation, rasterization, dithering, protocol encoding,
spooling, and transport. We want each to be independently testable, reusable,
and replaceable, and we want power users to script the flow with Unix pipes.

## Decision

Model the toolchain after `gcc`: a top-level `lbl` orchestrator drives a set of
single-purpose stages. Each stage is **both** a library crate (`lbl-*`) and a
standalone binary (`lbl-*`) that reads stdin and writes stdout. The orchestrator
composes them into `print`/`preview` flows and also exposes them as
subcommands.

## Consequences

- Clear contracts between stages (see Data Formats & Contracts).
- Easy unit testing per stage; the binaries are thin shells over libraries.
- Users can start/stop at any stage and pipe between tools.
- Slightly more boilerplate (a crate + a bin per stage) — accepted for the
  modularity gained.

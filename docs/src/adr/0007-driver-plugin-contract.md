# 0007 — Driver grouping & plugin contract

- Status: Accepted

## Context

We support proprietary (DYMO) and standard (ESC/POS, ZPL, TSPL) protocols, and
want adding a new one to be isolated and low-risk. Drivers should be uniform to
the rest of the toolchain.

## Decision

Define a minimal `Driver` trait in `lbl-driver-api`: given a `MonoBitmap` and an
`EncodeContext` (job + printer capabilities), produce protocol bytes. Group all
driver crates under `crates/drivers/` (`lbl-driver-dymo`, `-escpos`, `-zpl`,
`-tspl`, plus the shared `-api`). `lbl-encode` owns a `Registry` keyed by
`Protocol` and selects a driver at encode time.

## Consequences

- A new protocol = one small crate implementing one trait + one registration
  line; no changes elsewhere.
- Drivers are independently unit-tested against known byte sequences.
- Capabilities (e.g. cut support) are passed in, so drivers stay stateless and
  pure.

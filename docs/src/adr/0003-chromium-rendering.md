# 0003 — Chromium for rendering

- Status: Accepted

## Context

HTML input (ADR-0002) requires a faithful renderer that supports modern CSS
(flexbox), web fonts, and running JS (for QR/barcode libraries).

## Decision

Render with headless Chromium driven over the DevTools Protocol via
`chromiumoxide` (in-process, default). Provide an alternative `SidecarBackend`
that drives an external Node/Playwright process behind the same `RenderBackend`
trait, for environments that prefer it.

## Consequences

- High-fidelity rendering, including JS-rendered QR/barcodes and flex layouts.
- Requires a Chromium/Chrome binary at runtime; the `chromium` feature can be
  disabled to build without it (then use the sidecar).
- A single browser instance is reused across a batch for speed.
- Rendering is the heaviest stage; it is isolated behind a trait so it can be
  swapped or mocked.

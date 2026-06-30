# Architecture Overview

This document describes the structure and rationale of the `lbl` toolchain.

## Goals

1. **Modularity** — every pipeline stage is independently usable as a library
   and a binary, and easy to test, replace, or extend.
2. **Format flexibility** — HTML in, raster out; data in JSON/TOML/YAML; output
   for several printer protocols.
3. **Quality** — photographic content survives the trip to a 1-bit thermal head
   via two-pass rendering and photo-aware dithering.
4. **Operability** — discovery, configuration, persistence, and a spooler that
   tolerates disconnects.

## The pipeline at a glance

```text
            ┌─────────┐   ┌──────────────────┐   ┌──────────────────┐
  text ───▶ │ lbl-text│   │   lbl-template   │   │ lbl-transpile-   │
  data ───▶ │         │──▶│ (data + template │──▶│ html             │
  html ───▶ │         │   │  → N authoring   │   │ (custom elems,   │
            └─────────┘   │  HTML labels)    │   │  flex → browser  │
                          └──────────────────┘   │  HTML; print/    │
                                                  │  preview)        │
                                                  └────────┬─────────┘
                                                           │
                            ┌──────────────┐   ┌───────────▼───────┐
                            │  lbl-dither  │◀──│     lbl-render     │
                            │ (1-bit,      │   │ (HTML → raster,    │
                            │  photo-aware)│   │  two-pass)         │
                            └──────┬───────┘   └────────────────────┘
                                   │
                          ┌────────▼────────┐   ┌────────────┐   ┌────────────┐
                          │   lbl-encode    │──▶│  lbl-spool │──▶│ lbl-device │──▶ printer
                          │ (driver select) │   │ (queue,    │   │ (USB / TCP)│
                          └─────────────────┘   │  retry)    │   └────────────┘
                                                └────────────┘
```

The `lbl` binary is the orchestrator. It composes these stages into the `print`
and `preview` flows, and also exposes the individual stages as subcommands.

## Stages

| Stage | Crate | Responsibility |
| ----- | ----- | -------------- |
| Text front-end | `lbl-text` | Plain text/CLI → authoring HTML (inline `{{…}}` directives, `--raw`) |
| Preprocessor | `lbl-template` | Render data through a template → N authoring HTML labels; fetch/inline resources |
| Transpiler | `lbl-transpile-html` | Expand `<qr>`/`<barcode>`/flex into browser-ready HTML; print vs preview |
| Rasterizer | `lbl-render` | HTML → raster image, two-pass (supersample then downscale) |
| Ditherer | `lbl-dither` | Raster → 1-bit `MonoBitmap`, photo-aware |
| Encoder | `lbl-encode` | Select a driver by protocol, encode `MonoBitmap` → protocol bytes |
| Drivers | `drivers/lbl-driver-*` | Protocol-specific encoders (DYMO, NIIMBOT, ESC/POS, ZPL, TSPL) |
| Spooler | `lbl-spool` | Job queue, sequential dispatch, per-item cut, retry, disconnect handling |
| Device | `lbl-device` | Discovery (USB) and transport (USB bulk / TCP / bidirectional serial) |
| Catalog | `lbl-catalog` | Known media SKUs and printer compatibility |
| Config | `lbl-config` | Layered configuration + printer profile persistence |
| Core | `lbl-core` | Shared types: units, geometry, media, printer, job, `MonoBitmap` |
| Orchestrator | `lbl` | Runs the pipeline; `print`/`preview` flows + stage subcommands |
| API | `lbl-server` | HTTP API for programmatic access and integrations |

## Data contracts between stages

Stages communicate through a few stable, inspectable formats:

- **Authoring HTML** — a `<div class="lbl-label">` document using custom
  `<qr>` / `<barcode>` elements and flex utility classes.
- **Browser-ready HTML** — standard HTML with libraries/CSS injected.
- **Raster** — PNG (RGBA) between render and dither.
- **`MonoBitmap` / PBM (P4)** — 1-bit packed image (MSB-first, `1` = ink); the
  driver hand-off format.
- **Protocol bytes** — the final printer-native stream.

See [Data Formats & Contracts](./reference/contracts.md) for details.

## Rendering quality strategy

Thermal label heads are 1-bit. Naively thresholding a photo destroys it. `lbl`:

1. renders the page at `supersample×` the device resolution (the first pass),
2. downscales with a Lanczos3 filter to the exact device dots (anti-aliased),
3. dithers with photo-aware Floyd-Steinberg, which keeps text/line art crisp
   (near-pure pixels are hard-thresholded and excluded from error diffusion)
   while diffusing photographic mid-tones.

The **`supersample`** factor is user-configurable (`--supersample`, config,
API). It controls both the high-res raster pass and how millimetre style sizes
are converted to CSS pixels during transpilation. See
[Rendering Quality & Supersampling](./guides/rendering-quality.md) for defaults,
tuning guidance, and template-authoring notes.

See [ADR-0004](./adr/0004-two-pass-dither.md).

## Configuration & persistence

Configuration is merged from many sources in idiomatic precedence
(defaults < system < user < project < env < CLI) using `figment`, with
provenance tracking so tools can show *which layer* set each value. Printers are
persisted separately so a disconnected device keeps its desired configuration.
See [Configuration](./guides/configuration.md) and
[ADR-0006](./adr/0006-layered-config.md).

## Extensibility

New protocols are added by implementing the `Driver` trait in a small crate
under `crates/drivers/` and registering it in `lbl-encode`. See
[Writing a Driver](./drivers/authoring.md) and
[ADR-0007](./adr/0007-driver-plugin-contract.md).

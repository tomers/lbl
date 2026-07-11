# Crates

The workspace is a Cargo monorepo. Library crates double as binaries where it
makes sense.

| Crate | Path | Binary | Summary |
| ----- | ---- | ------ | ------- |
| `lbl-core` | `crates/lbl-core` | — | Shared types: units, geometry, media, printer, job, `MonoBitmap` |
| `lbl-config` | `crates/lbl-config` | `lbl-config` | Layered config (figment) + printer profile persistence |
| `lbl-catalog` | `crates/lbl-catalog` | `lbl-catalog` | Media SKU database + compatibility |
| `lbl-text` | `crates/lbl-text` | `lbl-text` | Text/CLI → authoring HTML |
| `lbl-template` | `crates/lbl-template` | `lbl-template` | Data + template → labels; resource inlining |
| `lbl-transpile-html` | `crates/lbl-transpile-html` | `lbl-transpile-html` | Authoring → browser-ready HTML |
| `lbl-render` | `crates/lbl-render` | `lbl-render` | HTML → raster, two-pass (Chromium/sidecar) |
| `lbl-dither` | `crates/lbl-dither` | `lbl-dither` | Raster → 1-bit, photo-aware |
| `lbl-encode` | `crates/lbl-encode` | `lbl-encode` | Driver registry + protocol selection |
| `lbl-driver-api` | `crates/drivers/lbl-driver-api` | — | The `Driver` plugin contract |
| `lbl-driver-dymo` | `crates/drivers/lbl-driver-dymo` | — | DYMO drivers: LabelManager tape (`dymo`) + LabelWriter 550 raster (`dymo-lw`) |
| `lbl-driver-escpos` | `crates/drivers/lbl-driver-escpos` | — | ESC/POS driver |
| `lbl-driver-esclabel` | `crates/drivers/lbl-driver-esclabel` | — | Epson ESC/Label (ColorWorks) |
| `lbl-driver-zpl` | `crates/drivers/lbl-driver-zpl` | — | ZPL driver |
| `lbl-driver-tspl` | `crates/drivers/lbl-driver-tspl` | — | TSPL driver |
| `lbl-driver-slcs` | `crates/drivers/lbl-driver-slcs` | — | Bixolon SLCS driver |
| `lbl-driver-ezpl` | `crates/drivers/lbl-driver-ezpl` | — | Godex EZPL driver |
| `lbl-driver-sbpl` | `crates/drivers/lbl-driver-sbpl` | — | SATO SBPL driver |
| `lbl-driver-dpl` | `crates/drivers/lbl-driver-dpl` | — | Honeywell / Citizen DPL driver |
| `lbl-driver-tpcl` | `crates/drivers/lbl-driver-tpcl` | — | Toshiba TEC TPCL driver |
| `lbl-driver-niimbot` | `crates/drivers/lbl-driver-niimbot` | — | NIIMBOT packet driver (`niimbot`; D11/D110 family) |
| `lbl-driver-file` | `crates/drivers/lbl-driver-file` | — | Virtual printer → file (raster: png/bmp/tiff/gif/pbm; vector: pdf via orchestrator) |
| `lbl-driver-console` | `crates/drivers/lbl-driver-console` | — | Console preview → terminal art (`console`) |
| `lbl-device` | `crates/lbl-device` | `lbl-device` | USB/network discovery + transport |
| `lbl-spool` | `crates/lbl-spool` | `lbl-spool` | Internal print spooler |
| `lbl` | `crates/lbl` | `lbl` | Orchestrator |
| `lbl-server` | `crates/lbl-server` | `lbl-server` | HTTP API for programmatic access |

Generate API docs with:

```bash
cargo doc --workspace --no-deps --open
```

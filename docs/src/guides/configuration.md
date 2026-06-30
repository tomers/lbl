# Configuration

`lbl` uses layered configuration with idiomatic precedence (lowest to highest):

1. Built-in defaults
2. System file — `/etc/lbl/config.toml`
3. User file — `~/.config/lbl/config.toml`
4. Project file — `./lbl.toml`
5. Environment — `LBL_*` (nested with `__`, e.g. `LBL_RENDER__SUPERSAMPLE=4`)
6. Explicit CLI flags

## Inspect

```bash
lbl-config show      # effective merged config (JSON)
lbl-config sources   # which layer supplied each value
lbl-config paths     # resolved file locations
```

The provenance view is exposed by `GET /api/config/sources` for HTTP clients.

## Keys

```toml
[general]
default_printer = "my-dymo"     # matches a saved profile id
# cache_dir = "/var/cache/lbl"

[render]
supersample = 3   # high-res first pass factor (>= 1); see Rendering Quality guide
dither = "floyd-steinberg"      # auto | floyd-steinberg | ordered | none
use_sidecar = false
orientation = "landscape"       # portrait | landscape (default: landscape)

[catalog]
affiliate_enabled = true
# affiliate_tag = "mytag"
extra_paths = ["./my-catalog.toml"]
```

`[render] supersample` applies to `lbl print` (unless overridden by
`--supersample`) and is exposed on the HTTP print API. Preview rasterization
uses a fixed factor of `2` for speed. See
[Rendering Quality & Supersampling](./rendering-quality.md) for what the factor
does, how it interacts with style sizing, and tuning advice.

`[render] orientation` sets the default layout orientation for `lbl print`
(override per-run with `--orientation`, and add quarter-turns with
`--rotate-cw` / `--rotate-ccw`). It defaults to `landscape` because stripe
labels are usually printed along their longer dimension. Orientation changes
only how content is laid out and rotated onto the head; it never changes the
media's physical width or feed length.

## Style (fonts, QR, barcodes)

Physical sizes for text and codes, in millimetres:

```toml
[style]
font_size_mm = 2.0
qr_size_mm = 15.0
# QR error correction (redundancy): L (~7%), M (~15%), Q (~25%), H (~30%)
qr_error_correction = "M"
# Quiet zone around the QR, in modules (0 = none)
qr_margin = 0
qr_dark = "#000000"
qr_light = "#ffffff"
padding_mm = 2.0
border_width_mm = 0.0
# How the label root fills the media: auto (fill fixed-length media), fill, content
label_fit = "auto"
```

Override per run with `--font-size-mm`, `--qr-size-mm`, `--qr-ec`, `--qr-margin`,
`--qr-dark`, `--qr-light`, and `--label-fit` (`auto`, `fill`, or `content`).
Per-code overrides in authoring HTML:
`<qr ec="H" margin="2">payload</qr>`.

## Print defaults (`lbl print`)

Runtime options for `lbl print` can live in config or environment so scripts
do not need long flag lists. CLI flags always win when explicitly passed.

```toml
[print]
confirm = true
protocol = "niimbot"
bluetooth = "D110"
dither = "auto"
copies = 1
backend = "chromium"
# niimbot_task = "standard"
```

Environment (same keys, nested with `__`):

```bash
export LBL_PRINT__CONFIRM=1
export LBL_PRINT__PROTOCOL=niimbot
export LBL_PRINT__BLUETOOTH=D110
lbl print --text "Hello" --media niimbot-12x22
```

Supported `[print]` / `LBL_PRINT__*` keys: `confirm`, `debug`, `cut`,
`supports_cut`, `copies`, `dither`, `protocol`, `backend`, `bluetooth`,
`serial`, `usb`, `network`, `niimbot_task`, `media_type`.

## Printer profiles

User-owned printers are persisted separately (in `printers.toml`) so a
disconnected printer keeps its desired configuration. Manage them via the API
(`/api/printers/profiles`).

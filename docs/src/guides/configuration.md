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
lbl config show      # effective merged config (JSON)
lbl config sources   # which layer supplied each value
lbl config paths     # resolved paths, existence, and entry counts

lbl-config show      # same (standalone binary)
lbl-config sources
lbl-config paths
```

The provenance view is exposed by `GET /api/config/sources` for HTTP clients.

## Keys

```toml
[general]
default_printer = "my-dymo"     # matches a saved profile id
# cache_dir = "/var/cache/lbl"

[render]
supersample = 4   # high-res first pass factor (>= 1); see Rendering Quality guide
efficiency_warn_below = 0.55   # warn when print time ÷ total time falls below 55%
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
element_gap_mm = 4.0
border_width_mm = 0.0
# How the label root fills the media: auto (fill fixed-length media), fill, content
label_fit = "auto"
# Cross-axis alignment when the media width is known: start, center, end
label_align = "center"
# Main-axis alignment in fill mode: start, center, end
label_valign = "center"
# Fit-box scale in fill mode (1.0 = full media; 0.8 or "80%" = 80% height/width)
label_fit_scale = 1.0
# Calibration inset from the physical sticker edge (mm); more specific fields override
media_inset_mm = 0.0
# media_inset_horizontal_mm = 1.0
# media_inset_vertical_mm = 2.0
# media_inset_start_mm = 3.0      # top in portrait (main-axis start)
# media_inset_end_mm = 1.0        # bottom
# media_inset_cross_start_mm = 1.5  # left
# media_inset_cross_end_mm = 1.5    # right
```

### Padding and insets

Every label gets **inner padding** automatically: transpilation sets
`padding` on the root `.lbl-label` from `padding_mm` (default **2.0 mm** on
all sides). You do not add padding in the template — it is applied even for
plain `--text` and text templates. Set `padding_mm = 0` (or pass
`--padding-mm 0`) when you want content flush to the label edge, e.g. on small
NIIMBOT tape.

**Media inset** (`media_inset_*`) is separate: it shrinks the layout shell
*inside the physical sticker* to compensate for printer feed/cut tolerance. It
does not replace inner padding — both can be set together.

```toml
[style]
padding_mm = 0          # no inner gutter (good for 12×30 mm labels)
border_width_mm = 0.0   # optional outline around .lbl-label (0 = off)
media_inset_mm = 0.5    # keep art off the die-cut edge
```

```bash
# Per run
lbl print --text "Hi" --media 12x30 --padding-mm 0 --protocol console

# Project default via environment
export LBL_STYLE__PADDING_MM=0
```

Override per run with `--font-size-mm`, `--qr-size-mm`, `--qr-ec`, `--qr-margin`,
`--qr-dark`, `--qr-light`, `--padding-mm`, `--element-gap-mm`, `--border-mm`, `--label-fit`
(`auto`, `fill`, or `content`), `--label-align` (`start`, `center`, or `end`),
`--label-valign` (`start`, `center`, or `end`), `--label-fit-scale` (`0.8`,
`80%`, …), and `--media-inset-mm` / `--media-inset-horizontal-mm` /
`--media-inset-vertical-mm` / `--media-inset-start-mm` / `--media-inset-end-mm`
/ `--media-inset-cross-start-mm` / `--media-inset-cross-end-mm`.
Per-code overrides in authoring HTML:
`<qr ec="H" margin="2">payload</qr>`.

## Print defaults (`lbl print`)

Runtime options for `lbl print` can live in config or environment so scripts
do not need long flag lists. CLI flags always win when explicitly passed.

```toml
[print]
# Example: default to vector PDF file export
protocol = "virtual"
export_mode = "vector"   # raster | vector | pdf (aliases: bitmap/image → raster)
media_type = "png"       # raster only: png | bmp | tiff | gif | pbm

# Example: default to a Bluetooth NIIMBOT printer instead
# protocol = "niimbot"
# bluetooth = "D110"
# dither = "auto"
# copies = 1
# backend = "chromium"
# niimbot_task = "standard"
```

DYMO LabelWriter 550-series engine options live under the protocol bag:

```toml
[print.driver.dymo]
output_mode = "graphics"   # text | graphics
speed = "high"             # normal | high
```

`export_mode = "vector"` makes `--protocol virtual` emit PDF without passing
`--export-mode` on every run. `media_type` is ignored when `export_mode` is
`vector`.

Environment (same keys, nested with `__`):

```bash
export LBL_PRINT__PROTOCOL=virtual
export LBL_PRINT__EXPORT_MODE=vector
lbl print --text "Hello {{qr:https://x}}" --media 30252 --file hello.pdf

export LBL_PRINT__PROTOCOL=niimbot
export LBL_PRINT__BLUETOOTH=D110
lbl print --text "Hello" --media niimbot-12x22

export LBL_PRINT__DRIVER__DYMO__OUTPUT_MODE=graphics
export LBL_PRINT__DRIVER__DYMO__SPEED=high
```

CLI overrides use the same dotted paths:

```bash
lbl print --driver-opt dymo.output_mode=graphics --driver-opt dymo.speed=high ...
```

Supported `[print]` / `LBL_PRINT__*` keys: `confirm`, `debug`, `cut`,
`supports_cut`, `copies`, `dither`, `protocol`, `export_mode`, `media_type`,
`backend`, `bluetooth`, `serial`, `usb`, `network`, `niimbot_task`, plus
nested `[print.driver.*]` / `LBL_PRINT__DRIVER__*` (currently `dymo.output_mode`,
`dymo.speed`).

## Printer profiles

User-owned printers are persisted separately (in `printers.toml`) so a
disconnected printer keeps its desired configuration. Manage them via the API
(`/api/printers/profiles`).

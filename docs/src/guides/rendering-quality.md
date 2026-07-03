# Rendering Quality & Supersampling

Thermal label printers are **1-bit** devices: each dot is either ink or no ink.
Getting crisp text, scannable barcodes, and recognizable photos on the same
label requires rendering at higher resolution first, then reducing to the exact
printer dot grid before dithering.

**Supersampling** (`supersample`, sometimes written `SUPERSAMPLE` in tests) is
the knob that controls that high-resolution first pass.

## What supersampling does

`lbl-render` uses a **two-pass** strategy (see
[ADR-0004](../adr/0004-two-pass-dither.md)):

1. **First pass (high-res)** — Headless Chromium rasterizes the label HTML at
   `supersample ×` the target width and height in device dots. For a 57 mm
   label at 300 DPI with `supersample = 3`, the viewport is roughly three times
   wider and taller than the final image before dithering.
2. **Downscale** — A Lanczos3 filter shrinks that RGBA image to the exact device
   dimensions. Edges are anti-aliased and photos retain more tonal detail than
   they would if rendered directly at 1× resolution.
3. **Dither** — The downscaled grayscale image is converted to 1-bit using the
   chosen algorithm (`auto`, Floyd–Steinberg, ordered, or none).

Higher supersample → smoother curves and better photos → **more pixels to render**
(first-pass cost scales roughly with `supersample²`).

```text
  Authoring HTML
        │
        ▼
  lbl-transpile-html  (QR/barcode/font sizes resolved using supersample)
        │
        ▼
  lbl-render pass 1   viewport = width_dots × supersample
        │             (Chromium screenshot at high-res)
        ▼
  lbl-render pass 2   Lanczos3 → width_dots × height_dots RGBA
        │
        ▼
  lbl-dither          → 1-bit MonoBitmap / PBM
```

## Supersample also sets the HTML viewport scale

Supersampling is **not** only a post-processing step. The transpiler converts
physical sizes (millimetres from config / `--font-size-mm`, `--qr-size-mm`, …)
into CSS pixels using:

```text
px_per_mm = dpi × supersample / 25.4
```

So QR codes, barcodes, padding, borders, and base font size in the browser-ready
HTML all grow with `supersample`. The high-res Chromium viewport matches those
pixel sizes; after downscaling, elements land on the correct physical size on
the printed label.

**Important for template authors:** raw CSS `mm`/`cm`/`in` units inside your
HTML follow the **browser’s reference pixel density** (~96 DPI), *not* this
scale. Prefer `em` (relative to the label’s configured font size), percentages,
or the `<qr>` / `<barcode>` elements and style config instead of hard-coded `mm`
in inline styles. See the identity-card fixture under `crates/lbl/tests/fixtures/`
for an example.

## Defaults

| Context | Default | Configurable? |
| ------- | ------- | ------------- |
| `lbl print` | `4` | `--supersample`, `[render] supersample`, `LBL_RENDER__SUPERSAMPLE` |
| `lbl-render` | `4` | `--supersample` |
| HTTP `POST /api/print` | `4` | JSON `supersample` field |
| `lbl preview --render` | `2` (fixed) | No — preview PNGs prioritize speed |
| Web UI preview (Studio) | `2` (fixed) | No |

Values below `1` are treated as `1` (no supersampling).

## How to set it

### CLI — full print pipeline

```bash
lbl print --text "Hello" --media 11352 --protocol zpl --network 192.168.1.50:9100 --supersample 4
```

### CLI — render stage only

```bash
lbl-transpile-html --mode print < label.html \
  | lbl-render --width-dots 672 --height-dots 1200 --supersample 4 \
  > label.png
```

### Configuration file

```toml
[render]
supersample = 4
dither = "auto"
```

Inspect the merged value:

```bash
lbl-config show | jq .render.supersample
```

Environment override (same precedence as other config keys):

```bash
export LBL_RENDER__SUPERSAMPLE=4
lbl print …   # uses 4 unless --supersample is also passed
```

CLI `--supersample` wins over config and environment.

### HTTP API

```json
POST /api/print
{
  "text": "Hello",
  "media": "11352",
  "protocol": "zpl",
  "network": "192.168.1.50:9100",
  "supersample": 4
}
```

## Choosing a value

| Factor | Guidance |
| ------ | -------- |
| **Text & barcodes only** | `3`–`4` (default `4`) is a safe general-purpose choice. |
| **Photos or gradients** | `4` noticeably reduces stair-stepping and dither noise. |
| **Fine detail / small type** | Try `4`–`6`; diminishing returns beyond that for typical 203–300 DPI heads. |
| **Batch size / speed** | Each step up multiplies first-pass pixel count. Large batches on a slow machine may prefer `3`. |
| **Virtual / file output (raster)** | Same rules apply — the PNG or PBM reflects the chosen factor. |
| **Virtual / file output (vector PDF)** | Supersample and dither are **not used** — see below. |

There is no single “best” value: it trades **quality vs render time**. When
debugging quality, compare outputs at `3`, `4`, and `6` with
`--protocol virtual --file out.png` before printing hardware labels.

## Raster vs vector virtual export

The virtual printer (`--protocol virtual`) supports two export modes:

| | **Raster** (default) | **Vector** |
| --- | --- | --- |
| **Flag** | `--export-mode raster` (default) | `--export-mode vector` |
| **Use when** | Preview how ink will look after 1-bit dithering | Share/print PDFs; sharp text & codes at any zoom |
| **Pipeline** | transpile → render → dither → encode image | transpile → Chromium PrintToPdf |
| **Supersample** | Yes (same as hardware) | No |
| **Dither** | Yes | No |
| **Printer DPI** | Sets dot grid and style px scale | Page size only (mm from media); layout uses 300 CSS dpi |
| **Output** | PNG / BMP / TIFF / GIF / PBM | PDF |

```bash
# Vector PDF on catalog media
lbl print --markdown "# Invite\n\n{{qr:https://example.com/invite}}" \
  --printer LW550 --media 30252 --orientation portrait \
  --protocol virtual --export-mode vector --file invite.pdf

# Raster PNG for comparison (same label, dithered)
lbl print --markdown "# Invite\n\n{{qr:https://example.com/invite}}" \
  --printer LW550 --media 30252 --orientation portrait \
  --protocol virtual --file invite.png
```

**Layout reference DPI:** vector export converts millimetre style sizes to CSS
pixels at **300 dpi** (`CSS_LAYOUT_REFERENCE_DPI` in `lbl-core`). This affects
browser layout math only — text, QR, and barcodes remain **vectors** in the PDF
and scale cleanly for professional print. Embedded `<img>` / `{{image:…}}`
content is still raster inside the PDF.

QR codes and barcodes are drawn as **SVG** in the transpiled HTML (then either
screenshot or embedded as vectors in the PDF).

## Preprocessing warnings

Before rendering, `lbl print` estimates preprocessing cost from label count,
device dot dimensions, supersample factor, and (via `sysinfo`) this machine's
CPU core count and installed RAM. When the adjusted weight exceeds an internal
threshold, a yellow callout on stderr suggests mitigations — typically lowering
`--supersample` or printing fewer labels with `--first` / `--last` / `--indices`.

During **batch** jobs, a similar reminder appears every **10 seconds** of
accumulated render time so long runs are not silent. These warnings cover
preparation only (HTML → raster → dither → encode), not time spent spooling to
the printer.

After a **hardware** print completes, `lbl print` prints a one-line summary:
total time, preprocess vs print breakdown, feed throughput (mm/s, cm/s, or
mm/min depending on speed), and **print efficiency** (`print time ÷ total
time`). When efficiency falls below `[render] efficiency_warn_below` (default
`0.55`), the same mitigation hints as the pre-flight warning are shown again.
Set the threshold to `0` to disable.

## What supersampling does *not* change

- **Final dot dimensions** — Always determined by media width/length and DPI
  (`--media`, `--width-mm`, `--dpi`). Supersample only affects how those dots
  are *filled*.
- **Preview HTML in the gallery** — Browser preview uses screen CSS; only
  rasterized preview PNGs (`lbl preview --render`) go through the two-pass
  path (at the fixed preview factor of `2`).
- **Dither algorithm** — Controlled separately via `--dither` / `[render] dither`.
- **Driver protocol** — Encoding happens after dither; supersample is invisible
  to ZPL/ESC/POS/DYMO bytes except through image quality.

## Related reading

- [Architecture — Rendering quality strategy](../architecture.md#rendering-quality-strategy)
- [ADR-0004 — Two-pass render + photo-aware dither](../adr/0004-two-pass-dither.md)
- [Configuration](./configuration.md)
- [The Pipeline](../reference/pipeline.md)
- [`lbl-render` crate README](https://github.com/labelle-org/lbl/blob/main/crates/lbl-render/README.md)

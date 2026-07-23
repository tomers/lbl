# Data Formats & Contracts

The stages are decoupled by a handful of stable formats.

## Authoring HTML

The output of `lbl-text`/`lbl-template` and the input to `lbl-transpile-html`.

- Root: `<div class="lbl-label">…</div>` (a full document wrapper is allowed).
- Text blocks: `<div class="lbl-text">…</div>`.
- QR: `<qr>PAYLOAD</qr>`.
- Barcode: `<barcode type="CODE128">DATA</barcode>` (the `type` attribute is the
  symbology; it defaults to `CODE128`). Classic 1D names (`CODE128`, `EAN13`,
  `CODE39`, …) render via JsBarcode. Industrial / postal / GS1 names
  (`PDF417`, `DATAMATRIX`, `AZTEC`, `MAXICODE`, `DATABAR`, `POSTNET`,
  `GS1128`, …) render via bwip-js.
- Images: standard `<img src="…">` (path or URL; `lbl-template` can inline these
  as `data:` URIs).
- Flex utility classes: `lbl-row`, `lbl-col`, `lbl-center`, `lbl-between`,
  `lbl-grow`, `lbl-wrap`, `lbl-justify-*` / `lbl-items-*` (independent main- and
  cross-axis alignment), `lbl-slot` (empty flex child for nested layouts),
  `lbl-frame` (bordered padded box).

On fixed-length media (`label_fit` = `fill` / `auto`), a lone
`<div class="lbl-text">` child is auto-sized at transpile time to the largest
font that fits the printable area (width, height, and line wrapping), with a
CSS fallback when viewport geometry is not known.

## Style sizing (transpiler)

`lbl-transpile-html` injects CSS on `.lbl-label` from `[style]` / CLI flags
(millimetres converted to pixels at the render DPI and supersample factor):

| Key / flag | Default | Effect |
| ---------- | ------- | ------ |
| `padding_mm` / `--padding-mm` | 2.0 mm | Inner gutter between the label box and content (uniform base) |
| `padding_horizontal_mm` / `padding_vertical_mm` | — | Axis overrides (both sides) |
| `padding_top_mm` / `padding_right_mm` / `padding_bottom_mm` / `padding_left_mm` | — | Per-side overrides |
| `border_width_mm` / `--border-mm` | 0 | Optional border drawn around `.lbl-label` |
| `font_size_mm` / `--font-size-mm` | 2.0 mm | Base text size (before auto-fit on lone text blocks) |
| `media_inset_*` / `--media-inset-*` | 0 | Shrink the layout shell inside the physical media edge |

Padding uses the same cascade as media insets (uniform → axis → side). It is
always applied by the pipeline — it is not part of authoring HTML.
See [Configuration](../guides/configuration.md#padding-and-insets).

## Browser-ready HTML

The output of `lbl-transpile-html`. Custom elements are rewritten to placeholder
`<div>`s (`.lbl-qr[data-qr]`, `.lbl-barcode[data-symbology][data-value]`), the
base/flex CSS is inlined, and only the needed JS libraries are referenced. QR
codes render to **SVG**; barcodes render to **SVG** via JsBarcode (1D) or
bwip-js (2D / GS1 / postal).

- **Print mode**: bare, deterministic document for the rasterizer and vector PDF
  export.
- **Preview mode**: wrapped in `.lbl-preview[data-label-index][data-label-count]`
  with screen-friendly chrome, for the gallery.

## Raster (PNG)

`lbl-render` emits an RGBA PNG. The two-pass strategy renders at
`supersample×` the target device dots, then downscales with a Lanczos3 filter
to the exact width/height before dithering. The same factor is used when
converting millimetre style sizes (font, QR, barcode, padding) to CSS pixels
during transpilation.

See [Rendering Quality & Supersampling](../guides/rendering-quality.md).

## Vector PDF

Produced by `lbl print --protocol virtual --export-mode vector` (Chromium
PrintToPdf). Input is browser-ready HTML with an `@page { size: Wmm Hmm }` rule
sized to the configured media. Output is a PDF whose page dimensions match the
physical label; text, QR, and barcode paths stay vector. This format is **not**
interchangeable with PBM or hardware driver hand-off — it is for file export and
professional print workflows.

Embedded images remain raster inside the PDF. Layout uses a fixed 300 CSS dpi
reference (`CSS_LAYOUT_REFERENCE_DPI`); printer `--dpi` sets page size via media
mm, not PDF resolution.

## MonoBitmap / PBM (P4)

The 1-bit hand-off to drivers. Bits are packed **MSB-first**, rows are
byte-aligned (`stride = ceil(width/8)`), and a set bit means **ink**. This maps
exactly onto the binary PBM (`P4`) format, so `lbl-dither` emits PBM and
`lbl-encode` reads it with zero conversion.

## Protocol bytes

The final printer-native stream produced by a driver (DYMO column data,
NIIMBOT row packets, ESC/POS `GS v 0` raster, ZPL `^GFA`, TSPL `BITMAP`, …).
Delivered verbatim by `lbl-device`/`lbl-spool`.

## Batch manifests

- `lbl-template --out-dir` writes `label-NNNN.html` + `manifest.json`.
- `lbl preview --out-dir` writes `preview-NNNN.html` (+ `.png` with `--render`)
  and `gallery.json` (`{count, labels:[{index, html[, png]}]}`).
- Multi-label stdout uses NDJSON (`{"index":N,"html":"…"}` per line).

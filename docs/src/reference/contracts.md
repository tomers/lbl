# Data Formats & Contracts

The stages are decoupled by a handful of stable formats.

## Authoring HTML

The output of `lbl-text`/`lbl-template` and the input to `lbl-transpile-html`.

- Root: `<div class="lbl-label">…</div>` (a full document wrapper is allowed).
- Text blocks: `<div class="lbl-text">…</div>`.
- QR: `<qr>PAYLOAD</qr>`.
- Barcode: `<barcode type="CODE128">DATA</barcode>` (the `type` attribute is the
  symbology; it defaults to `CODE128`).
- Images: standard `<img src="…">` (path or URL; `lbl-template` can inline these
  as `data:` URIs).
- Flex utility classes: `lbl-row`, `lbl-col`, `lbl-center`, `lbl-between`,
  `lbl-grow`, `lbl-wrap`.

## Browser-ready HTML

The output of `lbl-transpile-html`. Custom elements are rewritten to placeholder
`<div>`s (`.lbl-qr[data-qr]`, `.lbl-barcode[data-symbology][data-value]`), the
base/flex CSS is inlined, and only the needed JS libraries are referenced.

- **Print mode**: bare, deterministic document for the rasterizer.
- **Preview mode**: wrapped in `.lbl-preview[data-label-index][data-label-count]`
  with screen-friendly chrome, for the gallery.

## Raster (PNG)

`lbl-render` emits an RGBA PNG. The two-pass strategy renders at
`supersample×` the target device dots, then downscales with a Lanczos3 filter
to the exact width/height before dithering. The same factor is used when
converting millimetre style sizes (font, QR, barcode, padding) to CSS pixels
during transpilation.

See [Rendering Quality & Supersampling](../guides/rendering-quality.md).

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
  + `gallery.json` (`{count, labels:[{index, html[, png]}]}`).
- Multi-label stdout uses NDJSON (`{"index":N,"html":"…"}` per line).

# lbl-render

Render HTML to a raster image sized for printer media, or export vector PDF.

## Two-pass rendering

To preserve photographic quality through the eventual 1-bit dithering, the page
is rendered at high resolution (`supersample` × the target) and then downscaled
with a high-quality Lanczos3 filter to the exact device dimensions. Render big,
shrink smoothly.

The same factor scales millimetre style sizes (font, QR, barcode, padding) into
CSS pixels during transpilation. See `docs/src/guides/rendering-quality.md` for
defaults, tuning, and template-authoring notes.

## Backends

A `RenderBackend` rasterizes the HTML:

- `ChromiumBackend` (default, feature `chromium`): drives a headless Chromium
  in-process via [chromiumoxide](https://docs.rs/chromiumoxide). A single
  browser is reused across a batch. Requires a Chromium/Chrome binary present.
- `SidecarBackend`: drives an external Node/Playwright process. The process
  reads HTML on stdin, gets `LBL_RENDER_WIDTH`/`LBL_RENDER_HEIGHT` from the
  environment, and writes a PNG to stdout. See `sidecar/`.

Build without the in-process browser using `--no-default-features` and select
`--backend sidecar` at runtime.

## Vector PDF export

`RenderBackend::export_pdf` drives Chromium PrintToPdf for the orchestrator's
vector virtual export (`--protocol virtual --export-mode vector`). The HTML is
expected to include an `@page` rule from transpilation; page dimensions come from
configured media in millimetres. Layout uses `CSS_LAYOUT_REFERENCE_DPI` (300) in
`lbl-core`.

## CLI

```bash
cat label.html | lbl-render --width-dots 640 --height-dots 1200 --out label.png
lbl-render label.html --width-dots 640 --supersample 4 --backend sidecar --out label.png
```

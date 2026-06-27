# lbl-render

Render HTML to a raster image sized for printer media.

## Two-pass rendering

To preserve photographic quality through the eventual 1-bit dithering, the page
is rendered at high resolution (`supersample` x the target) and then downscaled
with a high-quality Lanczos3 filter to the exact device dimensions. Render big,
shrink smoothly.

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

## CLI

```bash
cat label.html | lbl-render --width-dots 640 --height-dots 1200 --out label.png
lbl-render label.html --width-dots 640 --supersample 4 --backend sidecar --out label.png
```

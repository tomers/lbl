# lbl-dither

Convert a raster image to a printer's bit depth (1-bit) with photo-aware
dithering, emitting a `MonoBitmap` serialized as binary PBM (P4).

## Algorithms

- `auto` (default): photo-aware Floyd-Steinberg. Near-pure source pixels (text,
  line art) are hard-thresholded and excluded from error diffusion so edges stay
  crisp, while mid-tones (photos) dither smoothly.
- `floyd-steinberg`: plain Floyd-Steinberg error diffusion everywhere.
- `ordered`: Bayer 8x8 ordered dithering (fast, tile-free).
- `none` / `threshold`: hard threshold at `--threshold` (0-255).

PBM packs bits MSB-first with byte-aligned rows and treats `1` as ink, which is
exactly the `MonoBitmap` layout — so it is a zero-conversion hand-off to drivers.

## CLI

```bash
lbl-render label.html --width-dots 640 --out - | lbl-dither --algorithm auto --out label.pbm
lbl-dither photo.png --algorithm ordered --preview-png preview.png --out photo.pbm
```

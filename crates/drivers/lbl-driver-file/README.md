# lbl-driver-file

Virtual printer driver: writes label output to a file instead of hardware.

## Export modes

| Mode | CLI | Pipeline | Output |
| ---- | --- | -------- | ------ |
| **Raster** (default) | `--export-mode raster` | dither → encode image | PNG, BMP, TIFF, GIF, PBM |
| **Vector** | `--export-mode vector` | Chromium PrintToPdf (orchestrator) | PDF |

Raster mode uses this crate's [`FileDriver`](src/lib.rs) to turn a dithered
[`MonoBitmap`](../../lbl-core/src/bitmap.rs) into image bytes. Vector PDF export
bypasses the driver — the orchestrator calls `lbl-render`'s `export_pdf` directly.

## Types

- [`VirtualExportMode`](src/lib.rs) — `raster` or `vector` (aliases: `bitmap`/`image`, `pdf`)
- [`MediaType`](src/lib.rs) — raster formats plus `pdf` (vector only, not via `encode_image`)

## CLI

```bash
# Raster PNG (default virtual export)
lbl print --text "Hi" --width-mm 25 --length-mm 54 \
  --protocol virtual --file label.png

# Vector PDF
lbl print --text "Hi {{qr:x}}" --media 30252 \
  --protocol virtual --export-mode vector --file label.pdf
```

See `docs/src/guides/rendering-quality.md` and `docs/src/reference/cli.md`.

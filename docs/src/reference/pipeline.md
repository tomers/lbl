# The Pipeline

Each stage reads from the previous and can be run on its own. The canonical
order is:

1. **`lbl-text`** — text/CLI → authoring HTML.
2. **`lbl-template`** — data + template → N authoring HTML labels.
3. **`lbl-transpile-html`** — authoring HTML → browser-ready HTML.
4. **`lbl-render`** — HTML → raster (PNG), two-pass.
5. **`lbl-dither`** — raster → 1-bit PBM.
6. **`lbl-encode`** — PBM → protocol bytes (driver selected by protocol).
7. **`lbl-spool` / `lbl-device`** — dispatch bytes to the printer.

## Composing by hand

```bash
lbl-text "Order [[barcode:CODE128:4006381333931]]" \
  | lbl-transpile-html --mode print \
  | lbl-render --width-dots 672 --supersample 3 \
  | lbl-dither --algorithm auto \
  | lbl-encode --protocol zpl --width-mm 56 --length-mm 89 \
  > order.zpl
```

Stages that take optional input read **stdin** when no file argument is given,
and write to **stdout** unless `--out`/`--out-dir` is provided — so they pipe
cleanly.

## Composing via the orchestrator

`lbl print` runs the full chain and dispatches; `lbl preview` runs the
authoring → transpile (preview) portion and optionally rasterizes for a gallery.

```bash
lbl print --template card.html --data people.json \
  --media 99014 --protocol zpl --network 192.168.1.50:9100 --cut --supports-cut
```

## Skipping stages

Because the contracts are explicit, you can start anywhere:

- Already have authoring HTML? Pipe it straight into `lbl-transpile-html`.
- Already have a 1-bit PBM? Pipe it into `lbl-encode`.
- Calibrating margins? Use `lbl-pattern` (or `lbl print --sample-pattern`) to
  emit a fixed test raster straight into `lbl-encode` — no render or dither.
- Want only an image? Stop after `lbl-render` or `lbl-dither --preview-png`.
- Want a vector PDF (no dither)? Use
  `lbl print --protocol virtual --export-mode vector --file out.pdf`
  (orchestrator only; skips render/dither).

## Virtual export modes

When `--protocol virtual` is selected, the orchestrator branches:

| Mode | Steps | Output |
| ---- | ----- | ------ |
| **Raster** (default) | transpile → render → dither → `lbl-driver-file` | PNG / BMP / TIFF / GIF / PBM |
| **Vector** | transpile → Chromium `export_pdf` | PDF sized to media mm |

Vector export skips `lbl-dither` and the file driver; QR/barcodes are SVG in
the transpiled HTML. See [Rendering Quality](../guides/rendering-quality.md#raster-vs-vector-virtual-export).

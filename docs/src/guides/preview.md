# Previewing & the Gallery

Preview mode produces screen-oriented HTML you can review before committing ink.

## CLI

```bash
lbl preview --template card.html --data people.json --out-dir preview/
```

This writes `preview-NNNN.html` per label and a `gallery.json`:

```json
{ "count": 2, "labels": [ { "index": 0, "html": "preview-0000.html" }, … ] }
```

Add `--render` to also rasterize each label to `preview-NNNN.png` (requires a
browser).

## Gallery-friendly output

Each preview label is wrapped in:

```html
<div class="lbl-preview" data-label-index="0" data-label-count="200"> … </div>
```

The `data-label-index` / `data-label-count` attributes let a viewer page
left/right through a batch. HTTP clients (for example `POST /api/preview`) use
this to render a gallery you can step through with Prev/Next or index chips.

## Via the HTTP API

`POST /api/preview` accepts a source body and returns gallery HTML. QR/barcodes
render live via the injected JS libraries in the transpiled output.

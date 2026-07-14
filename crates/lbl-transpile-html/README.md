# lbl-transpile-html

Transpile *authoring HTML* into *browser-ready HTML*.

Authoring HTML uses compact custom concepts that this stage expands:

- `<qr>PAYLOAD</qr>` -> SVG via the QR JS library (`QRCode.toString`, `type: 'svg'`)
- `<barcode type="CODE128">DATA</barcode>` -> SVG via JsBarcode
- flex utility classes: `lbl-row`, `lbl-col`, `lbl-center`, `lbl-between`,
  `lbl-grow`, `lbl-wrap`

It injects the base/flex CSS and only the third-party libraries actually needed.

## Output modes

- `--mode print` (default): a bare, deterministic document for raster rendering
  and vector PDF export (`@page` sizing when `page_size` is set).
- `--mode preview`: a screen-oriented, **gallery-friendly** document. The label
  is wrapped in `.lbl-preview[data-label-index][data-label-count]` so the
  gallery viewers can page through `--index N` of `--count M`.

## Assets

QR/barcode libraries load from a CDN by default; pass `--assets-base /assets`
(or a `file://` path) to serve vendored copies.

Label web fonts come from the self-hosted catalog (`FONT_ASSETS_BASE_URL`,
default `https://fonts.lblprint.com/v1`). Transpile injects `@font-face` rules
for known `data-lbl-font` slugs only — never Google Fonts. See
[`tooling/fonts/README.md`](../../../tooling/fonts/README.md).

## CLI

```bash
cat label.html | lbl-transpile-html --mode print
lbl-transpile-html label.html --mode preview --index 7 --count 200
```

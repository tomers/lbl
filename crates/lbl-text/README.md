# lbl-text

The quick front-end of the pipeline: turn plain text (and directives) into
*authoring HTML* that flows through `lbl-transpile-html` and the rest of the
toolchain.

```bash
lbl-text "hello, world!"
echo "piped text" | lbl-text
```

## Inline mini-syntax (default)

- `{{qr:https://example.com}}` — a QR code
- `{{barcode:CODE128:12345}}` — a barcode (symbology optional: `{{barcode:12345}}`)
- `{{image:./photo.jpg}}` — an image (local path or remote URL)

```bash
lbl-text "ship to {{qr:https://example.com/order/42}}"
```

Unrecognized `{{...}}` is left as literal text.

## Raw mode

```bash
lbl-text --raw "literal {{qr:x}} stays as text"
```

`--raw` disables inline parsing. Flag-based directives still work and are
appended after the text:

```bash
lbl-text --raw "Order #42" --qr "https://example.com/order/42" --barcode "EAN13:4006381333931"
```

## Output

By default emits a full authoring HTML document; pass `--fragment` for just the
`<div class="lbl-label">` element.

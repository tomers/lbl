# Printing Text

The quickest path is `lbl-text` (or `lbl text`), which turns a string into
authoring HTML.

## Inline mini-syntax (default)

```bash
lbl-text "Ship to {{qr:https://example.com/order/42}}"
lbl-text "SKU {{barcode:EAN13:4006381333931}}"
lbl-text "Photo {{image:./logo.png}} next to text"
lbl-text "Total {{size:2:$42.00}}"
```

- `{{qr:…}}` — QR code
- `{{barcode:[SYMBOLOGY:]data}}` — barcode (defaults to `CODE128`)
- `{{image:URI}}` — image (local path or URL)
- `{{size:SCALE:text}}` — text at `SCALE`× the base font size (aliases:
  `font-size`, `fs`; `SCALE` accepts `1.5`, `1.5x`, or `150%`)

Unrecognized `{{…}}` is left as literal text.

`{{size:…}}` is relative, so it scales with the configured base font size
(`[style] font_size_mm` or `--font-size-mm`). It flows inline within Markdown
(`lbl-markdown`); in `lbl-text` it sits on its own line like the other
directives.

## Raw mode

Disable inline parsing when your text legitimately contains `{{ }}`:

```bash
lbl-text --raw "Use {{ }} for templates" --qr "https://example.com"
```

Flag directives (`--qr`, `--barcode`, `--image`) still work and are appended
after the text.

## End to end

```bash
lbl print --text "Hello {{qr:https://example.com}}" \
  --media 11352 --protocol dymo --usb 0922:1001
```

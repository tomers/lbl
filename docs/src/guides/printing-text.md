# Printing Text

The quickest path is `lbl-text` (or `lbl text`), which turns a string into
authoring HTML.

## Inline mini-syntax (default)

```bash
lbl-text "Ship to {{qr:https://example.com/order/42}}"
lbl-text "SKU {{barcode:EAN13:4006381333931}}"
lbl-text "Photo {{image:./logo.png}} next to text"
```

- `{{qr:…}}` — QR code
- `{{barcode:[SYMBOLOGY:]data}}` — barcode (defaults to `CODE128`)
- `{{image:URI}}` — image (local path or URL)

Unrecognized `{{…}}` is left as literal text.

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

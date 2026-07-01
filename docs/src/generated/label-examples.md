# Fixed-size label examples

Each preview highlights a different capability on **fixed-length** media. Commands show
the flags that matter for each example; protocol and output path come from project
config (`lbl.toml`) or the doc generator defaults.

Regenerate with `just doc-examples`.

## Inline QR, barcode, and sizing

Text mini-syntax embeds QR codes, barcodes, and relative font scaling in a plain string — no HTML required.

*DYMO 11352 · 25×54 mm* · [Printing text →](../guides/printing-text.md#inline-mini-syntax-default)

```console
$ lbl print \
  --text 'Ship {{size:1.2:Alice}}{{barcode:L42}}{{qr:https://track/42}}' \
  --qr-size-mm 9 \
  --barcode-height-mm 7 \
  --padding-mm 1
```

<img src="images/inline-syntax.png" alt="Inline QR, barcode, and sizing" width="320"/>
## Markdown input

Headings, emphasis, and inline directives compose on fixed-length media via `--markdown`.

*DYMO 11352 · 25×54 mm* · [Printing text →](../guides/printing-text.md)

```console
$ lbl print \
  --markdown $'# Order 44\n\nShip **fast** to dock 4\n\n{{qr:https://track/42}}' \
  --orientation portrait
```

<img src="images/markdown-label.png" alt="Markdown input" width="320"/>
## Batch HTML template

One HTML layout rendered against a JSON array — here, name badges with QR for two people.

*DYMO 11352 · 25×54 mm* · [Batch printing →](../guides/batch-printing.md#template--data)

[card.html](../../examples/batch-card/card.html)

```html
<div class="lbl-label lbl-row lbl-center">
  <div class="lbl-col">
    <strong>{{ name }}</strong>
    <span>{{ title }}</span>
    <qr>{{ url }}</qr>
  </div>
</div>
```

[people.json](../../examples/batch-card/people.json)

```json
[
  {
    "name": "Alice",
    "title": "Engineer",
    "url": "https://example.com/alice"
  },
  {
    "name": "Bob",
    "title": "Designer",
    "url": "https://example.com/bob"
  }
]
```

```console
$ cd docs/examples/batch-card
$ lbl print \
  --template card.html \
  --template-format html \
  --data people.json \
  --orientation portrait \
  --qr-size-mm 9 \
  --padding-mm 1
```

<img src="images/batch-card.png" alt="Batch HTML template" width="320"/>
## Shell iteration with seq | xargs

`seq | xargs -n1 lbl print` runs once per value — each line becomes `--data`.
`{{ it }}` is the scalar in the template.

*NIIMBOT 12×22 mm @ 203 dpi* · [Batch printing →](../guides/batch-printing.md#shell-iteration-seq-and-xargs)

```console
$ seq 1 3 | xargs -n1 lbl print \
  --template 'Hello user #{{ it }}, my friend' \
  --padding-mm 0 \
  --data
```

<img src="images/shell-template.png" alt="Shell iteration with seq | xargs" width="320"/>
## Zero padding on small tape

Default inner padding is 2 mm; set `--padding-mm 0` (or `padding_mm` in config) so content uses the full printable area.

*NIIMBOT 12×30 mm @ 203 dpi* · [Configuration →](../guides/configuration.md#padding-and-insets)

```console
$ lbl print \
  --text $'Aisle 4\nBin 12\nQty 60' \
  --padding-mm 0
```

<img src="images/zero-padding.png" alt="Zero padding on small tape" width="320"/>
## Config-driven print defaults

With `[print] protocol`, `bluetooth`, and media set in `lbl.toml`, the command only needs the label content.

*NIIMBOT 12×22 mm @ 203 dpi* · [Configuration →](../guides/configuration.md#print-defaults-lbl-print)

```console
$ lbl print \
  --text 'Scan {{qr:https://x/p}}'
```

<img src="images/config-defaults.png" alt="Config-driven print defaults" width="320"/>
## Supersampling for print quality

Side by side: `--supersample 2` (left) vs `--supersample 8` (right).
More render dots before downscaling yield sharper barcodes and small type.

*DYMO 11352 · 25×54 mm* · [Rendering quality →](../guides/rendering-quality.md#how-to-set-it)

```console
# supersample 2 (left)
$ lbl print \
  --text '{{barcode:EAN13:4006381333931}} {{size:0.7:SKU 7788}}' \
  --supersample 2

# supersample 8 (right)
$ lbl print \
  --text '{{barcode:EAN13:4006381333931}} {{size:0.7:SKU 7788}}' \
  --supersample 8
```

<img src="images/supersample.png" alt="Supersampling for print quality" width="320"/>
## NIIMBOT catalog media

Use `--media 12x40` to pick a die-cut size from the bundled catalog instead of passing raw `--width-mm` / `--length-mm`.

*NIIMBOT 12×40 mm @ 203 dpi* · [Printers & media →](../guides/printers-media.md#niimbot)

```console
$ lbl print \
  --media 12x40 \
  --dpi 203 \
  --text 'Ship to {{size:1.2:Dock 4}}{{qr:https://t/42}}' \
  --orientation portrait \
  --padding-mm 0 \
  --qr-size-mm 9
```

<img src="images/niimbot-catalog.png" alt="NIIMBOT catalog media" width="320"/>
## Explicit media dimensions

When no catalog SKU fits, pass `--width-mm`, `--length-mm`, and `--dpi` directly for fixed-size stock.

*56×89 mm @ 300 dpi* · [Printers & media →](../guides/printers-media.md#media)

```console
$ lbl print \
  --text $'Receipt\nItem ×2  $18.00\nTotal      $36.00' \
  --width-mm 56 \
  --length-mm 89 \
  --dpi 300
```

<img src="images/fixed-dimensions.png" alt="Explicit media dimensions" width="320"/>

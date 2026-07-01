# Fixed-size label examples

Each preview highlights a different capability on **fixed-length** media. Commands show
content and layout flags only — media size, DPI, protocol, and output path come from
project config (`lbl.toml`) or environment.

Regenerate with `just doc-examples`.

## Inline QR, barcode, and sizing

Text mini-syntax embeds QR codes, barcodes, and relative font scaling in a plain string — no HTML required.

*DYMO 11352 · 25×54 mm* · [Printing text →](../guides/printing-text.md#inline-mini-syntax-default)

```bash
lbl print --text 'Ship {{size:1.5:Alice}} {{barcode:LBL-42}} {{qr:https://track/42}}'
```

<img src="images/inline-syntax.png" alt="Inline QR, barcode, and sizing" width="320"/>
## Markdown input

Headings, emphasis, and inline directives compose on fixed-length media via `--markdown`.

*DYMO 11352 · 25×54 mm* · [Printing text →](../guides/printing-text.md)

```bash
lbl print --markdown '# Order 44

Ship **fast** to dock 4

{{qr:https://track/42}}'
```

<img src="images/markdown-label.png" alt="Markdown input" width="320"/>
## Batch HTML template

One HTML layout rendered against a JSON array — here, a name badge with QR on portrait die-cut stock.

*DYMO 99014 · 54×101 mm* · [Batch printing →](../guides/batch-printing.md#template--data)

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

```bash
lbl print --template card.html --template-format html --data people.json --one
```

<img src="images/batch-card.png" alt="Batch HTML template" width="320"/>
## Template from shell input

Run `lbl print` once per value: `xargs` appends each line as `--data`, and `{{ it }}` is the scalar.

*NIIMBOT 12×30 mm @ 203 dpi* · [Batch printing →](../guides/batch-printing.md#shell-iteration-seq-and-xargs)

```bash
lbl print --template 'Hello user #{{ it }}, my friend' --data 1 --padding-mm 0
```

<img src="images/shell-template.png" alt="Template from shell input" width="320"/>
## Zero padding on small tape

Default inner padding is 2 mm; set `--padding-mm 0` (or `padding_mm` in config) so content uses the full printable area.

*NIIMBOT 12×30 mm @ 203 dpi* · [Configuration →](../guides/configuration.md#padding-and-insets)

```bash
lbl print --text 'Aisle 4
Bin 12
Qty 60' --padding-mm 0
```

<img src="images/zero-padding.png" alt="Zero padding on small tape" width="320"/>
## Config-driven print defaults

With `[print] protocol`, `bluetooth`, and media set in `lbl.toml`, the command only needs the label content.

*NIIMBOT 12×22 mm @ 203 dpi* · [Configuration →](../guides/configuration.md#print-defaults-lbl-print)

```bash
lbl print --text 'Scan to pair {{qr:https://lbl.example/pair}}'
```

<img src="images/config-defaults.png" alt="Config-driven print defaults" width="320"/>
## Supersampling for print quality

Higher `--supersample` renders at more dots before downscaling — sharper barcodes and small type on 1-bit heads.

*DYMO 11352 · 25×54 mm* · [Rendering quality →](../guides/rendering-quality.md#how-to-set-it)

```bash
lbl print --text '{{barcode:EAN13:4006381333931}} {{size:0.7:SKU 7788}}' --supersample 4
```

<img src="images/supersample.png" alt="Supersampling for print quality" width="320"/>
## NIIMBOT catalog media

Die-cut tape sizes (`12x40`, `12x30`, …) resolve from the bundled catalog by SKU instead of raw millimetres.

*NIIMBOT 12×40 mm @ 203 dpi* · [Printers & media →](../guides/printers-media.md#niimbot)

```bash
lbl print --text 'Ship to {{size:1.2:Dock 4}} {{qr:https://track/42}}' --padding-mm 0
```

<img src="images/niimbot-catalog.png" alt="NIIMBOT catalog media" width="320"/>
## Explicit media dimensions

When no catalog SKU fits, pass `--width-mm`, `--length-mm`, and `--dpi` directly for fixed-size stock.

*56×89 mm @ 300 dpi* · [Printers & media →](../guides/printers-media.md#media)

```bash
lbl print --text 'Receipt
Item ×2  $18.00
Total      $36.00'
```

<img src="images/fixed-dimensions.png" alt="Explicit media dimensions" width="320"/>

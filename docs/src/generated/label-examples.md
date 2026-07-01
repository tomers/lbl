# Examples

Each preview highlights a different `lbl print` capability. Commands show
the flags that matter for each example; protocol and output path come from project
config (`lbl.toml`) or the doc generator defaults.

Regenerate with `just doc-examples`.

## Inline mini-syntax

Text mini-syntax embeds QR codes, barcodes, and relative font scaling in one string.
Elements flow in a row; spacing comes from `element_gap_mm` in config (override with `LBL_STYLE__ELEMENT_GAP_MM`).

*DYMO 11352 · 25×54 mm* · [Printing text →](../guides/printing-text.md#inline-mini-syntax-default)

```console
# default element gap
$ lbl print \
  --text 'Ship {{size:1.2:Alice}}{{barcode:L42}}{{qr:https://track/42}}'

# element gap 8 mm
LBL_STYLE__ELEMENT_GAP_MM=8 $ lbl print \
  --text 'Ship {{size:1.2:Alice}}{{barcode:L42}}{{qr:https://track/42}}'
```

<img src="images/inline-syntax.png" alt="Inline mini-syntax" width="320"/>
<img src="images/inline-syntax-01.png" alt="Inline mini-syntax" width="320"/>
---

## Markdown input

Headings, emphasis, and inline directives compose via `--markdown`.

*NIIMBOT 12×22 mm @ 203 dpi* · [Printing text →](../guides/printing-text.md)

```console
$ lbl print \
  --markdown $'# Order 44\n\nShip **fast** to dock 4\n\n{{qr:https://track/42}}'
```

<img src="images/markdown-label.png" alt="Markdown input" width="320"/>
---

## Cross-axis alignment

Position content across the label width with `--label-align` (`start`, `center`, or `end`).

*54×15 mm @ 300 dpi* · [Configuration →](../guides/configuration.md#style-fonts-qr-barcodes)

```console
# align left
# example
$ lbl print --label-align start --text 'align left' --width-mm 54 --length-mm 15 --dpi 300

# align center
# example
$ lbl print --label-align center --text 'align center' --width-mm 54 --length-mm 15 --dpi 300

# align right
# example
$ lbl print --label-align end --text 'align right' --width-mm 54 --length-mm 15 --dpi 300
```

<img src="images/label-align.png" alt="Cross-axis alignment" width="320"/>
<img src="images/label-align-01.png" alt="Cross-axis alignment" width="320"/>
<img src="images/label-align-02.png" alt="Cross-axis alignment" width="320"/>
---

## Inner padding

Inner padding (`--padding-mm`, default 2 mm) gutters content from the label edge.

*54×15 mm @ 300 dpi* · [Configuration →](../guides/configuration.md#padding-and-insets)

```console
# padding 0 (left)
# example
$ lbl print --text Hi --padding-mm 0 --width-mm 54 --length-mm 15 --dpi 300

# padding 4 mm (right)
# example
$ lbl print --text Hi --padding-mm 4 --width-mm 54 --length-mm 15 --dpi 300
```

<img src="images/zero-padding.png" alt="Inner padding" width="320"/>
---

## Config-driven print defaults

When transport and protocol live in `lbl.toml`, the run command only needs label content.

*NIIMBOT 12×40 mm @ 203 dpi* · [Configuration →](../guides/configuration.md#print-defaults-lbl-print)

[lbl.toml](../../examples/config-defaults/lbl.toml)

```toml
[print]
protocol  = "niimbot"
bluetooth = "D110"

[render]
orientation = "landscape"
```

```console
# example
$ lbl print --text 'Hello {{qr:https://x/p}}'
```

<img src="images/config-defaults.png" alt="Config-driven print defaults" width="320"/>
---

## Supersampling for print quality

Side by side: `--supersample 1` (left) vs `--supersample 8` (right).
More render dots before downscaling yield sharper barcodes and small type.

*DYMO 11352 · 25×54 mm* · [Rendering quality →](../guides/rendering-quality.md#how-to-set-it)

```console
# supersample 1 (left)
# example
$ lbl print --text '{{barcode:EAN13:4006381333931}} {{size:0.7:SKU 7788}}' --supersample 1

# supersample 8 (right)
# example
$ lbl print --text '{{barcode:EAN13:4006381333931}} {{size:0.7:SKU 7788}}' --supersample 8
```

<img src="images/supersample.png" alt="Supersampling for print quality" width="320"/>
---

## Catalog media SKU

Pick a die-cut size from the bundled catalog with `--media` instead of raw `--width-mm` / `--length-mm`.

*NIIMBOT 12×40 mm @ 203 dpi* · [Printers & media →](../guides/printers-media.md#niimbot)

```console
# example
$ lbl print --media 12x40 --text 12x40
```

<img src="images/niimbot-catalog.png" alt="Catalog media SKU" width="320"/>
---

## Another catalog SKU

Another die-cut size from the same catalog — only `--media` and content change.

*NIIMBOT 12×22 mm @ 203 dpi* · [Printers & media →](../guides/printers-media.md#niimbot)

```console
# example
$ lbl print --media 12x22 --text 12x22
```

<img src="images/niimbot-catalog-2.png" alt="Another catalog SKU" width="320"/>
---

## Explicit media dimensions

When no catalog SKU fits, pass `--width-mm`, `--length-mm`, and `--dpi` directly.

*48×76 mm @ 300 dpi* · [Printers & media →](../guides/printers-media.md#media)

```console
$ lbl print \
  --text $'Receipt\nItem ×2  $18.00\nTotal      $36.00' \
  --width-mm 48 \
  --length-mm 76 \
  --dpi 300
```

<img src="images/fixed-dimensions.png" alt="Explicit media dimensions" width="320"/>
---

## Complex HTML batch

Rich HTML with photos, QR, and barcodes — Tony and Carmela identity cards from the test suite.

*DYMO 99014 · 54×101 mm* · [Batch printing →](../guides/batch-printing.md#template--data)

[sopranos.lbl](../../examples/sopranos/sopranos.lbl)

```text
---json
[
  {
    "name": "Tony Soprano",
    "role": "Boss",
    "department": "DiMeo Crime Family",
    "employee_id": "NJ-BOSS-001",
    "event": "The Sopranos",
    "access": "CAPO DI TUTTI CAPI",
    "badge_number": "001",
    "photo": "https://upload.wikimedia.org/wikipedia/commons/4/4d/Tony_Soprano_%28The_Sopranos_Family_Tree%29.jpg"
  },
  {
    "name": "Carmela Soprano",
    "role": "First Lady",
    "department": "Soprano Household",
    "employee_id": "NJ-CARM-002",
    "event": "The Sopranos",
    "access": "NORTH CALDWELL",
    "badge_number": "002",
    "photo": "https://upload.wikimedia.org/wikipedia/commons/c/c1/Carmela_Soprano_%28The_Sopranos_Family_Tree%29.jpg"
  }
]
---
<div class="lbl-label lbl-col lbl-center">
  <div class="lbl-text" style="font-size:0.62em">{{ event }}</div>
  <img src="{{ photo }}" alt="" style="width:12em;height:14em;object-fit:cover" />
  <span class="lbl-text" style="font-size:1.2em;font-weight:bold">{{ name }}</span>
  <span class="lbl-text" style="font-size:0.85em">{{ role }} · {{ department }}</span>
  <qr>https://id.lbl.example/{{ employee_id }}</qr>
  <barcode type="CODE128">{{ employee_id }}</barcode>
  <div class="lbl-row lbl-between" style="width:100%">
    <span class="lbl-text" style="font-size:0.55em">{{ access }}</span>
    <span class="lbl-text" style="font-size:0.55em">#{{ badge_number }}</span>
  </div>
</div>
```

```console
# example
$ lbl print --template sopranos.lbl --template-format html
```

<img src="images/sopranos-cards.png" alt="Complex HTML batch" width="320"/>
---

## Templating

### HTML template

One HTML layout rendered against a JSON array — name badges with QR for Alice and Bob.

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#template--data)

[card.html](../../examples/batch-card/card.html)

```html
<div class="lbl-label lbl-row lbl-center">
  <div class="lbl-col">
    <strong>{{ name }}</strong>
    <span>{{ title }}</span>
  </div>
  <qr>{{ url }}</qr>
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
  --data people.json
```

<img src="images/batch-card.png" alt="HTML template" width="320"/>
---

### Single-file template and data (HTML)

Data and template in one file — a JSON frontmatter array batches without `--each`.

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#single-file-frontmatter)

[combined.html](../../examples/batch-combined/combined.html)

```html
---json
[
  { "name": "Alice", "title": "Engineer", "url": "https://example.com/alice" },
  { "name": "Bob", "title": "Designer", "url": "https://example.com/bob" }
]
---
<div class="lbl-label lbl-row lbl-center">
  <div class="lbl-col">
    <strong>{{ name }}</strong>
    <span>{{ title }}</span>
  </div>
  <qr>{{ url }}</qr>
</div>
```

```console
$ lbl print \
  --template combined.html \
  --template-format html
```

<img src="images/batch-combined.png" alt="Single-file template and data (HTML)" width="320"/>
---

### Single-file template and data (LBL)

The same frontmatter batch works in a `.lbl` file — HTML template syntax inside.

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#single-file-frontmatter)

[combined.lbl](../../examples/batch-combined/combined.lbl)

```text
---json
[
  { "name": "Alice", "title": "Engineer", "url": "https://example.com/alice" },
  { "name": "Bob", "title": "Designer", "url": "https://example.com/bob" }
]
---
<div class="lbl-label lbl-row lbl-center">
  <div class="lbl-col">
    <strong>{{ name }}</strong>
    <span>{{ title }}</span>
  </div>
  <qr>{{ url }}</qr>
</div>
```

```console
$ lbl print \
  --template combined.lbl \
  --template-format html
```

<img src="images/batch-combined-lbl.png" alt="Single-file template and data (LBL)" width="320"/>
---

### Command pipelining

Pipe one JSON object per line into `lbl print` — each line becomes `--data` for one badge.

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#shell-iteration-seq-and-xargs)

```console
$ cd docs/examples/batch-card
# example
$ cat people.ndjson | xargs -n1 lbl print --template card.html --template-format html --data
```

<img src="images/shell-template.png" alt="Command pipelining" width="320"/>
---

## Iterators

### `--one`

Print only the first label from a batch selection.

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#batch-selection)

```console
# example
$ lbl print --template card.html --template-format html --data people.json --one
```

<img src="images/iter-one.png" alt="`--one`" width="320"/>
---

### `--index`

Select a label by zero-based index — here, Bob at index 1.

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#batch-selection)

```console
# example
$ lbl print --template card.html --template-format html --data people.json --index 1
```

<img src="images/iter-index.png" alt="`--index`" width="320"/>
---

### `--filter`

Keep only labels whose data fields contain a substring (case-insensitive).

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#batch-selection)

```console
# example
$ lbl print --template card.html --template-format html --data people.json --filter Bob
```

<img src="images/iter-filter.png" alt="`--filter`" width="320"/>
---

### `--skip` and `--take`

Skip the first N labels, then print at most M from the remainder.

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#batch-selection)

```console
# example
$ lbl print --template card.html --template-format html --data people.json --skip 1 --take 1
```

<img src="images/iter-skip-take.png" alt="`--skip` and `--take`" width="320"/>

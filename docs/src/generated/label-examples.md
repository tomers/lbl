# Examples

<!-- markdownlint-disable-file MD014 -->

Each preview highlights a different `lbl print` capability. Commands show
the flags that matter for each example; protocol and output path come from project
config (`lbl.toml`) or the doc generator defaults.

Regenerate with `just doc-examples`.

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
  <div class="lbl-text" style="font-size:0.8em">{{ event }}</div>
  <img src="{{ photo }}" alt="" style="width:12em;height:14em;object-fit:cover" />
  <span class="lbl-text" style="font-size:1.45em;font-weight:bold">{{ name }}</span>
  <span class="lbl-text" style="font-size:1em">{{ role }} · {{ department }}</span>
  <qr>https://id.lbl.example/{{ employee_id }}</qr>
  <barcode type="CODE128">{{ employee_id }}</barcode>
  <div class="lbl-row lbl-between" style="width:100%">
    <span class="lbl-text" style="font-size:0.68em">{{ access }}</span>
    <span class="lbl-text" style="font-size:0.68em">#{{ badge_number }}</span>
  </div>
</div>
```

```console
$ lbl print --template sopranos.lbl --orientation portrait
```

<img src="images/sopranos-cards.png" alt="Complex HTML batch" width="320"/>
---

## Inline mini-syntax

Text mini-syntax embeds QR codes, barcodes, and relative font scaling in one string.

*80×20 mm* · [Printing text →](../guides/printing-text.md#inline-mini-syntax-default)

```console
$ lbl print --text 'Text [[size:2.5:Title]][[barcode:Barcode]][[qr:QR]]'
```

<img src="images/inline-syntax.png" alt="Inline mini-syntax" width="320"/>
---

## Element spacing

Inline elements in a row are separated by `element_gap_mm` (default 4 mm).
Override with `--element-gap-mm`, `LBL_STYLE__ELEMENT_GAP_MM`, or config
(see [configuration precedence](docs/src/guides/configuration.md#configuration)).

*200×30 mm @ 300 dpi* · [Configuration →](../guides/configuration.md#style-fonts-qr-barcodes)

```console
# default element gap
$ lbl print --text 'Text [[size:2.5:Title]][[barcode:Barcode]][[qr:QR]]'

# element gap 10 mm
$ lbl print --text 'Text [[size:2.5:Title]][[barcode:Barcode]][[qr:QR]]' --element-gap-mm 10
```

<img src="images/element-gap.png" alt="Element spacing" width="320"/>
<img src="images/element-gap-01.png" alt="Element spacing" width="320"/>
---

## Markdown input

Headings, emphasis, and inline directives compose via `--markdown`.

*NIIMBOT 12×40 mm @ 203 dpi* · [Printing text →](../guides/printing-text.md)

```console
$ lbl print --markdown $'# Order #1234\n\n⚠️ Caution **fragile**\n\n[[qr:https://track/1234]]'
```

<img src="images/markdown-label.png" alt="Markdown input" width="320"/>
---

## Label alignment

Position content on the label with `--label-align` and `--label-valign`.

*90×15 mm @ 300 dpi* · [Configuration →](../guides/configuration.md#style-fonts-qr-barcodes)

```console
$ lbl print --label-align start --label-valign start --text top-left

$ lbl print --label-align center --label-valign start --text top-center

$ lbl print --label-align end --label-valign start --text top-right

$ lbl print --label-align start --label-valign center --text center-left

$ lbl print --label-align center --label-valign center --text center

$ lbl print --label-align end --label-valign center --text center-right

$ lbl print --label-align start --label-valign end --text bottom-left

$ lbl print --label-align center --label-valign end --text bottom-center

$ lbl print --label-align end --label-valign end --text bottom-right
```

<img src="images/label-align.png" alt="Label alignment" width="320"/>
---

## Inner padding

Inner padding (`--padding-mm`, default 2 mm) gutters content from the label edge.

*54×15 mm @ 300 dpi* · [Configuration →](../guides/configuration.md#padding-and-insets)

```console
# padding 0 (left)
$ lbl print --text Hi --padding-mm 0

# padding 4 mm (right)
$ lbl print --text Hi --padding-mm 4
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
$ lbl print --text 'Hello [[qr:https://x/p]]'
```

<img src="images/config-defaults.png" alt="Config-driven print defaults" width="320"/>
---

## Supersampling for print quality

Side by side: `--supersample 1` (left) vs `--supersample 8` (right).
More render dots before downscaling yield sharper text and fine detail.

*12×40 mm* · [Rendering quality →](../guides/rendering-quality.md#how-to-set-it)

```console
# Supersample 1
$ lbl print --text 'Supersample 1' --supersample 1

# Supersample 8
$ lbl print --text 'Supersample 8' --supersample 8
```

<img src="images/supersample.png" alt="Supersampling for print quality" width="320"/>
---

## Catalog media SKU

Pick a die-cut size from the bundled catalog with `--media` instead of raw `--width-mm` / `--length-mm`.

*NIIMBOT 12×40 & 12×22 mm @ 203 dpi* · [Printers & media →](../guides/printers-media.md#niimbot)

```console
# 12×40
$ lbl print --media 12x40 --text 12x40

# 12×22
$ lbl print --media 12x22 --text 12x22
```

<img src="images/niimbot-catalog.png" alt="Catalog media SKU" width="320"/>
---

## Explicit media dimensions

When no catalog SKU fits, pass `--width-mm` and `--length-mm` directly.

*30×20 mm* · [Printers & media →](../guides/printers-media.md#media)

```console
$ lbl print --text '30×20' --width-mm 30 --length-mm 20
```

<img src="images/fixed-dimensions.png" alt="Explicit media dimensions" width="320"/>
---

## Templating

### HTML template

One HTML layout rendered against a JSON array — name badges with QR for Alice and Bob.

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#template--data)

[card.html](../../examples/batch-card/card.html)

```html
<div class="lbl-label lbl-col lbl-center">
  <strong>{{ name }}</strong>
  <span>{{ title }}</span>
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
$ lbl print --template card.html --data people.json
```

<img src="images/batch-card.png" alt="HTML template" width="320"/>
---

### Single-file template and data (text)

Data and template in one file — frontmatter plus a text body with inline mini-syntax.

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#single-file-frontmatter)

[combined.txt](../../examples/batch-combined/combined.txt)

```text
---json
[
  { "name": "Alice", "title": "Engineer", "url": "https://example.com/alice" },
  { "name": "Bob", "title": "Designer", "url": "https://example.com/bob" }
]
---
[[size:1.4:{{ name }}]]
{{ title }}
[[qr:{{ url }}]]
```

```console
$ cd docs/examples/batch-combined
$ lbl print --template combined.txt
```

<img src="images/batch-combined-text.png" alt="Single-file template and data (text)" width="320"/>
---

### Single-file template and data (markdown)

Same frontmatter pattern with a Markdown body (`.md` extension).

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#single-file-frontmatter)

[combined.md](../../examples/batch-combined/combined.md)

```markdown
<!-- markdownlint-disable-file MD022 MD041 MD036 -->
---json
[
  { "name": "Alice", "title": "Engineer", "url": "https://example.com/alice" },
  { "name": "Bob", "title": "Designer", "url": "https://example.com/bob" }
]
---
**{{ name }}**

*{{ title }}*

[[qr:{{ url }}]]
```

```console
$ cd docs/examples/batch-combined
$ lbl print --template combined.md
```

---

### Single-file template and data (html)

Same frontmatter pattern with an HTML body (`.html` or `.lbl` extension).

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#single-file-frontmatter)

[combined.html](../../examples/batch-combined/combined.html)

```html
---json
[
  { "name": "Alice", "title": "Engineer", "url": "https://example.com/alice" },
  { "name": "Bob", "title": "Designer", "url": "https://example.com/bob" }
]
---
<div class="lbl-label lbl-col lbl-center">
  <strong>{{ name }}</strong>
  <span>{{ title }}</span>
  <qr>{{ url }}</qr>
</div>
```

```console
$ cd docs/examples/batch-combined
$ lbl print --template combined.html
```

---

### Command pipelining

Pipe one JSON object per line into `lbl print` — each line becomes `--data` for one badge.

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#shell-iteration-seq-and-xargs)

```console
$ cat people.ndjson | xargs -n1 lbl print --template card.html --data
```

<img src="images/shell-template.png" alt="Command pipelining" width="320"/>
---

## Iterators

### `--first`

Print only the first label from a batch selection.

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#batch-selection)

```console
$ cd docs/examples/batch-card
$ lbl print --template card.html --data people.json --first
```

<img src="images/iter-first.png" alt="`--first`" width="320"/>
---

### `--index`

Select a label by zero-based index — here, Bob at index 1.

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#batch-selection)

```console
$ cd docs/examples/batch-card
$ lbl print --template card.html --data people.json --index 1
```

<img src="images/iter-index.png" alt="`--index`" width="320"/>
---

### `--filter`

Keep only labels whose data fields contain a substring (case-insensitive).

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#batch-selection)

```console
$ cd docs/examples/batch-card
$ lbl print --template card.html --data people.json --filter Bob
```

<img src="images/iter-filter.png" alt="`--filter`" width="320"/>
---

### `--skip` and `--take`

Skip the first five of ten shell-generated records, then print the next three (users 6–8).

*DYMO 2112286 · 25×25 mm* · [Batch printing →](../guides/batch-printing.md#batch-selection)

```console
$ seq 1 10 | xargs -n1 lbl print --template 'User #{{ it }}' --skip 5 --take 3 --data
```

<img src="images/iter-skip-take.png" alt="`--skip` and `--take`" width="320"/>

# lbl

`lbl` is a modular command-line toolchain for label printing, written in Rust.

Input is HTML; output is printer-native protocol bytes. The work is split into a
pipeline of small, single-purpose stages. Each stage is both a reusable
**library crate** and a standalone **`lbl-*` binary**, so you can pipe them
together by hand or let the top-level `lbl` command run the whole pipeline for
you.

```text
text/data/HTML
  -> lbl-text         (plain text / CLI args -> authoring HTML)
  -> lbl-template     (data + template -> N authoring HTML, resource fetch)
  -> lbl-transpile-html (custom <qr>/<barcode> + flex -> browser-ready HTML)
  -> lbl-render       (HTML -> raster, 2-pass via headless Chromium)
  -> lbl-dither       (raster -> printer bit depth, photo-aware)
  -> lbl-encode       (bitmap -> protocol bytes; pluggable drivers)
  -> lbl-spool        (internal spooler; queue + cut control)
  -> lbl-device       (USB / network transport)
  -> Printer
```

## Workspace layout

| Crate | Role |
| --- | --- |
| `lbl-core` | Shared types: units, geometry, media, printers, jobs |
| `lbl-config` | Layered configuration (defaults < file < env < CLI) |
| `lbl-catalog` | Curated media SKU database + printer compatibility |
| `lbl-text` | Plain text / CLI args -> authoring HTML |
| `lbl-template` | Data + template -> N HTML, with resource fetching |
| `lbl-transpile-html` | Custom elements + flex -> browser-ready HTML |
| `lbl-render` | HTML -> raster (headless Chromium, two-pass) |
| `lbl-dither` | Raster -> printer bit depth (photo-aware dithering) |
| `crates/drivers/*` | Printer drivers: api, dymo, escpos, zpl, tspl, niimbot, file (virtual), console (terminal) |
| `lbl-encode` | Bitmap -> protocol bytes (driver selection) |
| `lbl-device` | Device discovery + USB/network transport |
| `lbl-spool` | Internal print spooler |
| `lbl` | Orchestrator (subcommands + full-pipeline flows) |
| `lbl-server` | HTTP/WebSocket API for programmatic access and integrations |
| `docs/` | Architecture document + thorough documentation |

## Development

Dev tooling is managed by [mise](https://mise.jdx.dev/). It is the entrypoint for
pinned CLIs — **`just`**, **`pre-commit`**, **`cargo-nextest`**, and the rest listed in
[`mise.toml`](mise.toml).

### Install mise

```bash
# macOS / Linux — see https://mise.jdx.dev/getting-started.html
curl https://mise.run | sh

# Homebrew (macOS / Linux)
brew install mise
```

Activate mise in your shell (add to `~/.zshrc`, `~/.bashrc`, or equivalent):

```bash
eval "$(mise activate zsh)"   # or: bash, fish
```

### Install project tools

From the repo root:

```bash
mise install                              # just, pre-commit, cargo-nextest, …
pre-commit install --install-hooks        # git pre-commit + commit-msg hooks
```

Without shell activation you can run recipes through mise:

```bash
mise exec -- just lint
```

Common recipes (run `just` for the full list):

```bash
just serve                          # run the lbl-server API on the host (127.0.0.1:8787)
just lint                           # lint the Rust workspace (rustc + clippy + rustfmt)
just lint-fix                       # apply autofixes (clippy --fix + rustfmt)
just lint-fix-allow-dirty           # same, but allow a dirty working tree
just test                           # run the Rust test suite (cargo-nextest)
just doc-examples                   # regenerate fixed-size label README/doc previews
just doc-examples-check             # fail if generated doc examples are stale
just maintenance cargo-upgrade      # bump Cargo.toml deps + refresh Cargo.lock
just pre-commit-all                 # run the full pre-commit suite on all files
```

## Building

```bash
cargo build           # build the whole workspace
cargo test            # run the test suite
```

<!-- doc-examples:start -->

## Fixed-size label examples

Each preview highlights a different capability on **fixed-length** media. Commands show
content and layout flags only — media size, DPI, protocol, and output path come from
project config (`lbl.toml`) or environment.

Regenerate from [`docs/examples/manifest.toml`](docs/examples/manifest.toml) with `just doc-examples`.

<table>
<tr>
<td valign="top">

### Inline QR, barcode, and sizing

Text mini-syntax embeds QR codes, barcodes, and relative font scaling in a plain string — no HTML required.

*DYMO 11352 · 25×54 mm* · [Printing text →](docs/src/guides/printing-text.md#inline-mini-syntax-default)

```bash
lbl print --text 'Ship {{size:1.5:Alice}} {{barcode:LBL-42}} {{qr:https://track/42}}'
```

</td>
<td>

<img src="docs/src/generated/images/inline-syntax.png" alt="Inline QR, barcode, and sizing" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

### Markdown input

Headings, emphasis, and inline directives compose on fixed-length media via `--markdown`.

*DYMO 11352 · 25×54 mm* · [Printing text →](docs/src/guides/printing-text.md)

```bash
lbl print --markdown '# Order 44

Ship **fast** to dock 4

{{qr:https://track/42}}'
```

</td>
<td>

<img src="docs/src/generated/images/markdown-label.png" alt="Markdown input" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

### Batch HTML template

One HTML layout rendered against a JSON array — here, a name badge with QR on portrait die-cut stock.

*DYMO 99014 · 54×101 mm* · [Batch printing →](docs/src/guides/batch-printing.md#template--data)

[card.html](docs/examples/batch-card/card.html)

```html
<div class="lbl-label lbl-row lbl-center">
  <div class="lbl-col">
    <strong>{{ name }}</strong>
    <span>{{ title }}</span>
    <qr>{{ url }}</qr>
  </div>
</div>
```

[people.json](docs/examples/batch-card/people.json)

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

</td>
<td>

<img src="docs/src/generated/images/batch-card.png" alt="Batch HTML template" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

### Template from shell input

Run `lbl print` once per value: `xargs` appends each line as `--data`, and `{{ it }}` is the scalar.

*NIIMBOT 12×30 mm @ 203 dpi* · [Batch printing →](docs/src/guides/batch-printing.md#shell-iteration-seq-and-xargs)

```bash
lbl print --template 'Hello user #{{ it }}, my friend' --data 1 --padding-mm 0
```

</td>
<td>

<img src="docs/src/generated/images/shell-template.png" alt="Template from shell input" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

### Zero padding on small tape

Default inner padding is 2 mm; set `--padding-mm 0` (or `padding_mm` in config) so content uses the full printable area.

*NIIMBOT 12×30 mm @ 203 dpi* · [Configuration →](docs/src/guides/configuration.md#padding-and-insets)

```bash
lbl print --text 'Aisle 4
Bin 12
Qty 60' --padding-mm 0
```

</td>
<td>

<img src="docs/src/generated/images/zero-padding.png" alt="Zero padding on small tape" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

### Config-driven print defaults

With `[print] protocol`, `bluetooth`, and media set in `lbl.toml`, the command only needs the label content.

*NIIMBOT 12×22 mm @ 203 dpi* · [Configuration →](docs/src/guides/configuration.md#print-defaults-lbl-print)

```bash
lbl print --text 'Scan to pair {{qr:https://lbl.example/pair}}'
```

</td>
<td>

<img src="docs/src/generated/images/config-defaults.png" alt="Config-driven print defaults" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

### Supersampling for print quality

Higher `--supersample` renders at more dots before downscaling — sharper barcodes and small type on 1-bit heads.

*DYMO 11352 · 25×54 mm* · [Rendering quality →](docs/src/guides/rendering-quality.md#how-to-set-it)

```bash
lbl print --text '{{barcode:EAN13:4006381333931}} {{size:0.7:SKU 7788}}' --supersample 4
```

</td>
<td>

<img src="docs/src/generated/images/supersample.png" alt="Supersampling for print quality" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

### NIIMBOT catalog media

Die-cut tape sizes (`12x40`, `12x30`, …) resolve from the bundled catalog by SKU instead of raw millimetres.

*NIIMBOT 12×40 mm @ 203 dpi* · [Printers & media →](docs/src/guides/printers-media.md#niimbot)

```bash
lbl print --text 'Ship to {{size:1.2:Dock 4}} {{qr:https://track/42}}' --padding-mm 0
```

</td>
<td>

<img src="docs/src/generated/images/niimbot-catalog.png" alt="NIIMBOT catalog media" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

### Explicit media dimensions

When no catalog SKU fits, pass `--width-mm`, `--length-mm`, and `--dpi` directly for fixed-size stock.

*56×89 mm @ 300 dpi* · [Printers & media →](docs/src/guides/printers-media.md#media)

```bash
lbl print --text 'Receipt
Item ×2  $18.00
Total      $36.00'
```

</td>
<td>

<img src="docs/src/generated/images/fixed-dimensions.png" alt="Explicit media dimensions" width="240"/>

</td>
</tr>
</table>

<!-- doc-examples:end -->

## Documentation

The documentation is an [mdBook](https://rust-lang.github.io/mdBook/) under
[`docs/`](docs/):

```bash
mdbook serve docs   # or: mdbook build docs
```

- [Architecture overview](docs/src/architecture.md) and
  [ADRs](docs/src/adr/README.md)
- User guides (getting started, text, batch, preview, configuration,
  printers & media)
- Reference (pipeline, data contracts, CLI, catalog, crates)
- [Writing a driver](docs/src/drivers/authoring.md)

API docs: `cargo doc --workspace --no-deps --open`.

## License

License TBD. `lbl` is not affiliated with DYMO or any other manufacturer.

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

Commands show content and layout flags only. Media size, DPI, protocol, and output path are supplied by project config (`lbl.toml`) or environment — see [Configuration](docs/src/guides/configuration.md). Preview images are generated from the manifest in [`docs/examples/manifest.toml`](docs/examples/manifest.toml) via `just doc-examples`.

<table>
<tr>
<td valign="top">

DYMO 11352 · 25×54 mm

```bash
lbl print --text 'Hello {{qr:https://example.com}}'
```

</td>
<td>

<img src="docs/src/generated/images/hello-qr.png" alt="DYMO 11352 · 25×54 mm" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

DYMO 11352 · 25×54 mm

```bash
lbl print --text Hello
```

</td>
<td>

<img src="docs/src/generated/images/hello.png" alt="DYMO 11352 · 25×54 mm" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

DYMO 99014 · 54×101 mm

```bash
lbl print --template card.html --template-format html --data people.json --one
```

</td>
<td>

<img src="docs/src/generated/images/batch-card.png" alt="DYMO 99014 · 54×101 mm" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

NIIMBOT 12×30 mm @ 203 dpi

```bash
lbl print --template 'User #{{ it }}' --data 1 --padding-mm 0
```

</td>
<td>

<img src="docs/src/generated/images/user-number.png" alt="NIIMBOT 12×30 mm @ 203 dpi" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

NIIMBOT 12×30 mm @ 203 dpi

```bash
lbl print --text Hi --padding-mm 0
```

</td>
<td>

<img src="docs/src/generated/images/hi-no-padding.png" alt="NIIMBOT 12×30 mm @ 203 dpi" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

NIIMBOT 12×22 mm @ 203 dpi

```bash
lbl print --text Hello
```

</td>
<td>

<img src="docs/src/generated/images/hello-niimbot.png" alt="NIIMBOT 12×22 mm @ 203 dpi" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

DYMO 11352 · 25×54 mm · supersample 4

```bash
lbl print --text Hello --supersample 4
```

</td>
<td>

<img src="docs/src/generated/images/hello-supersample.png" alt="DYMO 11352 · 25×54 mm · supersample 4" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

NIIMBOT 12×40 mm @ 203 dpi

```bash
lbl print --text Ship --padding-mm 0
```

</td>
<td>

<img src="docs/src/generated/images/niimbot-tape.png" alt="NIIMBOT 12×40 mm @ 203 dpi" width="240"/>

</td>
</tr>
</table>

<table>
<tr>
<td valign="top">

56×89 mm @ 300 dpi

```bash
lbl print --text 'Receipt line'
```

</td>
<td>

<img src="docs/src/generated/images/fixed-dimensions.png" alt="56×89 mm @ 300 dpi" width="240"/>

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

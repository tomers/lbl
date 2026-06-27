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
| `crates/drivers/*` | Printer drivers: api, dymo, escpos, zpl, tspl, file (virtual) |
| `lbl-encode` | Bitmap -> protocol bytes (driver selection) |
| `lbl-device` | Device discovery + USB/network transport |
| `lbl-spool` | Internal print spooler |
| `lbl` | Orchestrator (subcommands + full-pipeline flows) |
| `lbl-server` | HTTP/WebSocket API for programmatic access and integrations |
| `docs/` | Architecture document + thorough documentation |

## Development

Tooling is managed by [mise](https://mise.jdx.dev/) and tasks are run with
[just](https://github.com/casey/just). Install the toolchain once, then enable
the git hooks:

```bash
mise install          # just, pre-commit, cargo-nextest, ...
pre-commit install --install-hooks
```

Common recipes (run `just` for the full list):

```bash
just serve            # run the lbl-server API on the host (127.0.0.1:8787)
just lint             # lint the Rust workspace
just lint-fix         # apply autofixes
just test             # run the Rust test suite (cargo-nextest)
```

## Building

```bash
cargo build           # build the whole workspace
cargo test            # run the test suite
```

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

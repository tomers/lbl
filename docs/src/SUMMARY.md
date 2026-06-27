# Summary

[Introduction](./introduction.md)

# Architecture

- [Architecture Overview](./architecture.md)
- [The Pipeline](./reference/pipeline.md)
- [Data Formats & Contracts](./reference/contracts.md)

# Guides

- [Getting Started](./guides/getting-started.md)
- [Printing Text](./guides/printing-text.md)
- [Batch Printing](./guides/batch-printing.md)
- [Previewing & the Gallery](./guides/preview.md)
- [Configuration](./guides/configuration.md)
- [Managing Printers & Media](./guides/printers-media.md)

# Reference

- [CLI Reference](./reference/cli.md)
- [Media Catalog](./reference/catalog.md)
- [Crates](./reference/crates.md)

# Extending lbl

- [Writing a Driver](./drivers/authoring.md)

# Decisions

- [ADR Index](./adr/README.md)
  - [0001 — GCC-style modular pipeline](./adr/0001-gcc-style-pipeline.md)
  - [0002 — HTML as the input format](./adr/0002-html-input.md)
  - [0003 — Chromium for rendering](./adr/0003-chromium-rendering.md)
  - [0004 — Two-pass render + photo-aware dither](./adr/0004-two-pass-dither.md)
  - [0005 — Internal spooler](./adr/0005-internal-spooler.md)
  - [0006 — Layered configuration with figment](./adr/0006-layered-config.md)
  - [0007 — Driver grouping & plugin contract](./adr/0007-driver-plugin-contract.md)
  - [0008 — Media catalog & image licensing](./adr/0008-media-catalog-licensing.md)

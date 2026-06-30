# Introduction

**lbl** is a modular command-line toolchain for printing labels.

It turns content — plain text, an HTML template fed with data, or raw HTML —
into crisp output on a wide range of label printers, spanning both proprietary
protocols (DYMO, NIIMBOT) and industry standards (ESC/POS, ZPL, TSPL).

## Philosophy

The toolchain is a pipeline of small, single-purpose stages, orchestrated by the
top-level `lbl` command. Each stage is:

- a **library crate** (`lbl-*`) usable from Rust, and
- a **standalone binary** (`lbl-*`) usable from the shell and Unix pipes.

This means you can run the whole flow with one command:

```bash
lbl print --text "Hello {{qr:https://example.com}}" --media 11352 --protocol dymo --usb 0922:1001
```

…or drive each stage yourself and pipe between them:

```bash
lbl-text "Hello {{qr:https://example.com}}" \
  | lbl-transpile-html --mode print \
  | lbl-render --width-dots 672 \
  | lbl-dither --algorithm auto \
  | lbl-encode --protocol escpos --width-mm 56 \
  | lbl-device send --usb 0922:1001
```

## What's in the box

- An HTML-first content model with QR/barcode/flex support.
- Two-pass, photo-aware rendering for quality on 1-bit thermal heads.
- Pluggable drivers and an internal print spooler.
- A media catalog that maps SKUs (e.g. `11352`) to physical media.
- An HTTP API (`lbl-server`) for programmatic preview, configuration, and printing.

Continue to the [Architecture Overview](./architecture.md).

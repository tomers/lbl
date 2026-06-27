# Getting Started

## Prerequisites

- Rust (stable, see `rust-toolchain.toml`).
- A Chromium/Chrome binary on `PATH` for rendering (or use the Node/Playwright
  sidecar with `--backend sidecar`).

## Build

```bash
cargo build --workspace
```

Run the test suite:

```bash
cargo test --workspace
```

## Your first label (dry run)

No printer required — write the encoded bytes to a directory:

```bash
lbl print --text "Hello, world!" --width-mm 25 --protocol escpos --out-dir out/
ls out/   # label-0000.bin
```

## Print to a device

```bash
# USB (vendor:product in hex)
lbl print --text "Hello" --media 11352 --protocol dymo --usb 0922:1001

# Network (raw TCP, e.g. port 9100)
lbl print --text "Hello" --width-mm 56 --protocol escpos --network 192.168.1.50:9100
```

## Run the HTTP API

```bash
cargo run -p lbl-server -- --bind 127.0.0.1:8787
```

Next: [Printing Text](./printing-text.md).

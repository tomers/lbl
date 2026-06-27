# lbl-device

Printer discovery and transport.

- **Discovery**: enumerates connected USB printers via
  [nusb](https://docs.rs/nusb) (pure Rust) and maps known vendor/product ids to
  models and protocols.
- **Transport**: `NetworkTransport` (raw TCP, e.g. port 9100) and
  `UsbTransport` (USB bulk-out) implement a common `Transport::send` trait.
- **Media**: `resolve_media` prefers an explicit override, then device
  auto-detection (most label printers don't report media electronically).

USB support is behind the default `usb` feature.

## CLI

```bash
lbl-device list
cat label.bin | lbl-device send --usb 0922:1001
lbl-device send --network 192.168.1.50:9100 label.zpl
```

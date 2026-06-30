# lbl-device

Printer discovery and transport.

- **Discovery**: enumerates connected USB printers via
  [nusb](https://docs.rs/nusb) (pure Rust) and maps known vendor/product ids to
  models and protocols.
- **Transport**: a common `Transport` trait with `NetworkTransport` (raw TCP,
  e.g. port 9100), `UsbTransport` (USB bulk-out), and `SerialTransport`
  (bidirectional USB CDC-ACM, e.g. NIIMBOT D-series on `/dev/ttyACM0`). The
  trait is bidirectional: `is_bidirectional()` and `receive()` let protocols
  that handshake (NIIMBOT status polling) read device replies; write-only
  transports inherit no-op defaults.
- **Media**: `resolve_media` prefers an explicit override, then device
  auto-detection (most label printers don't report media electronically).

USB support is behind the default `usb` feature; serial behind `serial`.

## CLI

```bash
lbl-device list
cat label.bin | lbl-device send --usb 0922:1001
lbl-device send --network 192.168.1.50:9100 label.zpl
cat label.niimbot | lbl-device send --serial /dev/ttyACM0
```

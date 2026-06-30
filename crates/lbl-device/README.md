# lbl-device

Printer discovery and transport.

- **Discovery**: enumerates connected printers and maps known vendor/product
  ids to models and protocols. `discover_usb` lists USB **bulk** devices via
  [nusb](https://docs.rs/nusb) (pure Rust); `discover_serial` lists USB
  **serial** ports (NIIMBOT B-series and other CDC-ACM printers) via
  [serialport](https://docs.rs/serialport), each with the device `path` to pass
  to `--serial`; `discover_ble` (opt-in `ble` feature) scans for nearby
  Bluetooth LE printers (NIIMBOT D-series); `discover` merges all three,
  recognized printers first.
- **Transport**: a common `Transport` trait with `NetworkTransport` (raw TCP,
  e.g. port 9100), `UsbTransport` (USB bulk-out), `SerialTransport`
  (bidirectional USB CDC-ACM, e.g. NIIMBOT B-series on `/dev/ttyACM0`), and
  `BleTransport` (bidirectional Bluetooth LE GATT, e.g. NIIMBOT D110). The
  trait is bidirectional: `is_bidirectional()` and `receive()` let protocols
  that handshake (NIIMBOT status polling) read device replies; write-only
  transports inherit no-op defaults.
- **Media**: `resolve_media` prefers an explicit override, then device
  auto-detection (most label printers don't report media electronically).

USB support is behind the default `usb` feature; serial behind `serial`;
Bluetooth LE behind the opt-in `ble` feature (pulls in [btleplug](https://docs.rs/btleplug);
on Linux it vendors `libdbus` so no system package is required).

## CLI

```bash
lbl-device list                          # USB bulk + serial (+ BLE with `ble`)
cat label.bin | lbl-device send --usb 0922:1001
lbl-device send --network 192.168.1.50:9100 label.zpl
cat label.niimbot | lbl-device send --serial /dev/ttyACM0
cat label.niimbot | lbl-device send --bluetooth D110   # requires `ble` feature
```

Build with BLE support:

```bash
cargo build -p lbl-device --features ble
cargo build -p lbl --features ble
```

`lbl-device list` prints serial candidates with `"connection": "serial"` and a
`path` (e.g. `/dev/ttyACM0`) to hand to `--serial`, and BLE candidates with
`"connection": "ble"` and the advertised name for `--bluetooth`.

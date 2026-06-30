# Managing Printers & Media

## Discover printers

```bash
lbl device list           # via the orchestrator
lbl-device list           # standalone
```

This enumerates **both** kinds of connection and prints them as JSON:

- **USB bulk** devices (DYMO, ESC/POS, …), matched against a known-printer
  table to suggest a model and protocol.
- **USB serial** ports (NIIMBOT B-series and other CDC-ACM printers). Each entry
  carries a `path` (e.g. `/dev/ttyACM0`) to hand straight to `--serial`.

```jsonc
[
  {
    "vendor_id": 6790,
    "product_id": 29987,
    "brand": "NIIMBOT",          // filled in when the USB descriptor says so
    "model": "B1",
    "protocol": "niimbot",
    "connection": "serial",
    "path": "/dev/ttyACM0"       // <- pass this to `--serial`
  }
]
```

Recognized printers (those with a `protocol`) are listed first. Serial ports
whose USB descriptor doesn't identify the vendor are still listed (with `brand`,
`model`, and `protocol` left `null`) so you can spot the candidate `path`.

The `lbl-server` API exposes the same discovery (`/api/printers`) and lets you
adopt a discovered device as a saved profile (`/api/printers/profiles`).

## Profiles

Profiles persist a printer's desired configuration (name, model, protocol,
capabilities, transport, default media) so it survives disconnects. They're
stored in `printers.toml` next to your config.

Manage them via the API (`/api/printers/profiles`).

## Media

Refer to media by SKU:

```bash
lbl-catalog compatible --printer "LabelWriter 550"
lbl print --media 11352 ...        # resolves to 25x54mm return-address labels
```

Or specify dimensions directly:

```bash
lbl print --width-mm 56 --length-mm 89 --dpi 300 ...   # fixed
lbl print --width-mm 56 --dpi 203 ...                  # continuous
```

The `lbl-server` API serves the catalog (`/api/catalog`) with images, filtering
by printer compatibility, and purchase links.

## NIIMBOT

NIIMBOT printers use the `niimbot` protocol. The D110 has a 12 mm (96-dot)
thermal head at 203 dpi, so labels are 96 dots wide regardless of tape width.
Their die-cut tape sizes (`12x40`, `12x30`, `15x30`, …) ship in the bundled
catalog under the `NIIMBOT` brand:

```bash
lbl-catalog compatible --printer "D110"
# Print over the USB cable (serial / CDC-ACM):
lbl print --media 12x40 --dpi 203 --protocol niimbot --serial /dev/ttyACM0
# or by dimensions:
lbl print --width-mm 12 --length-mm 40 --dpi 203 --protocol niimbot --serial /dev/ttyACM0
```

These printers talk over a USB CDC-ACM **serial** port (e.g. `/dev/ttyACM0`, or
`COM3` on Windows) rather than USB bulk transfer, so they're discovered via
serial-port enumeration, not the USB bulk table. Pass `--serial <path>`
(optionally `<path>:<baud>`, default 115200) and select `niimbot` as the
protocol.

NIIMBOT is a request/response protocol, so the serial transport is
**bidirectional**: after each label's bytes are sent, `lbl` polls the printer's
status and waits for the page to finish before dispatching the next one. (Over a
write-only transport — a file or raw socket — the job is still emitted, just
without the completion handshake.)

### Which models can print over USB?

> **Heads-up:** not every NIIMBOT prints over USB.
>
> - **B-series (B1, B18, B21, …)** expose a USB **data** port and show up as a
>   serial device — these work with `--serial`.
> - **Pocket D-series (D11, D110, …)** are **Bluetooth-only**. Their USB-C port
>   **only charges the battery**; it carries no print data, so the printer never
>   appears as a serial port no matter which cable you use.
>
> If your D11/D110 doesn't show up below, that's expected — it isn't a wiring or
> permissions problem. Bluetooth LE is not yet supported as an `lbl` transport,
> so a USB-data-capable model (B-series) is currently required for cable
> printing.

### Finding the right serial port

First, let `lbl` find it for you:

```bash
lbl-device list      # serial ports appear with "connection": "serial" and a "path"
```

Use the `path` from that output verbatim. Note the path is `/dev/ttyACM0` —
**not** `/dev/tty/ttyACM0` (there's no `tty/` directory; a leading slash typo
like that is a common reason the port "won't connect").

If auto-detection comes up empty, check by hand.

#### Linux

```bash
lsusb                                   # is the printer enumerated at all?
ls -l /dev/ttyACM* /dev/ttyUSB*         # CDC-ACM shows as ttyACM*, CH340 as ttyUSB*
ls -l /dev/serial/by-id/                # stable names that survive replug
dmesg | grep -iE 'tty|cdc|acm|usb'      # what the kernel bound on plug-in
udevadm info -q property -n /dev/ttyACM0 | grep -i id_   # VID/PID/vendor strings
```

Walk down the list: if `lsusb` shows nothing new when you plug the printer in,
the cable is charge-only or the USB port carries no data (see the model note
above) — no serial device will ever appear. If `lsusb` lists the device but no
`/dev/ttyACM*`/`/dev/ttyUSB*` node exists, the matching kernel module (e.g.
`cdc_acm`) didn't bind. If the node exists but `lbl` reports a permission error,
add yourself to the `dialout` group and log back in:

```bash
sudo usermod -aG dialout "$USER"        # then log out/in (or `newgrp dialout`)
```

#### macOS

USB serial ports appear as `/dev/tty.usbmodemXXXX` (CDC-ACM) or
`/dev/tty.usbserialXXXX`; list them with `ls /dev/tty.usb*`.

#### Windows

Ports are named `COM3`, `COM4`, … Find the number under *Device Manager → Ports
(COM & LPT)* and pass it as `--serial COM3`.

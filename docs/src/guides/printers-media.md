# Managing Printers & Media

## Discover printers

```bash
lbl device list           # via the orchestrator
lbl-device list           # standalone
```

USB printers are matched against a known-printer table to suggest a model and
protocol. In the web app, the **Printers** page lists discovered devices and
lets you adopt one as a saved profile.

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

The **Media** page in the web app browses the catalog with images, lets you
filter by printer compatibility, and links out to purchase pages.

## NIIMBOT D11 / D110

NIIMBOT D-series printers use the `niimbot` protocol. The D110 has a 12 mm
(96-dot) thermal head at 203 dpi, so labels are 96 dots wide regardless of tape
width. Their die-cut tape sizes (`12x40`, `12x30`, `15x30`, …) ship in the
bundled catalog under the `NIIMBOT` brand:

```bash
lbl-catalog compatible --printer "D110"
lbl print --media 12x40 --dpi 203 ...
# or by dimensions:
lbl print --width-mm 12 --length-mm 40 --dpi 203 --protocol niimbot ...
```

D-series printers connect over Bluetooth LE or a USB CDC-ACM serial port
(e.g. `/dev/ttyACM0`) rather than USB bulk transfer, so they aren't listed by
USB discovery — adopt one manually and select `niimbot` as the protocol.

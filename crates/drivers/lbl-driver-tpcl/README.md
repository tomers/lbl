# lbl-driver-tpcl

Toshiba TEC **TPCL** (TEC Printer Command Language) driver for B-series
industrial / desktop label printers (B-EV, B-FV, B-SV, B-SX, B-SA, BV/BX).

Emits binary `ESC` … `LF` `NUL` framing (BX External Interface Spec):

- `ESC D` — label pitch / print width / print length in 0.1 mm
- `ESC C` — clear image buffer
- `ESC SG` — graphic in **hex** mode (8 dots/byte, 1 = ink)
- `ESC XS` — issue (copies, cut interval, sensor, batch)

Media sense maps to the XS sensor digit (`0` continuous, `1` reflective /
black-mark, `2` transmissive / gap).

**Deferred:** TOPIX compression, braced `{…|}` command set, peel/RFID, status
readback. Research notes: `docs/research-tpcl-citizen.md`.

Typical transport: raw TCP `:9100` (USB `08a6:*` PIDs TBD — catalog omits
VID-only wildcards).

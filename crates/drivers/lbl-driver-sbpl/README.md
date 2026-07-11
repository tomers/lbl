# lbl-driver-sbpl

SATO **SBPL** (SATO Barcode Printer Language) driver for CL / CT / WW-class
industrial label printers.

Emits:

- `ESC A` / `ESC Z` — job framing
- `ESC A1` — media size in dots (height, width)
- `ESC H` / `ESC V` — graphic origin
- `ESC ~A` — cutter interval when requested
- `ESC G` — custom graphic as 8×8 binary blocks (set bits = black)
- `ESC Q` — print quantity

Typical transport: USB (`0828:*`) or raw TCP `:9100`.

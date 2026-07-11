# lbl-driver-ezpl

Godex **EZPL** (EZ Printer Language) driver for DT / G / RT-class desktop label
printers.

Emits:

- `^Q` / `^W` — label length/gap and width (mm)
- `^D` — cutter interval when requested
- `~EB` — download a 1-bit BMP graphic into volatile memory
- `^L` … `Y0,0,…` … `E` — place the graphic and end the label format
- `~P` — print copies
- `~MDELG` — delete the temporary graphic

Typical transport: USB (`6495:*`) or raw TCP `:9100`.

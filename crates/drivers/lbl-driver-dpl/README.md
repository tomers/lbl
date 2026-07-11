# lbl-driver-dpl

Honeywell / Datamax-O'Neil **DPL** (Datamax Programming Language) driver for
PC42 / PM / PX-class industrial label printers.

Emits:

- `SOH D` — disable immediate commands before 8-bit image download
- `STX e` / `STX r` / `STX c` — gap / black-mark / continuous media sense
- `STX I` — download a 1-bit BMP graphic into DRAM module `D`
- `STX L` … image record … `Q` … `E` — place, quantity, print
- `:` — cut-by amount when requested
- `STX x` — delete the temporary graphic

Typical transport: USB (`0b0b:*`) or raw TCP `:9100`.

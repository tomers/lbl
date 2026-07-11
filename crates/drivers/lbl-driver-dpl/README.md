# lbl-driver-dpl

Honeywell / Datamax-O'Neil / Citizen **DPL** (Datamax Programming Language)
driver for PC42 / PM / PX-class industrials and Citizen CL-S / CL-E warehouse
printers (native Datamax mode; ZPL-mode SKUs can stay on `zpl`).

Emits:

- `SOH D` — disable immediate commands before 8-bit image download
- `STX e` / `STX r` / `STX c` — gap / black-mark / continuous media sense
- `STX I` — download a 1-bit BMP graphic into DRAM module `D`
- `STX L` … image record … `Q` … `E` — place, quantity, print
- `:` — cut-by amount when requested
- `STX x` — delete the temporary graphic

Typical transport: USB (`0b0b:*` Honeywell/Datamax, `1d90:*` / `2730:*` /
`08bd:*` Citizen) or raw TCP `:9100`. CLI/API aliases: `dpl`, `honeywell`,
`datamax`, `citizen`.

# lbl-driver-slcs

Bixolon **SLCS** (Samsung Label Command Set) driver for SLP / XD / TX-class
desktop label printers.

Emits:

- `SW` / `SL` — label width and length (dots) with gap / continuous / black-mark sense
- `CB` — clear image buffer
- `CUT` — auto-cutter when requested (`CUTy` / `CUTy,n` / `CUTn`)
- `LD` — raw 1-bit bitmap at `(0,0)` (set bits = black, matching `MonoBitmap`)
- `P` — print (`CR`-terminated per SLCS requirements)

Typical transport: USB (`1504:*`) or raw TCP `:9100`.

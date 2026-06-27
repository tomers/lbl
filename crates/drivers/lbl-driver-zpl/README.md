# lbl-driver-zpl

ZPL (Zebra Programming Language) driver. Emits `^XA ... ^XZ` wrapping a `^GFA`
(Graphic Field, ASCII-hex) image; ZPL set bits are black dots, matching the
`MonoBitmap` layout. A requested cut adds `^MMC` (cut mode).

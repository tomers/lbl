# lbl-driver-esclabel

Epson **ESC/Label** driver for ColorWorks inkjet label printers (CW-C4000 /
CW-C6000 / CW-C6500 and related).

ESC/Label is ZPL II–compatible with Epson media-layout extensions. This driver
emits a monochrome raster job:

- `^S(CLS,…)` — label width / length / gap in dots
- `^MN*` — gap / black-mark / continuous sensing
- `^MMC` / `^MMT` — cut vs tear when requested
- `^GFA` — graphic field (set bits = black, matching `MonoBitmap`)

Full-color PNG registration (`~DY` + `^IM`) is not implemented yet; that needs
a color encode path beyond the current 1-bit pipeline. ColorWorks models are
still catalogued with `supports_color = true` for UI affordances.

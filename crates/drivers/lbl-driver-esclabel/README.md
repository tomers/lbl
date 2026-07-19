# lbl-driver-esclabel

Epson **ESC/Label** driver for ColorWorks inkjet label printers (CW-C4000 /
CW-C6000 / CW-C6500 and related).

ESC/Label is ZPL II–compatible with Epson media-layout extensions. The driver
emits:

- `^S(CLS,…)` — label width / length / gap in dots
- `^MN*` — gap / black-mark / continuous sensing
- `^MMC` / `^MMT` — cut vs tear when requested

**Monochrome** (default when no color PNG is supplied):

- `^GFA` — graphic field (set bits = black, matching `MonoBitmap`)

**Full color** (when `DeviceCapabilities::supports_color` and
`EncodeContext::color_png` are set — the pipeline attaches the rendered RGBA
as PNG for ColorWorks catalog models):

- `~DYR:…,B,P,…` — register PNG in volatile memory
- `^IMR:….PNG` — recall the registered graphic
- `^ID` — clear leftover objects before/after the job

# lbl-encode

Select a printer driver by protocol and encode a 1-bit `MonoBitmap` (read as
PBM) into printer-native protocol bytes.

`lbl-encode` owns the `Registry` of drivers. `Registry::with_builtin_drivers()`
includes every bundled driver: DYMO LabelManager tape (`dymo`), DYMO
LabelWriter 550 raster (`dymo-lw`), ESC/POS, ZPL, and TSPL. Additional drivers
can be registered into a custom `Registry`.

## CLI

```bash
lbl-dither label.png --out label.pbm
lbl-encode label.pbm --protocol escpos --width-mm 58 --dpi 203 --out label.bin
# or piped:
cat label.pbm | lbl-encode --protocol zpl --width-mm 100 --length-mm 150 --cut --supports-cut > label.zpl
```

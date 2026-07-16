# lbl-encode

Select a printer driver by protocol and encode a 1-bit `MonoBitmap` (read as
PBM) into printer-native protocol bytes.

`lbl-encode` owns the `Registry` of drivers. `Registry::with_builtin_drivers()`
includes every bundled driver: DYMO LabelManager tape (`dymo`), DYMO
LabelWriter 550 raster (`dymo-lw`), ESC/POS, ZPL, TSPL, NIIMBOT
(`niimbot`; D11/D110 family), Brother QL raster (`brother-ql`; QL-820NWB
family), and the non-hardware preview drivers `virtual`
(raster image file or vector PDF via the orchestrator) and `console` (terminal
art). Additional drivers can be registered into a custom `Registry`.

Protocol-specific firmware/task overrides go through
`Registry::with_driver_variant(protocol, variant)` (or
`with_printer_key` when resolving from a catalog key). Callers pass an opaque
variant string; the registered [`Driver`] interprets it via
`override_for_variant` / `variant_for_printer_key`.

The virtual driver's raster path encodes a dithered bitmap to PNG/BMP/TIFF/GIF/PBM.
Vector PDF export skips `lbl-encode` and uses `lbl-render::export_pdf` instead.

## CLI

```bash
lbl-dither label.png --out label.pbm
lbl-encode label.pbm --protocol escpos --width-mm 58 --dpi 203 --out label.bin
# or piped:
cat label.pbm | lbl-encode --protocol zpl --width-mm 100 --length-mm 150 --cut --supports-cut > label.zpl
```

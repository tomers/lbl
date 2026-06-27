# lbl-driver-escpos

ESC/POS raster driver. Encodes the bitmap with `GS v 0` (raster bit image),
which matches the `MonoBitmap` layout (MSB-first, `1` = printed dot). Emits
`ESC @` init, the raster, a short feed, and an optional `GS V` full cut when the
job requests it and the printer supports it.

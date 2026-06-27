# lbl-driver-tspl

TSPL (TSC Printer Language) driver. Emits `SIZE`/`GAP`/`DIRECTION`/`CLS`/
`BITMAP`/`PRINT`. Note TSPL's `BITMAP` uses the opposite ink convention to
`MonoBitmap` (`1` = white), so the driver inverts the bytes. A requested cut
adds `SET CUTTER 1`.

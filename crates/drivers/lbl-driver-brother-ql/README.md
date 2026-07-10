# lbl-driver-brother-ql

Brother QL-series raster protocol driver for `lbl`.

Encodes a 1-bit [`MonoBitmap`] into the raster command stream documented in
Brother's *Raster Command Reference* for the QL-800 / QL-810W / QL-820NWB
family (including the QL-820NWBc). The print head is 720 dots (90 bytes) wide
at 300 dpi; rows are mirrored left-to-right before transmission.

Supports die-cut and continuous DK media up to 62 mm, auto-cut, and USB or
raw-TCP (port 9100) delivery via the rest of the toolchain.

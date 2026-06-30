# lbl-driver-niimbot

NIIMBOT thermal label driver (protocol `niimbot`), covering the D11/D110 family
and compatibles.

Unlike the streaming page languages, NIIMBOT speaks a packet-framed protocol.
Every command is wrapped as `55 55 <cmd> <len> <data…> <csum> AA AA`, where the
checksum is the XOR of the command, length, and data bytes. A job is a fixed
sequence of setup packets (`SetDensity`, `SetLabelType`, `StartPrint`,
`StartPagePrint`, `SetDimension`, `SetQuantity`) followed by one
`PrintBitmapRow` (`0x85`) packet per raster line and a `EndPagePrint` /
`EndPrint` teardown.

The print head is horizontal (96 dots / 12 mm at 203 dpi on the D110), so the
bitmap's width is the dots across the head and its height is the feed length.
Row payloads pack `ceil(width / 8)` bytes MSB-first with `1` = ink — exactly the
`MonoBitmap` layout — so no conversion is needed. Copies are handled by
`SetQuantity` (the printer repeats the page) rather than re-emitting rows.

D110-family printers reach the host over a USB CDC-ACM serial port, not USB bulk
transfer, so they are driven through `lbl-device`'s bidirectional
`SerialTransport` (`lbl print --protocol niimbot --serial /dev/ttyACM0`).

Because NIIMBOT handshakes, this crate also exposes `status_query()` and
`parse_status()` (plus `frame_packet()` for arbitrary commands): after a page is
sent, the caller polls `GetPrintStatus` and waits for the printer to report the
page complete before sending the next label.

Protocol reference: the
[NIIMBOT community docs](https://printers.niim.blue/interfacing/proto/). `lbl`
is not affiliated with NIIMBOT.

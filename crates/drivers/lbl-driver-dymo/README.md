# lbl-driver-dymo

DYMO drivers for `lbl`. DYMO uses two very different protocols, so this crate
ships two drivers:

## `DymoDriver` — LabelManager tape protocol (`Protocol::Dymo`)

The reference *proprietary* driver. DYMO tape printers have a vertical print
head, so each transmitted line is one **column** of dots across the tape and the
tape feeds horizontally. This driver transposes the `MonoBitmap` into columns
and emits:

- `ESC C 0` — tape color
- `ESC B 0` — reset dot-tab bias (firmware can carry margin across jobs)
- `ESC D n` — bytes per line (`ceil(height/8)`)
- per column: `SYN` (0x16) + `n` column bytes (MSB-first across the tape)
- `ESC A` — status query (host should read the IN endpoint)
- `ESC E` — form feed / cut

The command set is modeled on
[labelle](https://github.com/labelle-org/labelle) (derived from dymoprint).

## `LabelWriter550Driver` — LabelWriter 550 raster protocol (`Protocol::DymoLw`)

Covers the **LabelWriter 550, 550 Turbo, and 5XL** (USB VID `0x0922`, PIDs
`0x0028`/`0x0029`/`0x002A`; 300 dpi; 672-dot / 1248-dot heads), per DYMO's
*LabelWriter 550 Series Technical Reference*. These printers have a horizontal
head and print row-by-row, which maps directly onto the row-major `MonoBitmap`.
It emits a structured print job:

- `ESC s <job-id:u32>` — start of print job
- `ESC i` — select graphics output mode (best for dithered raster)
- `ESC L <lines:u32>` — set length to continuous stock (continuous media only)
- per label: `ESC n <index:u16>`, then
  `ESC D <bpp> <align> <width:u32> <height:u32> <data…>` (width = number of
  lines, height = dots across the head)
- `ESC G` between labels (short form feed) / `ESC E` after the last (feed to
  tear), then `ESC Q` to end the job

Multi-byte fields are little-endian.

`lbl` is not affiliated with DYMO; see the repository disclaimer.

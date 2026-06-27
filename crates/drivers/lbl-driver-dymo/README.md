# lbl-driver-dymo

The reference *proprietary* driver: DYMO LabelManager tape protocol.

DYMO tape printers have a vertical print head, so each transmitted line is one
**column** of dots across the tape and the tape feeds horizontally. This driver
transposes the `MonoBitmap` into columns and emits:

- `ESC C 0` — tape color
- `ESC D n` — bytes per line (`ceil(height/8)`)
- per column: `SYN` (0x16) + `n` column bytes (MSB-first across the tape)
- `ESC E` — form feed / cut

The command set is modeled on
[labelle](https://github.com/labelle-org/labelle) (derived from dymoprint).

`lbl` is not affiliated with DYMO; see the repository disclaimer.

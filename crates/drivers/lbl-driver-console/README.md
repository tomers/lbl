# lbl-driver-console

Console (terminal) preview "printer" for [`lbl`](../../../README.md).

Where [`lbl-driver-file`](../lbl-driver-file/README.md) turns the dithered
`MonoBitmap` into an image *file*, this driver turns it into **terminal art** so
you can eyeball a label without a printer or an image viewer. The raster is
drawn with Unicode half-block characters (`▀`), packing two vertical pixels into
each character cell.

## Two looks

`render_terminal` takes [`TerminalOptions`]:

- **Plain** (`color: false`, the default and what the `Driver` emits): ink is
  drawn with block glyphs (`█ ▀ ▄`) and blank media as spaces. No ANSI escapes,
  so it is safe to redirect to a file or pipe.
- **Color** (`color: true`): each half-cell is painted with ANSI
  foreground/background so the label shows as black ink on white media — the way
  it prints.

Wide rasters are box-downsampled to `max_width` columns (aspect-preserving);
narrow rasters are never upscaled.

## Where it is used

The same `render_terminal` function backs three `lbl` features so they all
agree pixel-for-pixel:

- `lbl print --protocol console` — dump the raster to the terminal.
- `lbl print --confirm …` — preview each label, then ask before printing.
- `lbl print --debug …` — show the dithered raster (and other stages) inline.

```rust
use lbl_driver_console::{render_terminal, TerminalOptions};
use lbl_core::bitmap::MonoBitmap;

let mut bmp = MonoBitmap::new(8, 8);
bmp.set(0, 0, true);
let art = render_terminal(&bmp, &TerminalOptions::default());
print!("{art}");
```

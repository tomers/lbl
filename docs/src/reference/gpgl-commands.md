# GPGL cut commands (Graphtec / Silhouette)

Craft cutters in the Silhouette / Graphtec family speak **GPGL** (Graphtec
Plotter Graphics Language). This engine encodes cut-only jobs as ASCII commands
terminated by `ETX` (`0x03`).

Community reference:
[inkscape-silhouette Commands.md](https://github.com/fablabnbg/inkscape-silhouette/blob/main/Commands.md).

## Units

- Device coordinates: **1/20 mm**.
- Product artboard: millimeters, origin **top-left**, +y down.
- Encoder maps with an axis swap — device `(x, y) = (artboard_y, artboard_x)` —
  matching inkscape-silhouette (`move_mm_cmd(y, x)`).

## Cut settings command subset

| Bytes / command | Role |
| --- | --- |
| `ESC D` (`1B 04`) | Initialize |
| `ESC E` (`1B 05`) | Status: `0` ready, `1` moving, `2` unloaded, `3` paused, `4` cancelled |
| `ESC NUL mask` (`1B 00 xx`) | Simulate panel key (`01` down, `02` up, `04` right, `08` left, `00` release) |
| `TT` | Home cutter |
| `FO n` | Feed `n` device units (1/20 mm) |
| `FG` | Firmware query |
| `FN` / `TB50` | Orientation / regmark off for cut-only |
| `TG` | Cutting mat preset (`0` none, `1` 12×12, `2` 12×24, `8` 15×15, `9` 24×24) |
| `J` | Tool holder (1 / 2; `J0` idle at trailer) |
| `FX` / `!` / `FC` | Force / speed / tool offset (tool-scoped `,n` forms) |
| `TF` | Autoblade depth (`d,1`; Autoblade tool only) |
| `TJ` | Acceleration (`TJ0` then preset) |
| `FY` / `FU` | Track enhance on (`FY0`) / off (`FY1`); usable length |
| `FE` / `FF` | Lift / corner overcut extents (0.1 mm units) |
| `\` / `Z` | Lower-left / upper-right workspace |
| `M` / `D` | Move / draw polylines (repeated for multipass) |

Print-then-cut registration (`TB*`, `FQ5`) is intentionally omitted — see the
product cutter gaps document. Permanent calibration writes (`FB` / `TB72`) are
not emitted by the encoder.

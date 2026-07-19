# GPGL cut commands (Graphtec / Silhouette)

Craft cutters in the Silhouette / Graphtec family speak **GPGL** (Graphtec
Plotter Graphics Language). This engine encodes cut-only jobs as ASCII commands
terminated by `ETX` (`0x03`).

Community reference:
[inkscape-silhouette Commands.md](https://github.com/fablabnbg/inkscape-silhouette/blob/main/Commands.md).

## Units

- Device coordinates: **1/20 mm**.
- Product artboard: millimeters, origin **top-left**, +y down.
- Encoder flips Y to device lower-left origin.

## MVP command subset

| Bytes / command | Role |
| --- | --- |
| `ESC D` (`1B 04`) | Initialize |
| `ESC E` (`1B 05`) | Status: `0` ready, `1` moving, `2` unloaded |
| `FG` | Firmware query |
| `FN` / `TB50` | Orientation / regmark off for cut-only |
| `TG` | Cutting mat preset |
| `FX` / `!` / `FC` | Force / speed / tool offset |
| `\` / `Z` | Lower-left / upper-right workspace |
| `M` / `D` | Move / draw polylines |

Print-then-cut registration (`TB*`, `FQ5`) is intentionally omitted from the MVP
encoder — see the product cutter gaps document.

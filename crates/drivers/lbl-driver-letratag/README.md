# lbl-driver-letratag

Clean-room encoder for the **DYMO LetraTag LT-200B** Bluetooth LE print
protocol. Turns a `MonoBitmap` into the chunked GATT job the printer expects.

`lbl` is **not affiliated** with DYMO / Newell. LetraTag is a trademark of DYMO.

## GATT

| Role | UUID (canonical; first 8 hex digits stable) |
| --- | --- |
| Service | `be3dd650-2b3d-42f1-99c1-f0f749dd0678` |
| Write | `be3dd651-…` |
| Notify | `be3dd652-…` |
| Short command | `be3dd653-…` |

## Avatar job (default)

`ESC s` → `ESC #` → `ESC D` (bpp `0x81`) → `ESC p` → `ESC A` → `ESC Q`,
wrapped in a 9-byte header and ≤500-byte indexed chunks (index skips 27).

See `docs/research-letratag.md` for raster packing, Genie dialect, and license
notes. Do not copy AGPL sources from `alexhorn/lt200b`.

## Protocol name

Catalog / CLI: `letratag` — **never** alias to `dymo` (LabelManager USB).

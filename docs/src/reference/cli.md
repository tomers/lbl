# CLI Reference

All binaries support `--help`. This page summarizes the most common usage.

## `lbl` (orchestrator)

```text
lbl print     Run the full pipeline and print
lbl preview   Produce preview HTML (+ optional PNGs) for a gallery
lbl text      Text/CLI → authoring HTML
lbl transpile Authoring HTML → browser-ready HTML
lbl catalog   Browse the media catalog (list|show|compatible|search)
lbl device    Discover printers (list)
```

### `lbl print`

```text
--text <T> | --html <FILE|-> | --template <FILE> [--data <FILE>] [--each <PTR>]
--media <SKU> | --width-mm <MM> [--length-mm <MM>]   --dpi <DPI>
--protocol <dymo|dymo-lw|escpos|zpl|tspl|niimbot|virtual|console>
                    (dymo-lw = LabelWriter 550 series; niimbot = D11/D110;
                     virtual = image file; console = terminal art)
--supersample <N>   High-res render factor before downscale (default: 3, or
                    `[render] supersample` from config). See Rendering Quality guide.
--dither <auto|floyd-steinberg|ordered|none>
--orientation <portrait|landscape>   Layout orientation (default: landscape, or
                    `[render] orientation` from config)
--rotate-cw  --rotate-ccw   Extra 90° turns, repeatable, composed on top of
                    --orientation
--cut  --supports-cut  --copies <N>
--backend <chromium|sidecar>
--network <host:port> | --usb <vid:pid> | --serial <path[:baud]> | --out-dir <DIR> | --file <FILE>
--confirm   Preview each label as terminal art, then ask before printing
            (or `[print] confirm` / `LBL_PRINT__CONFIRM=1` in config)
--debug     Dump every pipeline stage to stderr (highlighted HTML, the dithered
            raster as terminal art, an encoded-byte preview)
--debug-html <FILE>   Write a standalone HTML report of every pipeline stage
--sample-pattern [<DOTS>]   Print a calibration pattern (no label input/render/dither).
                    Omit DOTS to use the media width in device dots from
                    `--media` / `--width-mm` at `--dpi` (e.g. 96 for NIIMBOT 12 mm
                    @ 203 dpi). Pass an explicit value to override (e.g. 64 on a
                    64-dot DYMO head).
```

Most `lbl print` flags have config/env equivalents under `[print]` /
`LBL_PRINT__*` (see the Configuration guide). `--protocol`, transport targets
(`--bluetooth`, `--serial`, …), `confirm`, `debug`, `cut`, `copies`, and
`dither` are typical candidates for project or shell profile defaults.

`--serial` reaches USB CDC-ACM printers (e.g. NIIMBOT D-series on
`/dev/ttyACM0`, default baud 115200). It is bidirectional, so NIIMBOT prints
wait for the printer's status to confirm completion between labels.

`--protocol console` renders the dithered raster to the terminal as Unicode
half-block art (black ink on white media when stdout is a TTY) instead of
sending it to a device — handy for a quick look without hardware or an image
viewer. With `--file`/`--out-dir` it writes the plain (uncolored) art to a file.

`--confirm` shows that same preview for each label and waits for a `y`
confirmation before printing to any non-console output. `--debug` is a
terminal-native companion to `--debug-html`: it prints each stage's artifacts
(syntax-highlighted when stderr is a TTY) as the pipeline runs. Color for both
follows the destination stream's TTY status and honors `NO_COLOR`.

Media is fixed in the printer (the head width and feed direction don't change),
so `--orientation` controls only how content is *laid out*: `landscape` (the
default) lays text out along the feed — the longer dimension of typical stripe
labels — by rendering in the transposed frame and turning the raster a quarter
onto the head; `portrait` keeps the media's natural `width × length` frame.
`--rotate-cw` / `--rotate-ccw` add further 90° turns on top (repeat them for
180°/270°), so e.g. `--orientation landscape --rotate-cw` prints upside-down
landscape.

### `lbl preview`

```text
(source flags as above)  --out-dir <DIR>  [--render]  [--assets-base <URL>]
```

## Stage binaries

| Binary | Purpose |
| ------ | ------- |
| `lbl-text` | text/CLI → authoring HTML (`--raw`, `--qr/--barcode/--image`, `--fragment`) |
| `lbl-template` | data + template → labels (`--data`, `--each`, `--inline-resources`, `--out-dir`) |
| `lbl-transpile-html` | `--mode print|preview`, `--assets-base`, `--index/--count` |
| `lbl-render` | `--width-dots`, `--height-dots` (either may be omitted for a content-determined axis), `--supersample` (default 3), `--backend`, `--out` |

See [Rendering Quality & Supersampling](../guides/rendering-quality.md) for what
`--supersample` controls and how to choose a value.
| `lbl-dither` | `--algorithm`, `--threshold`, `--preview-png`, `--out` |
| `lbl-pattern` | `--height [<DOTS>]`, `--width-mm`, `--dpi`, `--out` (calibration PBM) |
| `lbl-encode` | `--protocol`, `--sample-pattern [<DOTS>]`, `--width-mm`, `--length-mm`, `--dpi`, `--cut`, `--supports-cut` |
| `lbl-device` | `list`; `send --network host:port | --usb vid:pid | --serial path[:baud]` |
| `lbl-spool` | `--network|--usb|--serial` plus encoded files to queue |
| `lbl-config` | `show`, `sources`, `paths` |
| `lbl-catalog` | `list`, `show <key>`, `compatible --printer <m>`, `search <q>` |
| `lbl-server` | `--bind <addr>` (HTTP API) |

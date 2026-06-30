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
--cut  --supports-cut  --copies <N>
--backend <chromium|sidecar>
--network <host:port> | --usb <vid:pid> | --serial <path[:baud]> | --out-dir <DIR> | --file <FILE>
--confirm   Preview each label as terminal art, then ask before printing
--debug     Dump every pipeline stage to stderr (highlighted HTML, the dithered
            raster as terminal art, an encoded-byte preview)
--debug-html <FILE>   Write a standalone HTML report of every pipeline stage
```

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
| `lbl-render` | `--width-dots`, `--height-dots`, `--supersample` (default 3), `--backend`, `--out` |

See [Rendering Quality & Supersampling](../guides/rendering-quality.md) for what
`--supersample` controls and how to choose a value.
| `lbl-dither` | `--algorithm`, `--threshold`, `--preview-png`, `--out` |
| `lbl-encode` | `--protocol`, `--width-mm`, `--length-mm`, `--dpi`, `--cut`, `--supports-cut` |
| `lbl-device` | `list`; `send --network host:port | --usb vid:pid | --serial path[:baud]` |
| `lbl-spool` | `--network|--usb|--serial` plus encoded files to queue |
| `lbl-config` | `show`, `sources`, `paths` |
| `lbl-catalog` | `list`, `show <key>`, `compatible --printer <m>`, `search <q>` |
| `lbl-server` | `--bind <addr>` (HTTP API) |

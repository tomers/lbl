# CLI Reference

All binaries support `--help`. This page summarizes the most common usage.

## `lbl` (orchestrator)

```text
lbl print     Run the full pipeline and print
lbl preview   Produce preview HTML (+ optional PNGs) for a gallery
lbl text      Text/CLI → authoring HTML
lbl transpile Authoring HTML → browser-ready HTML
lbl catalog   Browse the media catalog (list|show|compatible|search)
lbl config    Inspect layered configuration (show|sources|paths)
lbl device    Discover printers (list|status). Status uses the same printer
              resolution as print: --printer, [print] usb, --profile /
              default_printer, or a single connected LW550.
```

### `lbl print`

```text
--text <T> | --markdown <T|FILE|-> | --html <FILE|-> |
  --template <INLINE|FILE|-> [--data <INLINE|FILE|>] [--each <PTR>]
  [--template-format text|markdown|html]   Override inferred template body format
--filter <TEXT>     Keep labels whose data fields contain TEXT (case-insensitive)
--first             Print the first label from the selection (same as --take 1)
--last              Print the last label from the selection
--skip <N>          Skip the first N labels in the selection (default: 0)
--take <N>          Print at most N labels from the selection
--index <N>         Print only batch index N (zero-based; repeat for several)
--media <SKU> | --width-mm <MM> [--length-mm <MM>]   --dpi <DPI>
--font-size-mm <MM>  --padding-mm <MM>  --border-mm <MM>
                    Inner padding and border on .lbl-label (defaults from
                    [style] padding_mm / border_width_mm, currently 2.0 / 0)
--protocol <dymo|dymo-lw|escpos|esc-label|zpl|tspl|niimbot|virtual|console|html>
                    (dymo-lw = LabelWriter 550 series; niimbot = D11/D110;
                     virtual = file output (raster image or vector PDF);
                     console = terminal art;
                     html = browser gallery of PNG previews)
--export-mode <raster|vector|pdf|bitmap|image>
                    Virtual printer only (default: raster). `raster` runs the
                    full pipeline (supersample → dither → bitmap file).
                    `vector` / `pdf` skip rasterization and emit a vector PDF
                    sized to the configured media via Chromium PrintToPdf.
                    Overrides `[print] export_mode` / `LBL_PRINT__EXPORT_MODE`.
--media-type <png|bmp|tiff|gif|pbm>
                    Virtual printer + raster export only (default: png).
                    Ignored when `--export-mode vector` (output is always PDF).
                    Overrides `[print] media_type` / `LBL_PRINT__MEDIA_TYPE`.
--supersample <N>   High-res render factor before downscale (default: 4, or
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
--preview   Same terminal preview as `--confirm`, then stop (no print)
--debug     Print effective configuration (syntax-highlighted JSON with
            provenance when stderr is a TTY), then dump every pipeline stage
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

`--protocol html` writes a browser gallery backed by a Nuxt UI app: `index.html`
plus an `images/` subdirectory of full-resolution PNGs (one per label). Tabs cover
**Labels** (filterable cards with optional per-label data), **Printer**, **Media**,
**Template**, and **Data**. By default the bundle lands in a temp directory; pass
`--out-dir` or `--file path/to/index.html` to choose the location.
`--open-browser` serves the bundle on `http://127.0.0.1:<port>/` and opens it
(browsers block ES modules on `file://`, so double-clicking `index.html` will
not work). Without `--open-browser`, serve the directory yourself, e.g.
`python3 -m http.server 8080 --bind 127.0.0.1`. Rebuild the embedded UI with
`just preview-ui-build` after editing `crates/lbl/preview-ui/`.

`--confirm` shows that same preview for each label and waits for a single
`y` keypress (`n` or `q` cancels) before printing to any non-console output.
`--preview` shows the preview and exits without printing.
`--debug` is a
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

### Virtual printer (`--protocol virtual`)

Save labels to disk without hardware. Two export modes:

| Mode | Flag | Pipeline | Output |
| ---- | ---- | -------- | ------ |
| **Raster** (default) | `--export-mode raster` | transpile → render → dither → encode image | PNG, BMP, TIFF, GIF, or PBM |
| **Vector** | `--export-mode vector` | transpile → PrintToPdf (no dither) | PDF |

```bash
# Vector PDF — sharp text/QR/barcode at any zoom; page size = media mm
lbl print --text "Hello [[qr:https://x]]" --media 30252 --protocol virtual --export-mode vector --file label.pdf

# Raster PNG — emulates how ink looks after 1-bit dithering on a print head
lbl print --text "Hello [[qr:https://x]]" --media 30252 --protocol virtual --file label.png

# Raster TIFF
lbl print --text "Hello" --width-mm 25 --length-mm 54 --protocol virtual --media-type tiff --file label.tiff
```

- **`--file out.pdf`** / **`--file out.png`**: write one label. With multiple
  labels, siblings are numbered (`out-01.pdf`, `out-02.pdf`, …).
- **`--out-dir dir/`**: write `label-0000.ext`, `label-0001.ext`, …
- **`--supersample`** and **`--dither`** apply to **raster** export only.
- **`--media-type`** is ignored in vector mode (always PDF).
- QR codes and barcodes are rendered as **SVG** in the browser document (sharp
  in both raster screenshots and vector PDF).

See [Rendering Quality — Raster vs vector](../guides/rendering-quality.md#raster-vs-vector-virtual-export)
for a catalog-media PDF example.

### `lbl preview`

```text
(source flags as above)  --out-dir <DIR>  [--render]  [--assets-base <URL>]
```

### `lbl config`

```text
lbl config show      Effective merged config (JSON)
lbl config sources   Provenance: key<TAB>source (default, file path, LBL_* env, …)
lbl config paths     Resolved paths with existence and entry counts
```

Same as the standalone `lbl-config` binary.

## Stage binaries

| Binary | Purpose |
| ------ | ------- |
| `lbl-text` | text/CLI → authoring HTML (`--raw`, `--qr/--barcode/--image`, `--fragment`) |
| `lbl-template` | data + template → labels (`--data`, `--each`, `--inline-resources`, `--out-dir`) |
| `lbl-transpile-html` | `--mode print|preview`,`--assets-base`,`--index/--count` |
| `lbl-render` | `--width-dots`, `--height-dots` (either may be omitted for a content-determined axis), |
| | `--supersample` (default 4), `--backend`, `--out` |
| `lbl-dither` | `--algorithm`, `--threshold`, `--preview-png`, `--out` |
| `lbl-pattern` | `--height [<DOTS>]`, `--width-mm`, `--dpi`, `--out` (calibration PBM) |
| `lbl-encode` | `--protocol`, `--sample-pattern [<DOTS>]`, `--width-mm`, `--length-mm`, |
| | `--dpi`, `--cut`, `--supports-cut` |
| `lbl-device` | `list`; `status --usb vid:pid`; `send --network host:port | --usb vid:pid | --serial path[:baud]` |
| `lbl-spool` | `--network|--usb|--serial` plus encoded files to queue |
| `lbl-config` | `show`, `sources`, `paths` |
| `lbl-catalog` | `list`, `show <key>`, `compatible --printer <m>`, `search <q>` |
| `lbl-server` | `--bind <addr>` (HTTP API) |

See [Rendering Quality & Supersampling](../guides/rendering-quality.md) for what
`--supersample` controls and how to choose a value.

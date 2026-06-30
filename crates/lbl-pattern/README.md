# lbl-pattern

Generate a printer calibration **sample pattern** as a 1-bit [`MonoBitmap`] (PBM).
The layout matches [Labelle](https://github.com/labelle-org/labelle)'s
`SamplePatternRenderEngine`: corner line groups, vertical rules, checkerboards, and
row numbering for margin/head calibration.

The raster is produced at exact device dots and is meant to pass **straight to
[`lbl-encode`](../lbl-encode/)** — no rescaling, rotation, or dithering.

## CLI

```bash
# Emit PBM on stdout, then encode for a DYMO LabelManager (64-dot head):
lbl-pattern --height 64 | lbl-encode --protocol dymo --width-mm 12 --dpi 180

# Or print directly via the orchestrator (head height from `--media`):
lbl print --sample-pattern --media niimbot-12x30 --protocol niimbot --bluetooth D110

# Override head height explicitly (e.g. 64-dot DYMO head):
lbl print --sample-pattern 64 --width-mm 12 --dpi 180 --protocol dymo
```

`--height` / `--sample-pattern` sets the pattern height in dots **across the print head**.
When omitted on `lbl print` / `lbl-encode`, it defaults to the resolved media width
(`--media` / `--width-mm` at `--dpi` — e.g. 96 dots for NIIMBOT 12 mm @ 203 dpi).
Pass an explicit value to override (e.g. `64` for a 64-dot DYMO head).
[margin calibration guide](https://github.com/labelle-org/labelle/blob/main/doc/margin-calibration-howto.md).

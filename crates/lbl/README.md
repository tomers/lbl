# lbl

The orchestrator for the label-printing pipeline. `lbl` runs the pipeline stages
in sequence — each of which is also a standalone `lbl-*` binary and a reusable
library — so a single command can take you from content to a printed label.

## High-level flows

```bash
# Print plain text (text -> transpile -> render -> dither -> encode -> spool)
lbl print --text "Hello {{qr:https://example.com}}" \
  --media 11352 --protocol dymo --usb 0922:1001

# Batch print from a template + data
lbl print --template card.html --data people.json \
  --media 99014 --protocol zpl --network 192.168.1.50:9100 --cut --supports-cut

# Dry run: write encoded bytes to files instead of a printer
lbl print --text "test" --width-mm 25 --protocol escpos --out-dir out/

# Build a preview gallery (browser-ready HTML + optional PNGs + gallery.json)
lbl preview --template card.html --data people.json --out-dir preview/ --render
```

## Stage subcommands

```bash
lbl text "ship to {{qr:...}}"          # text -> authoring HTML
lbl transpile label.html --mode preview
lbl catalog show 11352
lbl device list
lbl config show
```

Granular stages also ship as dedicated binaries: `lbl-text`, `lbl-template`,
`lbl-transpile-html`, `lbl-render`, `lbl-dither`, `lbl-encode`, `lbl-device`,
`lbl-spool`, `lbl-config`, `lbl-catalog`.

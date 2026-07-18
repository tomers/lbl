# Media Catalog

The catalog (`lbl-catalog`) maps stable keys to physical media so you can say
`--media 11352` instead of remembering dimensions.

## Entries

Each entry has:

- `brand` — manufacturer.
- `keys` — SKUs/aliases; the first is canonical (e.g. `11352`, alias `S0722520`).
- `name` — human-friendly label.
- `media` — physical `MediaSpec`: stock `width_mm` across the head, `length`
  (fixed/continuous), `material`, `adhesive`, `color`. Combine with a DPI to
  get device-ready media. Printable-band clamps live on the printer
  (`max_width_mm`, optional `head_printable_height_mm`), not by shrinking
  `width_mm` to the inkable width.
- `image` — optional product image: `url`, `license`, `attribution`,
  `redistributable`.
- `purchase_url` — optional buy link (an affiliate tag may be appended at
  display time).
- `compatible` — printer model strings this media works with.

## Printers

Each `[[printers]]` entry describes a known model (protocol, maturity, DPI,
head width, media, connections). Optional manufacturer support links:

```toml
support = { product_url = "https://…", brand_url = "https://…" }
```

- `product_url` — model-specific support page (drivers, manuals, FAQs).
- `brand_url` — brand-level support hub.

At display time prefer `product_url`, then fall back to `brand_url`.

## Overlays

The bundled catalog (`crates/lbl-catalog/data/catalog.toml`) can be extended
with user TOML/JSON files. Later entries that share a key replace earlier ones:

```bash
lbl-catalog --catalog my-extra.toml list
```

## Image & copyright policy

Product images are referenced by URL with an explicit license. Images may be
**cached locally** regardless of license; only entries marked
`redistributable = true` may be **bundled/redistributed** with the catalog.
Others are hotlinked for display only. See
[ADR-0008](../adr/0008-media-catalog-licensing.md).

## Usage

```bash
lbl-catalog list
lbl-catalog show S0722520
lbl-catalog compatible --printer "LabelWriter 550"
lbl-catalog search durable
```

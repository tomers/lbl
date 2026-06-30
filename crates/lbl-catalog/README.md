# lbl-catalog

Curated database of known label/tape media and printer models.

## Bundled catalog

`data/catalog.toml` ships with the crate and contains:

- **`[[entries]]`** — media SKUs keyed by part number or alias. Each entry
  describes physical dimensions (`width_mm`, fixed or continuous `length`),
  material, adhesive, and color. Device resolution is applied from the target
  printer at resolve time.
- **`[[printers]]`** — known printer models with native DPI, head width,
  protocol, and a `supported_media` list naming the media keys each model can use.
  `connections` declares how to reach the printer (and USB entries double as
  discovery hints).

Users can overlay additional TOML/JSON catalog files; later entries that share
a key replace earlier ones.

## Usage

```rust
use lbl_catalog::Catalog;

let catalog = Catalog::bundled().unwrap();
let media = catalog.lookup("11352").unwrap();
let printer = catalog.lookup_printer("LabelWriter 550").unwrap();
let compatible = catalog.compatible_with("LabelWriter 550");
let dpi = catalog.resolve_dpi(Some("D110"), lbl_core::printer::Protocol::Niimbot, 300.0);
```

## CLI

```bash
lbl-catalog list
lbl-catalog show 11352
lbl-catalog compatible --printer "LabelWriter 550"
lbl-catalog printers list
lbl-catalog printers show D110
lbl-catalog search dymo
```

Overlays:

```bash
lbl-catalog --catalog my-extra.toml list
```

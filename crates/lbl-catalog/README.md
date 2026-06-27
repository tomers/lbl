# lbl-catalog

A curated, versioned database of known label/tape media (keyed by SKU) and their
compatibility with known printers.

Refer to media by a stable key instead of raw dimensions: `--media 11352` (or its
alias `--media S0722520`) resolves to "DYMO 11352 25x54mm Return Address Labels".

## Data

The bundled catalog lives in [`data/catalog.toml`](data/catalog.toml). Users can
overlay additional TOML/JSON catalog files; later entries that share a key
replace earlier ones.

Each entry carries: brand, keys/aliases, display name, a physical `MediaSpec`
(width, fixed/continuous length, material, adhesive, color), an optional
license-aware `image`, an optional `purchase_url` (affiliate tag applied at
display time), and a list of compatible printer models.

## Image / copyright policy

Each `image` records a `url`, a `license`, optional `attribution`, and a
`redistributable` flag. Images may be downloaded and cached locally regardless;
only `redistributable = true` images may be bundled/redistributed with the
catalog. Others are hotlinked for display only.

## Library

```rust
use lbl_catalog::Catalog;
let catalog = Catalog::bundled().unwrap();
let entry = catalog.lookup("11352").unwrap();
let compatible = catalog.compatible_with("LabelWriter 550");
```

## Binary

```bash
lbl-catalog list
lbl-catalog show 11352
lbl-catalog compatible --printer "LabelWriter 550"
lbl-catalog search durable
```

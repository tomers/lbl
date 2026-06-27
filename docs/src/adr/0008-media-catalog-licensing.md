# 0008 — Media catalog & image licensing

- Status: Accepted

## Context

Brands sell media for their printers (e.g. DYMO LabelWriter labels). Users think
in SKUs, not millimeters, and benefit from images and purchase links. Product
images carry varying copyright terms.

## Decision

Maintain a curated catalog (`lbl-catalog`) keying SKUs/aliases to a physical
`MediaSpec`, printer compatibility, optional image, and an optional purchase
URL. Images record an explicit `license`, `attribution`, and a
`redistributable` flag. Policy: images may always be **cached locally**; only
`redistributable = true` images may be **bundled/redistributed** with the
catalog — others are hotlinked for display. Purchase links may carry an
affiliate tag, gated by config (`catalog.affiliate_enabled`).

## Consequences

- Friendly `--media 11352` UX and a rich catalog browser in the UI.
- Copyright is respected by default; redistribution is opt-in per image.
- Users can overlay their own catalog files without forking the bundled data.

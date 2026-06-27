# 0002 — HTML as the input format

- Status: Accepted

## Context

Labels need rich layout: text, fonts, images, QR/barcodes, and flexible
arrangement. We need a content model that designers and developers already know
and that has mature layout/rendering engines.

## Decision

Use HTML (+ CSS) as the canonical input. Authoring HTML adds a few custom
concepts (`<qr>`, `<barcode>`, flex utility classes) that the transpiler expands
into standard, browser-ready HTML. Plain text and templated data are
front-ends that produce authoring HTML.

## Consequences

- Reuse of the entire web layout/typography stack and JS libraries for
  QR/barcodes.
- A single rendering path for both print and in-browser preview.
- Requires an HTML renderer in the pipeline (see ADR-0003).
- Custom elements need a transpilation step, which also lets us target either
  print or preview output.

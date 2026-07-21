# lbl-text

The quick front-end of the pipeline: turn plain text (and directives) into
*authoring HTML* that flows through `lbl-transpile-html` and the rest of the
toolchain.

```bash
lbl-text "hello, world!"
echo "piped text" | lbl-text
```

## Inline mini-syntax (default)

- `[[qr:https://example.com]]` — a QR code (payload only)
- `[[qr ec=low]]payload[[/qr]]` — QR with options (mirrors `<qr ec="low">…</qr>`)
- `[[barcode:CODE128:12345]]` — a barcode (symbology optional: `[[barcode:12345]]`)
- `[[image:./photo.jpg]]` — an image (local path or remote URL)
- `[[size:1.5:World]]` — text at 1.5× the base font size

```bash
lbl-text "ship to [[qr:https://example.com/order/42]]"
```

Unrecognized `[[...]]` is left as literal text; `{{ … }}` is never touched (it
belongs to the `lbl-template` templating layer).

### Sizing text

`[[size:SCALE:text]]` renders its text relative to the base font size, where
`SCALE` is a multiplier: a bare number (`1.5`), an `x` form (`1.5x`), or a
percentage (`150%`). The aliases `font-size` and `fs` also work:

```bash
lbl-text "Total: [[size:2:$42.00]] [[size:0.8:(incl. tax)]]"
```

Because it's relative, sized text scales with whatever base size you configure
(`[style] font_size_mm` or `--font-size-mm`). An invalid scale or empty text
leaves the `[[...]]` literal. It flows inline within Markdown (`lbl-markdown`);
in `lbl-text` it sits on its own line like the other directives.

## Raw mode

```bash
lbl-text --raw "literal [[qr:x]] stays as text"
```

`--raw` disables inline parsing. Flag-based directives still work and are
appended after the text:

```bash
lbl-text --raw "Order #42" --qr "https://example.com/order/42" --barcode "EAN13:4006381333931"
```

## Output

By default emits a full authoring HTML document; pass `--fragment` for just the
`<div class="lbl-label">` element.

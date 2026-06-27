# Batch Printing

Batch printing renders one template against many records.

## Template + data

`card.html`:

```html
<div class="lbl-label lbl-row lbl-center">
  <img src="{{ photo }}" style="height:80px" />
  <div class="lbl-col">
    <strong>{{ name }}</strong>
    <span>{{ title }}</span>
    <qr>{{ url }}</qr>
  </div>
</div>
```

`people.json`:

```json
[
  { "name": "Alice", "title": "Engineer", "url": "https://x/alice", "photo": "https://x/alice.jpg" },
  { "name": "Bob",   "title": "Designer", "url": "https://x/bob",   "photo": "https://x/bob.jpg" }
]
```

A top-level array produces one label per element. Within the template you have
the record's fields, plus `index`, `count`, `it`, and `data`.

```bash
lbl print --template card.html --data people.json \
  --media 99014 --protocol zpl --network 192.168.1.50:9100 --cut --supports-cut
```

## Selecting a sub-array

If your data wraps the records, point at them with `--each`:

```bash
lbl print --template card.html --data payload.json --each /people ...
```

## Single-file (frontmatter)

Data and template can live together:

```text
---toml
[[items]]
name = "Alice"
[[items]]
name = "Bob"
---
<div class="lbl-label">{{ name }}</div>
```

```bash
lbl-template --template combined.html --each /items --out-dir out/
```

## Resources

With `lbl-template --inline-resources`, `<img src>` references (local or remote)
are fetched and inlined as `data:` URIs so the renderer gets a self-contained
document — handy for ID cards with photos.

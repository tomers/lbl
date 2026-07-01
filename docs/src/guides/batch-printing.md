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

## Selecting which labels to print

After the batch is resolved, narrow it with the same substring filter as the
HTML preview UI (`--filter`), or with iterator-style flags:

```bash
# Only Carmela (case-insensitive match on any data field)
lbl print --template card.html --data people.json --filter car ...

# First label only
lbl print --template card.html --data people.json --one ...

# Skip the first two, then print up to three
lbl print --template card.html --data people.json --skip 2 --take 3 ...

# Explicit zero-based batch index (repeat for several)
lbl print --template card.html --data people.json --index 0 --index 2 ...
```

Selection order: `--index` (if any) → `--filter` → `--skip` → `--take` /
`--one`.

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

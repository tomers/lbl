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
the record's fields, plus `index`, `count`, `it`, and `data`. When a record
defines a field with one of those names, the record's value wins.

Templates default to plain **text** for inline templates and unknown
extensions. HTML layouts like `card.html` or `combined.lbl` are detected
automatically; override with `--template-format` when needed:

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
lbl print --template card.html --data people.json --first ...

# Last label only
lbl print --template card.html --data people.json --last ...

# Skip the first two, then print up to three
lbl print --template card.html --data people.json --skip 2 --take 3 ...

# Explicit zero-based batch index (repeat for several)
lbl print --template card.html --data people.json --index 0 --index 2 ...
```

Selection order: `--index` (if any) → `--filter` → `--skip` → `--take` /
`--first` / `--last`.

The HTTP API accepts the same selection as an optional `selection` object on
`/api/preview`, `/api/preview/html`, `/api/print`, and `/api/print/file`:

```json
{
  "template": "<div>{{ name }}</div>",
  "data": [{"name": "Alice"}, {"name": "Bob"}, {"name": "Carol"}],
  "selection": { "indices": [0, 2] }
}
```

Fields match the CLI: `filter` (string), `skip` (number), `take` (number),
`last` (boolean), and `indices` (array of zero-based batch indices). Omitted
`selection` means the full batch. Original batch indices are preserved on each
label, so template fields like `{{ index }}` stay correct for the selected
subset.

## Shell iteration (`seq` and `xargs`)

When values come from the shell instead of a JSON file, run `lbl print` once per
value:

```bash
seq 1 3 | xargs -n1 lbl print --template 'User #{{ it }}' --media 12x30 --dpi 203 --protocol console --data
```

Put `--data` **last**. `xargs -n1` appends each line from `seq` as the value of
`--data`, so the three runs are effectively `--data 1`, `--data 2`, and
`--data 3`. Inline JSON scalars are supported; in the template `{{ it }}` is
that number (`index` is always `0` and `count` is `1` on each run).

Templates default to **text** for inline templates and unknown extensions.
`.html`, `.htm`, and `.lbl` paths are treated as HTML; `.md` / `.markdown` as
Markdown. Override with `--template-format` when inference is wrong.

Inner **padding** (default 2 mm) is added automatically on `.lbl-label`; on
small tape you may want `--padding-mm 0` (or `padding_mm = 0` in config). See
[Configuration — padding and insets](../guides/configuration.md#padding-and-insets).

During development from a checkout:

```bash
seq 1 3 | xargs -n1 cargo run -q -p lbl --bin lbl -- print \
  --template 'User #{{ it }}' \
  --media 12x30 --dpi 203 --protocol console \
  --data
```

For a named field, substitute into a small JSON object with `xargs -I`:

```bash
seq 1 3 | xargs -I%n lbl print --template 'User #{{ n }}' --media 12x30 --dpi 203 --protocol console --data '{"n":%n}'
```

(`-I` runs one command per input line, like `-n1`, but replaces `%n` in the
command string.)

Compare with a **single** invocation that batches every record in one JSON array
(see above): one render/spool pass, shared transport, and one confirmation
prompt when `--confirm` is set.

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

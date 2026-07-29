# lbl-template

The templating "preprocessor": render a template against data (JSON/TOML/YAML)
into one or more authoring-HTML labels, fetching and inlining external image
resources along the way.

- Templating via [minijinja](https://docs.rs/minijinja) (Jinja2 semantics:
  `{{ name }}`, loops, conditionals).
- Data and template can share one file via frontmatter (JSX-like):

```text
---toml
name = "Alice"
---
<div class="lbl-label">{{ name }}</div>
```

- Batch: a top-level data array (or `--each /items`) expands into N labels.
  Each render exposes object fields at the top level plus `it`, `index`
  (0-based), `serial` (`index + 1`), `count`, and `data`. Record fields take
  precedence over those bindings (a record's own `index` or `serial` wins; the
  record is always reachable via `it`).
- Resources: `--inline-resources` fetches `<img src>` (local path or URL) and
  inlines them as `data:` URIs so the renderer gets a self-contained document.

## CLI

```bash
# Single label from inline frontmatter
lbl-template --template label.html

# Batch over a JSON array, inlining photos
lbl-template --template card.html --data people.json --inline-resources --out-dir out/

# Pipe a template in, fetch data from a URL
cat card.html | lbl-template --data https://example.com/people.yaml --each /people
```

Without `--out-dir`, a single label is printed to stdout; a batch is printed as
NDJSON (`{"index":N,"html":"..."}` per line).

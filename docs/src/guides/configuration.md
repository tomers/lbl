# Configuration

`lbl` uses layered configuration with idiomatic precedence (lowest to highest):

1. Built-in defaults
2. System file — `/etc/lbl/config.toml`
3. User file — `~/.config/lbl/config.toml`
4. Project file — `./lbl.toml`
5. Environment — `LBL_*` (nested with `__`, e.g. `LBL_RENDER__SUPERSAMPLE=4`)
6. Explicit CLI flags

## Inspect

```bash
lbl-config show      # effective merged config (JSON)
lbl-config sources   # which layer supplied each value
lbl-config paths     # resolved file locations
```

The provenance view is exposed by `GET /api/config/sources` for HTTP clients.

## Keys

```toml
[general]
default_printer = "my-dymo"     # matches a saved profile id
# cache_dir = "/var/cache/lbl"

[render]
supersample = 3   # high-res first pass factor (>= 1); see Rendering Quality guide
dither = "floyd-steinberg"      # auto | floyd-steinberg | ordered | none
use_sidecar = false
orientation = "landscape"       # portrait | landscape (default: landscape)

[catalog]
affiliate_enabled = true
# affiliate_tag = "mytag"
extra_paths = ["./my-catalog.toml"]
```

`[render] supersample` applies to `lbl print` (unless overridden by
`--supersample`) and is exposed on the HTTP print API. Preview rasterization
uses a fixed factor of `2` for speed. See
[Rendering Quality & Supersampling](./rendering-quality.md) for what the factor
does, how it interacts with style sizing, and tuning advice.

`[render] orientation` sets the default layout orientation for `lbl print`
(override per-run with `--orientation`, and add quarter-turns with
`--rotate-cw` / `--rotate-ccw`). It defaults to `landscape` because stripe
labels are usually printed along their longer dimension. Orientation changes
only how content is laid out and rotated onto the head; it never changes the
media's physical width or feed length.

## Printer profiles

User-owned printers are persisted separately (in `printers.toml`) so a
disconnected printer keeps its desired configuration. Manage them via the API
(`/api/printers/profiles`).

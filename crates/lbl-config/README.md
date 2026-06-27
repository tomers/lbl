# lbl-config

Layered configuration for the `lbl` toolchain, built on
[figment](https://docs.rs/figment).

## Precedence

Sources are merged lowest-to-highest:

1. Built-in defaults
2. System config file (`/etc/lbl/config.toml`)
3. User config file (`~/.config/lbl/config.toml`)
4. Project/local config file (`./lbl.toml`)
5. Environment variables (`LBL_*`, nested with `__`, e.g. `LBL_RENDER__SUPERSAMPLE=4`)
6. Explicit CLI overrides

## Library

```rust,no_run
use lbl_config::Loader;

let cfg = Loader::new().load().unwrap();
println!("supersample = {}", cfg.render.supersample);
```

`describe_sources()` reports which layer supplied each effective value, which
powers the `GET /api/config/sources` provenance view.

## Printer profile persistence

`ProfileStore` persists user-owned printers in `printers.toml` so a
disconnected printer retains its desired configuration across runs.

## Binary

```bash
lbl-config show      # effective config as JSON
lbl-config sources   # provenance of each value
lbl-config paths     # resolved file locations
```

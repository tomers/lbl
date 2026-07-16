# Writing a Driver

A driver translates a 1-bit `MonoBitmap` into a printer's protocol bytes. Adding
support for a new printer family is a small, self-contained task.

## 1. Create the crate

Add `crates/drivers/lbl-driver-foo/` and list it in the workspace `members`.

```toml
# crates/drivers/lbl-driver-foo/Cargo.toml
[package]
name = "lbl-driver-foo"
version.workspace = true
edition.workspace = true

[dependencies]
lbl-core = { workspace = true }
lbl-driver-api = { workspace = true }
```

## 2. Implement the `Driver` trait

```rust
use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

pub struct FooDriver;

impl Driver for FooDriver {
    fn protocol(&self) -> Protocol { Protocol::Foo } // add the variant to lbl-core
    fn name(&self) -> &'static str { "foo" }
    fn aliases(&self) -> &'static [&'static str] { &["foo"] }

    fn encode(&self, bmp: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        let mut out = Vec::new();
        for _ in 0..ctx.copies() {
            // Emit header...
            // Walk rows/columns of `bmp`. Bit set (`bmp.get(x, y)`) means ink.
            // `bmp.stride()` is bytes per row, MSB-first.
            if ctx.should_cut() {
                // ...cut command...
            }
        }
        Ok(out)
    }
}
```

### Things to get right

- **Ink convention.** `MonoBitmap` uses `1 = ink`. Some protocols invert this
  (e.g. TSPL uses `1 = white`); invert the bytes if so.
- **Head orientation.** Tape printers (DYMO) have a vertical head — transpose
  the bitmap into columns. Most others are row-oriented and can use
  `bmp.data` / `bmp.row(y)` directly.
- **Copies & cut.** Honor `ctx.copies()` and only cut when `ctx.should_cut()`
  (job requested **and** printer supports it).
- **Limits.** Reject inputs that exceed the protocol's field sizes with
  `DriverError::Unsupported`.
- **Client handshake.** If bidirectional clients must pace or wait for status
  after encode, override `handshake()` (default is fire-and-forget). Do not
  add protocol branches in the server — the driver owns the strategy.
- **Protocol aliases.** Override `aliases()` with the wire / CLI / API ids
  (and brand synonyms) this driver claims. `Registry::resolve_protocol` picks
  the unique match; conflicts fail. Do not list printer model catalog keys
  here — those go through catalog resolution and `variant_for_printer_key`.

## 3. Register it

In `lbl-encode`'s `Registry::with_builtin_drivers`:

```rust
registry.register(Box::new(lbl_driver_foo::FooDriver::new()));
```

## 4. Test it

Encode a tiny known bitmap and assert on the header/data bytes — see the
existing drivers' unit tests for the pattern. No hardware required.

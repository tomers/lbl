# lbl-driver-api

The driver plugin contract for `lbl`.

A `Driver` encodes a dithered `MonoBitmap` into the byte stream a specific
printer protocol expects:

```rust
use lbl_driver_api::{Driver, EncodeContext, MonoBitmap, Protocol};

pub struct MyDriver;
impl Driver for MyDriver {
    fn protocol(&self) -> Protocol { Protocol::EscPos }
    fn name(&self) -> &'static str { "my-driver" }
    fn encode(&self, bmp: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, lbl_driver_api::DriverError> {
        // ... emit protocol bytes ...
        Ok(Vec::new())
    }
}
```

`EncodeContext` exposes the job and printer capabilities, plus helpers like
`should_cut()` (job requested a cut AND the printer supports it) and `copies()`.
Optional planes: `with_secondary` for dual-ink media, `with_color_png` for
full-color graphic registration (e.g. ESC/Label `~DY`).

Optional hooks (defaults are no-ops / fire-and-forget):

- `aliases` / `matches_id` — wire / CLI / API ids this driver claims;
  `lbl-encode::Registry::resolve_protocol` selects the unique match
- `handshake` — client delivery strategy after encode (`ClientHandshake`)
- `variant_for_printer_key` / `override_for_variant` — firmware/task profiles;
  the registry forwards opaque variant strings without naming individual drivers

Concrete drivers live alongside this crate under `crates/drivers/`:
`lbl-driver-dymo`, `lbl-driver-escpos`, `lbl-driver-zpl`, `lbl-driver-tspl`,
`lbl-driver-niimbot`, and the non-hardware preview drivers `lbl-driver-file`
(image file + HTML gallery) and `lbl-driver-console` (terminal art).
`lbl-encode` aggregates them and selects one by `Protocol`.

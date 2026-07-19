# lbl-core

Shared types for the `lbl` label-printing toolchain. Every pipeline stage and
binary depends on this crate for a common vocabulary:

- `units` — `Millimeters`, `Dots`, `Dpi` and conversions between physical and
  device units.
- `geometry` — `Size<T>` and `Margins`.
- `media` — `Media` profiles (width, fixed/continuous length, material,
  adhesive, color).
- `printer` — `DeviceModel`, `Transport` (USB/network), `DeviceCapabilities`,
  and the persisted `DeviceProfile`.
- `job` — `JobSpec` and `OutputMode` (print vs preview).

This crate is dependency-light (only `serde` + `thiserror`) so it can sit at the
bottom of the dependency graph.

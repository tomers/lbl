//! Shared CLI presentation for lbl binaries.
//!
//! Help styling follows the same preset as `cargo` subcommands (via
//! [`clap_cargo::style::CLAP_STYLING`]): green section headers, cyan flags,
//! and colored error hints — without hand-picking ANSI codes in each binary.

pub use clap_cargo::style::CLAP_STYLING;

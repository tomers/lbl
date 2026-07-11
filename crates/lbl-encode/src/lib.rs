//! Select a printer driver by [`Protocol`] and encode a bitmap into protocol
//! bytes.
//!
//! `lbl-encode` owns the [`Registry`] of available [`Driver`]s. The default
//! registry includes every bundled driver (DYMO, ESC/POS, ZPL, TSPL); a custom
//! registry can register additional drivers.
//!
//! ```
//! use lbl_encode::Registry;
//! use lbl_core::{bitmap::MonoBitmap, job::JobSpec, media::Media, printer::{PrinterCapabilities, Protocol}, units::Dpi};
//! use lbl_driver_api::EncodeContext;
//!
//! let registry = Registry::with_builtin_drivers();
//! let driver = registry.get(Protocol::EscPos).unwrap();
//! let bmp = MonoBitmap::new(8, 1);
//! let job = JobSpec::new(Media::continuous(58.0, Dpi(203.0)));
//! let caps = PrinterCapabilities::default();
//! let bytes = driver.encode(&bmp, &EncodeContext::new(&job, &caps)).unwrap();
//! assert!(!bytes.is_empty());
//! ```

use std::collections::HashMap;

use lbl_driver_api::{Driver, Protocol};

/// Errors produced by the encode stage.
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    /// No driver is registered for the requested protocol.
    #[error("no driver registered for protocol {0:?}")]
    NoDriver(Protocol),

    /// A driver failed to encode.
    #[error(transparent)]
    Driver(#[from] lbl_driver_api::DriverError),
}

/// A registry of available drivers, keyed by protocol.
#[derive(Default)]
pub struct Registry {
    drivers: HashMap<Protocol, Box<dyn Driver>>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry pre-populated with every bundled driver.
    pub fn with_builtin_drivers() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(lbl_driver_dymo::DymoDriver::new()));
        registry.register(Box::new(lbl_driver_dymo::LabelWriter550Driver::new()));
        registry.register(Box::new(lbl_driver_dymo::LabelWriter450Driver::new()));
        registry.register(Box::new(lbl_driver_escpos::EscPosDriver::new()));
        registry.register(Box::new(lbl_driver_escpos::PhomemoDriver::new()));
        registry.register(Box::new(lbl_driver_escpos::PhomemoM02xDriver::new()));
        registry.register(Box::new(lbl_driver_escpos::PhomemoM110Driver::new()));
        registry.register(Box::new(lbl_driver_escpos::PhomemoD30Driver::new()));
        registry.register(Box::new(lbl_driver_zpl::ZplDriver::new()));
        registry.register(Box::new(lbl_driver_tspl::TsplDriver::new()));
        registry.register(Box::new(lbl_driver_niimbot::NiimbotDriver::new()));
        registry.register(Box::new(lbl_driver_brother_ql::BrotherQlDriver::new()));
        registry.register(Box::new(lbl_driver_brother_pt::BrotherPtDriver::new()));
        registry.register(Box::new(lbl_driver_file::FileDriver::default()));
        registry.register(Box::new(lbl_driver_console::ConsoleDriver::new()));
        registry
    }

    /// Register (or replace) the driver for its protocol.
    pub fn register(&mut self, driver: Box<dyn Driver>) {
        self.drivers.insert(driver.protocol(), driver);
    }

    /// Look up the driver for a protocol.
    pub fn get(&self, protocol: Protocol) -> Option<&dyn Driver> {
        self.drivers.get(&protocol).map(|d| d.as_ref())
    }

    /// Protocols with a registered driver.
    pub fn protocols(&self) -> Vec<Protocol> {
        self.drivers.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_has_all_protocols() {
        let registry = Registry::with_builtin_drivers();
        for p in [
            Protocol::Dymo,
            Protocol::DymoLw,
            Protocol::DymoLwClassic,
            Protocol::EscPos,
            Protocol::Phomemo,
            Protocol::PhomemoM02x,
            Protocol::PhomemoM110,
            Protocol::PhomemoD30,
            Protocol::Zpl,
            Protocol::Tspl,
            Protocol::Niimbot,
            Protocol::BrotherQl,
            Protocol::BrotherPt,
            Protocol::Virtual,
            Protocol::Console,
        ] {
            assert!(registry.get(p).is_some(), "missing driver for {p:?}");
        }
    }
}

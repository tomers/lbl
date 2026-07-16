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

use lbl_driver_api::{ClientHandshake, Driver, Protocol};

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
        registry.register(Box::new(lbl_driver_esclabel::EscLabelDriver::new()));
        registry.register(Box::new(lbl_driver_tspl::TsplDriver::new()));
        registry.register(Box::new(lbl_driver_slcs::SlcsDriver::new()));
        registry.register(Box::new(lbl_driver_ezpl::EzplDriver::new()));
        registry.register(Box::new(lbl_driver_sbpl::SbplDriver::new()));
        registry.register(Box::new(lbl_driver_dpl::DplDriver::new()));
        registry.register(Box::new(lbl_driver_tpcl::TpclDriver::new()));
        registry.register(Box::new(lbl_driver_niimbot::NiimbotDriver::new()));
        registry.register(Box::new(lbl_driver_letratag::LetraTagDriver::new()));
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

    /// Override the builtin driver for `protocol` when `variant` selects a
    /// non-default implementation (firmware / task profile, etc.).
    ///
    /// Protocols with no variant support ignore `variant`. Callers stay
    /// protocol-agnostic: pass the string through; the matching driver
    /// interprets it via [`Driver::override_for_variant`].
    pub fn with_driver_variant(mut self, protocol: Protocol, variant: Option<&str>) -> Self {
        let override_driver = self
            .get(protocol)
            .and_then(|d| d.override_for_variant(variant));
        if let Some(driver) = override_driver {
            self.register(driver);
        }
        self
    }

    /// Resolve a catalog printer key to a driver-variant string for `protocol`.
    ///
    /// Asks the registered driver for that protocol
    /// ([`Driver::variant_for_printer_key`]); returns `None` when there is no
    /// driver or no mapping.
    pub fn driver_variant_for_printer_key(
        protocol: Protocol,
        printer_key: &str,
    ) -> Option<&'static str> {
        Self::with_builtin_drivers()
            .get(protocol)
            .and_then(|d| d.variant_for_printer_key(printer_key))
    }

    /// Client delivery handshake for `protocol` ([`Driver::handshake`]).
    ///
    /// Returns [`ClientHandshake::FireAndForget`] when no driver is registered.
    pub fn handshake_for(protocol: Protocol) -> ClientHandshake {
        Self::with_builtin_drivers()
            .get(protocol)
            .map(|d| d.handshake())
            .unwrap_or_default()
    }

    /// Apply any catalog-key driver override for `protocol`.
    pub fn with_printer_key(self, protocol: Protocol, printer_key: Option<&str>) -> Self {
        let variant = printer_key.and_then(|key| {
            self.get(protocol)
                .and_then(|d| d.variant_for_printer_key(key))
        });
        self.with_driver_variant(protocol, variant)
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
            Protocol::EscLabel,
            Protocol::Tspl,
            Protocol::Slcs,
            Protocol::Ezpl,
            Protocol::Sbpl,
            Protocol::Dpl,
            Protocol::Tpcl,
            Protocol::Niimbot,
            Protocol::LetraTag,
            Protocol::BrotherQl,
            Protocol::BrotherPt,
            Protocol::Virtual,
            Protocol::Console,
        ] {
            assert!(registry.get(p).is_some(), "missing driver for {p:?}");
        }
    }

    #[test]
    fn with_driver_variant_overrides_niimbot_b1() {
        let registry =
            Registry::with_builtin_drivers().with_driver_variant(Protocol::Niimbot, Some("b1"));
        let driver = registry.get(Protocol::Niimbot).unwrap();
        assert_eq!(driver.name(), "niimbot");
    }

    #[test]
    fn with_driver_variant_ignores_unrelated_protocol() {
        let registry =
            Registry::with_builtin_drivers().with_driver_variant(Protocol::EscPos, Some("b1"));
        assert!(registry.get(Protocol::EscPos).is_some());
    }

    #[test]
    fn handshake_comes_from_driver() {
        assert_eq!(
            Registry::handshake_for(Protocol::Dymo),
            ClientHandshake::DymoD1
        );
        assert_eq!(
            Registry::handshake_for(Protocol::DymoLw),
            ClientHandshake::DymoLw
        );
        assert_eq!(
            Registry::handshake_for(Protocol::DymoLwClassic),
            ClientHandshake::FireAndForget
        );
        assert_eq!(
            Registry::handshake_for(Protocol::Niimbot),
            ClientHandshake::NiimbotPoll
        );
        assert_eq!(
            Registry::handshake_for(Protocol::LetraTag),
            ClientHandshake::LetraTagNotify
        );
        assert_eq!(
            Registry::handshake_for(Protocol::EscPos),
            ClientHandshake::FireAndForget
        );
    }

    #[test]
    fn driver_variant_for_printer_key_maps_b1() {
        assert_eq!(
            Registry::driver_variant_for_printer_key(Protocol::Niimbot, "B21"),
            Some("b1")
        );
        assert_eq!(
            Registry::driver_variant_for_printer_key(Protocol::EscPos, "B21"),
            None
        );
    }
}

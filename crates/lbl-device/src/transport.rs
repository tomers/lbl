//! Transports that deliver encoded bytes to a printer.

use crate::DeviceError;

/// A transport sends a finished protocol byte stream to a printer.
pub trait Transport {
    /// Send all bytes to the device.
    fn send(&mut self, data: &[u8]) -> Result<(), DeviceError>;
}

/// A network (raw TCP, e.g. port 9100) transport.
#[derive(Debug, Clone)]
pub struct NetworkTransport {
    host: String,
    port: u16,
}

impl NetworkTransport {
    /// Create a transport to `host:port`.
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }
}

impl Transport for NetworkTransport {
    fn send(&mut self, data: &[u8]) -> Result<(), DeviceError> {
        use std::io::Write;
        let mut stream = std::net::TcpStream::connect((self.host.as_str(), self.port))
            .map_err(|e| DeviceError::Transport(format!("connect {}:{}: {e}", self.host, self.port)))?;
        stream
            .write_all(data)
            .map_err(|e| DeviceError::Transport(format!("write: {e}")))?;
        stream
            .flush()
            .map_err(|e| DeviceError::Transport(format!("flush: {e}")))?;
        Ok(())
    }
}

/// A USB bulk-out transport, identifying the device by vendor/product id (and
/// optionally serial number).
#[cfg(feature = "usb")]
#[derive(Debug, Clone)]
pub struct UsbTransport {
    /// USB vendor id.
    pub vendor_id: u16,
    /// USB product id.
    pub product_id: u16,
    /// Optional serial number to disambiguate multiple identical devices.
    pub serial: Option<String>,
    /// Interface number to claim.
    pub interface: u8,
    /// Bulk OUT endpoint address.
    pub endpoint: u8,
}

#[cfg(feature = "usb")]
impl UsbTransport {
    /// Create a USB transport. Defaults to interface 0 and endpoint 0x01 if not
    /// overridden afterward.
    pub fn new(vendor_id: u16, product_id: u16, serial: Option<String>) -> Self {
        Self {
            vendor_id,
            product_id,
            serial,
            interface: 0,
            endpoint: 0x01,
        }
    }
}

#[cfg(feature = "usb")]
impl Transport for UsbTransport {
    fn send(&mut self, data: &[u8]) -> Result<(), DeviceError> {
        let device_info = nusb::list_devices()
            .map_err(|e| DeviceError::Transport(format!("listing usb devices: {e}")))?
            .find(|d| {
                d.vendor_id() == self.vendor_id
                    && d.product_id() == self.product_id
                    && self
                        .serial
                        .as_deref()
                        .map(|s| d.serial_number() == Some(s))
                        .unwrap_or(true)
            })
            .ok_or_else(|| {
                DeviceError::NotFound(format!(
                    "usb {:04x}:{:04x}",
                    self.vendor_id, self.product_id
                ))
            })?;

        let device = device_info
            .open()
            .map_err(|e| DeviceError::Transport(format!("opening device: {e}")))?;
        let interface = device
            .claim_interface(self.interface)
            .map_err(|e| DeviceError::Transport(format!("claiming interface: {e}")))?;

        pollster::block_on(async {
            let completion = interface.bulk_out(self.endpoint, data.to_vec()).await;
            completion
                .status
                .map_err(|e| DeviceError::Transport(format!("bulk out: {e}")))
        })
    }
}

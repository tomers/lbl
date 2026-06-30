//! Transports that deliver encoded bytes to a printer.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::DeviceError;

/// A transport sends a finished protocol byte stream to a printer, and — for
/// bidirectional links — can read responses back.
///
/// Most printer languages (DYMO, ESC/POS, ZPL, TSPL) are write-only streams, so
/// [`Transport::receive`] and [`Transport::is_bidirectional`] have defaults that
/// model a one-way link. Transports over a duplex link (e.g. a serial port)
/// override them so protocols that handshake (e.g. NIIMBOT status polling) can
/// read device replies.
pub trait Transport {
    /// Send all bytes to the device.
    fn send(&mut self, data: &[u8]) -> Result<(), DeviceError>;

    /// Whether this transport can read responses back from the device.
    ///
    /// Defaults to `false` (write-only).
    fn is_bidirectional(&self) -> bool {
        false
    }

    /// Read bytes the device has sent back, waiting up to `timeout` for the
    /// first byte to arrive.
    ///
    /// Returns the bytes read (possibly empty if the device stayed silent).
    /// Write-only transports return an empty vector without waiting.
    fn receive(&mut self, timeout: Duration) -> Result<Vec<u8>, DeviceError> {
        let _ = timeout;
        Ok(Vec::new())
    }
}

/// A "virtual printer" transport that writes each job's bytes to a file.
///
/// The first job is written to the configured path verbatim; subsequent jobs
/// get a `-NN` suffix inserted before the extension (`out.png`, `out-01.png`,
/// ...), so a multi-label batch produces one file per label.
#[derive(Debug, Clone)]
pub struct FileTransport {
    path: PathBuf,
    written: usize,
}

impl FileTransport {
    /// Create a transport that writes to `path` (and numbered siblings for
    /// additional jobs).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            written: 0,
        }
    }

    /// The path the next job will be written to.
    fn next_path(&self) -> PathBuf {
        if self.written == 0 {
            return self.path.clone();
        }
        numbered_path(&self.path, self.written)
    }
}

/// Insert a zero-padded `-NN` suffix before the file extension.
fn numbered_path(path: &Path, n: usize) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("label");
    let name = match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{stem}-{n:02}.{ext}"),
        None => format!("{stem}-{n:02}"),
    };
    path.with_file_name(name)
}

impl Transport for FileTransport {
    fn send(&mut self, data: &[u8]) -> Result<(), DeviceError> {
        let target = self.next_path();
        if let Some(parent) = target.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    DeviceError::Transport(format!("create {}: {e}", parent.display()))
                })?;
            }
        }
        std::fs::write(&target, data)
            .map_err(|e| DeviceError::Transport(format!("write {}: {e}", target.display())))?;
        self.written += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_path_inserts_suffix_before_extension() {
        let p = numbered_path(Path::new("/tmp/out.png"), 3);
        assert_eq!(p, PathBuf::from("/tmp/out-03.png"));
        let p = numbered_path(Path::new("out"), 1);
        assert_eq!(p, PathBuf::from("out-01"));
    }

    #[test]
    fn file_transport_writes_numbered_siblings() {
        let dir = std::env::temp_dir().join(format!("lbl-file-transport-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("label.png");
        let mut t = FileTransport::new(base.clone());
        t.send(b"first").unwrap();
        t.send(b"second").unwrap();

        assert_eq!(std::fs::read(&base).unwrap(), b"first");
        assert_eq!(std::fs::read(dir.join("label-01.png")).unwrap(), b"second");
        std::fs::remove_dir_all(&dir).ok();
    }
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
        let mut stream =
            std::net::TcpStream::connect((self.host.as_str(), self.port)).map_err(|e| {
                DeviceError::Transport(format!("connect {}:{}: {e}", self.host, self.port))
            })?;
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

/// Default baud rate for NIIMBOT-style USB CDC-ACM serial printers.
#[cfg(feature = "serial")]
pub const DEFAULT_SERIAL_BAUD: u32 = 115_200;

/// A bidirectional serial-port transport (USB CDC-ACM, e.g. `/dev/ttyACM0`).
///
/// Unlike the write-only transports, this keeps the port open across calls so a
/// protocol can interleave writes and reads (e.g. send a print job, then poll
/// the printer for status). The port is opened lazily on first use so that
/// constructing the transport never fails.
#[cfg(feature = "serial")]
pub struct SerialTransport {
    path: String,
    baud: u32,
    port: Option<Box<dyn serialport::SerialPort>>,
}

#[cfg(feature = "serial")]
impl std::fmt::Debug for SerialTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SerialTransport")
            .field("path", &self.path)
            .field("baud", &self.baud)
            .field("open", &self.port.is_some())
            .finish()
    }
}

#[cfg(feature = "serial")]
impl SerialTransport {
    /// Create a serial transport for `path` at `baud` (use
    /// [`DEFAULT_SERIAL_BAUD`] if unsure).
    pub fn new(path: impl Into<String>, baud: u32) -> Self {
        Self {
            path: path.into(),
            baud,
            port: None,
        }
    }

    /// Open (lazily) and return the underlying port.
    fn port(&mut self) -> Result<&mut dyn serialport::SerialPort, DeviceError> {
        if self.port.is_none() {
            let port = serialport::new(&self.path, self.baud)
                .timeout(Duration::from_millis(500))
                .open()
                .map_err(|e| DeviceError::Transport(format!("open serial {}: {e}", self.path)))?;
            self.port = Some(port);
        }
        Ok(self.port.as_mut().expect("just opened").as_mut())
    }
}

#[cfg(feature = "serial")]
impl Transport for SerialTransport {
    fn send(&mut self, data: &[u8]) -> Result<(), DeviceError> {
        let port = self.port()?;
        port.write_all(data)
            .map_err(|e| DeviceError::Transport(format!("serial write: {e}")))?;
        port.flush()
            .map_err(|e| DeviceError::Transport(format!("serial flush: {e}")))?;
        Ok(())
    }

    fn is_bidirectional(&self) -> bool {
        true
    }

    fn receive(&mut self, timeout: Duration) -> Result<Vec<u8>, DeviceError> {
        let port = self.port()?;
        // Wait up to `timeout` for the first byte, then drain the rest of the
        // frame with a short inter-byte timeout.
        port.set_timeout(timeout)
            .map_err(|e| DeviceError::Transport(format!("serial set_timeout: {e}")))?;
        let mut out = Vec::new();
        let mut buf = [0u8; 256];
        loop {
            match port.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    out.extend_from_slice(&buf[..n]);
                    port.set_timeout(Duration::from_millis(40)).ok();
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => break,
                Err(e) => return Err(DeviceError::Transport(format!("serial read: {e}"))),
            }
        }
        Ok(out)
    }
}

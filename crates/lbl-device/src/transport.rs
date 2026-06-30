//! Transports that deliver encoded bytes to a printer.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::DeviceError;

#[cfg(feature = "ble")]
use std::time::Instant;

#[cfg(feature = "ble")]
use btleplug::api::{
    CharPropFlags, Characteristic, Central, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
#[cfg(feature = "ble")]
use btleplug::platform::{Adapter, Manager, Peripheral};
#[cfg(feature = "ble")]
use futures::StreamExt;
#[cfg(feature = "ble")]
use tokio::runtime::Runtime;
#[cfg(feature = "ble")]
use tokio::time::{sleep, timeout as wait_for};
#[cfg(feature = "ble")]
use uuid::Uuid;

#[cfg(feature = "ble")]
use crate::ble::{NIIMBOT_CHAR, peripheral_label, peripheral_matches_target};

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

/// Default per-write payload size for BLE (ATT MTU minus overhead).
#[cfg(feature = "ble")]
pub const BLE_DEFAULT_CHUNK: usize = 20;

/// Default time to scan for the target peripheral before giving up.
#[cfg(feature = "ble")]
pub const BLE_DEFAULT_SCAN_SECS: u64 = 15;

/// Timeout for establishing a GATT connection.
#[cfg(feature = "ble")]
const BLE_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// A bidirectional Bluetooth Low Energy (GATT) transport.
///
/// This is how cable-less label printers such as the NIIMBOT D-series (D11,
/// D110, …) are reached: they expose no USB data port, only a BLE GATT service
/// with a writable characteristic (for the job byte stream) and a notify
/// characteristic (for status replies). The transport finds the printer by its
/// advertised name (or address), connects, and — unless overridden — picks the
/// write/notify characteristics automatically, so the same NIIMBOT byte stream
/// used over serial works here too, including the status handshake.
///
/// The connection is opened lazily on first use and kept open across calls so a
/// protocol can interleave writes and reads.
#[cfg(feature = "ble")]
pub struct BleTransport {
    /// Advertised local-name or address substring used to find the device
    /// (case-insensitive). Empty matches the first peripheral seen.
    target: String,
    /// Explicit write characteristic UUID (auto-detected when `None`).
    write_uuid: Option<Uuid>,
    /// Explicit notify characteristic UUID (auto-detected when `None`).
    notify_uuid: Option<Uuid>,
    /// Maximum bytes per BLE write.
    chunk: usize,
    /// How long to scan for the device before giving up.
    scan: Duration,
    rt: Option<Runtime>,
    state: Option<BleConnection>,
}

/// A live BLE link: the connected peripheral, its chosen characteristics, and
/// the notification stream subscribed for status replies.
#[cfg(feature = "ble")]
struct BleConnection {
    peripheral: Peripheral,
    write_char: Characteristic,
    /// Notify characteristic we subscribed to, if any (for clean unsubscribe).
    notify_char: Option<Characteristic>,
    write_type: WriteType,
    notifications: std::pin::Pin<Box<dyn futures::Stream<Item = btleplug::api::ValueNotification> + Send>>,
}

#[cfg(feature = "ble")]
impl std::fmt::Debug for BleTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BleTransport")
            .field("target", &self.target)
            .field("chunk", &self.chunk)
            .field("connected", &self.state.is_some())
            .finish()
    }
}

#[cfg(feature = "ble")]
impl BleTransport {
    /// Create a transport that connects to the first BLE peripheral whose
    /// advertised name or address contains `target` (case-insensitive).
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            write_uuid: Some(NIIMBOT_CHAR),
            notify_uuid: Some(NIIMBOT_CHAR),
            chunk: BLE_DEFAULT_CHUNK,
            scan: Duration::from_secs(BLE_DEFAULT_SCAN_SECS),
            rt: None,
            state: None,
        }
    }

    /// Override the scan window used to find the device.
    pub fn with_scan(mut self, scan: Duration) -> Self {
        self.scan = scan;
        self
    }

    /// Pin the write/notify characteristics by UUID instead of auto-detecting
    /// them (each `None` keeps auto-detection for that characteristic).
    pub fn with_characteristics(mut self, write: Option<Uuid>, notify: Option<Uuid>) -> Self {
        self.write_uuid = write;
        self.notify_uuid = notify;
        self
    }

    /// Override the maximum bytes sent per BLE write.
    pub fn with_chunk(mut self, chunk: usize) -> Self {
        self.chunk = chunk.max(1);
        self
    }

    /// Build (once) the Tokio runtime that drives `btleplug`'s async API.
    fn runtime(&mut self) -> Result<(), DeviceError> {
        if self.rt.is_none() {
            self.rt = Some(
                Runtime::new()
                    .map_err(|e| DeviceError::Transport(format!("ble runtime: {e}")))?,
            );
        }
        Ok(())
    }

    /// Connect (lazily) to the target peripheral and subscribe for status.
    fn ensure_connected(&mut self) -> Result<(), DeviceError> {
        if self.state.is_some() {
            return Ok(());
        }
        self.runtime()?;
        let rt = self.rt.as_ref().expect("runtime built");
        let connection = rt.block_on(connect_ble(
            &self.target,
            self.write_uuid,
            self.notify_uuid,
            self.scan,
        ))?;
        self.state = Some(connection);
        Ok(())
    }
}

/// Disconnect and tear down a BLE session inside the Tokio runtime.
///
/// `bluez-async` (used by `btleplug` on Linux) expects D-Bus cleanup to run
/// from a runtime context; dropping the connection synchronously after the
/// runtime has shut down panics with "there is no reactor running".
#[cfg(feature = "ble")]
async fn teardown_ble(conn: BleConnection) {
    drop(conn.notifications);
    if let Some(nc) = &conn.notify_char {
        let _ = conn.peripheral.unsubscribe(nc).await;
    }
    let _ = conn.peripheral.disconnect().await;
}

#[cfg(feature = "ble")]
impl Drop for BleTransport {
    fn drop(&mut self) {
        let state = self.state.take();
        let rt = self.rt.take();
        if let (Some(rt), Some(conn)) = (rt, state) {
            let _ = rt.block_on(teardown_ble(conn));
        }
    }
}

#[cfg(feature = "ble")]
impl Transport for BleTransport {
    fn send(&mut self, data: &[u8]) -> Result<(), DeviceError> {
        self.ensure_connected()?;
        let rt = self.rt.as_ref().expect("connected");
        let state = self.state.as_ref().expect("connected");
        rt.block_on(async {
            for frame in niimbot_frames(data) {
                state
                    .peripheral
                    .write(&state.write_char, frame, state.write_type)
                    .await
                    .map_err(|e| DeviceError::Transport(format!("ble write: {e}")))?;
            }
            Ok(())
        })
    }

    fn is_bidirectional(&self) -> bool {
        true
    }

    fn receive(&mut self, timeout: Duration) -> Result<Vec<u8>, DeviceError> {
        // Without a live connection there is nothing subscribed to read.
        if self.state.is_none() {
            return Ok(Vec::new());
        }
        let rt = self.rt.as_ref().expect("connected");
        let state = self.state.as_mut().expect("connected");
        rt.block_on(async {
            let mut out = Vec::new();
            // Wait up to `timeout` for the first notification, then drain any
            // immediately-following ones with a short idle timeout.
            match wait_for(timeout, state.notifications.next()).await {
                Ok(Some(n)) => out.extend_from_slice(&n.value),
                _ => return Ok(out),
            }
            loop {
                match wait_for(
                    Duration::from_millis(40),
                    state.notifications.next(),
                )
                .await
                {
                    Ok(Some(n)) => out.extend_from_slice(&n.value),
                    _ => break,
                }
            }
            Ok(out)
        })
    }
}

/// Find, connect to, and prepare a BLE peripheral for printing.
#[cfg(feature = "ble")]
async fn connect_ble(
    target: &str,
    write_uuid: Option<Uuid>,
    notify_uuid: Option<Uuid>,
    scan: Duration,
) -> Result<BleConnection, DeviceError> {
    tracing::info!("ble: scanning for {target:?} (up to {scan:?})");
    let adapter = ble_adapter().await?;
    adapter
        .start_scan(ScanFilter::default())
        .await
        .map_err(|e| DeviceError::Transport(format!("ble start_scan: {e}")))?;

    let peripheral = find_peripheral(&adapter, target, scan).await?;
    adapter.stop_scan().await.ok();
    tracing::info!("ble: connecting to {}", peripheral.address());

    wait_for(BLE_CONNECT_TIMEOUT, peripheral.connect())
        .await
        .map_err(|_| DeviceError::Transport("ble connect timed out".into()))?
        .map_err(|e| DeviceError::Transport(format!("ble connect: {e}")))?;
    wait_for(BLE_CONNECT_TIMEOUT, peripheral.discover_services())
        .await
        .map_err(|_| DeviceError::Transport("ble discover_services timed out".into()))?
        .map_err(|e| DeviceError::Transport(format!("ble discover_services: {e}")))?;
    tracing::info!("ble: connected");

    let chars = peripheral.characteristics();
    let write_char = pick_characteristic(&chars, write_uuid, true).ok_or_else(|| {
        DeviceError::NotFound("no writable BLE characteristic on the device".into())
    })?;
    let write_type = WriteType::WithoutResponse;

    // Subscribe for status notifications when the device offers a notify
    // characteristic; printers that never reply just leave this stream empty.
    let (notify_char, notifications): (
        Option<Characteristic>,
        std::pin::Pin<Box<dyn futures::Stream<Item = btleplug::api::ValueNotification> + Send>>,
    ) = match pick_characteristic(&chars, notify_uuid, false) {
        Some(nc) => {
            peripheral
                .subscribe(&nc)
                .await
                .map_err(|e| DeviceError::Transport(format!("ble subscribe: {e}")))?;
            let stream = peripheral
                .notifications()
                .await
                .map_err(|e| DeviceError::Transport(format!("ble notifications: {e}")))?;
            (Some(nc), stream)
        }
        None => (None, Box::pin(futures::stream::empty())),
    };

    Ok(BleConnection {
        peripheral,
        write_char,
        notify_char,
        write_type,
        notifications,
    })
}

/// Split a NIIMBOT framed byte stream into individual packets (`55 55 … aa aa`).
///
/// Each packet is sent as one BLE write so the printer sees whole frames rather
/// than arbitrary 20-byte chunks.
#[cfg(feature = "ble")]
fn niimbot_frames(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= data.len() {
        if data[i] != 0x55 || data[i + 1] != 0x55 {
            i += 1;
            continue;
        }
        let len = data[i + 3] as usize;
        let end = i + 4 + len + 3;
        if end > data.len() {
            break;
        }
        if data[end - 2] == 0xAA && data[end - 1] == 0xAA {
            out.push(&data[i..end]);
        }
        i = end;
    }
    if out.is_empty() && !data.is_empty() {
        out.push(data);
    }
    out
}

/// Get the first available Bluetooth adapter.
#[cfg(feature = "ble")]
async fn ble_adapter() -> Result<Adapter, DeviceError> {
    let manager = Manager::new()
        .await
        .map_err(|e| DeviceError::Transport(format!("ble manager: {e}")))?;
    manager
        .adapters()
        .await
        .map_err(|e| DeviceError::Transport(format!("ble adapters: {e}")))?
        .into_iter()
        .next()
        .ok_or_else(|| DeviceError::NotFound("no bluetooth adapter".into()))
}

/// Poll the scan results until a peripheral matching `target` appears or the
/// scan window elapses.
#[cfg(feature = "ble")]
async fn find_peripheral(
    adapter: &Adapter,
    target: &str,
    scan: Duration,
) -> Result<Peripheral, DeviceError> {
    let deadline = Instant::now() + scan;
    let mut seen = Vec::new();
    loop {
        let peripherals = adapter
            .peripherals()
            .await
            .map_err(|e| DeviceError::Transport(format!("ble peripherals: {e}")))?;
        for p in &peripherals {
            if peripheral_matches_target(p, target).await {
                return Ok(p.clone());
            }
            let label = peripheral_label(p).await;
            if !seen.iter().any(|s: &String| s == &label) {
                seen.push(label);
            }
        }
        if Instant::now() >= deadline {
            let mut msg = format!("no BLE device matching {target:?} within {scan:?}");
            if seen.is_empty() {
                msg.push_str("; no peripherals seen — is the printer on and not connected to another device (e.g. the NIIMBOT phone app)?");
            } else {
                msg.push_str("; nearby: ");
                msg.push_str(&seen.join(", "));
            }
            return Err(DeviceError::NotFound(msg));
        }
        sleep(Duration::from_millis(250)).await;
    }
}

/// Choose a characteristic by explicit UUID, else by capability: for writing,
/// prefer write-without-response then any writable; for reading, prefer notify
/// then indicate.
#[cfg(feature = "ble")]
fn pick_characteristic(
    chars: &std::collections::BTreeSet<Characteristic>,
    want: Option<Uuid>,
    writable: bool,
) -> Option<Characteristic> {
    if let Some(uuid) = want {
        return chars.iter().find(|c| c.uuid == uuid).cloned();
    }
    let (primary, secondary) = if writable {
        (CharPropFlags::WRITE_WITHOUT_RESPONSE, CharPropFlags::WRITE)
    } else {
        (CharPropFlags::NOTIFY, CharPropFlags::INDICATE)
    };
    chars
        .iter()
        .find(|c| c.properties.contains(primary))
        .or_else(|| chars.iter().find(|c| c.properties.contains(secondary)))
        .cloned()
}

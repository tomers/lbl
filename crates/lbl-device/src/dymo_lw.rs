//! DYMO LabelWriter 550-series (LW5) USB print session.
//!
//! The 550 protocol is bidirectional: the host must acquire a print-engine lock,
//! send each label segment, then drain a 32-byte status reply on bulk IN before
//! the next OUT write. Streaming a whole job in one bulk transfer stalls the
//! firmware until power-cycle. See DYMO's *LabelWriter 550 Series Technical
//! Reference* and <https://thermal-label.github.io/labelwriter/protocol/lw5-raster>.

use crate::DeviceError;

#[cfg(feature = "usb")]
use crate::transport::{open_usb_bulk_session, Transport, UsbBulkSession, UsbTransport};

/// ESC prefix byte for LW5 commands.
pub const ESC: u8 = 0x1B;

/// Length of a print-engine status reply on bulk IN.
pub const STATUS_REPLY_LEN: usize = 32;

/// Length of an `ESC U` NFC consumable-info reply on bulk IN.
pub const SKU_INFO_REPLY_LEN: usize = 63;

/// Length of an `ESC V` engine-version reply on bulk IN.
pub const ENGINE_VERSION_REPLY_LEN: usize = 34;

/// Magic at bytes 0–1 of a valid `ESC U` reply (`0xCAB6` LE).
pub const SKU_INFO_MAGIC: u16 = 0xCAB6;

/// Lock byte for [`status_request`]: acquire the print engine before a job.
pub const LOCK_ACQUIRE: u8 = 1;

/// Lock byte for [`status_request`]: handshake between labels in one job.
pub const LOCK_INTER_LABEL: u8 = 2;

/// Lock byte for [`status_request`]: handshake after the last label in a job.
pub const LOCK_RELEASE: u8 = 0;

/// First byte of the 12-character SKU field in a 32-byte status reply.
const STATUS_SKU_START: usize = 11;

/// Length of the SKU field in a status reply.
const STATUS_SKU_LEN: usize = 12;

/// First byte of the 12-character SKU field in a 63-byte `ESC U` reply.
const SKU_INFO_SKU_START: usize = 8;

/// Offset of total label count (u16 LE) in an `ESC U` reply.
const SKU_INFO_TOTAL_LABEL_COUNT: usize = 50;

/// Build an `ESC A` status request with the given lock byte.
pub fn status_request(lock: u8) -> [u8; 3] {
    [ESC, b'A', lock]
}

/// Build an `ESC U` request for the inserted consumable's NFC dump.
pub fn sku_info_request() -> [u8; 2] {
    [ESC, b'U']
}

/// Build an `ESC V` request for the print-engine HW/FW/PID block.
pub fn engine_version_request() -> [u8; 2] {
    [ESC, b'V']
}

/// Main-bay status: media present and OK (NFC-valid genuine roll).
const BAY_MEDIA_OK: u8 = 8;

/// Main-bay status: NFC rejected the inserted roll as non-genuine.
const BAY_MEDIA_COUNTERFEIT: u8 = 10;

/// Print-engine status: reply before lock is granted to this host.
const PRINT_STATUS_NO_LOCK: u8 = 5;

/// Parsed fields from the 32-byte `ESC A` print-engine status reply.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Lw550PrintStatus {
    /// Byte 0 — print engine state (idle, printing, error, …).
    pub print_status: Lw550PrintEngineStatus,
    /// Raw byte 0.
    pub print_status_code: u8,
    /// Bytes 1–4 — job id of the ongoing print process (0 when idle).
    pub print_job_id: u32,
    /// Bytes 5–6 — label/page index currently being printed.
    pub label_index: u16,
    /// Byte 8 — thermal print head health.
    pub print_head_status: Lw550PrintHeadStatus,
    /// Raw byte 8.
    pub print_head_status_code: u8,
    /// Byte 9 — print density setting in percent (0 disables printing; 1–200).
    pub print_density: u8,
    /// Byte 10 — main media bay / NFC roll state.
    pub main_bay_status: Lw550MainBayStatus,
    /// Raw byte 10.
    pub main_bay_status_code: u8,
    /// Bytes 11–22 — NFC-reported consumable SKU (when present).
    pub sku: Option<String>,
    /// Bytes 23–26 — present error code (0 = none).
    pub error_id: u32,
    /// Bytes 27–28 — remaining label count on the inserted roll.
    pub label_count: u16,
    /// Full-roll label count from `ESC U` (NFC), when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_total: Option<u16>,
    /// Byte 29 (low nibble) — external power supply detected.
    pub eps_present: bool,
    /// Byte 30 (low nibble) — print head supply voltage.
    pub print_head_voltage: Lw550PrintHeadVoltage,
    /// Raw byte 30 low nibble.
    pub print_head_voltage_code: u8,
    /// Hardware version string from `ESC V`, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hardware_version: Option<String>,
    /// Firmware version (`major.minor`) from `ESC V`, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware_version: Option<String>,
    /// Firmware kind from `ESC V` (`FWAP` application / `FWBL` bootloader).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware_kind: Option<String>,
    /// Firmware release date from `ESC V` (`MMYY`), when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware_date: Option<String>,
    /// USB product id from `ESC V`, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb_pid: Option<u16>,
}

/// Fields from the 63-byte `ESC U` NFC consumable-info reply.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Lw550SkuInfo {
    /// Bytes 8–19 — NFC-reported consumable SKU (when present).
    pub sku: Option<String>,
    /// Bytes 50–51 — labels on a full (unused) roll.
    pub total_label_count: u16,
}

/// Fields from the 34-byte `ESC V` engine-version reply.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Lw550EngineVersion {
    /// Bytes 0–15 — hardware version (NUL-padded UTF-8).
    pub hardware_version: String,
    /// Bytes 16–19 — firmware kind (`FWAP` / `FWBL`).
    pub firmware_kind: String,
    /// Bytes 20–27 — composed `major.minor` ASCII version.
    pub firmware_version: String,
    /// Bytes 28–31 — release date (`MMYY`).
    pub firmware_date: String,
    /// Bytes 32–33 — USB product id.
    pub usb_pid: u16,
}

/// JSON-friendly view of [`Lw550PrintStatus`] for APIs and CLI output.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Lw550PrintStatusView {
    pub print_status: String,
    pub print_status_code: u8,
    pub print_job_id: u32,
    pub label_index: u16,
    pub print_head_status: String,
    pub print_head_status_code: u8,
    pub print_density: u8,
    pub main_bay_status: String,
    pub main_bay_status_code: u8,
    pub sku: Option<String>,
    pub error_id: u32,
    pub label_count: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_total: Option<u16>,
    pub eps_present: bool,
    pub print_head_voltage: String,
    pub print_head_voltage_code: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hardware_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb_pid: Option<u16>,
}

impl From<&Lw550PrintStatus> for Lw550PrintStatusView {
    fn from(status: &Lw550PrintStatus) -> Self {
        Self {
            print_status: status.print_status.label().into(),
            print_status_code: status.print_status_code,
            print_job_id: status.print_job_id,
            label_index: status.label_index,
            print_head_status: status.print_head_status.label().into(),
            print_head_status_code: status.print_head_status_code,
            print_density: status.print_density,
            main_bay_status: status.main_bay_status.label().into(),
            main_bay_status_code: status.main_bay_status_code,
            sku: status.sku.clone(),
            error_id: status.error_id,
            label_count: status.label_count,
            label_total: status.label_total,
            eps_present: status.eps_present,
            print_head_voltage: status.print_head_voltage.label().into(),
            print_head_voltage_code: status.print_head_voltage_code,
            hardware_version: status.hardware_version.clone(),
            firmware_version: status.firmware_version.clone(),
            firmware_kind: status.firmware_kind.clone(),
            firmware_date: status.firmware_date.clone(),
            usb_pid: status.usb_pid,
        }
    }
}

impl Lw550PrintStatus {
    /// Convert to a JSON-friendly view with human-readable status strings.
    pub fn to_view(&self) -> Lw550PrintStatusView {
        Lw550PrintStatusView::from(self)
    }
}

/// Byte 0 of the status reply (`Print status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lw550PrintEngineStatus {
    Idle,
    Printing,
    Error,
    Cancel,
    Busy,
    NoLock,
    Unknown(u8),
}

impl Lw550PrintEngineStatus {
    pub fn from_byte(value: u8) -> Self {
        match value {
            0 => Self::Idle,
            1 => Self::Printing,
            2 => Self::Error,
            3 => Self::Cancel,
            4 => Self::Busy,
            5 => Self::NoLock,
            other => Self::Unknown(other),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Printing => "printing",
            Self::Error => "error",
            Self::Cancel => "cancel",
            Self::Busy => "busy",
            Self::NoLock => "no lock (another host may be printing)",
            Self::Unknown(_) => "unknown",
        }
    }
}

/// Byte 8 of the status reply (`Print head status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lw550PrintHeadStatus {
    Ok,
    Overheated,
    Unknown,
    UnknownCode(u8),
}

impl Lw550PrintHeadStatus {
    pub fn from_byte(value: u8) -> Self {
        match value {
            0 => Self::Ok,
            1 => Self::Overheated,
            2 => Self::Unknown,
            other => Self::UnknownCode(other),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Overheated => "overheated",
            Self::Unknown => "status unknown",
            Self::UnknownCode(_) => "unknown",
        }
    }
}

/// Byte 10 of the status reply (`Main bay status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lw550MainBayStatus {
    BayUnknown,
    BayOpen,
    NoMedia,
    MediaNotInsertedProperly,
    MediaPresentUnknown,
    MediaEmpty,
    MediaCriticallyLow,
    MediaLow,
    MediaOk,
    MediaJammed,
    MediaCounterfeit,
    Unknown(u8),
}

impl Lw550MainBayStatus {
    pub fn from_byte(value: u8) -> Self {
        match value {
            0 => Self::BayUnknown,
            1 => Self::BayOpen,
            2 => Self::NoMedia,
            3 => Self::MediaNotInsertedProperly,
            4 => Self::MediaPresentUnknown,
            5 => Self::MediaEmpty,
            6 => Self::MediaCriticallyLow,
            7 => Self::MediaLow,
            8 => Self::MediaOk,
            9 => Self::MediaJammed,
            10 => Self::MediaCounterfeit,
            other => Self::Unknown(other),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::BayUnknown => "bay status unknown",
            Self::BayOpen => "bay open; media presence unknown",
            Self::NoMedia => "no media present",
            Self::MediaNotInsertedProperly => "media not inserted properly",
            Self::MediaPresentUnknown => "media present — status unknown",
            Self::MediaEmpty => "media present — empty",
            Self::MediaCriticallyLow => "media present — critically low",
            Self::MediaLow => "media present — low",
            Self::MediaOk => "media present — ok",
            Self::MediaJammed => "media present — jammed",
            Self::MediaCounterfeit => "media present — counterfeit media",
            Self::Unknown(_) => "unknown",
        }
    }
}

/// Byte 30 (low nibble) of the status reply (`Print head voltage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lw550PrintHeadVoltage {
    Unknown,
    Ok,
    Low,
    CriticallyLow,
    TooLowForPrinting,
    UnknownCode(u8),
}

impl Lw550PrintHeadVoltage {
    pub fn from_nibble(value: u8) -> Self {
        match value & 0x0f {
            0 => Self::Unknown,
            1 => Self::Ok,
            2 => Self::Low,
            3 => Self::CriticallyLow,
            4 => Self::TooLowForPrinting,
            other => Self::UnknownCode(other),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Ok => "ok",
            Self::Low => "low",
            Self::CriticallyLow => "critically low",
            Self::TooLowForPrinting => "too low for printing",
            Self::UnknownCode(_) => "unknown",
        }
    }
}

/// Parse a 32-byte `ESC A` print-engine status reply.
///
/// `label_total` and engine-version fields are left unset; callers that also
/// issue `ESC U` / `ESC V` should fill them (see [`query_print_status`]).
///
/// Rejects payloads that are clearly not an `ESC A` reply — including an
/// `ESC U` NFC dump whose magic (`0xCAB6`) would otherwise decode as
/// print-status `182`.
pub fn parse_print_status(status: &[u8]) -> Result<Lw550PrintStatus, DeviceError> {
    if status.len() < STATUS_REPLY_LEN {
        return Err(DeviceError::Transport(format!(
            "short status reply ({} bytes, expected {STATUS_REPLY_LEN})",
            status.len()
        )));
    }
    let magic = u16::from_le_bytes(status[0..2].try_into().unwrap());
    if magic == SKU_INFO_MAGIC {
        return Err(DeviceError::Transport(
            "status reply looks like an ESC U NFC dump (USB read desync); retry the status query"
                .into(),
        ));
    }
    if status[0] > 5 {
        return Err(DeviceError::Transport(format!(
            "invalid print-engine status byte {} (expected 0–5)",
            status[0]
        )));
    }

    Ok(Lw550PrintStatus {
        print_status: Lw550PrintEngineStatus::from_byte(status[0]),
        print_status_code: status[0],
        print_job_id: u32::from_le_bytes(status[1..5].try_into().unwrap()),
        label_index: u16::from_le_bytes(status[5..7].try_into().unwrap()),
        print_head_status: Lw550PrintHeadStatus::from_byte(status[8]),
        print_head_status_code: status[8],
        print_density: status[9],
        main_bay_status: Lw550MainBayStatus::from_byte(status[10]),
        main_bay_status_code: status[10],
        sku: parse_status_sku(status),
        error_id: u32::from_le_bytes(status[23..27].try_into().unwrap()),
        label_count: u16::from_le_bytes(status[27..29].try_into().unwrap()),
        label_total: None,
        eps_present: status[29] & 0x0f == 1,
        print_head_voltage: Lw550PrintHeadVoltage::from_nibble(status[30]),
        print_head_voltage_code: status[30] & 0x0f,
        hardware_version: None,
        firmware_version: None,
        firmware_kind: None,
        firmware_date: None,
        usb_pid: None,
    })
}

/// Parse a 63-byte `ESC U` NFC consumable-info reply.
pub fn parse_sku_info(data: &[u8]) -> Result<Lw550SkuInfo, DeviceError> {
    if data.len() < SKU_INFO_REPLY_LEN {
        return Err(DeviceError::Transport(format!(
            "short ESC U reply ({} bytes, expected {SKU_INFO_REPLY_LEN})",
            data.len()
        )));
    }
    let magic = u16::from_le_bytes(data[0..2].try_into().unwrap());
    if magic != SKU_INFO_MAGIC {
        return Err(DeviceError::Transport(format!(
            "invalid ESC U magic {magic:#06x} (expected {SKU_INFO_MAGIC:#06x})"
        )));
    }
    Ok(Lw550SkuInfo {
        sku: parse_fixed_sku(&data[SKU_INFO_SKU_START..SKU_INFO_SKU_START + STATUS_SKU_LEN]),
        total_label_count: u16::from_le_bytes(
            data[SKU_INFO_TOTAL_LABEL_COUNT..SKU_INFO_TOTAL_LABEL_COUNT + 2]
                .try_into()
                .unwrap(),
        ),
    })
}

fn parse_padded_ascii(raw: &[u8]) -> String {
    let len = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    std::str::from_utf8(&raw[..len])
        .unwrap_or("")
        .trim()
        .to_string()
}

fn compose_firmware_version(major: &str, minor: &str) -> String {
    match (major.is_empty(), minor.is_empty()) {
        (false, false) => format!("{major}.{minor}"),
        (false, true) => major.to_string(),
        (true, false) => minor.to_string(),
        (true, true) => String::new(),
    }
}

/// Parse a 34-byte `ESC V` engine-version reply.
pub fn parse_engine_version(data: &[u8]) -> Result<Lw550EngineVersion, DeviceError> {
    if data.len() < ENGINE_VERSION_REPLY_LEN {
        return Err(DeviceError::Transport(format!(
            "short ESC V reply ({} bytes, expected {ENGINE_VERSION_REPLY_LEN})",
            data.len()
        )));
    }
    let major = parse_padded_ascii(&data[20..24]);
    let minor = parse_padded_ascii(&data[24..28]);
    Ok(Lw550EngineVersion {
        hardware_version: parse_padded_ascii(&data[0..16]),
        firmware_kind: parse_padded_ascii(&data[16..20]),
        firmware_version: compose_firmware_version(&major, &minor),
        firmware_date: parse_padded_ascii(&data[28..32]),
        usb_pid: u16::from_le_bytes(data[32..34].try_into().unwrap()),
    })
}

/// Query the full print-engine status without acquiring the print lock.
///
/// Issues `ESC A` for engine/remaining-count fields, `ESC V` for HW/FW/PID,
/// then `ESC U` for the full-roll total when media appears present.
pub fn query_print_status(session: &mut UsbBulkSession) -> Result<Lw550PrintStatus, DeviceError> {
    session.transfer_out(&status_request(LOCK_RELEASE))?;
    let status = session.transfer_in(STATUS_REPLY_LEN)?;
    let mut parsed = parse_print_status(&status)?;
    if let Ok(ver) = query_engine_version(session) {
        apply_engine_version(&mut parsed, &ver);
    }
    if media_likely_present(parsed.main_bay_status_code) {
        if let Ok(info) = query_sku_info(session) {
            if info.total_label_count > 0 {
                parsed.label_total = Some(info.total_label_count);
            }
            if parsed.sku.is_none() {
                parsed.sku = info.sku;
            }
        }
    }
    Ok(parsed)
}

fn apply_engine_version(status: &mut Lw550PrintStatus, ver: &Lw550EngineVersion) {
    if !ver.hardware_version.is_empty() {
        status.hardware_version = Some(ver.hardware_version.clone());
    }
    if !ver.firmware_version.is_empty() {
        status.firmware_version = Some(ver.firmware_version.clone());
    }
    if !ver.firmware_kind.is_empty() {
        status.firmware_kind = Some(ver.firmware_kind.clone());
    }
    if !ver.firmware_date.is_empty() {
        status.firmware_date = Some(ver.firmware_date.clone());
    }
    status.usb_pid = Some(ver.usb_pid);
}

/// Query the NFC consumable dump (`ESC U`) without acquiring the print lock.
pub fn query_sku_info(session: &mut UsbBulkSession) -> Result<Lw550SkuInfo, DeviceError> {
    session.transfer_out(&sku_info_request())?;
    let data = session.transfer_in(SKU_INFO_REPLY_LEN)?;
    parse_sku_info(&data)
}

/// Query the engine-version block (`ESC V`) without acquiring the print lock.
pub fn query_engine_version(
    session: &mut UsbBulkSession,
) -> Result<Lw550EngineVersion, DeviceError> {
    session.transfer_out(&engine_version_request())?;
    let data = session.transfer_in(ENGINE_VERSION_REPLY_LEN)?;
    parse_engine_version(&data)
}

fn media_likely_present(bay_code: u8) -> bool {
    matches!(bay_code, 4..=10)
}

/// Query print-engine status over USB.
#[cfg(feature = "usb")]
pub fn query_status(usb: &UsbTransport) -> Result<Lw550PrintStatus, DeviceError> {
    let mut session = open_usb_bulk_session(usb)?;
    query_print_status(&mut session)
}

/// Query the loaded media SKU over USB.
#[cfg(feature = "usb")]
pub fn query_loaded_media(usb: &UsbTransport) -> Result<Option<String>, DeviceError> {
    let mut session = open_usb_bulk_session(usb)?;
    query_loaded_media_sku(&mut session)
}

#[cfg(feature = "usb")]
pub(crate) fn send_dymo_lw_job(
    session: &mut UsbBulkSession,
    payload: &[u8],
) -> Result<(), DeviceError> {
    acquire_lock(session)?;
    dispatch_job_segments(session, payload)?;
    Ok(())
}

/// USB transport for the DYMO LabelWriter 550-series (LW5) protocol.
///
/// Unlike [`UsbTransport`], this keeps the device open across jobs and performs
/// the mandatory lock acquisition and 32-byte status handshakes on bulk IN.
#[cfg(feature = "usb")]
pub struct DymoLwUsbTransport {
    usb: UsbTransport,
    session: Option<UsbBulkSession>,
}

#[cfg(feature = "usb")]
impl DymoLwUsbTransport {
    /// Wrap a [`UsbTransport`] configured for the target printer.
    pub fn new(usb: UsbTransport) -> Self {
        Self { usb, session: None }
    }

    fn ensure_session(&mut self) -> Result<&mut UsbBulkSession, DeviceError> {
        if self.session.is_none() {
            self.session = Some(open_usb_bulk_session(&self.usb)?);
        }
        Ok(self.session.as_mut().expect("session opened"))
    }
}

#[cfg(feature = "usb")]
impl Transport for DymoLwUsbTransport {
    fn send(&mut self, data: &[u8]) -> Result<(), DeviceError> {
        let session = self.ensure_session()?;
        send_dymo_lw_job(session, data)
    }

    fn is_bidirectional(&self) -> bool {
        true
    }
}

fn acquire_lock(session: &mut UsbBulkSession) -> Result<(), DeviceError> {
    session.transfer_out(&status_request(LOCK_ACQUIRE))?;
    let status = session.transfer_in(STATUS_REPLY_LEN)?;
    interpret_status(&status, "acquiring print lock")
}

fn handshake(session: &mut UsbBulkSession, lock: u8) -> Result<(), DeviceError> {
    session.transfer_out(&status_request(lock))?;
    let status = session.transfer_in(STATUS_REPLY_LEN)?;
    interpret_status(&status, "label handshake")
}

fn dispatch_job_segments(session: &mut UsbBulkSession, payload: &[u8]) -> Result<(), DeviceError> {
    let mut pos = 0usize;
    require_esc_cmd(payload, &mut pos, b's')?;
    pos += 4; // job id

    skip_job_header(payload, &mut pos)?;

    let preamble_end = pos;
    let mut label_ranges = Vec::new();
    while pos + 1 < payload.len() {
        if payload[pos] == ESC && payload[pos + 1] == b'E' {
            break;
        }
        if payload[pos] == ESC && payload[pos + 1] == b'Q' {
            return Err(DeviceError::Transport(
                "dymo-lw job missing ESC E before ESC Q".into(),
            ));
        }

        let label_start = pos;
        require_esc_cmd(payload, &mut pos, b'n')?;
        pos += 2; // label index

        skip_label_data(payload, &mut pos)?;
        require_esc_cmd(payload, &mut pos, b'G')?;
        label_ranges.push((label_start, pos));
    }

    if label_ranges.is_empty() {
        return Err(DeviceError::Transport(
            "dymo-lw job missing label data".into(),
        ));
    }

    require_esc_cmd(payload, &mut pos, b'E')?;
    require_esc_cmd(payload, &mut pos, b'Q')?;
    if pos != payload.len() {
        return Err(DeviceError::Transport(format!(
            "dymo-lw job has {0} trailing bytes after ESC Q",
            payload.len() - pos
        )));
    }

    let finalize = &payload[payload.len() - 4..];

    if preamble_end > 0 {
        session.transfer_out(&payload[..preamble_end])?;
    }

    for (i, &(start, end)) in label_ranges.iter().enumerate() {
        session.transfer_out(&payload[start..end])?;
        handshake(
            session,
            if i + 1 == label_ranges.len() {
                LOCK_RELEASE
            } else {
                LOCK_INTER_LABEL
            },
        )?;
    }

    session.transfer_out(finalize)?;
    Ok(())
}

fn skip_job_header(payload: &[u8], pos: &mut usize) -> Result<(), DeviceError> {
    while *pos + 1 < payload.len() && payload[*pos] == ESC {
        match payload[*pos + 1] {
            b'L' => *pos += 6,
            b'h' | b'i' | b'e' => *pos += 2,
            b'T' | b'C' => *pos += 3,
            b'n' | b'D' => return Ok(()),
            b'Q' => {
                return Err(DeviceError::Transport(
                    "dymo-lw job missing label data".into(),
                ))
            }
            other => {
                return Err(DeviceError::Transport(format!(
                    "unexpected dymo-lw header command ESC {other:#04x}"
                )))
            }
        }
    }
    Ok(())
}

fn skip_label_data(payload: &[u8], pos: &mut usize) -> Result<(), DeviceError> {
    require_esc_cmd(payload, pos, b'D')?;
    *pos += 2; // bpp + align
    let width = read_u32_le(payload, *pos)?;
    *pos += 4;
    let height = read_u32_le(payload, *pos)?;
    *pos += 4;
    let data_len = width
        .checked_mul(height.div_ceil(8))
        .ok_or_else(|| DeviceError::Transport("dymo-lw label data length overflow".into()))?
        as usize;
    let end = pos
        .checked_add(data_len)
        .ok_or_else(|| DeviceError::Transport("dymo-lw label data length overflow".into()))?;
    if end > payload.len() {
        return Err(DeviceError::Transport(format!(
            "dymo-lw label data truncated (need {data_len} bytes, have {})",
            payload.len().saturating_sub(*pos)
        )));
    }
    *pos = end;
    Ok(())
}

fn read_u32_le(payload: &[u8], pos: usize) -> Result<u32, DeviceError> {
    let end = pos
        .checked_add(4)
        .ok_or_else(|| DeviceError::Transport("dymo-lw field truncated".into()))?;
    if end > payload.len() {
        return Err(DeviceError::Transport("dymo-lw field truncated".into()));
    }
    Ok(u32::from_le_bytes(payload[pos..end].try_into().unwrap()))
}

fn require_esc_cmd(payload: &[u8], pos: &mut usize, cmd: u8) -> Result<(), DeviceError> {
    if *pos + 1 >= payload.len() || payload[*pos] != ESC || payload[*pos + 1] != cmd {
        return Err(DeviceError::Transport(format!(
            "expected ESC {cmd} at offset {pos}, job may be malformed"
        )));
    }
    *pos += 2;
    Ok(())
}

/// Read the NFC-reported consumable SKU from a 32-byte LW5 status reply.
pub fn parse_status_sku(status: &[u8]) -> Option<String> {
    if status.len() < STATUS_SKU_START + 1 {
        return None;
    }
    if status.len() > 10 && status[10] == 2 {
        return None;
    }
    let end = STATUS_SKU_START + STATUS_SKU_LEN.min(status.len() - STATUS_SKU_START);
    parse_fixed_sku(&status[STATUS_SKU_START..end])
}

fn parse_fixed_sku(raw: &[u8]) -> Option<String> {
    let len = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    if len == 0 {
        return None;
    }
    std::str::from_utf8(&raw[..len])
        .ok()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Query the SKU of the roll currently loaded in the printer.
pub fn query_loaded_media_sku(session: &mut UsbBulkSession) -> Result<Option<String>, DeviceError> {
    Ok(query_print_status(session)?.sku)
}

fn interpret_status(status: &[u8], phase: &str) -> Result<(), DeviceError> {
    if status.len() < STATUS_REPLY_LEN {
        return Err(DeviceError::Transport(format!(
            "{phase}: short status reply ({} bytes)",
            status.len()
        )));
    }

    match status[0] {
        PRINT_STATUS_NO_LOCK => {
            return Err(DeviceError::Transport(format!(
                "{phase}: printer did not grant the print lock (another host may be using it)"
            )));
        }
        2 => {
            return Err(DeviceError::Transport(format!(
                "{phase}: printer reported an error (status byte {})",
                status[0]
            )));
        }
        _ => {}
    }

    if status.len() > 10 {
        match status[10] {
            BAY_MEDIA_OK => {}
            BAY_MEDIA_COUNTERFEIT => {
                return Err(DeviceError::Transport(
                    "printer rejected the loaded media (NFC reports non-genuine labels); \
                     LabelWriter 550 requires authentic DYMO rolls"
                        .into(),
                ));
            }
            2 => {
                return Err(DeviceError::Transport(format!(
                    "{phase}: no media loaded in the printer"
                )));
            }
            5..=7 => {
                return Err(DeviceError::Transport(format!(
                    "{phase}: media roll is empty or nearly empty (bay status {})",
                    status[10]
                )));
            }
            9 => {
                return Err(DeviceError::Transport(format!(
                    "{phase}: media jam reported by printer"
                )));
            }
            _ => {}
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_single_label_job() -> Vec<u8> {
        // Minimal job: ESC s, ESC i, one 8×1-dot label, ESC E, ESC Q.
        let mut out = vec![
            ESC, b's', 1, 0, 0, 0, //
            ESC, b'i', //
            ESC, b'n', 0, 0, //
            ESC, b'D', 1, 2, //
        ];
        out.extend_from_slice(&1u32.to_le_bytes()); // width = 1 line
        out.extend_from_slice(&8u32.to_le_bytes()); // height = 8 dots
        out.push(0x80); // one line of print data
        out.extend_from_slice(&[ESC, b'G', ESC, b'E', ESC, b'Q']);
        out
    }

    #[test]
    fn status_request_bytes() {
        assert_eq!(status_request(LOCK_ACQUIRE), [ESC, b'A', 1]);
        assert_eq!(status_request(LOCK_INTER_LABEL), [ESC, b'A', 2]);
        assert_eq!(sku_info_request(), [ESC, b'U']);
        assert_eq!(engine_version_request(), [ESC, b'V']);
    }

    #[test]
    fn parse_sku_info_total_label_count() {
        let mut raw = vec![0u8; SKU_INFO_REPLY_LEN];
        raw[0..2].copy_from_slice(&SKU_INFO_MAGIC.to_le_bytes());
        raw[8..12].copy_from_slice(b"3025");
        raw[50..52].copy_from_slice(&350u16.to_le_bytes());

        let parsed = parse_sku_info(&raw).unwrap();
        assert_eq!(parsed.sku.as_deref(), Some("3025"));
        assert_eq!(parsed.total_label_count, 350);
    }

    #[test]
    fn parse_engine_version_block() {
        let mut raw = vec![0u8; ENGINE_VERSION_REPLY_LEN];
        raw[0..6].copy_from_slice(b"LW550\0");
        raw[16..20].copy_from_slice(b"FWAP");
        raw[20..24].copy_from_slice(b"0001");
        raw[24..28].copy_from_slice(b"0023");
        raw[28..32].copy_from_slice(b"0124");
        raw[32..34].copy_from_slice(&0x0028u16.to_le_bytes());

        let parsed = parse_engine_version(&raw).unwrap();
        assert_eq!(parsed.hardware_version, "LW550");
        assert_eq!(parsed.firmware_kind, "FWAP");
        assert_eq!(parsed.firmware_version, "0001.0023");
        assert_eq!(parsed.firmware_date, "0124");
        assert_eq!(parsed.usb_pid, 0x0028);
    }

    #[test]
    fn parse_sku_info_rejects_bad_magic() {
        let raw = vec![0u8; SKU_INFO_REPLY_LEN];
        let err = parse_sku_info(&raw).unwrap_err();
        assert!(err.to_string().contains("magic"));
    }

    #[test]
    fn interpret_status_flags_counterfeit_media() {
        let mut status = vec![0u8; STATUS_REPLY_LEN];
        status[10] = BAY_MEDIA_COUNTERFEIT;
        let err = interpret_status(&status, "test").unwrap_err();
        assert!(err.to_string().contains("non-genuine"));
    }

    #[test]
    fn parse_print_status_fields() {
        let mut raw = vec![0u8; STATUS_REPLY_LEN];
        raw[0] = 1; // printing
        raw[1..5].copy_from_slice(&42u32.to_le_bytes());
        raw[5..7].copy_from_slice(&3u16.to_le_bytes());
        raw[8] = 0; // head ok
        raw[9] = 100;
        raw[10] = BAY_MEDIA_OK;
        raw[11..15].copy_from_slice(b"3025");
        raw[23..27].copy_from_slice(&7u32.to_le_bytes());
        raw[27..29].copy_from_slice(&250u16.to_le_bytes());
        raw[29] = 1; // EPS present
        raw[30] = 1; // voltage ok

        let parsed = parse_print_status(&raw).unwrap();
        assert_eq!(parsed.print_status, Lw550PrintEngineStatus::Printing);
        assert_eq!(parsed.print_job_id, 42);
        assert_eq!(parsed.label_index, 3);
        assert_eq!(parsed.print_density, 100);
        assert_eq!(parsed.main_bay_status, Lw550MainBayStatus::MediaOk);
        assert_eq!(parsed.sku.as_deref(), Some("3025"));
        assert_eq!(parsed.error_id, 7);
        assert_eq!(parsed.label_count, 250);
        assert!(parsed.eps_present);
        assert_eq!(parsed.print_head_voltage, Lw550PrintHeadVoltage::Ok);
    }

    #[test]
    fn parse_print_status_rejects_esc_u_magic() {
        let mut raw = vec![0u8; STATUS_REPLY_LEN];
        raw[0..2].copy_from_slice(&SKU_INFO_MAGIC.to_le_bytes());
        let err = parse_print_status(&raw).unwrap_err();
        assert!(err.to_string().contains("ESC U"));
    }

    #[test]
    fn parse_print_status_rejects_out_of_range_engine_byte() {
        let mut raw = vec![0u8; STATUS_REPLY_LEN];
        raw[0] = 182;
        let err = parse_print_status(&raw).unwrap_err();
        assert!(err.to_string().contains("182"));
    }

    #[test]
    fn parse_single_label_job_layout() {
        let job = sample_single_label_job();
        let mut pos = 0;
        require_esc_cmd(&job, &mut pos, b's').unwrap();
        pos += 4;
        skip_job_header(&job, &mut pos).unwrap();
        require_esc_cmd(&job, &mut pos, b'n').unwrap();
        pos += 2;
        skip_label_data(&job, &mut pos).unwrap();
        require_esc_cmd(&job, &mut pos, b'G').unwrap();
        assert_eq!(pos, 27);
        assert_eq!(&job[pos..], &[ESC, b'E', ESC, b'Q']);
    }
}

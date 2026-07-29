//! DYMO LabelWriter 550-series (LW5) status protocol: request bytes and reply
//! parsing.
//!
//! The 550 protocol is bidirectional. This module is transport-agnostic: it
//! builds the request byte sequences (`ESC A`/`ESC U`/`ESC V`/`ESC @`) and
//! parses the fixed-length replies. Owning a device and performing the lock /
//! per-label handshakes is the transport layer's job. See DYMO's *LabelWriter
//! 550 Series Technical Reference* and
//! <https://thermal-label.github.io/labelwriter/protocol/lw5-raster>.

use crate::readiness::PrintReadiness;
use crate::StatusError;

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

/// Wire length fields in `ESC U` are tenths of a millimetre (deci-mm).
///
/// The Tech Ref labels them "Length in mm"; bench captures (e.g. S0722540
/// 57.1×31.7 mm → `571`/`317`) confirm ÷10 for real millimetres. Count,
/// strategy, and date fields are not length-scaled.
fn deci_mm_to_mm(raw: u16) -> f64 {
    f64::from(raw) / 10.0
}

/// Label length along the feed: `0` / `0xFFFF` mean continuous stock.
fn optional_die_cut_mm(raw: u16) -> Option<f64> {
    if raw == 0 || raw == 0xFFFF {
        None
    } else {
        Some(deci_mm_to_mm(raw))
    }
}

fn le_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
}

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

/// Build an `ESC @` soft-reboot request for the print engine.
///
/// Restarts the on-printer engine without a host power-cycle. No reply is
/// defined; use after a wedged lock or failed job when status polls hang.
pub fn soft_reboot_request() -> [u8; 2] {
    [ESC, b'@']
}

/// Parsed fields from the 32-byte `ESC A` print-engine status reply.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
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
    /// Full `ESC U` NFC consumable dump, when queried.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sku_info: Option<Lw550SkuInfo>,
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
///
/// Length fields are exposed in **millimetres** after converting the on-wire
/// deci-mm encoding (÷10). See [`deci_mm_to_mm`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Lw550SkuInfo {
    /// Bytes 8–19 — NFC-reported consumable SKU (when present).
    pub sku: Option<String>,
    /// Byte 20 — brand id (`0` = DYMO).
    pub brand_id: u8,
    /// Byte 21 — region (`0xFF` = global).
    pub region: u8,
    /// Byte 22 — material type code.
    pub material_type: u8,
    /// Byte 23 — label type (`0` continuous · `1` die · `2` card).
    pub label_type: u8,
    /// Byte 24 — label colour code.
    pub label_colour: u8,
    /// Byte 25 — content / ink colour code.
    pub content_colour: u8,
    /// Byte 26 — marker / cut-edge geometry (`0..3`).
    pub marker_type: u8,
    /// Bytes 28–29 — marker pitch (mm).
    pub marker_pitch_mm: f64,
    /// Bytes 30–31 — marker 1 width (mm).
    pub marker1_width_mm: f64,
    /// Bytes 32–33 — marker 1 to start of label (mm).
    pub marker1_to_label_start_mm: f64,
    /// Bytes 34–35 — marker 2 width (mm).
    pub marker2_width_mm: f64,
    /// Bytes 36–37 — marker 2 offset (mm).
    pub marker2_offset_mm: f64,
    /// Bytes 38–39 — vertical offset (mm).
    pub vertical_offset_mm: f64,
    /// Bytes 40–41 — label length along feed (mm); `None` on continuous stock.
    pub label_length_mm: Option<f64>,
    /// Bytes 42–43 — label width across head (mm).
    pub label_width_mm: f64,
    /// Bytes 44–45 — printable horizontal offset (mm).
    pub printable_h_offset_mm: f64,
    /// Bytes 46–47 — printable vertical offset (mm).
    pub printable_v_offset_mm: f64,
    /// Bytes 48–49 — liner width (mm).
    pub liner_width_mm: f64,
    /// Bytes 50–51 — labels on a full (unused) roll.
    pub total_label_count: u16,
    /// Bytes 52–53 — roll length (mm).
    pub total_length_mm: f64,
    /// Bytes 54–55 — counter margin used for remaining-label math.
    pub counter_margin: u16,
    /// Byte 56 — counter strategy (`0` up · `1` down).
    pub counter_strategy: u8,
    /// Bytes 60–61 — production date (ASCII, Tech Ref `DDYY` packing).
    pub production_date: String,
    /// Bytes 62–63 — production time (ASCII, Tech Ref `HHMM` packing).
    pub production_time: String,
}

impl Default for Lw550SkuInfo {
    fn default() -> Self {
        Self {
            sku: None,
            brand_id: 0,
            region: 0xff,
            material_type: 0,
            label_type: 0,
            label_colour: 0,
            content_colour: 0,
            marker_type: 0,
            marker_pitch_mm: 0.0,
            marker1_width_mm: 0.0,
            marker1_to_label_start_mm: 0.0,
            marker2_width_mm: 0.0,
            marker2_offset_mm: 0.0,
            vertical_offset_mm: 0.0,
            label_length_mm: None,
            label_width_mm: 0.0,
            printable_h_offset_mm: 0.0,
            printable_v_offset_mm: 0.0,
            liner_width_mm: 0.0,
            total_label_count: 0,
            total_length_mm: 0.0,
            counter_margin: 0,
            counter_strategy: 0,
            production_date: String::new(),
            production_time: String::new(),
        }
    }
}

/// Fields from the 34-byte `ESC V` engine-version reply.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// JSON-friendly view of [`Lw550PrintStatus`] for APIs and WASM.
///
/// Machine-stable enum tokens only — consumers map tokens to display copy.
/// Round-trippable: follow-up fields (`label_total`, `sku_info`, engine version)
/// default to `None` when absent (see [`merge_dymo_lw_status_view`]).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Lw550PrintStatusView {
    pub print_status: Lw550PrintEngineStatus,
    pub print_job_id: u32,
    pub label_index: u16,
    pub print_head_status: Lw550PrintHeadStatus,
    pub print_density: u8,
    pub main_bay_status: Lw550MainBayStatus,
    pub sku: Option<String>,
    pub error_id: u32,
    pub label_count: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_total: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sku_info: Option<Lw550SkuInfo>,
    pub eps_present: bool,
    pub print_head_voltage: Lw550PrintHeadVoltage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardware_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usb_pid: Option<u16>,
    /// Whether the device can accept a new print job.
    #[serde(default)]
    pub readiness: PrintReadiness,
}

impl From<&Lw550PrintStatus> for Lw550PrintStatusView {
    fn from(status: &Lw550PrintStatus) -> Self {
        Self {
            print_status: status.print_status,
            print_job_id: status.print_job_id,
            label_index: status.label_index,
            print_head_status: status.print_head_status,
            print_density: status.print_density,
            main_bay_status: status.main_bay_status,
            sku: status.sku.clone(),
            error_id: status.error_id,
            label_count: status.label_count,
            label_total: status.label_total,
            sku_info: status.sku_info.clone(),
            eps_present: status.eps_present,
            print_head_voltage: status.print_head_voltage,
            hardware_version: status.hardware_version.clone(),
            firmware_version: status.firmware_version.clone(),
            firmware_kind: status.firmware_kind.clone(),
            firmware_date: status.firmware_date.clone(),
            usb_pid: status.usb_pid,
            readiness: status.readiness(),
        }
    }
}

impl Lw550PrintStatus {
    /// Convert to a JSON-friendly token view.
    pub fn to_view(&self) -> Lw550PrintStatusView {
        Lw550PrintStatusView::from(self)
    }

    /// Whether the device can accept a new print job.
    pub fn readiness(&self) -> PrintReadiness {
        readiness_from_fields(
            self.print_status,
            self.print_head_status,
            self.main_bay_status,
            self.print_head_voltage,
            self.error_id,
        )
    }
}

impl Lw550PrintStatusView {
    /// Recompute dispatch readiness from the fields on this view.
    pub fn recompute_readiness(&mut self) {
        self.readiness = readiness_from_fields(
            self.print_status,
            self.print_head_status,
            self.main_bay_status,
            self.print_head_voltage,
            self.error_id,
        );
    }
}

fn readiness_from_fields(
    print_status: Lw550PrintEngineStatus,
    print_head_status: Lw550PrintHeadStatus,
    main_bay_status: Lw550MainBayStatus,
    print_head_voltage: Lw550PrintHeadVoltage,
    error_id: u32,
) -> PrintReadiness {
    use Lw550MainBayStatus as Bay;
    use Lw550PrintEngineStatus as Eng;
    use Lw550PrintHeadStatus as Head;
    use Lw550PrintHeadVoltage as Volt;

    // In-progress jobs are healthy enough for status purposes; hosts still
    // serialize dispatch with their own print mutex.
    if matches!(print_status, Eng::Printing | Eng::Busy) {
        return PrintReadiness::ready();
    }
    if matches!(print_status, Eng::Error | Eng::Cancel) {
        return PrintReadiness::not_ready(print_status.as_str());
    }
    if print_head_status != Head::Ok {
        return PrintReadiness::not_ready(print_head_status.as_str());
    }
    match main_bay_status {
        Bay::NoMedia => return PrintReadiness::not_ready("no_media"),
        Bay::MediaEmpty => return PrintReadiness::not_ready("media_empty"),
        Bay::MediaJammed => return PrintReadiness::not_ready("media_jammed"),
        Bay::MediaCounterfeit => return PrintReadiness::not_ready("media_counterfeit"),
        Bay::BayOpen => return PrintReadiness::not_ready("bay_open"),
        Bay::MediaNotInsertedProperly => {
            return PrintReadiness::not_ready("media_not_inserted_properly")
        }
        // media_low / critically_low / ok / present_unknown: still printable
        _ => {}
    }
    if matches!(
        print_head_voltage,
        Volt::TooLowForPrinting | Volt::CriticallyLow
    ) {
        return PrintReadiness::not_ready(print_head_voltage.as_str());
    }
    if error_id != 0 {
        return PrintReadiness::not_ready("error");
    }
    if print_status == Eng::NoLock {
        return PrintReadiness::not_ready("no_lock");
    }
    PrintReadiness::ready()
}

/// Whether the bay byte indicates media is likely loaded (gate for `ESC U`).
pub fn media_likely_present(bay: Lw550MainBayStatus) -> bool {
    matches!(
        bay,
        Lw550MainBayStatus::MediaPresentUnknown
            | Lw550MainBayStatus::MediaEmpty
            | Lw550MainBayStatus::MediaCriticallyLow
            | Lw550MainBayStatus::MediaLow
            | Lw550MainBayStatus::MediaOk
            | Lw550MainBayStatus::MediaJammed
            | Lw550MainBayStatus::MediaCounterfeit
    )
}

/// Whether the print engine is actively working a job.
pub fn print_job_active(status: Lw550PrintEngineStatus) -> bool {
    matches!(
        status,
        Lw550PrintEngineStatus::Printing | Lw550PrintEngineStatus::Busy
    )
}

/// Whether the main bay reports a healthy media-present state.
pub fn bay_is_ok(bay: Lw550MainBayStatus) -> bool {
    matches!(bay, Lw550MainBayStatus::MediaOk)
}

/// Byte 0 of the status reply (`Print status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lw550PrintEngineStatus {
    Idle,
    Printing,
    Error,
    Cancel,
    Busy,
    NoLock,
    Unknown,
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
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Printing => "printing",
            Self::Error => "error",
            Self::Cancel => "cancel",
            Self::Busy => "busy",
            Self::NoLock => "no_lock",
            Self::Unknown => "unknown",
        }
    }
}

/// Byte 8 of the status reply (`Print head status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lw550PrintHeadStatus {
    Ok,
    Overheated,
    StatusUnknown,
    Unknown,
}

impl Lw550PrintHeadStatus {
    pub fn from_byte(value: u8) -> Self {
        match value {
            0 => Self::Ok,
            1 => Self::Overheated,
            2 => Self::StatusUnknown,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Overheated => "overheated",
            Self::StatusUnknown => "status_unknown",
            Self::Unknown => "unknown",
        }
    }
}

/// Byte 10 of the status reply (`Main bay status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    Unknown,
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
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::BayUnknown => "bay_unknown",
            Self::BayOpen => "bay_open",
            Self::NoMedia => "no_media",
            Self::MediaNotInsertedProperly => "media_not_inserted_properly",
            Self::MediaPresentUnknown => "media_present_unknown",
            Self::MediaEmpty => "media_empty",
            Self::MediaCriticallyLow => "media_critically_low",
            Self::MediaLow => "media_low",
            Self::MediaOk => "media_ok",
            Self::MediaJammed => "media_jammed",
            Self::MediaCounterfeit => "media_counterfeit",
            Self::Unknown => "unknown",
        }
    }
}

/// Byte 30 (low nibble) of the status reply (`Print head voltage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lw550PrintHeadVoltage {
    Unknown,
    Ok,
    Low,
    CriticallyLow,
    TooLowForPrinting,
}

impl Lw550PrintHeadVoltage {
    pub fn from_nibble(value: u8) -> Self {
        match value & 0x0f {
            0 => Self::Unknown,
            1 => Self::Ok,
            2 => Self::Low,
            3 => Self::CriticallyLow,
            4 => Self::TooLowForPrinting,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Ok => "ok",
            Self::Low => "low",
            Self::CriticallyLow => "critically_low",
            Self::TooLowForPrinting => "too_low_for_printing",
        }
    }
}

/// Parse a 32-byte `ESC A` print-engine status reply.
///
/// `label_total` and engine-version fields are left unset; callers that also
/// issue `ESC U` / `ESC V` should fill them with [`apply_engine_version`] and
/// the `ESC U` total.
///
/// Rejects payloads that are clearly not an `ESC A` reply — including an
/// `ESC U` NFC dump whose magic (`0xCAB6`) would otherwise decode as
/// print-status `182`.
pub fn parse_print_status(status: &[u8]) -> Result<Lw550PrintStatus, StatusError> {
    if status.len() < STATUS_REPLY_LEN {
        return Err(StatusError::Parse(format!(
            "short status reply ({} bytes, expected {STATUS_REPLY_LEN})",
            status.len()
        )));
    }
    let magic = u16::from_le_bytes(status[0..2].try_into().unwrap());
    if magic == SKU_INFO_MAGIC {
        return Err(StatusError::Parse(
            "status reply looks like an ESC U NFC dump (USB read desync); retry the status query"
                .into(),
        ));
    }
    if status[0] > 5 {
        return Err(StatusError::Parse(format!(
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
        sku_info: None,
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
///
/// Length fields are converted from on-wire deci-mm to millimetres (÷10).
pub fn parse_sku_info(data: &[u8]) -> Result<Lw550SkuInfo, StatusError> {
    if data.len() < SKU_INFO_REPLY_LEN {
        return Err(StatusError::Parse(format!(
            "short ESC U reply ({} bytes, expected {SKU_INFO_REPLY_LEN})",
            data.len()
        )));
    }
    let magic = le_u16(data, 0);
    if magic != SKU_INFO_MAGIC {
        return Err(StatusError::Parse(format!(
            "invalid ESC U magic {magic:#06x} (expected {SKU_INFO_MAGIC:#06x})"
        )));
    }
    Ok(Lw550SkuInfo {
        sku: parse_fixed_sku(&data[SKU_INFO_SKU_START..SKU_INFO_SKU_START + STATUS_SKU_LEN]),
        brand_id: data[20],
        region: data[21],
        material_type: data[22],
        label_type: data[23],
        label_colour: data[24],
        content_colour: data[25],
        marker_type: data[26],
        marker_pitch_mm: deci_mm_to_mm(le_u16(data, 28)),
        marker1_width_mm: deci_mm_to_mm(le_u16(data, 30)),
        marker1_to_label_start_mm: deci_mm_to_mm(le_u16(data, 32)),
        marker2_width_mm: deci_mm_to_mm(le_u16(data, 34)),
        marker2_offset_mm: deci_mm_to_mm(le_u16(data, 36)),
        vertical_offset_mm: deci_mm_to_mm(le_u16(data, 38)),
        label_length_mm: optional_die_cut_mm(le_u16(data, 40)),
        label_width_mm: deci_mm_to_mm(le_u16(data, 42)),
        printable_h_offset_mm: deci_mm_to_mm(le_u16(data, 44)),
        printable_v_offset_mm: deci_mm_to_mm(le_u16(data, 46)),
        liner_width_mm: deci_mm_to_mm(le_u16(data, 48)),
        total_label_count: le_u16(data, 50),
        total_length_mm: deci_mm_to_mm(le_u16(data, 52)),
        counter_margin: le_u16(data, 54),
        counter_strategy: data[56],
        production_date: parse_padded_ascii(&data[60..62]),
        // Tech Ref table lists bytes 62–63, but the reply is 63 bytes (indices
        // 0..62); take whatever remains after the date field.
        production_time: parse_padded_ascii(&data[62..SKU_INFO_REPLY_LEN]),
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
pub fn parse_engine_version(data: &[u8]) -> Result<Lw550EngineVersion, StatusError> {
    if data.len() < ENGINE_VERSION_REPLY_LEN {
        return Err(StatusError::Parse(format!(
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

/// Fold an `ESC V` engine-version block into a parsed `ESC A` status.
///
/// Only non-empty fields overwrite the status; the USB product id is always
/// copied since `0` is a valid (if unusual) value.
pub fn apply_engine_version(status: &mut Lw550PrintStatus, ver: &Lw550EngineVersion) {
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

/// Fold an `ESC U` NFC consumable dump into a parsed `ESC A` status.
///
/// Stores the full dump on [`Lw550PrintStatus::sku_info`], copies the full-roll
/// label total when the roll reports one, and fills the SKU when the status
/// reply itself did not carry it (the `ESC A` SKU field is blank while the bay
/// is mid-transition).
pub fn apply_sku_info(status: &mut Lw550PrintStatus, info: &Lw550SkuInfo) {
    status.sku_info = Some(info.clone());
    if info.total_label_count > 0 {
        status.label_total = Some(info.total_label_count);
    }
    if status.sku.is_none() {
        status.sku = info.sku.clone();
    }
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

/// Combine a prior status with a fresh one, keeping fields the fresh reply
/// omits but the prior one knew.
///
/// A single `ESC A` handshake reply carries neither the full-roll label total
/// (`ESC U`) nor the engine-version block (`ESC V`). When polling repeatedly,
/// carry those forward from the previous status so they don't flicker away:
///
/// - `label_total` / `sku_info` stick only while the SKU is unchanged (a new
///   roll may have different capacity and NFC geometry).
/// - Engine-version fields carry only when the fresh reply has none of them.
pub fn merge_dymo_lw_status(prior: &Lw550PrintStatus, next: Lw550PrintStatus) -> Lw550PrintStatus {
    let label_total =
        carried_label_total(prior.label_total, &prior.sku, next.label_total, &next.sku);
    let sku_info = carried_sku_info(
        prior.sku_info.as_ref(),
        &prior.sku,
        next.sku_info,
        &next.sku,
    );

    let carry_engine = next.hardware_version.is_none() && next.firmware_version.is_none();
    if !carry_engine {
        return Lw550PrintStatus {
            label_total,
            sku_info,
            ..next
        };
    }

    Lw550PrintStatus {
        label_total,
        sku_info,
        hardware_version: next
            .hardware_version
            .or_else(|| prior.hardware_version.clone()),
        firmware_version: next
            .firmware_version
            .or_else(|| prior.firmware_version.clone()),
        firmware_kind: next.firmware_kind.or_else(|| prior.firmware_kind.clone()),
        firmware_date: next.firmware_date.or_else(|| prior.firmware_date.clone()),
        usb_pid: next.usb_pid.or(prior.usb_pid),
        ..next
    }
}

/// Carry a full-roll `label_total` forward across polls.
///
/// A fresh `ESC A` handshake omits the `ESC U` full-roll total; keep the prior
/// value only while the SKU is unchanged, since a new roll may have a different
/// capacity.
fn carried_label_total(
    prior_total: Option<u16>,
    prior_sku: &Option<String>,
    next_total: Option<u16>,
    next_sku: &Option<String>,
) -> Option<u16> {
    match next_total {
        Some(total) if total > 0 => Some(total),
        _ => match prior_total {
            Some(prev_total) if prev_total > 0 && prior_sku == next_sku => Some(prev_total),
            _ => next_total,
        },
    }
}

/// Carry a full `ESC U` dump forward across handshake-only polls.
fn carried_sku_info(
    prior_info: Option<&Lw550SkuInfo>,
    prior_sku: &Option<String>,
    next_info: Option<Lw550SkuInfo>,
    next_sku: &Option<String>,
) -> Option<Lw550SkuInfo> {
    match next_info {
        Some(info) => Some(info),
        None if prior_sku == next_sku => prior_info.cloned(),
        None => None,
    }
}

/// Merge two [`Lw550PrintStatusView`] polls, preserving fields the fresh view
/// omits but the prior one knew.
///
/// The view counterpart of [`merge_dymo_lw_status`]: consumers that poll and
/// receive JSON views (e.g. over a serialization boundary) can carry the
/// `ESC U` full-roll total and `ESC V` engine-version block forward without
/// reconstructing the raw status. Fields absent from a handshake-only view are
/// `None` (see [`Lw550PrintStatusView`]).
pub fn merge_dymo_lw_status_view(
    prior: &Lw550PrintStatusView,
    next: Lw550PrintStatusView,
) -> Lw550PrintStatusView {
    let label_total =
        carried_label_total(prior.label_total, &prior.sku, next.label_total, &next.sku);
    let sku_info = carried_sku_info(
        prior.sku_info.as_ref(),
        &prior.sku,
        next.sku_info,
        &next.sku,
    );

    let carry_engine = next.hardware_version.is_none() && next.firmware_version.is_none();
    let mut merged = if !carry_engine {
        Lw550PrintStatusView {
            label_total,
            sku_info,
            ..next
        }
    } else {
        Lw550PrintStatusView {
            label_total,
            sku_info,
            hardware_version: next
                .hardware_version
                .or_else(|| prior.hardware_version.clone()),
            firmware_version: next
                .firmware_version
                .or_else(|| prior.firmware_version.clone()),
            firmware_kind: next.firmware_kind.or_else(|| prior.firmware_kind.clone()),
            firmware_date: next.firmware_date.or_else(|| prior.firmware_date.clone()),
            usb_pid: next.usb_pid.or(prior.usb_pid),
            ..next
        }
    };
    merged.recompute_readiness();
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_request_bytes() {
        assert_eq!(status_request(LOCK_ACQUIRE), [ESC, b'A', 1]);
        assert_eq!(status_request(LOCK_INTER_LABEL), [ESC, b'A', 2]);
        assert_eq!(sku_info_request(), [ESC, b'U']);
        assert_eq!(engine_version_request(), [ESC, b'V']);
        assert_eq!(soft_reboot_request(), [ESC, b'@']);
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
    fn parse_sku_info_deci_mm_lengths_to_mm() {
        // S0722540-class capture: 57.1 × 31.7 mm die-cut reports 571 / 317.
        let mut raw = vec![0u8; SKU_INFO_REPLY_LEN];
        raw[0..2].copy_from_slice(&SKU_INFO_MAGIC.to_le_bytes());
        raw[8..16].copy_from_slice(b"S0722540");
        raw[23] = 1; // die-cut
        raw[40..42].copy_from_slice(&571u16.to_le_bytes());
        raw[42..44].copy_from_slice(&317u16.to_le_bytes());
        raw[50..52].copy_from_slice(&350u16.to_le_bytes());

        let parsed = parse_sku_info(&raw).unwrap();
        assert_eq!(parsed.sku.as_deref(), Some("S0722540"));
        assert_eq!(parsed.label_type, 1);
        assert_eq!(parsed.label_length_mm, Some(57.1));
        assert!((parsed.label_width_mm - 31.7).abs() < 1e-9);
        assert_eq!(parsed.total_label_count, 350);
    }

    #[test]
    fn parse_sku_info_continuous_length_is_none() {
        let mut raw = vec![0u8; SKU_INFO_REPLY_LEN];
        raw[0..2].copy_from_slice(&SKU_INFO_MAGIC.to_le_bytes());
        raw[40..42].copy_from_slice(&0xFFFFu16.to_le_bytes());
        let parsed = parse_sku_info(&raw).unwrap();
        assert_eq!(parsed.label_length_mm, None);
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
    fn parse_print_status_fields() {
        let mut raw = vec![0u8; STATUS_REPLY_LEN];
        raw[0] = 1; // printing
        raw[1..5].copy_from_slice(&42u32.to_le_bytes());
        raw[5..7].copy_from_slice(&3u16.to_le_bytes());
        raw[8] = 0; // head ok
        raw[9] = 100;
        raw[10] = 8; // media present — ok
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
    fn merge_keeps_label_total_across_same_sku() {
        let mut prior = parse_status_from(8, Some("30252"), 82);
        prior.label_total = Some(350);
        prior.sku_info = Some(Lw550SkuInfo {
            sku: Some("30252".into()),
            total_label_count: 350,
            label_width_mm: 31.7,
            ..Default::default()
        });
        let next = parse_status_from(8, Some("30252"), 81);

        let merged = merge_dymo_lw_status(&prior, next);
        assert_eq!(merged.label_count, 81);
        assert_eq!(merged.label_total, Some(350));
        assert!((merged.sku_info.as_ref().unwrap().label_width_mm - 31.7).abs() < 1e-9);
    }

    #[test]
    fn merge_drops_label_total_when_sku_changes() {
        let mut prior = parse_status_from(8, Some("30252"), 82);
        prior.label_total = Some(350);
        let next = parse_status_from(8, Some("99014"), 200);

        let merged = merge_dymo_lw_status(&prior, next);
        assert_eq!(merged.label_total, None);
    }

    #[test]
    fn merge_carries_engine_fields() {
        let mut prior = parse_status_from(8, Some("30252"), 82);
        prior.hardware_version = Some("LW550".into());
        prior.firmware_version = Some("0001.0023".into());
        prior.usb_pid = Some(0x0028);
        let next = parse_status_from(8, Some("30252"), 81);

        let merged = merge_dymo_lw_status(&prior, next);
        assert_eq!(merged.hardware_version.as_deref(), Some("LW550"));
        assert_eq!(merged.firmware_version.as_deref(), Some("0001.0023"));
        assert_eq!(merged.usb_pid, Some(0x0028));
    }

    #[test]
    fn apply_sku_info_fills_total_and_missing_sku() {
        let mut status = parse_status_from(4, None, 40);
        let info = Lw550SkuInfo {
            sku: Some("30252".into()),
            total_label_count: 350,
            label_width_mm: 31.7,
            label_length_mm: Some(57.1),
            ..Default::default()
        };
        apply_sku_info(&mut status, &info);
        assert_eq!(status.label_total, Some(350));
        assert_eq!(status.sku.as_deref(), Some("30252"));
        assert_eq!(
            status.sku_info.as_ref().map(|i| i.total_label_count),
            Some(350)
        );
        assert!((status.sku_info.as_ref().unwrap().label_width_mm - 31.7).abs() < 1e-9);
    }

    #[test]
    fn apply_sku_info_keeps_existing_sku_and_ignores_zero_total() {
        let mut status = parse_status_from(8, Some("30252"), 40);
        let info = Lw550SkuInfo {
            sku: Some("99014".into()),
            total_label_count: 0,
            ..Default::default()
        };
        apply_sku_info(&mut status, &info);
        assert_eq!(status.label_total, None);
        assert_eq!(status.sku.as_deref(), Some("30252"));
    }

    #[test]
    fn merge_view_carries_total_and_engine_across_same_sku() {
        let mut prior = parse_status_from(8, Some("30252"), 82).to_view();
        prior.label_total = Some(350);
        prior.hardware_version = Some("LW550".into());
        prior.firmware_version = Some("0001.0023".into());
        prior.usb_pid = Some(0x0028);
        let next = parse_status_from(8, Some("30252"), 81).to_view();

        let merged = merge_dymo_lw_status_view(&prior, next);
        assert_eq!(merged.label_count, 81);
        assert_eq!(merged.label_total, Some(350));
        assert_eq!(merged.hardware_version.as_deref(), Some("LW550"));
        assert_eq!(merged.usb_pid, Some(0x0028));
    }

    #[test]
    fn merge_view_drops_total_when_sku_changes() {
        let mut prior = parse_status_from(8, Some("30252"), 82).to_view();
        prior.label_total = Some(350);
        let next = parse_status_from(8, Some("99014"), 200).to_view();

        let merged = merge_dymo_lw_status_view(&prior, next);
        assert_eq!(merged.label_total, None);
    }

    #[test]
    fn view_json_round_trips_through_deserialize() {
        let view = parse_status_from(8, Some("30252"), 82).to_view();
        let json = serde_json::to_string(&view).unwrap();
        let parsed: Lw550PrintStatusView = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, view);
    }

    fn parse_status_from(bay: u8, sku: Option<&str>, label_count: u16) -> Lw550PrintStatus {
        let mut raw = vec![0u8; STATUS_REPLY_LEN];
        raw[10] = bay;
        if let Some(sku) = sku {
            raw[11..11 + sku.len()].copy_from_slice(sku.as_bytes());
        }
        raw[27..29].copy_from_slice(&label_count.to_le_bytes());
        parse_print_status(&raw).unwrap()
    }
}

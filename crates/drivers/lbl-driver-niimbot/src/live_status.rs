//! Live-status probes for NIIMBOT printers: query builders and reply parsers
//! for RFID, heartbeat, print progress, and `PrinterInfo` (model / version /
//! serial).
//!
//! Unlike a print job, live status is a set of independent request/response
//! probes over a bidirectional transport. Each query is a framed command
//! packet; each reply is a framed response whose **payload** (the bytes between
//! the length byte and the checksum) carries the status fields. Callers that
//! own the transport frequently unframe the reply themselves — matching the
//! response command in the notify stream and handing back the payload — so the
//! parsers here operate on payload bytes. [`first_packet_payload`] exposes the
//! same frame scan for callers that only have a raw framed buffer.
//!
//! Field layouts follow the NIIMBOT community protocol notes
//! (<https://printers.niim.blue/interfacing/proto/>) and `niimbluelib`. `lbl`
//! is not affiliated with NIIMBOT; see the repository disclaimer.

use crate::{frame_packet, PrintStatus, GET_PRINT_STATUS, HEARTBEAT, PRINTER_INFO};

// Query command identifiers (request side).
const RFID_INFO: u8 = 0x1A;
const RFID_INFO2: u8 = 0x1C;

/// `PrinterInfo` sub-code selecting the numeric model id.
pub const PRINTER_INFO_MODEL_ID: u8 = 0x08;
/// `PrinterInfo` sub-code selecting the software (firmware) version.
pub const PRINTER_INFO_SOFTWARE_VERSION: u8 = 0x09;
/// `PrinterInfo` sub-code selecting the device serial number.
pub const PRINTER_INFO_SERIAL_NUMBER: u8 = 0x0B;
/// `PrinterInfo` sub-code selecting the hardware version.
pub const PRINTER_INFO_HARDWARE_VERSION: u8 = 0x0C;

/// Response command for an [`rfid_info_query`] reply.
pub const RFID_INFO_RESPONSE: u8 = 0x1B;
/// Response command for an [`rfid_info2_query`] reply.
pub const RFID_INFO2_RESPONSE: u8 = 0x1D;
/// Response command for an Advanced-2 heartbeat (protocol v3+ units, e.g. B1).
pub const HEARTBEAT_ADVANCED2_RESPONSE: u8 = 0xD9;
/// Response commands for an Advanced-1 heartbeat (D-series and relatives).
pub const HEARTBEAT_ADVANCED1_RESPONSES: [u8; 3] = [0xDD, 0xDE, 0xDF];
/// Response command for a [`print_progress_query`] reply.
pub const PRINT_STATUS_RESPONSE: u8 = 0xB3;
/// Response command for a `PrinterInfo(ModelId)` reply.
pub const PRINTER_MODEL_ID_RESPONSE: u8 = 0x48;
/// Response command for a `PrinterInfo(SoftwareVersion)` reply.
pub const PRINTER_SOFTWARE_VERSION_RESPONSE: u8 = 0x49;
/// Response command for a `PrinterInfo(SerialNumber)` reply.
pub const PRINTER_SERIAL_NUMBER_RESPONSE: u8 = 0x4B;
/// Response command for a `PrinterInfo(HardwareVersion)` reply.
pub const PRINTER_HARDWARE_VERSION_RESPONSE: u8 = 0x4C;

/// Print-progress snapshot from a `GetPrintStatus` reply (alias of the
/// job-side [`PrintStatus`], which carries `page` / `progress1` / `progress2`).
pub type NiimbotPrintProgress = PrintStatus;

/// RFID tag contents from a `RfidInfo` (`0x1b`) or `RfidInfo2` (`0x1d`) reply.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NiimbotRfidInfo {
    /// 8-byte tag UUID as lowercase hex.
    pub uuid: String,
    /// Manufacturer product/barcode string encoded on the tag.
    pub barcode: String,
    /// Per-roll serial string.
    pub serial: String,
    /// Total labels on a full roll (`0` when the tag omits the count).
    pub total_len: u16,
    /// Labels already consumed.
    pub used_len: u16,
    /// Label-type byte (`1` = gap/die-cut).
    pub label_type: u8,
}

/// Heartbeat sensor snapshot. Fields are `None` when the reply length does not
/// carry them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct NiimbotHeartbeat {
    /// Lid/cover closed (interpreted per model; see [`lid_closed_inverted`]).
    pub lid_closed: Option<bool>,
    /// Media/paper detected in the bay.
    pub paper_inserted: Option<bool>,
    /// RFID tag readable.
    pub rfid_ok: Option<bool>,
    /// Raw charge level as reported (0–4 bucket or an already-normalized 0–100).
    pub battery_level: Option<u8>,
}

/// Stable device identity from `PrinterInfo` queries.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct NiimbotDeviceInfo {
    /// Software/firmware version (`major.minor`).
    pub firmware_version: Option<String>,
    /// Hardware version (`major.minor`).
    pub hardware_version: Option<String>,
    /// Device serial number.
    pub serial: Option<String>,
}

/// Physical label size parsed from an RFID barcode (e.g. `T50X30-125`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NiimbotBarcodeDimensions {
    pub width_mm: u32,
    pub length_mm: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub roll_count: Option<u32>,
}

/// Aggregated live status assembled from the individual probes.
///
/// This is the protocol-level shape; the `protocol` discriminator is added by
/// the unified `lbl-status` `PrintStatus` when this is tagged for JSON. All
/// fields are optional so a partial poll (only some probes answered) still
/// produces a well-formed status.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
pub struct NiimbotLiveStatus {
    pub rfid: Option<NiimbotRfidInfo>,
    pub heartbeat: Option<NiimbotHeartbeat>,
    pub print_status: Option<NiimbotPrintProgress>,
    pub media_barcode: Option<String>,
    pub media_width_mm: Option<u32>,
    pub media_length_mm: Option<u32>,
    pub device_info: Option<NiimbotDeviceInfo>,
}

impl From<NiimbotPrintProgress> for NiimbotLiveStatus {
    /// Build a live status carrying only print progress (the framed
    /// `GetPrintStatus` reply on its own).
    fn from(progress: NiimbotPrintProgress) -> Self {
        Self {
            print_status: Some(progress),
            ..Self::default()
        }
    }
}

// ---- Query builders ---------------------------------------------------------

/// `RfidInfo` (`0x1a`) query; reply command [`RFID_INFO_RESPONSE`].
pub fn rfid_info_query() -> Vec<u8> {
    frame_packet(RFID_INFO, &[0x01])
}

/// `RfidInfo2` (`0x1c`) query; reply command [`RFID_INFO2_RESPONSE`].
pub fn rfid_info2_query() -> Vec<u8> {
    frame_packet(RFID_INFO2, &[0x01])
}

/// Heartbeat query. `variant` selects the firmware heartbeat mode (`0x01` for
/// D-series Advanced-1, `0x04` for B1 Advanced-2).
pub fn heartbeat_query(variant: u8) -> Vec<u8> {
    frame_packet(HEARTBEAT, &[variant])
}

/// `GetPrintStatus` (`0xa3`) query; reply command [`PRINT_STATUS_RESPONSE`].
pub fn print_progress_query() -> Vec<u8> {
    frame_packet(GET_PRINT_STATUS, &[0x01])
}

/// `PrinterInfo` (`0x40`) query for an arbitrary sub-code.
pub fn printer_info_query(subcode: u8) -> Vec<u8> {
    frame_packet(PRINTER_INFO, &[subcode])
}

/// `PrinterInfo(ModelId)` query; reply command [`PRINTER_MODEL_ID_RESPONSE`].
pub fn printer_model_id_query() -> Vec<u8> {
    printer_info_query(PRINTER_INFO_MODEL_ID)
}

/// `PrinterInfo(SoftwareVersion)` query; reply [`PRINTER_SOFTWARE_VERSION_RESPONSE`].
pub fn printer_software_version_query() -> Vec<u8> {
    printer_info_query(PRINTER_INFO_SOFTWARE_VERSION)
}

/// `PrinterInfo(HardwareVersion)` query; reply [`PRINTER_HARDWARE_VERSION_RESPONSE`].
pub fn printer_hardware_version_query() -> Vec<u8> {
    printer_info_query(PRINTER_INFO_HARDWARE_VERSION)
}

/// `PrinterInfo(SerialNumber)` query; reply [`PRINTER_SERIAL_NUMBER_RESPONSE`].
pub fn printer_serial_number_query() -> Vec<u8> {
    printer_info_query(PRINTER_INFO_SERIAL_NUMBER)
}

// ---- Payload parsers --------------------------------------------------------

/// Parse an RFID reply payload (`RfidInfo` / `RfidInfo2`).
///
/// Returns `None` when the tag is absent (`data[0] == 0`) or the payload is
/// truncated. A tag that omits the length block still parses, with the counts
/// left at zero and `label_type` defaulting to `1` (gap labels).
pub fn parse_rfid_payload(data: &[u8]) -> Option<NiimbotRfidInfo> {
    if data.is_empty() || data[0] == 0 {
        return None;
    }
    let mut idx = 0usize;
    let uuid = bytes_to_hex(data.get(idx..idx + 8)?);
    idx += 8;

    let barcode_len = *data.get(idx)? as usize;
    idx += 1;
    let barcode = decode_utf8_lossy(data.get(idx..idx + barcode_len)?);
    idx += barcode_len;

    let serial_len = *data.get(idx)? as usize;
    idx += 1;
    let serial = decode_utf8_lossy(data.get(idx..idx + serial_len)?);
    idx += serial_len;

    if idx + 5 > data.len() {
        return Some(NiimbotRfidInfo {
            uuid,
            barcode,
            serial,
            total_len: 0,
            used_len: 0,
            label_type: 1,
        });
    }

    Some(NiimbotRfidInfo {
        uuid,
        barcode,
        serial,
        total_len: u16::from_be_bytes([data[idx], data[idx + 1]]),
        used_len: u16::from_be_bytes([data[idx + 2], data[idx + 3]]),
        label_type: data[idx + 4],
    })
}

/// Parse a `GetPrintStatus` reply payload into [`NiimbotPrintProgress`].
pub fn parse_print_progress_payload(data: &[u8]) -> Option<NiimbotPrintProgress> {
    if data.len() < 4 {
        return None;
    }
    Some(NiimbotPrintProgress {
        page: u16::from_be_bytes([data[0], data[1]]),
        progress1: data[2],
        progress2: data[3],
    })
}

/// Parse a `PrinterInfo(ModelId)` reply payload.
///
/// A single-byte payload is the high byte of the model id (`byte << 8`, e.g.
/// D110 = `0x09` → `2304`); a two-byte payload is a big-endian id.
pub fn parse_model_id_payload(data: &[u8]) -> Option<u16> {
    match data.len() {
        0 => None,
        1 => Some((data[0] as u16) << 8),
        _ => Some(u16::from_be_bytes([data[0], data[1]])),
    }
}

/// Format a `PrinterInfo` software/hardware version payload as `major.minor`.
///
/// D-series units (and the official app) report `[major, minor]`, rendered with
/// a zero-padded minor (`21.08`).
pub fn parse_version_payload(data: &[u8]) -> Option<String> {
    if data.len() < 2 {
        return None;
    }
    Some(format!("{}.{:02}", data[0], data[1]))
}

/// Parse a `PrinterInfo(SerialNumber)` payload.
///
/// Payloads of 8+ bytes are ASCII serials (NUL/whitespace trimmed); shorter
/// 4–7 byte payloads are hex-encoded (uppercase). Payloads under 4 bytes yield
/// `None`.
pub fn parse_device_serial_payload(data: &[u8]) -> Option<String> {
    if data.len() < 4 {
        return None;
    }
    if data.len() >= 8 {
        let text = decode_utf8_lossy(data);
        let trimmed = text.trim_end_matches('\0').trim();
        return if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    Some(bytes_to_hex(&data[0..4]).to_uppercase())
}

/// Parse a heartbeat reply payload.
///
/// `response_cmd` selects the layout ([`HEARTBEAT_ADVANCED2_RESPONSE`] uses the
/// Advanced-2 packing; anything else is Advanced-1). `model_id` is required to
/// interpret the lid bit correctly: some models report it inverted (see
/// [`lid_closed_inverted`]). The Advanced-2 lid bit is never inverted.
pub fn parse_heartbeat_payload(
    data: &[u8],
    response_cmd: Option<u8>,
    model_id: Option<u16>,
) -> NiimbotHeartbeat {
    if response_cmd == Some(HEARTBEAT_ADVANCED2_RESPONSE) {
        return parse_heartbeat_advanced2(data);
    }
    let mut hb = parse_heartbeat_advanced1(data);
    if let Some(lid) = hb.lid_closed {
        if lid_closed_inverted(model_id) {
            hb.lid_closed = Some(!lid);
        }
    }
    hb
}

/// Advanced-2 heartbeat (response `0xd9`), used by protocol v3+ units over BLE.
fn parse_heartbeat_advanced2(data: &[u8]) -> NiimbotHeartbeat {
    if data.len() < 9 {
        return NiimbotHeartbeat::default();
    }
    NiimbotHeartbeat {
        battery_level: Some(data[2]),
        lid_closed: Some(data[4] == 0),
        paper_inserted: Some(data[5] == 0),
        rfid_ok: Some(data[6] != 0),
    }
}

/// Advanced-1 heartbeat (responses `0xdd` / `0xde` / `0xdf`). The field offsets
/// depend on the reply length, which varies by firmware generation.
fn parse_heartbeat_advanced1(data: &[u8]) -> NiimbotHeartbeat {
    match data.len() {
        10 => NiimbotHeartbeat {
            lid_closed: Some(data[8] == 0),
            battery_level: Some(data[9]),
            paper_inserted: None,
            rfid_ok: None,
        },
        13 => NiimbotHeartbeat {
            lid_closed: Some(data[9] == 0),
            battery_level: Some(data[10]),
            paper_inserted: Some(data[11] == 0),
            rfid_ok: Some(data[12] != 0),
        },
        19 => NiimbotHeartbeat {
            lid_closed: Some(data[15] == 0),
            battery_level: Some(data[16]),
            paper_inserted: Some(data[17] == 0),
            rfid_ok: Some(data[18] != 0),
        },
        20 => NiimbotHeartbeat {
            lid_closed: None,
            battery_level: None,
            paper_inserted: Some(data[18] == 0),
            rfid_ok: Some(data[19] != 0),
        },
        len if len >= 9 => NiimbotHeartbeat {
            battery_level: Some(data[2]),
            lid_closed: Some(data[3] == 0),
            paper_inserted: Some(data[4] == 0),
            rfid_ok: Some(data[5] != 0),
        },
        _ => NiimbotHeartbeat::default(),
    }
}

/// Parse physical label size from an RFID barcode (e.g. `T50X30-125`).
///
/// Scans for the first `[T]<width><x|X|*><length>[-<roll>]` run so pack codes
/// with a leading prefix still resolve.
pub fn parse_barcode_dimensions(barcode: &str) -> Option<NiimbotBarcodeDimensions> {
    let b = barcode.as_bytes();
    let n = b.len();
    for start in 0..n {
        let mut j = start;
        if b[j] == b'T' || b[j] == b't' {
            j += 1;
        }
        let w0 = j;
        while j < n && b[j].is_ascii_digit() {
            j += 1;
        }
        if j == w0 {
            continue;
        }
        let width: u32 = match barcode[w0..j].parse() {
            Ok(w) => w,
            Err(_) => continue,
        };
        if j >= n || !matches!(b[j], b'x' | b'X' | b'*') {
            continue;
        }
        j += 1;
        let l0 = j;
        while j < n && b[j].is_ascii_digit() {
            j += 1;
        }
        if j == l0 {
            continue;
        }
        let length: u32 = match barcode[l0..j].parse() {
            Ok(l) => l,
            Err(_) => continue,
        };
        let mut roll_count = None;
        if j < n && b[j] == b'-' {
            let r0 = j + 1;
            let mut k = r0;
            while k < n && b[k].is_ascii_digit() {
                k += 1;
            }
            if k > r0 {
                roll_count = barcode[r0..k].parse().ok();
            }
        }
        return Some(NiimbotBarcodeDimensions {
            width_mm: width,
            length_mm: length,
            roll_count,
        });
    }
    None
}

/// Map a heartbeat charge level (0–4 bucket) to a percentage. Levels 5–100 are
/// treated as an already-normalized reading; anything else is `0`.
pub fn battery_percent(level: u8) -> u8 {
    match level {
        0..=4 => level * 25,
        5..=100 => level,
        _ => 0,
    }
}

/// Model ids whose Advanced-1 heartbeat reports the lid bit inverted
/// (`1` = closed) rather than the usual `0` = closed.
///
/// Mirrors the NIIMBOT community protocol notes and `niimbluelib`'s
/// `invertedLidModels`. Covers D11 (512), D11S (514), D110 (2304), D101 (2560),
/// B16 / B18 / C1 and related units.
const INVERTED_LID_MODEL_IDS: &[u16] = &[
    512, 514, 513, 2304, 1792, 3584, 5120, 2560, 3840, 4352, 272, 273, 274,
];

/// Whether `model_id` reports the heartbeat lid bit inverted.
pub fn lid_closed_inverted(model_id: Option<u16>) -> bool {
    model_id.is_some_and(|id| INVERTED_LID_MODEL_IDS.contains(&id))
}

// ---- Assembly + cross-poll merge -------------------------------------------

/// Assemble a [`NiimbotLiveStatus`] from the parsed probes, deriving the media
/// barcode and physical dimensions from the RFID tag when present.
pub fn assemble_live_status(
    rfid: Option<NiimbotRfidInfo>,
    heartbeat: Option<NiimbotHeartbeat>,
    print_status: Option<NiimbotPrintProgress>,
    device_info: Option<NiimbotDeviceInfo>,
) -> NiimbotLiveStatus {
    let dims = rfid
        .as_ref()
        .and_then(|r| parse_barcode_dimensions(&r.barcode));
    NiimbotLiveStatus {
        media_barcode: rfid.as_ref().map(|r| r.barcode.clone()),
        media_width_mm: dims.as_ref().map(|d| d.width_mm),
        media_length_mm: dims.as_ref().map(|d| d.length_mm),
        rfid,
        heartbeat,
        print_status,
        device_info,
    }
}

/// Merge a fresh poll onto the previous status, keeping the last successful
/// RFID / heartbeat / device fields when a poll returns partial data.
///
/// Print progress is intentionally **not** sticky: a `None` reply means
/// idle/unknown, and retaining a mid-job leftover would leave a UI stuck on
/// "printing" after the job finished.
pub fn merge_live_status(prev: &NiimbotLiveStatus, next: NiimbotLiveStatus) -> NiimbotLiveStatus {
    let rfid = merge_rfid(prev.rfid.clone(), next.rfid.clone());
    let dims = rfid
        .as_ref()
        .and_then(|r| parse_barcode_dimensions(&r.barcode));
    NiimbotLiveStatus {
        media_barcode: rfid
            .as_ref()
            .map(|r| r.barcode.clone())
            .or_else(|| next.media_barcode.clone())
            .or_else(|| prev.media_barcode.clone()),
        media_width_mm: dims
            .as_ref()
            .map(|d| d.width_mm)
            .or(next.media_width_mm)
            .or(prev.media_width_mm),
        media_length_mm: dims
            .as_ref()
            .map(|d| d.length_mm)
            .or(next.media_length_mm)
            .or(prev.media_length_mm),
        heartbeat: merge_heartbeat(prev.heartbeat, next.heartbeat),
        print_status: next.print_status,
        device_info: merge_device_info(prev.device_info.clone(), next.device_info.clone()),
        rfid,
    }
}

fn rfid_has_label_count(rfid: Option<&NiimbotRfidInfo>) -> bool {
    rfid.is_some_and(|r| r.total_len > 0)
}

fn merge_rfid(
    prev: Option<NiimbotRfidInfo>,
    next: Option<NiimbotRfidInfo>,
) -> Option<NiimbotRfidInfo> {
    if rfid_has_label_count(next.as_ref()) {
        return next;
    }
    if rfid_has_label_count(prev.as_ref()) {
        return prev;
    }
    next.or(prev)
}

/// Whether a heartbeat carries any populated sensor field.
pub fn heartbeat_has_fields(hb: &NiimbotHeartbeat) -> bool {
    hb.lid_closed.is_some()
        || hb.paper_inserted.is_some()
        || hb.rfid_ok.is_some()
        || hb.battery_level.is_some()
}

fn merge_heartbeat(
    prev: Option<NiimbotHeartbeat>,
    next: Option<NiimbotHeartbeat>,
) -> Option<NiimbotHeartbeat> {
    if !next.as_ref().is_some_and(heartbeat_has_fields) {
        return prev;
    }
    if !prev.as_ref().is_some_and(heartbeat_has_fields) {
        return next;
    }
    let (p, n) = (prev.unwrap(), next.unwrap());
    Some(NiimbotHeartbeat {
        lid_closed: n.lid_closed.or(p.lid_closed),
        paper_inserted: n.paper_inserted.or(p.paper_inserted),
        rfid_ok: n.rfid_ok.or(p.rfid_ok),
        battery_level: n.battery_level.or(p.battery_level),
    })
}

fn merge_device_info(
    prev: Option<NiimbotDeviceInfo>,
    next: Option<NiimbotDeviceInfo>,
) -> Option<NiimbotDeviceInfo> {
    if prev.is_none() && next.is_none() {
        return None;
    }
    Some(NiimbotDeviceInfo {
        firmware_version: next
            .as_ref()
            .and_then(|n| n.firmware_version.clone())
            .or_else(|| prev.as_ref().and_then(|p| p.firmware_version.clone())),
        hardware_version: next
            .as_ref()
            .and_then(|n| n.hardware_version.clone())
            .or_else(|| prev.as_ref().and_then(|p| p.hardware_version.clone())),
        serial: next
            .as_ref()
            .and_then(|n| n.serial.clone())
            .or_else(|| prev.as_ref().and_then(|p| p.serial.clone())),
    })
}

// ---- Frame helpers ----------------------------------------------------------

/// Return the payload of the first framed packet matching `command` in a raw
/// buffer, scanning for the `55 55` header and validating the trailer.
///
/// For callers that have a raw framed reply rather than an already-unframed
/// payload; the payload parsers otherwise take payload bytes directly.
pub fn first_packet_payload(bytes: &[u8], command: u8) -> Option<&[u8]> {
    let mut i = 0usize;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0x55 || bytes[i + 1] != 0x55 {
            i += 1;
            continue;
        }
        let cmd = bytes[i + 2];
        let len = bytes[i + 3] as usize;
        let data_start = i + 4;
        let data_end = data_start + len;
        if data_end + 3 > bytes.len() {
            break;
        }
        if cmd == command {
            return Some(&bytes[data_start..data_end]);
        }
        i = data_end + 3;
    }
    None
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn decode_utf8_lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(hex: &str) -> Vec<u8> {
        hex.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    fn heartbeat_data(len: usize, overrides: &[(usize, u8)]) -> Vec<u8> {
        let mut data = vec![0u8; len];
        for &(i, v) in overrides {
            data[i] = v;
        }
        data
    }

    #[test]
    fn rfid_parses_real_d110_15x30_tag() {
        // Captured from a physical D110 + genuine 15x30 roll: numeric barcode,
        // no size in the barcode, dimensions only via the length block.
        let data = bytes(
            "88 1d a8 4a b0 92 00 00 08 30 32 32 38 32 32 38 30 10 \
             50 5a 30 46 33 30 39 30 31 33 30 32 35 33 36 37 00 fc 00 1e 01",
        );
        let rfid = parse_rfid_payload(&data).unwrap();
        assert_eq!(rfid.uuid, "881da84ab0920000");
        assert_eq!(rfid.barcode, "02282280");
        assert_eq!(rfid.serial, "PZ0F309013025367");
        assert_eq!(rfid.total_len, 252);
        assert_eq!(rfid.used_len, 30);
        assert_eq!(rfid.label_type, 1);
    }

    #[test]
    fn rfid_rejects_absent_tag() {
        assert!(parse_rfid_payload(&[0x00]).is_none());
        assert!(parse_rfid_payload(&[]).is_none());
    }

    #[test]
    fn model_id_expands_single_byte_high() {
        assert_eq!(parse_model_id_payload(&[0x09]), Some(2304)); // D110
        assert_eq!(parse_model_id_payload(&[0x09, 0x00]), Some(2304));
        assert_eq!(parse_model_id_payload(&[0x10, 0x00]), Some(4096));
        assert_eq!(parse_model_id_payload(&[]), None);
    }

    #[test]
    fn lid_inversion_matches_model_semantics() {
        assert!(lid_closed_inverted(Some(2304))); // D110
        assert!(lid_closed_inverted(Some(512))); // D11
        assert!(lid_closed_inverted(Some(514))); // D11S
        assert!(lid_closed_inverted(Some(2560))); // D101
        assert!(!lid_closed_inverted(Some(4096))); // B1
        assert!(!lid_closed_inverted(Some(2305)));
        assert!(!lid_closed_inverted(None));
    }

    #[test]
    fn heartbeat_lid_inversion_needs_model_id() {
        // D110 reports byte=1 when the lid is physically closed.
        let data = heartbeat_data(10, &[(8, 1), (9, 4)]);
        assert_eq!(
            parse_heartbeat_payload(&data, Some(0xDD), None).lid_closed,
            Some(false)
        );
        let hb = parse_heartbeat_payload(&data, Some(0xDD), Some(2304));
        assert_eq!(hb.lid_closed, Some(true));
        assert_eq!(hb.battery_level, Some(4));
    }

    #[test]
    fn heartbeat_open_lid_on_inverted_model() {
        let data = heartbeat_data(10, &[(8, 0), (9, 4)]);
        let hb = parse_heartbeat_payload(&data, Some(0xDD), Some(2304));
        assert_eq!(hb.lid_closed, Some(false));
    }

    #[test]
    fn heartbeat_standard_model_not_inverted() {
        let data = heartbeat_data(10, &[(8, 0), (9, 4)]);
        let hb = parse_heartbeat_payload(&data, Some(0xDD), Some(4096));
        assert_eq!(hb.lid_closed, Some(true));
    }

    #[test]
    fn heartbeat_advanced2_never_inverts() {
        let data = heartbeat_data(9, &[(4, 0), (5, 0), (6, 1)]);
        let hb = parse_heartbeat_payload(&data, Some(0xD9), Some(2304));
        assert_eq!(hb.lid_closed, Some(true));
        assert_eq!(hb.paper_inserted, Some(true));
        assert_eq!(hb.rfid_ok, Some(true));
    }

    #[test]
    fn version_formats_major_minor() {
        assert_eq!(parse_version_payload(&[21, 8]).as_deref(), Some("21.08"));
        assert_eq!(parse_version_payload(&[21, 1]).as_deref(), Some("21.01"));
        assert_eq!(parse_version_payload(&[21]), None);
    }

    #[test]
    fn device_serial_ascii_and_hex() {
        assert_eq!(
            parse_device_serial_payload(b"E928010263").as_deref(),
            Some("E928010263")
        );
        assert_eq!(
            parse_device_serial_payload(&[0xE9, 0x28, 0x01, 0x02]).as_deref(),
            Some("E9280102")
        );
        assert_eq!(parse_device_serial_payload(&[1, 2, 3]), None);
    }

    #[test]
    fn battery_percent_buckets_and_passthrough() {
        assert_eq!(battery_percent(0), 0);
        assert_eq!(battery_percent(1), 25);
        assert_eq!(battery_percent(4), 100);
        assert_eq!(battery_percent(46), 46);
        assert_eq!(battery_percent(100), 100);
        assert_eq!(battery_percent(200), 0);
    }

    #[test]
    fn barcode_dimensions_variants() {
        let d = parse_barcode_dimensions("T50X30-125").unwrap();
        assert_eq!((d.width_mm, d.length_mm, d.roll_count), (50, 30, Some(125)));
        let d = parse_barcode_dimensions("15x30").unwrap();
        assert_eq!((d.width_mm, d.length_mm, d.roll_count), (15, 30, None));
        let d = parse_barcode_dimensions("40*60").unwrap();
        assert_eq!((d.width_mm, d.length_mm), (40, 60));
        assert!(parse_barcode_dimensions("02282280").is_none());
    }

    #[test]
    fn print_progress_payload_parses() {
        let p = parse_print_progress_payload(&[0x00, 0x01, 100, 100]).unwrap();
        assert_eq!(p.page, 1);
        assert!(p.is_complete());
        assert!(parse_print_progress_payload(&[0x00]).is_none());
    }

    #[test]
    fn assemble_derives_media_dimensions_from_barcode() {
        let rfid = NiimbotRfidInfo {
            uuid: "aa".into(),
            barcode: "T50X30-125".into(),
            serial: "ROLL1".into(),
            total_len: 100,
            used_len: 10,
            label_type: 1,
        };
        let status = assemble_live_status(Some(rfid), None, None, None);
        assert_eq!(status.media_barcode.as_deref(), Some("T50X30-125"));
        assert_eq!(status.media_width_mm, Some(50));
        assert_eq!(status.media_length_mm, Some(30));
    }

    #[test]
    fn merge_keeps_prior_heartbeat_when_next_empty() {
        let prev = NiimbotLiveStatus {
            heartbeat: Some(NiimbotHeartbeat {
                lid_closed: Some(true),
                paper_inserted: Some(true),
                rfid_ok: Some(true),
                battery_level: Some(4),
            }),
            ..Default::default()
        };
        let merged = merge_live_status(&prev, NiimbotLiveStatus::default());
        assert_eq!(merged.heartbeat.unwrap().battery_level, Some(4));
    }

    #[test]
    fn merge_does_not_retain_mid_job_progress() {
        let prev = NiimbotLiveStatus {
            print_status: Some(NiimbotPrintProgress {
                page: 0,
                progress1: 31,
                progress2: 0,
            }),
            ..Default::default()
        };
        let merged = merge_live_status(&prev, NiimbotLiveStatus::default());
        assert_eq!(merged.print_status, None);
    }

    #[test]
    fn merge_prefers_rfid_with_label_count() {
        let prev = NiimbotLiveStatus {
            rfid: Some(NiimbotRfidInfo {
                uuid: "aa".into(),
                barcode: "02282280".into(),
                serial: "R1".into(),
                total_len: 200,
                used_len: 5,
                label_type: 1,
            }),
            ..Default::default()
        };
        let next_rfid = NiimbotRfidInfo {
            uuid: "bb".into(),
            barcode: "".into(),
            serial: "".into(),
            total_len: 0,
            used_len: 0,
            label_type: 1,
        };
        let next = NiimbotLiveStatus {
            rfid: Some(next_rfid),
            ..Default::default()
        };
        let merged = merge_live_status(&prev, next);
        assert_eq!(merged.rfid.unwrap().total_len, 200);
    }

    #[test]
    fn live_status_json_round_trips() {
        let status = assemble_live_status(
            Some(NiimbotRfidInfo {
                uuid: "aa".into(),
                barcode: "T50X30".into(),
                serial: "R1".into(),
                total_len: 100,
                used_len: 10,
                label_type: 1,
            }),
            Some(NiimbotHeartbeat {
                lid_closed: Some(true),
                paper_inserted: Some(true),
                rfid_ok: Some(true),
                battery_level: Some(3),
            }),
            Some(NiimbotPrintProgress {
                page: 1,
                progress1: 40,
                progress2: 0,
            }),
            Some(NiimbotDeviceInfo {
                firmware_version: Some("21.08".into()),
                hardware_version: Some("21.01".into()),
                serial: Some("E928010263".into()),
            }),
        );
        let json = serde_json::to_string(&status).unwrap();
        let parsed: NiimbotLiveStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, status);
    }

    #[test]
    fn first_packet_payload_scans_frames() {
        let framed = print_progress_query();
        // The query frames GetPrintStatus with [0x01]; find it back.
        assert_eq!(
            first_packet_payload(&framed, GET_PRINT_STATUS),
            Some(&[0x01u8][..])
        );
        assert_eq!(first_packet_payload(&framed, 0x99), None);
    }
}

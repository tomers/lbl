//! NIIMBOT thermal label driver.
//!
//! NIIMBOT printers (D11, D110, B-series, ...) speak a small packet-framed
//! protocol rather than a streaming page language. Every command is wrapped as:
//!
//! ```text
//! 0x55 0x55 <command:u8> <len:u8> <data...> <checksum:u8> 0xAA 0xAA
//! ```
//!
//! where `checksum` is the XOR of the command byte, the length byte, and every
//! data byte. A print job is a fixed sequence of command packets framing a run
//! of one-row bitmap packets:
//!
//! ```text
//! SetDensity      (0x21) [density:u8]
//! SetLabelType    (0x23) [type:u8]        1 = gap/die-cut labels
//! StartPrint      (0x01) [0x01]
//! StartPagePrint  (0x03) [0x01]
//! SetDimension    (0x13) [rows:u16, cols:u16]   big-endian; rows = feed length
//! SetQuantity     (0x15) [copies:u16]     big-endian; printer repeats the page
//! per row (top → bottom):
//!   PrintBitmapRow (0x85) [y:u16, c0:u8, c1:u8, c2:u8, repeat:u8, data...]
//! EndPagePrint    (0xE3) [0x01]
//! EndPrint        (0xF3) [0x01]
//! ```
//!
//! The print head is horizontal (96 dots / 12 mm on the D110), so the printer
//! consumes one raster line per row of the image: `cols` = dots across the head
//! (bitmap width) and `rows` = lines in the feed direction (bitmap height). The
//! row payload packs `ceil(width / 8)` bytes MSB-first with `1` = ink, exactly
//! the [`MonoBitmap`] layout, so rows are emitted without conversion. The three
//! `c0..c2` bytes are the count of ink pixels in each third of the row (some
//! firmwares use them for progress/wear levelling; others ignore them).
//!
//! Protocol reference: the NIIMBOT community documentation
//! (<https://printers.niim.blue/interfacing/proto/>) and the `niimprint` project.
//! `lbl` is not affiliated with NIIMBOT; see the repository disclaimer.

use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

// Packet framing.
const HEAD: [u8; 2] = [0x55, 0x55];
const TAIL: [u8; 2] = [0xAA, 0xAA];

// Command identifiers.
const SET_DENSITY: u8 = 0x21;
const SET_LABEL_TYPE: u8 = 0x23;
const START_PRINT: u8 = 0x01;
const START_PAGE_PRINT: u8 = 0x03;
const SET_DIMENSION: u8 = 0x13;
const SET_QUANTITY: u8 = 0x15;
const PRINT_BITMAP_ROW: u8 = 0x85;
const PRINT_EMPTY_ROW: u8 = 0x84;
const END_PAGE_PRINT: u8 = 0xE3;
const END_PRINT: u8 = 0xF3;

// V4 task extras (D110M / 2025+ pocket printers over BLE).
const GET_PRINT_STATUS: u8 = 0xA3;
const HEARTBEAT: u8 = 0xDC;

// Status handshake (only meaningful over a bidirectional transport).
const PRINT_STATUS_RESPONSE: u8 = 0xB3;

// Label type 1 = gap-sensed die-cut labels (the common case for D-series tape).
const LABEL_TYPE_GAP: u8 = 0x01;

/// Which NIIMBOT print-task sequence to emit.
///
/// Pocket D-series firmware (D110M V4, 2025+) uses [`NiimbotTask::V4`]: a
/// 9-byte `PrintStart`, 13-byte `SetPageSize`, no `PageStart`, and a one-way
/// `Heartbeat` after `PrintEnd`. B1 / B21 (protocol 3, 203 dpi) use
/// [`NiimbotTask::B1`]: 7-byte PrintStart, 6-byte SetPageSize, total-mode row
/// counts, and a post-connect handshake over BLE. Older B-series USB units use
/// [`NiimbotTask::Standard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NiimbotTask {
    /// Legacy D110 / B-series: 1-byte PrintStart, PageStart, 4-byte dimensions
    /// + separate PrintQuantity.
    #[default]
    Standard,
    /// D110M V4 (typical for BLE): 9-byte PrintStart, 13-byte SetPageSize, no
    /// PageStart, status + heartbeat extras.
    V4,
    /// B1 / B21 (protocol 3): 7-byte PrintStart, PageStart, 6-byte SetPageSize,
    /// total-mode rows, empty-row opcode, run-length grouping.
    B1,
}

/// Driver for NIIMBOT thermal label printers (D11 / D110 family and
/// compatibles).
#[derive(Debug, Clone, Copy)]
pub struct NiimbotDriver {
    /// Print density / heat level. Valid ranges are model-specific (the D110
    /// accepts 1–3; B-series printers accept 1–5). Higher is darker.
    pub density: u8,
    /// Print-task variant (see [`NiimbotTask`]).
    pub task: NiimbotTask,
}

impl Default for NiimbotDriver {
    fn default() -> Self {
        // 3 is the darkest setting the D110 accepts and a safe mid value for
        // models with a wider range.
        Self {
            density: 3,
            task: NiimbotTask::Standard,
        }
    }
}

impl NiimbotDriver {
    /// Create a new driver with the default density and [`NiimbotTask::Standard`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a driver configured for the D110M V4 BLE print task.
    pub fn v4() -> Self {
        Self {
            task: NiimbotTask::V4,
            ..Self::default()
        }
    }

    /// Create a driver configured for the B1 / B21 print task (protocol 3).
    pub fn b1() -> Self {
        Self {
            task: NiimbotTask::B1,
            ..Self::default()
        }
    }

    /// Create a driver with an explicit print density.
    pub fn with_density(density: u8) -> Self {
        Self {
            density,
            ..Self::default()
        }
    }

    /// Create a driver with an explicit print task.
    pub fn with_task(task: NiimbotTask) -> Self {
        Self {
            task,
            ..Self::default()
        }
    }

    /// Stable config/API name for a print task.
    pub fn task_name(task: NiimbotTask) -> &'static str {
        match task {
            NiimbotTask::Standard => "standard",
            NiimbotTask::V4 => "v4",
            NiimbotTask::B1 => "b1",
        }
    }

    /// Parse a print-task name from config or the API (`standard`, `v4`, `b1`).
    pub fn parse_task_name(name: &str) -> Option<NiimbotTask> {
        match name.to_ascii_lowercase().as_str() {
            "standard" => Some(NiimbotTask::Standard),
            "v4" => Some(NiimbotTask::V4),
            "b1" => Some(NiimbotTask::B1),
            _ => None,
        }
    }

    /// Resolve a driver-variant string from config or the API.
    ///
    /// `None` selects [`NiimbotTask::Standard`]. Unrecognized names return
    /// `None` so callers can reject or fall back explicitly.
    pub fn resolve_task(driver_variant: Option<&str>) -> Option<NiimbotTask> {
        match driver_variant {
            None => Some(NiimbotTask::Standard),
            Some(name) => Self::parse_task_name(name),
        }
    }

    /// Non-default driver instance for `task`, if the builtin registry entry
    /// should be overridden.
    pub fn driver_for_task(task: NiimbotTask) -> Option<Self> {
        match task {
            NiimbotTask::Standard => None,
            NiimbotTask::V4 => Some(Self::v4()),
            NiimbotTask::B1 => Some(Self::b1()),
        }
    }

    /// Override driver for a generic variant string, if the builtin default is
    /// not correct.
    ///
    /// Returns `None` when the registry's standard entry should be kept
    /// (absent/`standard` variant, or an unrecognized name).
    pub fn for_variant(variant: Option<&str>) -> Option<Self> {
        Self::resolve_task(variant).and_then(Self::driver_for_task)
    }

    /// Infer the NIIMBOT print task from a catalog printer key.
    ///
    /// Returns `None` when the model has no known task override (callers fall
    /// back to config / `standard`).
    pub fn task_for_printer_key(key: &str) -> Option<NiimbotTask> {
        let k = key.to_ascii_lowercase();
        match k.as_str() {
            // Protocol 3 / B1 task (+ K3/K4, TT pocket N1 / M2_H per niimbluelib).
            "b1" | "b1_se" | "b1se" | "b2" | "b21" | "b21s" | "b21_c2b" | "b21c2b" | "b21_l2b"
            | "b21l2b" | "b203" | "b3s" | "b3s_p" | "b31" | "b4" | "k3" | "k3_w" | "k3w" | "k4"
            | "b18" | "b18s" | "n1" | "m2_h" | "m2h" => Some(NiimbotTask::B1),
            // 300 dpi Pro / H variants use the V4 task.
            "b1 pro" | "b1pro" | "b1_pro" | "b21 pro" | "b21pro" | "b21_pro" | "d11_h" | "d11h"
            | "d11_pro" | "d11pro" | "d110_m" | "d110m" => Some(NiimbotTask::V4),
            _ => None,
        }
    }

    /// Append one framed packet (`55 55 <cmd> <len> <data> <csum> AA AA`).
    fn push_packet(out: &mut Vec<u8>, command: u8, data: &[u8]) {
        let len = data.len() as u8;
        let mut checksum = command ^ len;
        for &b in data {
            checksum ^= b;
        }
        out.extend_from_slice(&HEAD);
        out.push(command);
        out.push(len);
        out.extend_from_slice(data);
        out.push(checksum);
        out.extend_from_slice(&TAIL);
    }

    /// Count ink pixels in each (roughly equal) third of row `y`, saturating at
    /// 255 to fit the single-byte header fields.
    fn row_chunk_counts(bitmap: &MonoBitmap, y: u32) -> [u8; 3] {
        let w = bitmap.width;
        let bounds = [0, w / 3, (2 * w) / 3, w];
        let mut counts = [0u8; 3];
        for (chunk, count) in counts.iter_mut().enumerate() {
            let mut n: u32 = 0;
            for x in bounds[chunk]..bounds[chunk + 1] {
                if bitmap.get(x, y) {
                    n += 1;
                }
            }
            *count = n.min(255) as u8;
        }
        counts
    }
}

impl Driver for NiimbotDriver {
    fn protocol(&self) -> Protocol {
        Protocol::Niimbot
    }

    fn name(&self) -> &'static str {
        "niimbot"
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.width == 0 || bitmap.height == 0 {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }
        // `rows`/`cols` and the row index are u16 fields.
        if bitmap.width > 0xFFFF || bitmap.height > 0xFFFF {
            return Err(DriverError::Unsupported(format!(
                "bitmap {}x{} exceeds the 16-bit dimension fields",
                bitmap.width, bitmap.height
            )));
        }
        // Each row packet carries a 6-byte header plus the packed row; the
        // packet length field is a single byte (max 255 data bytes).
        let stride = bitmap.stride();
        if stride + 6 > 0xFF {
            return Err(DriverError::Unsupported(format!(
                "row of {} dots is too wide for a niimbot packet (max 1992)",
                bitmap.width
            )));
        }

        let rows = bitmap.height as u16;
        let cols = bitmap.width as u16;
        let copies = ctx.copies().min(0xFFFF) as u16;
        let density = ctx.job.density.unwrap_or(self.density).clamp(1, 5);

        match self.task {
            NiimbotTask::Standard => {
                self.encode_standard(bitmap, rows, cols, copies, stride, density)
            }
            NiimbotTask::V4 => self.encode_v4(bitmap, rows, cols, copies, stride, density),
            NiimbotTask::B1 => self.encode_b1(bitmap, rows, cols, copies, stride, density),
        }
    }

    fn variant_for_printer_key(&self, key: &str) -> Option<&'static str> {
        Self::task_for_printer_key(key).map(Self::task_name)
    }

    fn override_for_variant(&self, variant: Option<&str>) -> Option<Box<dyn Driver>> {
        Self::for_variant(variant).map(|d| Box::new(d) as Box<dyn Driver>)
    }
}

impl NiimbotDriver {
    fn encode_standard(
        &self,
        bitmap: &MonoBitmap,
        rows: u16,
        cols: u16,
        copies: u16,
        stride: usize,
        density: u8,
    ) -> Result<Vec<u8>, DriverError> {
        let mut out = Vec::with_capacity(bitmap.data.len() + bitmap.height as usize * 12 + 64);

        Self::push_packet(&mut out, SET_DENSITY, &[density]);
        Self::push_packet(&mut out, SET_LABEL_TYPE, &[LABEL_TYPE_GAP]);
        Self::push_packet(&mut out, START_PRINT, &[0x01]);
        Self::push_packet(&mut out, START_PAGE_PRINT, &[0x01]);

        let mut dimension = Vec::with_capacity(4);
        dimension.extend_from_slice(&rows.to_be_bytes());
        dimension.extend_from_slice(&cols.to_be_bytes());
        Self::push_packet(&mut out, SET_DIMENSION, &dimension);
        Self::push_packet(&mut out, SET_QUANTITY, &copies.to_be_bytes());

        self.push_rows(&mut out, bitmap, stride);
        Self::push_packet(&mut out, END_PAGE_PRINT, &[0x01]);
        Self::push_packet(&mut out, END_PRINT, &[0x01]);
        Ok(out)
    }

    /// D110M V4 print task (2025+ pocket printers, especially over BLE).
    ///
    /// Reference: <https://printers.niim.blue/interfacing/print-tasks/#d110m_v4>
    fn encode_v4(
        &self,
        bitmap: &MonoBitmap,
        rows: u16,
        cols: u16,
        copies: u16,
        stride: usize,
        density: u8,
    ) -> Result<Vec<u8>, DriverError> {
        let mut out = Vec::with_capacity(bitmap.data.len() + bitmap.height as usize * 12 + 96);

        Self::push_packet(&mut out, SET_DENSITY, &[density]);
        Self::push_packet(&mut out, SET_LABEL_TYPE, &[LABEL_TYPE_GAP]);

        // 9-byte PrintStart: pages(u16), 0×4, pageColor, speed, flag.
        let mut start = [0u8; 9];
        start[0..2].copy_from_slice(&1u16.to_be_bytes()); // one page in this job
        Self::push_packet(&mut out, START_PRINT, &start);

        // BLE firmware drops the first post-start packet; absorb it with a
        // one-way status query (niimbluelib / community workaround).
        Self::push_packet(&mut out, GET_PRINT_STATUS, &[0x01]);

        // 13-byte SetPageSize: rows, cols, copies, cutHeight, cutType, 0, sendAll, partHeight.
        let mut page_size = [0u8; 13];
        page_size[0..2].copy_from_slice(&rows.to_be_bytes());
        page_size[2..4].copy_from_slice(&cols.to_be_bytes());
        page_size[4..6].copy_from_slice(&copies.to_be_bytes());
        page_size[6..8].copy_from_slice(&0u16.to_be_bytes()); // cutHeight
                                                              // page_size[8] cutType = 0
                                                              // page_size[9] = 0
                                                              // page_size[10] sendAll = 0
        page_size[11..13].copy_from_slice(&0u16.to_be_bytes()); // partHeight
        Self::push_packet(&mut out, SET_DIMENSION, &page_size);

        self.push_rows(&mut out, bitmap, stride);
        Self::push_packet(&mut out, END_PAGE_PRINT, &[0x01]);
        Self::push_packet(&mut out, END_PRINT, &[0x01]);

        // One-way heartbeat after PrintEnd (BLE session cleanup; also absorbs a
        // dropped-packet quirk on some firmwares).
        Self::push_packet(&mut out, HEARTBEAT, &[0x01]);

        Ok(out)
    }

    /// B1 / B21 print task (protocol 3, 203 dpi, BLE or USB serial).
    ///
    /// Reference: niim.blue / niimbot-web-bluetooth `b1` task variant.
    fn encode_b1(
        &self,
        bitmap: &MonoBitmap,
        rows: u16,
        cols: u16,
        copies: u16,
        stride: usize,
        density: u8,
    ) -> Result<Vec<u8>, DriverError> {
        let mut out = Vec::with_capacity(bitmap.data.len() + bitmap.height as usize * 12 + 96);

        Self::push_packet(&mut out, SET_DENSITY, &[density]);
        Self::push_packet(&mut out, SET_LABEL_TYPE, &[LABEL_TYPE_GAP]);

        // 7-byte PrintStart: page count in first u16, remainder zeroed.
        let mut start = [0u8; 7];
        start[0..2].copy_from_slice(&copies.max(1).to_be_bytes());
        Self::push_packet(&mut out, START_PRINT, &start);

        Self::push_packet(&mut out, START_PAGE_PRINT, &[0x01]);

        // 6-byte SetPageSize: rows, cols, copies (u16 big-endian).
        let mut page_size = [0u8; 6];
        page_size[0..2].copy_from_slice(&rows.to_be_bytes());
        page_size[2..4].copy_from_slice(&cols.to_be_bytes());
        page_size[4..6].copy_from_slice(&copies.max(1).to_be_bytes());
        Self::push_packet(&mut out, SET_DIMENSION, &page_size);

        self.push_rows_b1(&mut out, bitmap, stride);
        Self::push_packet(&mut out, END_PAGE_PRINT, &[0x01]);
        Self::push_packet(&mut out, END_PRINT, &[0x01]);
        Ok(out)
    }

    fn push_rows(&self, out: &mut Vec<u8>, bitmap: &MonoBitmap, stride: usize) {
        let mut row_data = Vec::with_capacity(6 + stride);
        for y in 0..bitmap.height {
            row_data.clear();
            row_data.extend_from_slice(&(y as u16).to_be_bytes());
            row_data.extend_from_slice(&Self::row_chunk_counts(bitmap, y));
            row_data.push(0x01); // repeat this line once
            row_data.extend_from_slice(bitmap.row(y));
            Self::push_packet(out, PRINT_BITMAP_ROW, &row_data);
        }
    }

    /// Emit B1 rows with total-mode counts, empty-row opcode, and run-length.
    fn push_rows_b1(&self, out: &mut Vec<u8>, bitmap: &MonoBitmap, stride: usize) {
        const MAX_RUN: u8 = 200;

        let mut y = 0u32;
        while y < bitmap.height {
            let row = bitmap.row(y);
            let blank = row.iter().all(|&b| b == 0);
            let mut run = 1u8;
            while run < MAX_RUN
                && y + (run as u32) < bitmap.height
                && rows_equal(bitmap, y, y + run as u32, stride)
            {
                run += 1;
            }

            if blank {
                let mut data = Vec::with_capacity(3);
                data.extend_from_slice(&(y as u16).to_be_bytes());
                data.push(run);
                Self::push_packet(out, PRINT_EMPTY_ROW, &data);
            } else {
                let total = Self::row_ink_total(bitmap, y);
                let mut data = Vec::with_capacity(6 + stride);
                data.extend_from_slice(&(y as u16).to_be_bytes());
                data.push(0x00);
                data.extend_from_slice(&total.to_le_bytes());
                data.push(run);
                data.extend_from_slice(row);
                Self::push_packet(out, PRINT_BITMAP_ROW, &data);
            }
            y += run as u32;
        }
    }

    /// Total ink pixels in row `y` (B1 total-mode row header).
    fn row_ink_total(bitmap: &MonoBitmap, y: u32) -> u16 {
        let mut n = 0u32;
        for x in 0..bitmap.width {
            if bitmap.get(x, y) {
                n += 1;
            }
        }
        n.min(0xFFFF) as u16
    }
}

fn rows_equal(bitmap: &MonoBitmap, a: u32, b: u32, stride: usize) -> bool {
    bitmap.row(a)[..stride] == bitmap.row(b)[..stride]
}

// B1 post-connect handshake (required before the printer will feed paper).
const PRINTER_STATUS_DATA: u8 = 0xA5;
const PRINTER_INFO: u8 = 0x40;

/// Sub-codes for [`PRINTER_INFO`] queries during the B1 connect handshake.
pub const B1_INFO_SUBCODES: [u8; 8] = [0x08, 0x0b, 0x0d, 0x0a, 0x07, 0x03, 0x0c, 0x09];

/// Initial BLE connect packet (raw, with leading `0x03` prefix).
pub const B1_BLE_CONNECT: [u8; 9] = [0x03, 0x55, 0x55, 0xC1, 0x01, 0x01, 0xC1, 0xAA, 0xAA];

/// Frame an arbitrary NIIMBOT command packet
/// (`55 55 <cmd> <len> <data> <csum> AA AA`). Exposed for callers that drive
/// the bidirectional handshake (status polling) outside the encode path.
pub fn frame_packet(command: u8, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 7);
    NiimbotDriver::push_packet(&mut out, command, data);
    out
}

/// The `GetPrintStatus` query packet. Send this over a bidirectional transport
/// and parse the reply with [`parse_status`] to track a running print.
pub fn status_query() -> Vec<u8> {
    frame_packet(GET_PRINT_STATUS, &[0x01])
}

/// Packets for the B1 post-connect handshake (after [`B1_BLE_CONNECT`]).
pub fn b1_handshake_packets() -> Vec<Vec<u8>> {
    let mut out = vec![frame_packet(PRINTER_STATUS_DATA, &[0x01])];
    for &sub in &B1_INFO_SUBCODES {
        out.push(frame_packet(PRINTER_INFO, &[sub]));
    }
    out.push(frame_packet(HEARTBEAT, &[0x04]));
    out
}

/// A decoded print-status reply from the printer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrintStatus {
    /// The page (label) currently being printed.
    pub page: u16,
    /// Primary progress percentage (0–100).
    pub progress1: u8,
    /// Secondary progress percentage (0–100).
    pub progress2: u8,
}

impl PrintStatus {
    /// Whether the current page has finished printing (Standard / V4 tasks).
    pub fn is_complete(&self) -> bool {
        self.progress1 >= 100 && self.progress2 >= 100
    }

    /// Whether the B1 task reports the page counter has advanced (page ≥ 1).
    pub fn is_page_done(&self) -> bool {
        self.page >= 1
    }
}

/// Parse the first `PrintStatusResponse` (`0xB3`) packet found in `bytes`.
///
/// Scans for the `55 55` frame header so it tolerates leading noise or other
/// packets in the buffer. Returns `None` if no well-formed status reply is
/// present (e.g. the printer is idle and the session has ended).
pub fn parse_status(bytes: &[u8]) -> Option<PrintStatus> {
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0x55 || bytes[i + 1] != 0x55 {
            i += 1;
            continue;
        }
        let command = bytes[i + 2];
        let len = bytes[i + 3] as usize;
        let data_start = i + 4;
        let data_end = data_start + len;
        // Need the data plus checksum + 2-byte tail to be a complete packet.
        if data_end + 3 > bytes.len() {
            break;
        }
        if command == PRINT_STATUS_RESPONSE && len >= 4 {
            let data = &bytes[data_start..data_end];
            return Some(PrintStatus {
                page: u16::from_be_bytes([data[0], data[1]]),
                progress1: data[2],
                progress2: data[3],
            });
        }
        i = data_end + 3; // skip this packet (checksum + tail)
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::job::JobSpec;
    use lbl_core::media::Media;
    use lbl_core::printer::PrinterCapabilities;
    use lbl_core::units::Dpi;

    fn ctx_job(copies: u32) -> JobSpec {
        let mut job = JobSpec::new(Media::fixed(12.0, 40.0, Dpi(203.0)));
        job.copies = copies;
        job
    }

    /// Locate the first framed packet of `command` and return its data slice.
    fn find_packet(bytes: &[u8], command: u8) -> Option<&[u8]> {
        let mut i = 0;
        while i + 4 <= bytes.len() {
            if bytes[i] == 0x55 && bytes[i + 1] == 0x55 {
                let cmd = bytes[i + 2];
                let len = bytes[i + 3] as usize;
                let data = &bytes[i + 4..i + 4 + len];
                if cmd == command {
                    return Some(data);
                }
                i += 4 + len + 3; // checksum + tail
            } else {
                i += 1;
            }
        }
        None
    }

    fn checksum_ok(bytes: &[u8]) -> bool {
        let mut i = 0;
        while i + 4 <= bytes.len() {
            assert_eq!(&bytes[i..i + 2], &[0x55, 0x55], "bad head at {i}");
            let cmd = bytes[i + 2];
            let len = bytes[i + 3] as usize;
            let data = &bytes[i + 4..i + 4 + len];
            let mut cs = cmd ^ (len as u8);
            for &b in data {
                cs ^= b;
            }
            assert_eq!(bytes[i + 4 + len], cs, "bad checksum at {i}");
            assert_eq!(&bytes[i + 5 + len..i + 7 + len], &[0xAA, 0xAA], "bad tail");
            i += 7 + len;
        }
        i == bytes.len()
    }

    #[test]
    fn emits_framed_setup_and_teardown() {
        let bmp = MonoBitmap::new(96, 4);
        let caps = PrinterCapabilities::default();
        let job = ctx_job(1);
        let bytes = NiimbotDriver::new()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();

        assert!(checksum_ok(&bytes), "every packet must frame and checksum");

        // Dimension: rows = height (4), cols = width (96), big-endian.
        let dim = find_packet(&bytes, SET_DIMENSION).unwrap();
        assert_eq!(dim, &[0x00, 0x04, 0x00, 0x60]);

        // Density and teardown present.
        assert_eq!(find_packet(&bytes, SET_DENSITY).unwrap(), &[3]);
        assert_eq!(find_packet(&bytes, END_PRINT).unwrap(), &[0x01]);
    }

    #[test]
    fn one_row_packet_per_line_with_ink_data() {
        let mut bmp = MonoBitmap::new(8, 2);
        bmp.set(0, 0, true); // first row, MSB
        let caps = PrinterCapabilities::default();
        let job = ctx_job(1);
        let bytes = NiimbotDriver::new()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();

        let rows = bytes
            .windows(3)
            .filter(|w| w[0] == 0x55 && w[1] == 0x55 && w[2] == PRINT_BITMAP_ROW)
            .count();
        assert_eq!(rows, 2);

        // First row: index 0, one ink pixel in the first third, repeat 1, 0x80.
        let row0 = find_packet(&bytes, PRINT_BITMAP_ROW).unwrap();
        assert_eq!(row0[0..2], [0x00, 0x00]); // y = 0
        assert_eq!(row0[2], 1); // first third has the single ink pixel
        assert_eq!(row0[5], 1); // repeat count
        assert_eq!(row0[6], 0x80); // packed row, MSB set
    }

    #[test]
    fn copies_set_quantity_not_repeated_rows() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = PrinterCapabilities::default();
        let job = ctx_job(5);
        let bytes = NiimbotDriver::new()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();

        // Quantity carries the copy count...
        assert_eq!(find_packet(&bytes, SET_QUANTITY).unwrap(), &[0x00, 0x05]);
        // ...and the row stream is emitted exactly once.
        let rows = bytes
            .windows(3)
            .filter(|w| w[0] == 0x55 && w[1] == 0x55 && w[2] == PRINT_BITMAP_ROW)
            .count();
        assert_eq!(rows, 1);
    }

    #[test]
    fn status_query_is_a_framed_get_status_packet() {
        let q = status_query();
        assert!(checksum_ok(&q));
        assert_eq!(q[2], GET_PRINT_STATUS);
        assert_eq!(find_packet(&q, GET_PRINT_STATUS).unwrap(), &[0x01]);
    }

    #[test]
    fn parses_status_reply_and_detects_completion() {
        // page = 1, progress 100/100 → complete.
        let done = frame_packet(PRINT_STATUS_RESPONSE, &[0x00, 0x01, 100, 100]);
        let s = parse_status(&done).unwrap();
        assert_eq!(s.page, 1);
        assert!(s.is_complete());

        // page = 0, progress 40/0 → still printing.
        let mid = frame_packet(PRINT_STATUS_RESPONSE, &[0x00, 0x00, 40, 0]);
        assert!(!parse_status(&mid).unwrap().is_complete());

        // No status packet present.
        assert!(parse_status(&[0x55, 0x55, 0x01, 0x01, 0x01]).is_none());
    }

    #[test]
    fn rejects_rows_too_wide_for_a_packet() {
        let bmp = MonoBitmap::new(2000, 1); // stride 250 > 249
        let caps = PrinterCapabilities::default();
        let job = ctx_job(1);
        let err = NiimbotDriver::new().encode(&bmp, &EncodeContext::new(&job, &caps));
        assert!(matches!(err, Err(DriverError::Unsupported(_))));
    }

    #[test]
    fn v4_uses_nine_byte_print_start_and_thirteen_byte_page_size() {
        let bmp = MonoBitmap::new(96, 4);
        let caps = PrinterCapabilities::default();
        let job = ctx_job(1);
        let bytes = NiimbotDriver::v4()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();

        assert!(checksum_ok(&bytes));
        // 9-byte PrintStart, no PageStart (0x03), no PrintQuantity (0x15).
        assert_eq!(find_packet(&bytes, START_PRINT).unwrap().len(), 9);
        assert!(find_packet(&bytes, START_PAGE_PRINT).is_none());
        assert!(find_packet(&bytes, SET_QUANTITY).is_none());
        assert_eq!(find_packet(&bytes, SET_DIMENSION).unwrap().len(), 13);
        assert!(find_packet(&bytes, GET_PRINT_STATUS).is_some());
        assert!(find_packet(&bytes, HEARTBEAT).is_some());
    }

    #[test]
    fn b1_uses_seven_byte_print_start_and_six_byte_page_size() {
        let bmp = MonoBitmap::new(384, 4);
        let caps = PrinterCapabilities::default();
        let job = ctx_job(3);
        let bytes = NiimbotDriver::b1()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();

        assert!(checksum_ok(&bytes));
        assert_eq!(find_packet(&bytes, START_PRINT).unwrap().len(), 7);
        assert!(find_packet(&bytes, START_PAGE_PRINT).is_some());
        assert!(find_packet(&bytes, SET_QUANTITY).is_none());
        let page_size = find_packet(&bytes, SET_DIMENSION).unwrap();
        assert_eq!(page_size.len(), 6);
        assert_eq!(page_size[4..6], [0x00, 0x03]); // copies u16 BE
        assert!(find_packet(&bytes, PRINT_EMPTY_ROW).is_some());
    }

    #[test]
    fn b1_row_uses_total_mode_pixel_count() {
        let mut bmp = MonoBitmap::new(8, 1);
        bmp.set(0, 0, true);
        let caps = PrinterCapabilities::default();
        let job = ctx_job(1);
        let bytes = NiimbotDriver::b1()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();
        let row = find_packet(&bytes, PRINT_BITMAP_ROW).unwrap();
        assert_eq!(row[2], 0x00);
        assert_eq!(row[3..5], [0x01, 0x00]); // total = 1, little-endian
    }

    #[test]
    fn b1_status_page_done_when_page_at_least_one() {
        let done = frame_packet(PRINT_STATUS_RESPONSE, &[0x00, 0x01, 50, 0]);
        let s = parse_status(&done).unwrap();
        assert!(s.is_page_done());
    }

    #[test]
    fn resolve_task_defaults_none_to_standard() {
        assert_eq!(
            NiimbotDriver::resolve_task(None),
            Some(NiimbotTask::Standard)
        );
    }

    #[test]
    fn resolve_task_parses_known_names() {
        assert_eq!(
            NiimbotDriver::resolve_task(Some("v4")),
            Some(NiimbotTask::V4)
        );
        assert_eq!(
            NiimbotDriver::resolve_task(Some("B1")),
            Some(NiimbotTask::B1)
        );
    }

    #[test]
    fn resolve_task_rejects_unknown_names() {
        assert_eq!(NiimbotDriver::resolve_task(Some("unknown")), None);
    }

    #[test]
    fn driver_for_task_overrides_only_non_standard() {
        assert!(NiimbotDriver::driver_for_task(NiimbotTask::Standard).is_none());
        assert_eq!(
            NiimbotDriver::driver_for_task(NiimbotTask::V4)
                .unwrap()
                .task,
            NiimbotTask::V4
        );
        assert_eq!(
            NiimbotDriver::driver_for_task(NiimbotTask::B1)
                .unwrap()
                .task,
            NiimbotTask::B1
        );
    }

    #[test]
    fn for_variant_selects_override_drivers() {
        assert!(NiimbotDriver::for_variant(None).is_none());
        assert!(NiimbotDriver::for_variant(Some("standard")).is_none());
        assert_eq!(
            NiimbotDriver::for_variant(Some("v4")).unwrap().task,
            NiimbotTask::V4
        );
        assert_eq!(
            NiimbotDriver::for_variant(Some("b1")).unwrap().task,
            NiimbotTask::B1
        );
        assert!(NiimbotDriver::for_variant(Some("unknown")).is_none());
    }

    #[test]
    fn task_for_printer_key_maps_b_series_to_b1() {
        for key in [
            "B1", "B1_SE", "B2", "B21", "B21S", "B21_C2B", "B21_L2B", "B203", "B3S", "B3S_P",
            "B31", "B4", "K3", "K3_W", "K4", "B18", "B18S", "N1", "M2_H", "M2H",
        ] {
            assert_eq!(
                NiimbotDriver::task_for_printer_key(key),
                Some(NiimbotTask::B1),
                "{key} should map to B1"
            );
        }
    }

    #[test]
    fn task_for_printer_key_maps_pro_h_to_v4() {
        for key in [
            "B1 Pro", "B1Pro", "B1_PRO", "B21 Pro", "B21Pro", "B21_PRO", "D11_H", "D11H",
            "D11_PRO", "D11Pro", "D110_M", "D110M",
        ] {
            assert_eq!(
                NiimbotDriver::task_for_printer_key(key),
                Some(NiimbotTask::V4),
                "{key} should map to V4"
            );
        }
    }

    #[test]
    fn task_for_printer_key_no_match_for_plain_models() {
        assert_eq!(NiimbotDriver::task_for_printer_key("D110"), None);
        assert_eq!(NiimbotDriver::task_for_printer_key("D101"), None);
        assert_eq!(NiimbotDriver::task_for_printer_key("unknown"), None);
    }
}

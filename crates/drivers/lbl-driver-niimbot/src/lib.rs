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
/// `Heartbeat` after `PrintEnd`. B-series and older D110 units over USB serial
/// use [`NiimbotTask::Standard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NiimbotTask {
    /// Legacy D110 / B-series: 1-byte PrintStart, PageStart, 4-byte dimensions
    /// + separate PrintQuantity.
    #[default]
    Standard,
    /// D110M V4 (typical for BLE): 9-byte PrintStart, 13-byte SetPageSize, no
    /// PageStart, status + heartbeat extras.
    V4,
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

        match self.task {
            NiimbotTask::Standard => self.encode_standard(bitmap, rows, cols, copies, stride),
            NiimbotTask::V4 => self.encode_v4(bitmap, rows, cols, copies, stride),
        }
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
    ) -> Result<Vec<u8>, DriverError> {
        let mut out = Vec::with_capacity(bitmap.data.len() + bitmap.height as usize * 12 + 64);

        Self::push_packet(&mut out, SET_DENSITY, &[self.density]);
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
    ) -> Result<Vec<u8>, DriverError> {
        let mut out = Vec::with_capacity(bitmap.data.len() + bitmap.height as usize * 12 + 96);

        Self::push_packet(&mut out, SET_DENSITY, &[self.density]);
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
}

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
    /// Whether the current page has finished printing.
    pub fn is_complete(&self) -> bool {
        self.progress1 >= 100 && self.progress2 >= 100
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
}

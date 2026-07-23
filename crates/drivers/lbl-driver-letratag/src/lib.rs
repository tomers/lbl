//! DYMO LetraTag LT-200B thermal label driver (Bluetooth LE job encoder).
//!
//! Encodes a [`MonoBitmap`] into the chunked GATT print-request stream used by
//! the LT-200B. This is a **clean-room** reimplementation of the publicly
//! documented wire format — see `docs/research-letratag.md` and the README in
//! this crate. Sources consulted (not vendored):
//!
//! - <https://github.com/ysfchn/dymo-bluetooth> (MIT)
//! - <https://github.com/thermal-label/letratag> protocol docs
//! - <https://github.com/alexhorn/lt200b> (AGPL — topology citation only)
//!
//! `lbl` is not affiliated with DYMO / Newell. LetraTag is a trademark of DYMO.

use lbl_driver_api::{ClientHandshake, Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

/// Name prefixes accepted by Web Bluetooth `requestDevice` filters.
pub const NAME_PREFIXES: &[&str] = &["Letratag", "DYMO LT-200B", "LT-200B", "LT20"];

/// Canonical GATT service UUID (first 8 hex digits are the stable prefix).
pub const SERVICE_UUID: &str = "be3dd650-2b3d-42f1-99c1-f0f749dd0678";
/// Print-request characteristic (write-without-response).
pub const WRITE_UUID: &str = "be3dd651-2b3d-42f1-99c1-f0f749dd0678";
/// Print-reply characteristic (notify).
pub const NOTIFY_UUID: &str = "be3dd652-2b3d-42f1-99c1-f0f749dd0678";
/// Short-command characteristic (set-cassette, no chunking).
pub const SHORT_CMD_UUID: &str = "be3dd653-2b3d-42f1-99c1-f0f749dd0678";

const ESC: u8 = 0x1b;
const MAGIC: [u8; 2] = [0x12, 0x34];
const HEADER_PREAMBLE: [u8; 2] = [0xff, 0xf0];
const JOB_ID: [u8; 4] = [0x9a, 0x02, 0x00, 0x00];
const HEAD_ROWS: u32 = 32;
const BYTES_PER_COLUMN: usize = 4;
/// Protocol body chunk payload ceiling (index byte is separate).
pub const CHUNK_PAYLOAD: usize = 500;
/// Skip sequence index 27 (0x1B) to avoid ESC collision in the index byte.
const SKIP_INDEX: u8 = 27;
/// Minimum feed columns after scaling (firmware tiny-print alternation quirk).
const MIN_FEED_COLUMNS: u32 = 30;
/// Default cassette id for 12 mm LT tape.
pub const CASSETTE_12MM: u8 = 3;

/// Firmware command dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LetraTagVariant {
    /// LT-200B path: `ESC #`, bpp `0x81`, `ESC p` cut (thermal-label Avatar).
    #[default]
    Avatar,
    /// Older RE path: bpp `0x01`, `ESC E` form-feed instead of cut (ysfchn Genie).
    Genie,
}

/// Driver for DYMO LetraTag LT-200B-class BLE printers.
#[derive(Debug, Clone, Copy)]
pub struct LetraTagDriver {
    /// Command dialect (see [`LetraTagVariant`]).
    pub variant: LetraTagVariant,
    /// Repeat each feed column this many times (mobile app uses 2 for aspect).
    pub column_scale: u32,
    /// Pad feed to at least this many columns after scaling.
    pub min_feed_columns: u32,
    /// Negotiated ATT MTU (default 247). Caps chunk payload via [`payload_ceiling`].
    pub mtu: u16,
}

impl Default for LetraTagDriver {
    fn default() -> Self {
        Self {
            variant: LetraTagVariant::Avatar,
            column_scale: 2,
            min_feed_columns: MIN_FEED_COLUMNS,
            mtu: 247,
        }
    }
}

impl LetraTagDriver {
    /// Avatar dialect with default column scale.
    pub fn new() -> Self {
        Self::default()
    }

    /// Genie dialect (form-feed instead of cut).
    pub fn genie() -> Self {
        Self {
            variant: LetraTagVariant::Genie,
            ..Self::default()
        }
    }
}

impl Driver for LetraTagDriver {
    fn protocol(&self) -> Protocol {
        Protocol::LetraTag
    }

    fn name(&self) -> &'static str {
        "letratag"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["letratag", "letra-tag"]
    }

    fn handshake(&self) -> ClientHandshake {
        ClientHandshake::LetraTagNotify
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.height > HEAD_ROWS {
            return Err(DriverError::Unsupported(format!(
                "letratag head height is {HEAD_ROWS} rows; got {}",
                bitmap.height
            )));
        }
        if bitmap.width == 0 || bitmap.height == 0 {
            return Err(DriverError::Unsupported("letratag bitmap is empty".into()));
        }

        let copies = ctx.copies().min(255) as u8;
        // Avatar always emits `ESC p`; cut vs suppress follows EncodeContext.
        // Genie uses form-feed and has no cut opcode.
        let do_cut = matches!(self.variant, LetraTagVariant::Avatar) && ctx.should_cut();

        let pixels = pack_raster(
            bitmap,
            self.column_scale.max(1),
            self.min_feed_columns.max(1),
        );
        let width = (pixels.len() / BYTES_PER_COLUMN) as u32;
        let body = match self.variant {
            LetraTagVariant::Avatar => build_body_avatar(copies, width, &pixels, do_cut),
            LetraTagVariant::Genie => build_body_genie(width, &pixels),
        };
        Ok(frame_job_with_mtu(&body, self.mtu))
    }
}

/// Build the 9-byte print-request header for a body of `body_len` bytes.
pub fn build_header(body_len: u32) -> [u8; 9] {
    let mut h = [0u8; 9];
    h[0] = HEADER_PREAMBLE[0];
    h[1] = HEADER_PREAMBLE[1];
    h[2] = MAGIC[0];
    h[3] = MAGIC[1];
    h[4..8].copy_from_slice(&body_len.to_le_bytes());
    h[8] = h[..8].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    h
}

/// Pack a 32-row head column into 4 wire bytes (byte-reversed MSB packing).
pub fn pack_column(rows: &[bool; HEAD_ROWS as usize]) -> [u8; BYTES_PER_COLUMN] {
    let mut out = [0u8; BYTES_PER_COLUMN];
    for (y, &ink) in rows.iter().enumerate() {
        if !ink {
            continue;
        }
        let byte_index = 3 - (y / 8);
        let bit_index = 7 - (y % 8);
        out[byte_index] |= 1u8 << bit_index;
    }
    out
}

/// Center `bitmap` in 32 head rows, scale feed columns, pad to min length, pack.
pub fn pack_raster(bitmap: &MonoBitmap, column_scale: u32, min_feed: u32) -> Vec<u8> {
    let scale = column_scale.max(1);
    let h = bitmap.height.min(HEAD_ROWS);
    let top = (HEAD_ROWS - h) / 2;
    let feed = bitmap.width;
    let scaled = feed.saturating_mul(scale);
    let total = scaled.max(min_feed.max(1));

    let mut out = Vec::with_capacity(total as usize * BYTES_PER_COLUMN);
    for col in 0..total {
        let src_x = if col < scaled { col / scale } else { feed };
        let mut rows = [false; HEAD_ROWS as usize];
        if src_x < feed {
            for y in 0..h {
                if bitmap.get(src_x, y) {
                    rows[(top + y) as usize] = true;
                }
            }
        }
        out.extend_from_slice(&pack_column(&rows));
    }
    out
}

fn push_esc(out: &mut Vec<u8>, opcode: u8) {
    out.push(ESC);
    out.push(opcode);
}

fn build_body_avatar(copies: u8, width: u32, pixels: &[u8], cut: bool) -> Vec<u8> {
    let mut body = Vec::with_capacity(12 + pixels.len() + 16);
    push_esc(&mut body, b's');
    body.extend_from_slice(&JOB_ID);
    push_esc(&mut body, b'#');
    body.push(copies.max(1));
    push_esc(&mut body, b'D');
    body.push(0x81); // bpp
    body.push(0x02); // align
    body.extend_from_slice(&width.to_le_bytes());
    body.extend_from_slice(&HEAD_ROWS.to_le_bytes());
    body.extend_from_slice(pixels);
    push_esc(&mut body, b'p');
    body.push(if cut { b'0' } else { b'1' });
    push_esc(&mut body, b'A');
    push_esc(&mut body, b'Q');
    body
}

fn build_body_genie(width: u32, pixels: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(12 + pixels.len() + 12);
    push_esc(&mut body, b's');
    body.extend_from_slice(&JOB_ID);
    push_esc(&mut body, b'D');
    body.push(0x01); // bpp (Genie / ysfchn)
    body.push(0x02);
    body.extend_from_slice(&width.to_le_bytes());
    body.extend_from_slice(&HEAD_ROWS.to_le_bytes());
    body.extend_from_slice(pixels);
    push_esc(&mut body, b'E'); // form feed
    push_esc(&mut body, b'A');
    push_esc(&mut body, b'Q');
    body
}

/// Sequence index for zero-based chunk position `i` (skips 27).
pub fn chunk_index(i: usize) -> u8 {
    let idx = i as u8;
    if idx >= SKIP_INDEX {
        idx.wrapping_add(1)
    } else {
        idx
    }
}

/// Split `body` into wire chunks (`index || payload [|| MAGIC]`).
///
/// `max_payload` is the body-byte ceiling per chunk (typically
/// `min(500, mtu.saturating_sub(1))`).
pub fn chunk_body_with_limit(body: &[u8], max_payload: usize) -> Vec<Vec<u8>> {
    let max_payload = max_payload.clamp(1, CHUNK_PAYLOAD);
    if body.is_empty() {
        return vec![vec![chunk_index(0), MAGIC[0], MAGIC[1]]];
    }
    let mut out = Vec::new();
    let mut offset = 0;
    let mut i = 0;
    while offset < body.len() {
        let remaining = body.len() - offset;
        let take = remaining.min(max_payload);
        let is_last = take == remaining;
        let mut chunk = Vec::with_capacity(1 + take + if is_last { 2 } else { 0 });
        chunk.push(chunk_index(i));
        chunk.extend_from_slice(&body[offset..offset + take]);
        if is_last {
            chunk.extend_from_slice(&MAGIC);
        }
        out.push(chunk);
        offset += take;
        i += 1;
    }
    out
}

/// Split `body` into wire chunks at the protocol 500-byte payload ceiling.
pub fn chunk_body(body: &[u8]) -> Vec<Vec<u8>> {
    chunk_body_with_limit(body, CHUNK_PAYLOAD)
}

/// Effective per-chunk payload ceiling for a negotiated ATT MTU.
pub fn payload_ceiling(mtu: u16) -> usize {
    let mtu = mtu.max(23) as usize;
    CHUNK_PAYLOAD.min(mtu.saturating_sub(1))
}

/// Wrap a job body as concatenated BLE TX frames: `[header][chunk…]`.
pub fn frame_job_with_mtu(body: &[u8], mtu: u16) -> Vec<u8> {
    let header = build_header(body.len() as u32);
    let chunks = chunk_body_with_limit(body, payload_ceiling(mtu));
    let mut out = Vec::with_capacity(header.len() + chunks.iter().map(Vec::len).sum::<usize>());
    out.extend_from_slice(&header);
    for c in chunks {
        out.extend_from_slice(&c);
    }
    out
}

/// Wrap a job body as concatenated BLE TX frames at the protocol max chunk size.
pub fn frame_job(body: &[u8]) -> Vec<u8> {
    frame_job_with_mtu(body, 247)
}

/// Split a framed job (output of [`frame_job`] / [`Driver::encode`]) into BLE
/// write payloads. Chunk size is inferred from the framed layout.
pub fn tx_frames(data: &[u8]) -> Vec<&[u8]> {
    tx_frames_with_limit(data, CHUNK_PAYLOAD)
}

/// Like [`tx_frames`], but with an explicit payload ceiling (e.g. from MTU).
pub fn tx_frames_with_limit(data: &[u8], max_payload: usize) -> Vec<&[u8]> {
    let max_payload = max_payload.clamp(1, CHUNK_PAYLOAD);
    if data.len() < 9 || data[0] != 0xff || data[1] != 0xf0 {
        return if data.is_empty() {
            Vec::new()
        } else {
            vec![data]
        };
    }
    let body_len = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let mut frames = vec![&data[..9]];
    let mut offset = 9;
    let mut remaining = body_len;
    let mut i = 0;
    while remaining > 0 || (body_len == 0 && i == 0) {
        let take = remaining.min(max_payload);
        let is_last = take == remaining;
        let wire_len = 1 + take + if is_last { 2 } else { 0 };
        if offset + wire_len > data.len() {
            break;
        }
        frames.push(&data[offset..offset + wire_len]);
        offset += wire_len;
        remaining = remaining.saturating_sub(take);
        i += 1;
        if body_len == 0 {
            break;
        }
    }
    frames
}

/// Stand-alone set-cassette payload for the short-command characteristic.
pub fn set_cassette_payload(cassette_id: u8) -> Vec<u8> {
    let mut body = Vec::with_capacity(14);
    push_esc(&mut body, b's');
    body.extend_from_slice(&JOB_ID);
    push_esc(&mut body, b'M');
    body.push(cassette_id);
    body.extend_from_slice(&[0, 0, 0]);
    push_esc(&mut body, b'Q');
    let header = build_header(body.len() as u32);
    let mut out = Vec::with_capacity(9 + body.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(&body);
    out
}

/// Parsed `ESC R` print-result notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrintResult {
    pub code: u8,
}

impl PrintResult {
    pub fn success(self) -> bool {
        matches!(self.code, 0 | 1 | 3)
    }
}

/// Parse a notify payload for `1B 52 <code>` (may be embedded in a longer buffer).
pub fn parse_result(bytes: &[u8]) -> Option<PrintResult> {
    bytes.windows(3).find_map(|w| {
        if w[0] == ESC && w[1] == b'R' {
            Some(PrintResult { code: w[2] })
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::job::{CutMode, JobSpec};
    use lbl_core::media::Media;
    use lbl_core::printer::DeviceCapabilities;
    use lbl_core::units::Dpi;
    use lbl_driver_api::EncodeContext;

    fn caps_cut() -> DeviceCapabilities {
        DeviceCapabilities {
            supports_cut: true,
            dpi: Dpi(203.0),
            max_width_mm: 12.0,
            ..Default::default()
        }
    }

    #[test]
    fn header_checksum_known_length() {
        let h = build_header(20);
        assert_eq!(&h[..4], &[0xff, 0xf0, 0x12, 0x34]);
        assert_eq!(&h[4..8], &20u32.to_le_bytes());
        let sum = h[..8].iter().fold(0u16, |a, &b| a + b as u16) as u8;
        assert_eq!(h[8], sum);
    }

    #[test]
    fn column_packing_single_pixels() {
        let mut rows = [false; 32];
        rows[0] = true;
        assert_eq!(pack_column(&rows), [0x00, 0x00, 0x00, 0x80]);

        let mut rows = [false; 32];
        rows[7] = true;
        assert_eq!(pack_column(&rows), [0x00, 0x00, 0x00, 0x01]);

        let mut rows = [false; 32];
        rows[24] = true;
        assert_eq!(pack_column(&rows), [0x80, 0x00, 0x00, 0x00]);

        let mut rows = [false; 32];
        rows[31] = true;
        assert_eq!(pack_column(&rows), [0x01, 0x00, 0x00, 0x00]);

        assert_eq!(pack_column(&[true; 32]), [0xff, 0xff, 0xff, 0xff]);
        assert_eq!(pack_column(&[false; 32]), [0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn chunk_index_skips_27() {
        assert_eq!(chunk_index(0), 0);
        assert_eq!(chunk_index(26), 26);
        assert_eq!(chunk_index(27), 28);
        assert_eq!(chunk_index(28), 29);
    }

    #[test]
    fn chunk_body_appends_magic_on_last() {
        let body = vec![0u8; 10];
        let chunks = chunk_body(&body);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0][0], 0);
        assert_eq!(&chunks[0][1..11], &body[..]);
        assert_eq!(&chunks[0][11..], &MAGIC);
    }

    #[test]
    fn chunk_body_splits_at_500() {
        let body = vec![0xab; 501];
        let chunks = chunk_body(&body);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 1 + 500);
        assert_eq!(chunks[0][0], 0);
        assert_eq!(chunks[1][0], 1);
        assert_eq!(&chunks[1][chunks[1].len() - 2..], &MAGIC);
    }

    #[test]
    fn avatar_job_sequence_and_tx_frames() {
        let mut bmp = MonoBitmap::new(1, 1);
        bmp.set(0, 0, true);
        let mut job = JobSpec::new(Media::continuous(12.0, Dpi(203.0)));
        job.cut_mode = CutMode::End;
        let caps = caps_cut();
        let ctx = EncodeContext::new(&job, &caps);
        let driver = LetraTagDriver {
            column_scale: 1,
            min_feed_columns: 1,
            ..Default::default()
        };
        let bytes = driver.encode(&bmp, &ctx).unwrap();

        assert_eq!(&bytes[..4], &[0xff, 0xf0, 0x12, 0x34]);
        let body_len = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let frames = tx_frames(&bytes);
        assert!(frames.len() >= 2);
        assert_eq!(frames[0].len(), 9);

        let body_start = 9 + 1; // skip first chunk index
        let body = &bytes[body_start..body_start + body_len];
        assert_eq!(&body[..6], &[0x1b, b's', 0x9a, 0x02, 0x00, 0x00]);
        assert_eq!(&body[6..9], &[0x1b, b'#', 0x01]);
        assert_eq!(&body[9..11], &[0x1b, b'D']);
        assert_eq!(body[11], 0x81);
        assert_eq!(body[12], 0x02);
        assert_eq!(&body[13..17], &1u32.to_le_bytes());
        assert_eq!(&body[17..21], &32u32.to_le_bytes());
        // top = (32-1)/2 = 15 → ink at head row 15
        let pixels = &body[21..25];
        let mut expect_rows = [false; 32];
        expect_rows[15] = true;
        assert_eq!(pixels, &pack_column(&expect_rows));
        assert_eq!(&body[25..], &[0x1b, b'p', b'0', 0x1b, b'A', 0x1b, b'Q']);
    }

    #[test]
    fn parse_result_codes() {
        assert_eq!(
            parse_result(&[0x1b, b'R', 0x00]),
            Some(PrintResult { code: 0 })
        );
        assert!(parse_result(&[0x1b, b'R', 0x00]).unwrap().success());
        assert!(!parse_result(&[0x00, 0x1b, b'R', 0x06]).unwrap().success());
        assert_eq!(parse_result(&[0x00, 0x01]), None);
    }

    #[test]
    fn set_cassette_is_23_bytes() {
        let p = set_cassette_payload(CASSETTE_12MM);
        assert_eq!(p.len(), 23);
        assert_eq!(&p[..4], &[0xff, 0xf0, 0x12, 0x34]);
    }

    #[test]
    fn honor_no_cut() {
        let bmp = MonoBitmap::new(1, 1);
        let mut job = JobSpec::new(Media::continuous(12.0, Dpi(203.0)));
        job.cut_mode = CutMode::None;
        let caps = caps_cut();
        let ctx = EncodeContext::new(&job, &caps);
        let driver = LetraTagDriver {
            column_scale: 1,
            min_feed_columns: 1,
            ..Default::default()
        };
        let bytes = driver.encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(3).any(|w| w == [0x1b, b'p', b'1']));
    }
}

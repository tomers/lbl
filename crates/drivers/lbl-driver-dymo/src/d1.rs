//! Pure DYMO LabelManager (D1) stream helpers for bidirectional delivery.
//!
//! The LabelManager USB stream is a mix of `ESC`-prefixed commands and
//! `SYN`-prefixed raster columns (see [`crate`] module docs). A host that pages
//! the stream over a bulk endpoint has to:
//!
//! - split the job on `SYN` (raster-column) boundaries so each bulk write
//!   carries a bounded number of columns ([`split_chunks`]),
//! - count the `ESC A` status queries a chunk contains so it drains exactly one
//!   bulk-IN reply per query ([`count_status_queries`]), and
//! - normalize job trailers to cut-then-status order so a drained `ESC A` reply
//!   means the chassis finished the cut/feed ([`normalize_job_trailers`]).
//!
//! These functions are transport-agnostic and operate on owned/borrowed byte
//! slices, so both native USB senders and WebUSB clients share one
//! implementation. Parsing skips `SYN` payloads: a raster byte that happens to
//! equal `0x1B`/`0x41` must not be mistaken for a command.

use crate::{ESC, SYN};

/// Bare `ESC A` status query.
pub const STATUS_REQUEST: [u8; 2] = [ESC, b'A'];

/// Max raster columns per USB chunk, matching a 64-byte bulk `wMaxPacketSize`.
pub const SYN_CHUNK_MAX: usize = 64;

/// Bulk-IN drain length for a one-byte D1 status reply.
///
/// The semantic reply is a single byte, but bulk IN `wMaxPacketSize` is 64 and
/// WebUSB rejects (`babble`) transfers longer than requested, so hosts read a
/// full max-packet block.
pub const STATUS_READ_LEN: usize = 64;

/// Split `payload` into chunks with at most `max_syn` `SYN` opcodes each.
///
/// Chunks are cut immediately before the `SYN` that would exceed the budget so
/// no raster column is split across a bulk write. A short tail (fewer than
/// `max_syn` SYNs) is returned intact. Empty chunks are dropped.
pub fn split_chunks(payload: &[u8], max_syn: usize) -> Vec<&[u8]> {
    let mut chunks = Vec::new();
    let mut pos = 0usize;
    while pos < payload.len() {
        let mut syn_count = 0usize;
        let mut cut: Option<usize> = None;
        for (i, &b) in payload.iter().enumerate().skip(pos) {
            if b == SYN {
                syn_count += 1;
                cut = Some(i);
                if syn_count >= max_syn {
                    break;
                }
            }
        }
        match cut {
            Some(cut) if syn_count >= max_syn => {
                if cut > pos {
                    chunks.push(&payload[pos..cut]);
                }
                pos = cut;
            }
            _ => {
                if pos < payload.len() {
                    chunks.push(&payload[pos..]);
                }
                break;
            }
        }
    }
    chunks.retain(|c| !c.is_empty());
    chunks
}

/// Walk a D1 command stream, invoking `on_esc(esc_index, cmd)` at each `ESC`.
///
/// `SYN` raster payloads are skipped using the running bytes-per-line set by the
/// most recent `ESC D`, so bitmap bytes are never interpreted as commands.
fn walk_commands(data: &[u8], mut on_esc: impl FnMut(usize, u8)) {
    let mut pos = 0usize;
    let mut bytes_per_line = 0usize;
    while pos < data.len() {
        let b = data[pos];
        if b == ESC {
            if pos + 1 >= data.len() {
                break;
            }
            let cmd = data[pos + 1];
            on_esc(pos, cmd);
            pos += 2;
            if matches!(cmd, b'B' | b'C' | b'D') {
                if pos >= data.len() {
                    break;
                }
                if cmd == b'D' {
                    bytes_per_line = data[pos] as usize;
                }
                pos += 1;
            }
            continue;
        }
        if b == SYN {
            pos += 1 + bytes_per_line;
            continue;
        }
        pos += 1;
    }
}

/// Count `ESC A` status opcodes in a D1 stream (one per job trailer / copy).
pub fn count_status_queries(data: &[u8]) -> usize {
    let mut count = 0usize;
    walk_commands(data, |_, cmd| {
        if cmd == b'A' {
            count += 1;
        }
    });
    count
}

/// Normalize job trailers to cut-then-status (`ESC E` `ESC A`) order.
///
/// Older encoders emitted status-then-cut (`ESC A` `ESC E`); a status reply
/// drained before the cut/feed lets the next job's pacing poll race a busy
/// chassis. Swapping in place makes a drained `ESC A` mean the cut is done.
/// Idempotent once already cut-then-status.
pub fn normalize_job_trailers(payload: &[u8]) -> Vec<u8> {
    let mut out = payload.to_vec();
    let mut esc_starts = Vec::new();
    walk_commands(&out, |i, _| esc_starts.push(i));
    for w in esc_starts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b != a + 2 {
            continue;
        }
        if out[a + 1] == b'A' && out[b + 1] == b'E' {
            out[a + 1] = b'E';
            out[b + 1] = b'A';
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_syn_budget_and_keeps_short_tail() {
        let mut payload = vec![ESC, b'C', 0, ESC, b'D', 1];
        for _ in 0..5 {
            payload.extend_from_slice(&[SYN, 0x00]);
        }
        payload.extend_from_slice(&[ESC, b'E', ESC, b'A']);

        let chunks = split_chunks(&payload, 3);
        assert_eq!(chunks.len(), 3);
        assert_eq!(
            chunks[0].iter().filter(|&&b| b == SYN).count(),
            2,
            "first cut is before the 3rd SYN"
        );
        let last = chunks[chunks.len() - 1];
        assert_eq!(&last[last.len() - 4..], &[ESC, b'E', ESC, b'A']);
    }

    #[test]
    fn counts_trailer_status_and_ignores_syn_payload() {
        let payload = [
            ESC, b'C', 0, ESC, b'D', 2, SYN, ESC, b'A', ESC, b'E', ESC, b'A',
        ];
        // The `ESC A` inside the 2-byte SYN payload is raster, not a command.
        assert_eq!(count_status_queries(&payload), 1);
    }

    #[test]
    fn counts_one_status_per_copy_trailer() {
        let one_copy = [ESC, b'C', 0, ESC, b'D', 1, SYN, 0x00, ESC, b'E', ESC, b'A'];
        let mut two = one_copy.to_vec();
        two.extend_from_slice(&one_copy);
        assert_eq!(count_status_queries(&one_copy), 1);
        assert_eq!(count_status_queries(&two), 2);
    }

    #[test]
    fn normalize_swaps_legacy_status_then_cut() {
        let legacy = [ESC, b'C', 0, ESC, b'D', 1, SYN, 0x00, ESC, b'A', ESC, b'E'];
        let normalized = normalize_job_trailers(&legacy);
        assert_eq!(&normalized[normalized.len() - 4..], &[ESC, b'E', ESC, b'A']);
        // Idempotent once already cut-then-status.
        assert_eq!(normalize_job_trailers(&normalized), normalized);
    }
}

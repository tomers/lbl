//! TIFF PackBits (Apple) run-length encoding for Brother raster rows.
//!
//! Used by QL/PT after `M 02`. If the compressed form is not shorter than the
//! raw row, callers should fall back to an uncompressed transfer for that row.

/// Whether every byte of `row` is zero (eligible for Brother `Z` blank opcode).
pub fn is_blank_row(row: &[u8]) -> bool {
    row.iter().all(|&b| b == 0)
}

/// Encode `row` with TIFF PackBits.
///
/// Returns `None` when the compressed payload is not strictly shorter than
/// `row` (or when `row` is empty). Header semantics:
/// - `0..=127`: next `n + 1` bytes are literal
/// - `-127..=-1` (as `u8`): next byte repeats `1 - n` times
/// - `-128` is unused (no-op)
pub fn compress(row: &[u8]) -> Option<Vec<u8>> {
    if row.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(row.len());
    let mut i = 0usize;
    while i < row.len() {
        // Prefer a repeat run of length ≥ 3 (or any leftover length ≥ 2 at end).
        let mut run = 1usize;
        while i + run < row.len() && row[i + run] == row[i] && run < 128 {
            run += 1;
        }
        if run >= 3 || (run >= 2 && i + run == row.len() && literal_lookahead_is_short(row, i)) {
            let n = run as i8;
            out.push((1i8.wrapping_sub(n)) as u8);
            out.push(row[i]);
            i += run;
            continue;
        }
        // Literal run: advance until a profitable repeat starts or max length.
        let start = i;
        i += 1;
        while i < row.len() && (i - start) < 128 {
            let mut peek = 1usize;
            while i + peek < row.len() && row[i + peek] == row[i] && peek < 128 {
                peek += 1;
            }
            if peek >= 3 {
                break;
            }
            i += 1;
        }
        let lit = &row[start..i];
        out.push((lit.len() - 1) as u8);
        out.extend_from_slice(lit);
    }
    if out.len() < row.len() {
        Some(out)
    } else {
        None
    }
}

fn literal_lookahead_is_short(_row: &[u8], _i: usize) -> bool {
    // Prefer emitting a short final repeat rather than a 2-byte literal when
    // compress() would otherwise leave a dangling pair at EOF.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_detection() {
        assert!(is_blank_row(&[0, 0, 0]));
        assert!(!is_blank_row(&[0, 1, 0]));
    }

    #[test]
    fn compresses_long_zero_run() {
        let row = vec![0u8; 70];
        let c = compress(&row).expect("zeros should compress");
        assert!(c.len() < 70);
    }

    #[test]
    fn falls_back_when_not_shorter() {
        // High-entropy 8 bytes rarely shrink under PackBits.
        let row = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF];
        assert!(compress(&row).is_none());
    }
}

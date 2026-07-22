//! Zebra ZPL `~HS` (host status) query.
//!
//! `~HS` asks the printer to return three STX/ETX-framed lines that summarise
//! its current state. The exact byte positions documented here match the Zebra
//! *ZPL II Programming Guide* (P1012728-008 or later), section on `~HS`.
//!
//! ## Response format
//!
//! The printer replies with three lines, each enclosed in `STX … ETX`:
//!
//! ```text
//! Line 1: <STX>aaa,b,c,dddd,eee,f,g,h,iii,j,k,l<ETX>
//! Line 2: <STX>mmm,n,o,ppp<ETX>
//! Line 3: <STX>qqq<ETX>
//! ```
//!
//! Field positions referenced in this module (0-indexed within the
//! comma-separated fields of line 1):
//!
//! | Index | Meaning |
//! |-------|---------|
//! | 0 | `aaa` — interface (3 ASCII digits, ignore) |
//! | 1 | `b` — paper-out / label-not-loaded (`1` = out) |
//! | 2 | `c` — pause (`1` = paused) |
//! | 3 | `dddd` — label length (dots, 4 digits) |
//! | 4 | `eee` — number of formats in receive buffer |
//! | 5 | `f` — buffer full flag |
//! | 6 | `g` — communication diagnostics (`1` = active) |
//! | 7 | `h` — partial format flag |
//! | 8 | `iii` — unused (reserved) |
//! | 9 | `j` — corrupt RAM (1 = yes) |
//! | 10 | `k` — temperature range fault (`1` = fault) |
//! | 11 | `l` — head open / ribbon-out (`1` = open/out) |
//!
//! **Uncertainty note**: specific firmware versions may omit trailing fields or
//! use slightly different field positions. The parser is therefore defensive:
//! missing fields are treated as `0`.

use crate::StatusError;

/// `~HS` host-status query command bytes.
pub const HOST_STATUS_CMD: &[u8] = b"~HS\r\n";

const STX: u8 = 0x02;
const ETX: u8 = 0x03;

/// Parsed subset of the `~HS` host-status reply.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ZplHostStatus {
    /// Printer reports paper / label stock is out.
    pub paper_out: bool,
    /// Printer is paused.
    pub pause: bool,
    /// Print head is open or ribbon is out.
    pub head_open: bool,
    /// Corrupt RAM detected.
    pub corrupt_ram: bool,
    /// Raw STX/ETX-stripped lines from the response (up to 3).
    pub raw_lines: Vec<String>,
}

/// Extract STX/ETX-framed segments from `buf`, returning them in order.
fn extract_framed_lines(buf: &[u8]) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
    let mut i = 0;
    while i < buf.len() {
        if buf[i] == STX {
            let start = i + 1;
            if let Some(end) = buf[start..].iter().position(|&b| b == ETX) {
                lines.push(buf[start..start + end].to_vec());
                i = start + end + 1;
            } else {
                break;
            }
        } else {
            i += 1;
        }
    }
    lines
}

fn field(fields: &[&str], idx: usize) -> u8 {
    fields
        .get(idx)
        .and_then(|s| s.trim().parse::<u8>().ok())
        .unwrap_or(0)
}

/// Parse a `~HS` response buffer into a [`ZplHostStatus`].
pub fn parse_host_status(buf: &[u8]) -> Result<ZplHostStatus, StatusError> {
    let framed = extract_framed_lines(buf);
    if framed.is_empty() {
        return Err(StatusError::Parse(
            "no STX/ETX-framed lines in ~HS response".into(),
        ));
    }

    let raw_lines: Vec<String> = framed
        .iter()
        .map(|l| String::from_utf8_lossy(l).into_owned())
        .collect();

    let line1 = String::from_utf8_lossy(&framed[0]);
    let fields: Vec<&str> = line1.split(',').collect();

    Ok(ZplHostStatus {
        paper_out: field(&fields, 1) != 0,
        pause: field(&fields, 2) != 0,
        head_open: field(&fields, 11) != 0,
        corrupt_ram: field(&fields, 9) != 0,
        raw_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(content: &[u8]) -> Vec<u8> {
        let mut v = vec![STX];
        v.extend_from_slice(content);
        v.push(ETX);
        v
    }

    fn sample_response() -> Vec<u8> {
        // Line 1: interface=000, paper_out=0, pause=0, len=0203, bufs=000,
        //         buf_full=0, diag=0, partial=0, rsvd=000, corrupt=0, temp=0, head_open=0
        let mut buf = framed(b"000,0,0,0203,000,0,0,0,000,0,0,0");
        buf.extend_from_slice(&framed(b"000,0,0,000"));
        buf.extend_from_slice(&framed(b"000"));
        buf
    }

    fn sample_response_error() -> Vec<u8> {
        // paper_out=1 (field 1), pause=1 (field 2), head_open=1 (field 11)
        framed(b"000,1,1,0203,000,0,0,0,000,0,0,1")
    }

    #[test]
    fn parses_healthy_printer() {
        let buf = sample_response();
        let status = parse_host_status(&buf).unwrap();
        assert!(!status.paper_out);
        assert!(!status.pause);
        assert!(!status.head_open);
        assert!(!status.corrupt_ram);
        assert_eq!(status.raw_lines.len(), 3);
    }

    #[test]
    fn parses_error_flags() {
        let buf = sample_response_error();
        let status = parse_host_status(&buf).unwrap();
        assert!(status.paper_out);
        assert!(status.pause);
        assert!(status.head_open);
    }

    #[test]
    fn empty_response_is_an_error() {
        assert!(parse_host_status(&[]).is_err());
        assert!(parse_host_status(b"no framing here").is_err());
    }

    #[test]
    fn tolerates_missing_trailing_fields() {
        let buf = framed(b"000,1,0");
        let status = parse_host_status(&buf).unwrap();
        assert!(status.paper_out);
        assert!(!status.head_open);
    }
}

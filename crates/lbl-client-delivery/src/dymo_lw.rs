//! DYMO LabelWriter 550-series (LW5) status-paced delivery.
//!
//! The LW5 protocol is bidirectional: the host acquires a print-engine lock,
//! sends the job preamble, then for each label writes the label segment and
//! drains a 32-byte `ESC A` status reply (an inter-label lock between labels, a
//! release after the last), and finally writes the trailer. Streaming the whole
//! job in one transfer wedges the firmware.
//!
//! Request builders and reply parsing live in [`lbl_status::dymo_lw`]; this
//! module owns the job segmentation and the per-label handshake sequencing,
//! mirroring the native `lbl-device` LW5 USB session.

use lbl_status::dymo_lw::{
    parse_print_status, status_request, Lw550PrintStatus, LOCK_ACQUIRE, LOCK_INTER_LABEL,
    LOCK_RELEASE, STATUS_REPLY_LEN,
};
use lbl_status::PrintStatus;

use crate::{DeliveryAction, DeliveryError, Event, Handshake};

const ESC: u8 = 0x1B;
/// Print-path status reads may wait through a label feed; longer than idle.
const PRINT_IO_TIMEOUT_MS: u32 = 30_000;

/// A parsed LW5 job split into its independently-written segments.
struct ParsedJob {
    preamble: Vec<u8>,
    labels: Vec<Vec<u8>>,
    finalize: Vec<u8>,
}

fn malformed(message: impl Into<String>) -> DeliveryError {
    DeliveryError::Malformed {
        protocol: "dymo_lw",
        message: message.into(),
    }
}

fn require_esc(payload: &[u8], pos: &mut usize, cmd: u8) -> Result<(), DeliveryError> {
    if *pos + 1 >= payload.len() || payload[*pos] != ESC || payload[*pos + 1] != cmd {
        return Err(malformed(format!(
            "expected ESC {} at offset {pos}",
            cmd as char
        )));
    }
    *pos += 2;
    Ok(())
}

fn read_u32_le(payload: &[u8], pos: usize) -> Result<u32, DeliveryError> {
    let end = pos
        .checked_add(4)
        .filter(|&e| e <= payload.len())
        .ok_or_else(|| malformed("field truncated"))?;
    Ok(u32::from_le_bytes(payload[pos..end].try_into().unwrap()))
}

fn skip_job_header(payload: &[u8], pos: &mut usize) -> Result<(), DeliveryError> {
    while *pos + 1 < payload.len() && payload[*pos] == ESC {
        match payload[*pos + 1] {
            b'L' => *pos += 6,
            b'h' | b'i' | b'e' => *pos += 2,
            b'T' | b'C' => *pos += 3,
            b'n' | b'D' => return Ok(()),
            b'Q' => return Err(malformed("job missing label data")),
            other => {
                return Err(malformed(format!(
                    "unexpected header command ESC {other:#04x}"
                )))
            }
        }
    }
    Ok(())
}

fn skip_label_data(payload: &[u8], pos: &mut usize) -> Result<(), DeliveryError> {
    require_esc(payload, pos, b'D')?;
    *pos += 2; // bpp + align
    let width = read_u32_le(payload, *pos)?;
    *pos += 4;
    let height = read_u32_le(payload, *pos)?;
    *pos += 4;
    let data_len = width
        .checked_mul(height.div_ceil(8))
        .ok_or_else(|| malformed("label data length overflow"))? as usize;
    let end = pos
        .checked_add(data_len)
        .filter(|&e| e <= payload.len())
        .ok_or_else(|| malformed("label data truncated"))?;
    *pos = end;
    Ok(())
}

fn parse_job(payload: &[u8]) -> Result<ParsedJob, DeliveryError> {
    let mut pos = 0usize;
    require_esc(payload, &mut pos, b's')?;
    pos += 4; // job id
    skip_job_header(payload, &mut pos)?;
    let preamble_end = pos;

    let mut labels = Vec::new();
    while pos + 1 < payload.len() {
        if payload[pos] == ESC && payload[pos + 1] == b'E' {
            break;
        }
        if payload[pos] == ESC && payload[pos + 1] == b'Q' {
            return Err(malformed("job missing ESC E before ESC Q"));
        }
        let label_start = pos;
        require_esc(payload, &mut pos, b'n')?;
        pos += 2; // label index
        skip_label_data(payload, &mut pos)?;
        require_esc(payload, &mut pos, b'G')?;
        labels.push(payload[label_start..pos].to_vec());
    }

    if labels.is_empty() {
        return Err(malformed("job missing label data"));
    }
    require_esc(payload, &mut pos, b'E')?;
    require_esc(payload, &mut pos, b'Q')?;
    if pos != payload.len() {
        return Err(malformed(format!(
            "{} trailing bytes after ESC Q",
            payload.len() - pos
        )));
    }

    Ok(ParsedJob {
        preamble: payload[..preamble_end].to_vec(),
        labels,
        finalize: payload[payload.len() - 4..].to_vec(),
    })
}

/// Interpret a status reply, rejecting no-lock / error / bad-media conditions.
fn interpret(bytes: &[u8], phase: &str) -> Result<Lw550PrintStatus, String> {
    let status = parse_print_status(bytes).map_err(|e| format!("{phase}: {e}"))?;
    match status.print_status_code {
        5 => {
            return Err(format!(
                "{phase}: printer did not grant the print lock (another host may be using it)"
            ))
        }
        2 => return Err(format!("{phase}: printer reported an error")),
        3 => return Err(format!("{phase}: print job was cancelled")),
        _ => {}
    }
    match status.main_bay_status_code {
        10 => {
            return Err(
                "printer rejected the loaded media (NFC reports non-genuine labels); \
                 LabelWriter 550 requires authentic DYMO rolls"
                    .into(),
            )
        }
        2 => return Err(format!("{phase}: no media loaded in the printer")),
        5..=7 => {
            return Err(format!(
                "{phase}: media roll is empty or nearly empty (bay status {})",
                status.main_bay_status_code
            ))
        }
        9 => return Err(format!("{phase}: media jam reported by printer")),
        _ => {}
    }
    Ok(status)
}

fn status_action(status: &Lw550PrintStatus) -> DeliveryAction {
    DeliveryAction::status(PrintStatus::DymoLw(status.to_view()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    LockSent,
    LockRecv,
    PreambleSent,
    LabelSent,
    HandshakeSent,
    HandshakeRecv,
    FinalizeSent,
}

pub(crate) struct DymoLw {
    job: ParsedJob,
    label_idx: usize,
    phase: Phase,
}

impl DymoLw {
    pub(crate) fn new(label_bytes: &[u8]) -> Result<Self, DeliveryError> {
        Ok(Self {
            job: parse_job(label_bytes)?,
            label_idx: 0,
            phase: Phase::LockSent,
        })
    }

    fn label_count(&self) -> u32 {
        self.job.labels.len() as u32
    }

    /// Emit the next label write (with a progress note), or the finalize write
    /// when all labels are done. `lead` is prepended to the returned batch
    /// (e.g. a status notification carried over from the prior handshake).
    fn send_current_label(&mut self, mut lead: Vec<DeliveryAction>) -> Vec<DeliveryAction> {
        let total = self.label_count();
        let num = self.label_idx as u32 + 1;
        self.phase = Phase::LabelSent;
        lead.push(DeliveryAction::label_progress(
            "sending_label",
            format!("Sending label {num} of {total}…"),
            self.label_idx as u32,
            total,
        ));
        lead.push(DeliveryAction::send(
            self.job.labels[self.label_idx].clone(),
        ));
        lead
    }
}

impl Handshake for DymoLw {
    fn start(&mut self) -> Vec<DeliveryAction> {
        self.phase = Phase::LockSent;
        vec![
            DeliveryAction::progress("acquiring_lock", "Acquiring print lock…"),
            DeliveryAction::send(status_request(LOCK_ACQUIRE).to_vec()),
        ]
    }

    fn advance(&mut self, event: Event) -> Vec<DeliveryAction> {
        match (self.phase, event) {
            (Phase::LockSent, Event::SendComplete) => {
                self.phase = Phase::LockRecv;
                vec![DeliveryAction::recv(STATUS_REPLY_LEN, PRINT_IO_TIMEOUT_MS)]
            }
            (Phase::LockRecv, Event::Rx(bytes)) => {
                match interpret(&bytes, "acquiring print lock") {
                    Err(message) => vec![DeliveryAction::error(message)],
                    Ok(status) => {
                        let mut lead = vec![status_action(&status)];
                        if !self.job.preamble.is_empty() {
                            self.phase = Phase::PreambleSent;
                            lead.push(DeliveryAction::progress(
                                "sending_preamble",
                                "Sending job header…",
                            ));
                            lead.push(DeliveryAction::send(self.job.preamble.clone()));
                            lead
                        } else {
                            self.send_current_label(lead)
                        }
                    }
                }
            }
            (Phase::PreambleSent, Event::SendComplete) => self.send_current_label(Vec::new()),
            (Phase::LabelSent, Event::SendComplete) => {
                self.phase = Phase::HandshakeSent;
                let last = self.label_idx as u32 + 1 == self.label_count();
                let lock = if last { LOCK_RELEASE } else { LOCK_INTER_LABEL };
                let total = self.label_count();
                let num = self.label_idx as u32 + 1;
                vec![
                    DeliveryAction::label_progress(
                        "handshake",
                        format!("Waiting for printer ({num}/{total})…"),
                        self.label_idx as u32,
                        total,
                    ),
                    DeliveryAction::send(status_request(lock).to_vec()),
                ]
            }
            (Phase::HandshakeSent, Event::SendComplete) => {
                self.phase = Phase::HandshakeRecv;
                vec![DeliveryAction::recv(STATUS_REPLY_LEN, PRINT_IO_TIMEOUT_MS)]
            }
            (Phase::HandshakeRecv, Event::Rx(bytes)) => {
                match interpret(&bytes, "label handshake") {
                    Err(message) => vec![DeliveryAction::error(message)],
                    Ok(status) => {
                        self.label_idx += 1;
                        let lead = vec![status_action(&status)];
                        if (self.label_idx as u32) < self.label_count() {
                            self.send_current_label(lead)
                        } else {
                            self.phase = Phase::FinalizeSent;
                            let mut lead = lead;
                            lead.push(DeliveryAction::send(self.job.finalize.clone()));
                            lead
                        }
                    }
                }
            }
            (Phase::FinalizeSent, Event::SendComplete) => {
                vec![
                    DeliveryAction::progress("finalizing", "Finalizing job…"),
                    DeliveryAction::Done,
                ]
            }
            _ => vec![DeliveryAction::error(
                "dymo-lw delivery received an out-of-order transport event",
            )],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientDeliverySession, ClientHandshake};

    /// One-label LW5 job: ESC s + ESC i preamble, one 8×1 label, ESC E ESC Q.
    fn single_label_job() -> Vec<u8> {
        let mut out = vec![
            ESC, b's', 1, 0, 0, 0, // job id
            ESC, b'i', // preamble
            ESC, b'n', 0, 0, // label index
            ESC, b'D', 1, 2, // bpp + align
        ];
        out.extend_from_slice(&1u32.to_le_bytes()); // width
        out.extend_from_slice(&8u32.to_le_bytes()); // height
        out.push(0x80); // one line of data
        out.extend_from_slice(&[ESC, b'G', ESC, b'E', ESC, b'Q']);
        out
    }

    fn ok_status() -> Vec<u8> {
        let mut s = vec![0u8; STATUS_REPLY_LEN];
        s[10] = 8; // media present — ok
        s
    }

    fn expect_status(action: &DeliveryAction) {
        assert!(
            matches!(action, DeliveryAction::Status { .. }),
            "{action:?}"
        );
    }

    #[test]
    fn parses_missing_label_data() {
        // ESC s + job id + ESC Q (no label) → malformed.
        let job = vec![ESC, b's', 0, 0, 0, 0, ESC, b'Q'];
        let err = ClientDeliverySession::start(ClientHandshake::DymoLw, None, &job).unwrap_err();
        assert!(matches!(err, DeliveryError::Malformed { .. }));
    }

    #[test]
    fn full_single_label_sequence() {
        let job = single_label_job();
        let parsed = parse_job(&job).unwrap();
        assert_eq!(parsed.labels.len(), 1);

        let (mut session, action) =
            ClientDeliverySession::start(ClientHandshake::DymoLw, None, &job).unwrap();

        // Progress → lock acquire write.
        assert!(matches!(action, DeliveryAction::Progress { .. }));
        let action = session.tick().unwrap();
        assert_eq!(
            action,
            DeliveryAction::send(status_request(LOCK_ACQUIRE).to_vec())
        );

        // Lock reply → status → preamble write.
        let action = session.on_send_complete().unwrap();
        assert_eq!(action, DeliveryAction::recv(STATUS_REPLY_LEN, 30_000));
        let action = session.feed_rx(&ok_status()).unwrap();
        expect_status(&action);
        let action = session.tick().unwrap(); // sending_preamble progress
        assert!(matches!(action, DeliveryAction::Progress { .. }));
        let action = session.tick().unwrap();
        assert_eq!(action, DeliveryAction::send(parsed.preamble.clone()));

        // Preamble sent → label progress → label write.
        let action = session.on_send_complete().unwrap();
        assert!(matches!(action, DeliveryAction::Progress { .. }));
        let action = session.tick().unwrap();
        assert_eq!(action, DeliveryAction::send(parsed.labels[0].clone()));

        // Label sent → handshake progress → release lock query.
        let action = session.on_send_complete().unwrap();
        assert!(matches!(action, DeliveryAction::Progress { .. }));
        let action = session.tick().unwrap();
        assert_eq!(
            action,
            DeliveryAction::send(status_request(LOCK_RELEASE).to_vec())
        );

        // Handshake reply → status → finalize write.
        let action = session.on_send_complete().unwrap();
        assert_eq!(action, DeliveryAction::recv(STATUS_REPLY_LEN, 30_000));
        let action = session.feed_rx(&ok_status()).unwrap();
        expect_status(&action);
        let action = session.tick().unwrap();
        assert_eq!(action, DeliveryAction::send(parsed.finalize.clone()));

        // Finalize sent → finalizing progress → done.
        let action = session.on_send_complete().unwrap();
        assert!(matches!(action, DeliveryAction::Progress { .. }));
        let action = session.tick().unwrap();
        assert_eq!(action, DeliveryAction::Done);
    }

    #[test]
    fn no_lock_reply_becomes_error() {
        let job = single_label_job();
        let (mut session, action) =
            ClientDeliverySession::start(ClientHandshake::DymoLw, None, &job).unwrap();
        // Advance to the lock Recv.
        assert!(matches!(action, DeliveryAction::Progress { .. }));
        session.tick().unwrap();
        session.on_send_complete().unwrap();
        // Status byte 5 = NoLock.
        let mut reply = vec![0u8; STATUS_REPLY_LEN];
        reply[0] = 5;
        let action = session.feed_rx(&reply).unwrap();
        match action {
            DeliveryAction::Error { message } => assert!(message.contains("lock")),
            other => panic!("expected Error, got {other:?}"),
        }
    }
}

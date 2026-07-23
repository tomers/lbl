//! DYMO LetraTag LT-200B notify-completion delivery.
//!
//! The framed job is written, then the GATT notify channel is polled for an
//! `ESC R <code>` print-result frame. As in the reference client, exhausting the
//! poll budget without a result is not treated as an error (the print may have
//! completed with the notify missed); the session finishes.
//!
//! Frame parsing comes from [`lbl_driver_letratag`].

use lbl_driver_letratag::parse_result;

use crate::{DeliveryAction, Event, Handshake};

/// Advisory per-poll notify read timeout (ms).
const NOTIFY_TIMEOUT_MS: u32 = 500;
/// Minimum useful notify length (`ESC R <code>` is 3 bytes).
const NOTIFY_MIN_LEN: usize = 3;
/// Iteration bound (~30 s at 500 ms/poll in the reference client).
const MAX_POLLS: u32 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    JobSent,
    AwaitingNotify,
}

pub(crate) struct LetraTag {
    bytes: Option<Vec<u8>>,
    phase: Phase,
    polls: u32,
}

impl LetraTag {
    pub(crate) fn new(label_bytes: &[u8]) -> Self {
        Self {
            bytes: Some(label_bytes.to_vec()),
            phase: Phase::JobSent,
            polls: 0,
        }
    }

    fn await_notify(&mut self) -> DeliveryAction {
        self.phase = Phase::AwaitingNotify;
        DeliveryAction::recv(NOTIFY_MIN_LEN, NOTIFY_TIMEOUT_MS)
    }
}

impl Handshake for LetraTag {
    fn start(&mut self) -> Vec<DeliveryAction> {
        self.phase = Phase::JobSent;
        let bytes = self.bytes.take().unwrap_or_default();
        vec![
            DeliveryAction::progress("sending"),
            DeliveryAction::send(bytes),
        ]
    }

    fn advance(&mut self, event: Event) -> Vec<DeliveryAction> {
        match (self.phase, event) {
            (Phase::JobSent, Event::SendComplete) => vec![self.await_notify()],
            (Phase::AwaitingNotify, Event::Rx(bytes)) => {
                self.polls += 1;
                if parse_result(&bytes).is_some() {
                    vec![DeliveryAction::progress("done"), DeliveryAction::Done]
                } else if self.polls >= MAX_POLLS {
                    vec![DeliveryAction::Done]
                } else {
                    vec![self.await_notify()]
                }
            }
            _ => vec![DeliveryAction::error(
                "letratag delivery received an out-of-order transport event",
            )],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientDeliverySession, ClientHandshake};

    #[test]
    fn sends_job_then_waits_for_esc_r() {
        let job = vec![0xff, 0xf0, 0x12, 0x34, 0, 0, 0, 0, 0];
        let (mut session, action) =
            ClientDeliverySession::start(ClientHandshake::LetraTagNotify, None, &job).unwrap();

        assert!(matches!(action, DeliveryAction::Progress { .. }));
        let action = session.tick().unwrap();
        assert_eq!(action, DeliveryAction::send(job.clone()));

        // First notify read: empty (timeout) → poll again.
        let action = session.on_send_complete().unwrap();
        assert_eq!(action, DeliveryAction::recv(3, 500));
        let action = session.feed_rx(&[]).unwrap();
        assert_eq!(action, DeliveryAction::recv(3, 500));

        // ESC R 0 (success) → done.
        let action = session.feed_rx(&[0x1b, b'R', 0x00]).unwrap();
        assert!(matches!(action, DeliveryAction::Progress { .. }));
        let action = session.tick().unwrap();
        assert_eq!(action, DeliveryAction::Done);
    }
}

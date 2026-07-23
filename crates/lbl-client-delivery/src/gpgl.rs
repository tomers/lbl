//! Graphtec / Silhouette GPGL cutter status-paced delivery.
//!
//! Mirrors the native GPGL paced send: initialize (`ESC D`), drain the reply,
//! poll `ESC E` status until ready, query firmware, poll ready again, write the
//! cut job, then poll ready until the cut/feed completes. `unloaded` status
//! aborts with an error; `moving`/unparsed replies keep polling up to a bound.
//!
//! Status bytes and parsing come from [`lbl_driver_gpgl`].

use lbl_driver_gpgl::{firmware_query, parse_status, GpglStatus, INIT_CMD, STATUS_QUERY};
use lbl_status::{GpglHostStatus, PrintStatus};

use crate::{DeliveryAction, Event, Handshake};

/// Bulk-IN drain length for a GPGL status packet.
const STATUS_READ_LEN: usize = 64;
/// Advisory status read timeout (ms).
const STATUS_TIMEOUT_MS: u32 = 6_000;
/// Nominal poll cadence used to bound ready-wait iterations.
const READY_POLL_MS: u32 = 100;
/// Ready-wait budget before the first cut (init + firmware phases).
const READY_TIMEOUT_MS: u32 = 30_000;
/// Ready-wait budget after the cut is written (cut + feed can be long).
const POST_CUT_READY_TIMEOUT_MS: u32 = 600_000;

/// What to do once the cutter reports `ready` from the current poll loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadyNext {
    QueryFirmware,
    SendCut,
    Finish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    InitSent,
    InitDrain,
    FirmwareSent,
    FirmwareDrain,
    ReadyQuerySent { next: ReadyNext },
    ReadyRecv { next: ReadyNext },
    CutSent,
}

pub(crate) struct Gpgl {
    cut_bytes: Vec<u8>,
    phase: Phase,
    polls: u32,
    ready_cap: u32,
}

impl Gpgl {
    pub(crate) fn new(label_bytes: &[u8]) -> Self {
        Self {
            cut_bytes: label_bytes.to_vec(),
            phase: Phase::InitSent,
            polls: 0,
            ready_cap: 0,
        }
    }

    /// Enter a ready-wait poll loop with a timeout-derived iteration bound.
    fn begin_ready_wait(&mut self, next: ReadyNext, timeout_ms: u32) -> DeliveryAction {
        self.polls = 0;
        self.ready_cap = timeout_ms / READY_POLL_MS;
        self.phase = Phase::ReadyQuerySent { next };
        DeliveryAction::send(STATUS_QUERY.to_vec())
    }

    /// Emit the first action for `next` once the cutter is ready.
    fn on_ready(&mut self, next: ReadyNext, lead: Vec<DeliveryAction>) -> Vec<DeliveryAction> {
        let mut lead = lead;
        match next {
            ReadyNext::QueryFirmware => {
                self.phase = Phase::FirmwareSent;
                lead.push(DeliveryAction::send(firmware_query()));
            }
            ReadyNext::SendCut => {
                self.phase = Phase::CutSent;
                lead.push(DeliveryAction::progress("cutting"));
                lead.push(DeliveryAction::send(self.cut_bytes.clone()));
            }
            ReadyNext::Finish => lead.push(DeliveryAction::Done),
        }
        lead
    }
}

fn status_note(status: GpglStatus) -> DeliveryAction {
    DeliveryAction::status(PrintStatus::Gpgl(GpglHostStatus::from(status).into()))
}

impl Handshake for Gpgl {
    fn start(&mut self) -> Vec<DeliveryAction> {
        self.phase = Phase::InitSent;
        vec![
            DeliveryAction::progress("init"),
            DeliveryAction::send(INIT_CMD.to_vec()),
        ]
    }

    fn advance(&mut self, event: Event) -> Vec<DeliveryAction> {
        match (self.phase, event) {
            (Phase::InitSent, Event::SendComplete) => {
                self.phase = Phase::InitDrain;
                vec![DeliveryAction::recv(STATUS_READ_LEN, STATUS_TIMEOUT_MS)]
            }
            // The init/firmware replies are soft drains; content is ignored.
            (Phase::InitDrain, Event::Rx(_)) => {
                vec![self.begin_ready_wait(ReadyNext::QueryFirmware, READY_TIMEOUT_MS)]
            }
            (Phase::FirmwareSent, Event::SendComplete) => {
                self.phase = Phase::FirmwareDrain;
                vec![DeliveryAction::recv(STATUS_READ_LEN, STATUS_TIMEOUT_MS)]
            }
            (Phase::FirmwareDrain, Event::Rx(_)) => {
                // 10 s ready wait before the cut in the reference client.
                vec![self.begin_ready_wait(ReadyNext::SendCut, 10_000)]
            }
            (Phase::ReadyQuerySent { next }, Event::SendComplete) => {
                self.phase = Phase::ReadyRecv { next };
                vec![DeliveryAction::recv(STATUS_READ_LEN, STATUS_TIMEOUT_MS)]
            }
            (Phase::ReadyRecv { next }, Event::Rx(bytes)) => {
                self.polls += 1;
                match parse_status(&bytes) {
                    Some(GpglStatus::Ready) => {
                        self.on_ready(next, vec![status_note(GpglStatus::Ready)])
                    }
                    Some(GpglStatus::Unloaded) => vec![
                        status_note(GpglStatus::Unloaded),
                        DeliveryAction::error("cutter reports media unloaded (status 2)"),
                    ],
                    Some(GpglStatus::Moving) => {
                        if self.polls >= self.ready_cap {
                            vec![
                                status_note(GpglStatus::Moving),
                                DeliveryAction::error("timed out waiting for cutter ready"),
                            ]
                        } else {
                            self.phase = Phase::ReadyQuerySent { next };
                            vec![
                                status_note(GpglStatus::Moving),
                                DeliveryAction::send(STATUS_QUERY.to_vec()),
                            ]
                        }
                    }
                    None => {
                        if self.polls >= self.ready_cap {
                            vec![DeliveryAction::error(
                                "timed out waiting for a recognizable GPGL status response",
                            )]
                        } else {
                            self.phase = Phase::ReadyQuerySent { next };
                            vec![DeliveryAction::send(STATUS_QUERY.to_vec())]
                        }
                    }
                }
            }
            (Phase::CutSent, Event::SendComplete) => {
                vec![self.begin_ready_wait(ReadyNext::Finish, POST_CUT_READY_TIMEOUT_MS)]
            }
            _ => vec![DeliveryAction::error(
                "gpgl delivery received an out-of-order transport event",
            )],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientDeliverySession, ClientHandshake};

    fn expect_send(action: DeliveryAction) -> Vec<u8> {
        match action {
            DeliveryAction::Send { bytes } => bytes,
            other => panic!("expected Send, got {other:?}"),
        }
    }

    /// Drive one ready-wait poll: query sent → recv → feed `reply`.
    fn poll_once(session: &mut ClientDeliverySession, reply: &[u8]) -> DeliveryAction {
        let action = session.on_send_complete().unwrap();
        assert_eq!(
            action,
            DeliveryAction::recv(STATUS_READ_LEN, STATUS_TIMEOUT_MS)
        );
        session.feed_rx(reply).unwrap()
    }

    #[test]
    fn full_cut_sequence_ready_paths() {
        let cut = firmware_query(); // arbitrary non-empty cut payload
        let (mut session, action) =
            ClientDeliverySession::start(ClientHandshake::Gpgl, None, &cut).unwrap();

        // init progress → ESC D.
        assert!(matches!(action, DeliveryAction::Progress { .. }));
        let action = session.tick().unwrap();
        assert_eq!(expect_send(action), INIT_CMD.to_vec());

        // Drain init reply → first ready query.
        let action = session.on_send_complete().unwrap();
        assert_eq!(
            action,
            DeliveryAction::recv(STATUS_READ_LEN, STATUS_TIMEOUT_MS)
        );
        let action = session.feed_rx(&[]).unwrap();
        assert_eq!(expect_send(action), STATUS_QUERY.to_vec());

        // Ready → firmware query.
        let action = poll_once(&mut session, b"0\x03");
        assert!(matches!(action, DeliveryAction::Status { .. }));
        let action = session.tick().unwrap();
        assert_eq!(expect_send(action), firmware_query());

        // Drain firmware reply → ready query (pre-cut).
        let action = session.on_send_complete().unwrap();
        assert_eq!(
            action,
            DeliveryAction::recv(STATUS_READ_LEN, STATUS_TIMEOUT_MS)
        );
        let action = session.feed_rx(&[]).unwrap();
        assert_eq!(expect_send(action), STATUS_QUERY.to_vec());

        // Moving once, then ready → cut job write.
        let action = poll_once(&mut session, b"1");
        assert!(matches!(action, DeliveryAction::Status { .. }));
        let action = session.tick().unwrap();
        assert_eq!(expect_send(action), STATUS_QUERY.to_vec());
        let action = poll_once(&mut session, b"0\x03");
        assert!(matches!(action, DeliveryAction::Status { .. }));
        let action = session.tick().unwrap(); // "cutting" progress
        assert!(matches!(action, DeliveryAction::Progress { .. }));
        let action = session.tick().unwrap();
        assert_eq!(expect_send(action), cut);

        // Post-cut ready wait → ready → done.
        let action = session.on_send_complete().unwrap();
        assert_eq!(expect_send(action), STATUS_QUERY.to_vec());
        let action = poll_once(&mut session, b"0\x03");
        assert!(matches!(action, DeliveryAction::Status { .. }));
        let action = session.tick().unwrap();
        assert_eq!(action, DeliveryAction::Done);
    }

    #[test]
    fn unloaded_media_aborts() {
        let cut = firmware_query();
        let (mut session, action) =
            ClientDeliverySession::start(ClientHandshake::Gpgl, None, &cut).unwrap();
        session.tick().unwrap(); // init progress → ESC D
        let _ = action;
        session.on_send_complete().unwrap(); // drain recv
        session.feed_rx(&[]).unwrap(); // → first ready query

        let action = poll_once(&mut session, b"2\x03");
        assert!(matches!(action, DeliveryAction::Status { .. }));
        let action = session.tick().unwrap();
        match action {
            DeliveryAction::Error { message } => assert!(message.contains("unloaded")),
            other => panic!("expected Error, got {other:?}"),
        }
    }
}

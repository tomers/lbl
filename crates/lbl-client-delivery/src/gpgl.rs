//! Graphtec / Silhouette GPGL cutter status-paced delivery.
//!
//! Initialize (`ESC D`), poll `ESC E` until ready, query firmware, poll ready
//! again, write the cut job in one bulk OUT, then poll until the cut/feed
//! completes. `unloaded` aborts; `moving`/unparsed replies keep polling.
//!
//! Init / firmware replies are intentionally **not** soft-drained. On WebUSB,
//! a timed-out bulk-IN cannot be cancelled (Chromium leaves an orphaned URB).
//! Soft-drain timeouts used to force `USBDevice.reset()` mid-handshake; Cameo
//! then keeps answering status while ignoring the cut payload. The WebUSB
//! transport now settles orphans without reset for GPGL — still avoid creating
//! those orphans here.
//!
//! After the cut payload is written, the cutter must report `moving` before
//! `ready`. Immediate `ready` or a streak of empty/unparsed status replies
//! means the job was ignored — fail fast.
//!
//! Status bytes and parsing come from [`lbl_driver_gpgl`].

use lbl_driver_gpgl::{firmware_query, parse_status, GpglStatus, INIT_CMD, STATUS_QUERY};
use lbl_status::{GpglHostStatus, PrintStatus};

use crate::{DeliveryAction, Event, Handshake};

/// Bulk-IN length for a GPGL status packet.
const STATUS_READ_LEN: usize = 64;
/// Per-poll status read timeout (ms). ESC E replies are tiny; keep this short
/// so empty WebUSB reads cannot wedge the UI for `ready_cap ×` long timeouts.
const STATUS_TIMEOUT_MS: u32 = 800;
/// Nominal poll cadence used to bound ready-wait iterations.
const READY_POLL_MS: u32 = 100;
/// Ready-wait budget before the first cut (init + firmware phases).
const READY_TIMEOUT_MS: u32 = 30_000;
/// Ready-wait budget after the cut once motion has been observed (cut + feed).
const POST_CUT_READY_TIMEOUT_MS: u32 = 600_000;
/// After cut, fail as idle if we have not seen `moving` within this many
/// status polls (~a few seconds at [`STATUS_TIMEOUT_MS`]).
const IDLE_DETECT_POLLS: u32 = 6;

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
    FirmwareSent,
    ReadyQuerySent { next: ReadyNext },
    ReadyRecv { next: ReadyNext },
    CutSent,
}

pub(crate) struct Gpgl {
    cut_bytes: Vec<u8>,
    phase: Phase,
    polls: u32,
    ready_cap: u32,
    /// True once post-cut polling has seen `moving` (or `paused`).
    saw_motion_after_cut: bool,
}

impl Gpgl {
    pub(crate) fn new(label_bytes: &[u8]) -> Self {
        Self {
            cut_bytes: label_bytes.to_vec(),
            phase: Phase::InitSent,
            polls: 0,
            ready_cap: 0,
            saw_motion_after_cut: false,
        }
    }

    /// Enter a ready-wait poll loop with a timeout-derived iteration bound.
    fn begin_ready_wait(&mut self, next: ReadyNext, timeout_ms: u32) -> DeliveryAction {
        self.polls = 0;
        self.ready_cap = timeout_ms / READY_POLL_MS;
        self.phase = Phase::ReadyQuerySent { next };
        DeliveryAction::send(STATUS_QUERY.to_vec())
    }

    fn idle_after_cut_error() -> Vec<DeliveryAction> {
        vec![DeliveryAction::error(
            "cutter stayed idle after cut payload (never reported moving)",
        )]
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
            (Phase::InitSent, Event::SendComplete) => vec![
                DeliveryAction::progress("handshake"),
                self.begin_ready_wait(ReadyNext::QueryFirmware, READY_TIMEOUT_MS),
            ],
            (Phase::FirmwareSent, Event::SendComplete) => vec![
                DeliveryAction::progress("handshake"),
                self.begin_ready_wait(ReadyNext::SendCut, 10_000),
            ],
            (Phase::ReadyQuerySent { next }, Event::SendComplete) => {
                self.phase = Phase::ReadyRecv { next };
                vec![DeliveryAction::recv(STATUS_READ_LEN, STATUS_TIMEOUT_MS)]
            }
            (Phase::ReadyRecv { next }, Event::Rx(bytes)) => {
                self.polls += 1;
                match parse_status(&bytes) {
                    Some(GpglStatus::Ready) => {
                        if next == ReadyNext::Finish && !self.saw_motion_after_cut {
                            let mut out = vec![status_note(GpglStatus::Ready)];
                            out.extend(Self::idle_after_cut_error());
                            return out;
                        }
                        self.on_ready(next, vec![status_note(GpglStatus::Ready)])
                    }
                    Some(GpglStatus::Unloaded) => vec![
                        status_note(GpglStatus::Unloaded),
                        DeliveryAction::error("cutter reports media unloaded (status 2)"),
                    ],
                    Some(GpglStatus::Cancelled) => vec![
                        status_note(GpglStatus::Cancelled),
                        DeliveryAction::error("cutter job was cancelled on the device (status 4)"),
                    ],
                    Some(status @ (GpglStatus::Moving | GpglStatus::Paused)) => {
                        if next == ReadyNext::Finish {
                            self.saw_motion_after_cut = true;
                        }
                        if self.polls >= self.ready_cap {
                            vec![
                                status_note(status),
                                DeliveryAction::error("timed out waiting for cutter ready"),
                            ]
                        } else {
                            self.phase = Phase::ReadyQuerySent { next };
                            vec![
                                status_note(status),
                                DeliveryAction::send(STATUS_QUERY.to_vec()),
                            ]
                        }
                    }
                    None => {
                        // Post-cut: empty/garbage replies without motion → idle, not a
                        // multi-minute poll storm (each empty read costs STATUS_TIMEOUT_MS).
                        if next == ReadyNext::Finish
                            && !self.saw_motion_after_cut
                            && self.polls >= IDLE_DETECT_POLLS
                        {
                            return Self::idle_after_cut_error();
                        }
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
                // Bound by IDLE_DETECT_POLLS until motion; then the Moving branch
                // keeps polling under POST_CUT_READY_TIMEOUT_MS via ready_cap.
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
    use lbl_driver_gpgl::{STATUS_REPLY_MOVING, STATUS_REPLY_READY, STATUS_REPLY_UNLOADED};

    fn expect_send(action: DeliveryAction) -> Vec<u8> {
        match action {
            DeliveryAction::Send { bytes } => bytes,
            other => panic!("expected Send, got {other:?}"),
        }
    }

    fn poll_once(session: &mut ClientDeliverySession, reply: &[u8]) -> DeliveryAction {
        let action = session.on_send_complete().unwrap();
        assert_eq!(
            action,
            DeliveryAction::recv(STATUS_READ_LEN, STATUS_TIMEOUT_MS)
        );
        session.feed_rx(reply).unwrap()
    }

    fn sample_cut() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(INIT_CMD);
        v.extend_from_slice(b"FN0\x03TG1\x03");
        v.extend_from_slice(b"M200,200\x03D200,500\x03");
        v
    }

    #[test]
    fn full_cut_sequence_ready_paths() {
        let cut = sample_cut();
        let (mut session, action) =
            ClientDeliverySession::start(ClientHandshake::Gpgl, None, &cut).unwrap();

        // init progress → ESC D.
        assert!(matches!(action, DeliveryAction::Progress { .. }));
        let action = session.tick().unwrap();
        assert_eq!(expect_send(action), INIT_CMD.to_vec());

        // No soft-drain: handshake progress → first ready query.
        let action = session.on_send_complete().unwrap();
        assert!(matches!(action, DeliveryAction::Progress { .. }));
        let action = session.tick().unwrap();
        assert_eq!(expect_send(action), STATUS_QUERY.to_vec());

        // Ready → firmware query.
        let action = poll_once(&mut session, STATUS_REPLY_READY);
        assert!(matches!(action, DeliveryAction::Status { .. }));
        let action = session.tick().unwrap();
        assert_eq!(expect_send(action), firmware_query());

        // After FG: handshake progress → ready query (pre-cut).
        let action = session.on_send_complete().unwrap();
        assert!(matches!(action, DeliveryAction::Progress { .. }));
        let action = session.tick().unwrap();
        assert_eq!(expect_send(action), STATUS_QUERY.to_vec());

        // Leftover FG ASCII is not a status — keep polling, then ready → cut.
        let action = poll_once(&mut session, b"CAMEO V1.10 \x03");
        assert_eq!(expect_send(action), STATUS_QUERY.to_vec());
        let action = poll_once(&mut session, STATUS_REPLY_READY);
        assert!(matches!(action, DeliveryAction::Status { .. }));
        let action = session.tick().unwrap(); // "cutting" progress
        assert!(matches!(action, DeliveryAction::Progress { .. }));
        let action = session.tick().unwrap();
        assert_eq!(expect_send(action), cut);

        // Post-cut: moving then ready → done.
        let action = session.on_send_complete().unwrap();
        assert_eq!(expect_send(action), STATUS_QUERY.to_vec());
        let action = poll_once(&mut session, STATUS_REPLY_MOVING);
        assert!(matches!(action, DeliveryAction::Status { .. }));
        let action = session.tick().unwrap();
        assert_eq!(expect_send(action), STATUS_QUERY.to_vec());
        let action = poll_once(&mut session, STATUS_REPLY_READY);
        assert!(matches!(action, DeliveryAction::Status { .. }));
        let action = session.tick().unwrap();
        assert_eq!(action, DeliveryAction::Done);
    }

    #[test]
    fn idle_after_cut_is_error() {
        let cut = sample_cut();
        let (mut session, _) =
            ClientDeliverySession::start(ClientHandshake::Gpgl, None, &cut).unwrap();
        session.tick().unwrap(); // init progress → ESC D
        session.on_send_complete().unwrap(); // handshake progress
        session.tick().unwrap(); // → first ready query
        let _ = poll_once(&mut session, STATUS_REPLY_READY);
        session.tick().unwrap(); // → FG
        session.on_send_complete().unwrap(); // handshake progress
        session.tick().unwrap(); // → pre-cut ready query
        let _ = poll_once(&mut session, STATUS_REPLY_READY);
        session.tick().unwrap(); // "cutting" progress
        session.tick().unwrap();
        session.on_send_complete().unwrap();
        let action = poll_once(&mut session, STATUS_REPLY_READY);
        assert!(matches!(action, DeliveryAction::Status { .. }));
        let action = session.tick().unwrap();
        match action {
            DeliveryAction::Error { message } => assert!(message.contains("stayed idle")),
            other => panic!("expected idle error, got {other:?}"),
        }
    }

    #[test]
    fn empty_status_streak_after_cut_is_idle() {
        let cut = sample_cut();
        let (mut session, _) =
            ClientDeliverySession::start(ClientHandshake::Gpgl, None, &cut).unwrap();
        session.tick().unwrap(); // init progress → ESC D
        session.on_send_complete().unwrap(); // handshake progress
        session.tick().unwrap(); // → first ready query
        let _ = poll_once(&mut session, STATUS_REPLY_READY);
        session.tick().unwrap(); // → FG
        session.on_send_complete().unwrap(); // handshake progress
        session.tick().unwrap(); // → pre-cut ready query
        let _ = poll_once(&mut session, STATUS_REPLY_READY);
        session.tick().unwrap(); // "cutting" progress
        session.tick().unwrap(); // → cut send
        session.on_send_complete().unwrap(); // → first post-cut status query

        // Empty replies (no motion) exhaust IDLE_DETECT_POLLS → idle error.
        for _ in 0..(IDLE_DETECT_POLLS - 1) {
            let action = poll_once(&mut session, &[]);
            assert_eq!(expect_send(action), STATUS_QUERY.to_vec());
        }
        let action = poll_once(&mut session, &[]);
        match action {
            DeliveryAction::Error { message } => assert!(message.contains("stayed idle")),
            other => panic!("expected idle error, got {other:?}"),
        }
    }

    #[test]
    fn unloaded_media_aborts() {
        let cut = sample_cut();
        let (mut session, action) =
            ClientDeliverySession::start(ClientHandshake::Gpgl, None, &cut).unwrap();
        session.tick().unwrap(); // init progress → ESC D
        let _ = action;
        // No soft-drain: on_send_complete yields handshake progress (not a recv).
        session.on_send_complete().unwrap(); // handshake progress
        session.tick().unwrap(); // → first ready query

        let action = poll_once(&mut session, STATUS_REPLY_UNLOADED);
        assert!(matches!(action, DeliveryAction::Status { .. }));
        let action = session.tick().unwrap();
        match action {
            DeliveryAction::Error { message } => assert!(message.contains("unloaded")),
            other => panic!("expected Error, got {other:?}"),
        }
    }
}

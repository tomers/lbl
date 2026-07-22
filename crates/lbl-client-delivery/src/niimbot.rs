//! NIIMBOT progress-polling delivery.
//!
//! The job is written (bundled into fewer BLE-sized writes for the B1 task),
//! then `GetPrintStatus` is polled until progress reaches 100/100 after having
//! observed sub-100 activity (so a stale "100/100" from a prior page is not
//! mistaken for completion). The B1 task defers its `EndPrint` frame until after
//! polling confirms the page finished.
//!
//! Framing, bundling, and status parsing come from [`lbl_driver_niimbot`].

use lbl_driver_niimbot::{
    bundle_frames, frames, parse_status, split_deferred_print_end, status_query, NiimbotDriver,
    B1_BUNDLE_MAX,
};
use lbl_status::{NiimbotLiveStatus, PrintStatus};

use crate::{DeliveryAction, DeliveryError, Event, Handshake};

/// Advisory per-poll read timeout (ms).
const POLL_TIMEOUT_MS: u32 = 500;
/// Minimum useful status-reply length (`55 55 B3 04 …` is 11 bytes).
const STATUS_MIN_LEN: usize = 8;
/// Iteration bound so a silent device cannot poll forever. The authoritative
/// ~25 s cap is enforced by the caller's per-`Recv` timeout; at ~700 ms per
/// poll that is ~35 iterations, so 60 leaves comfortable headroom.
const MAX_POLLS: u32 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    SendingJob,
    PollQuerySent,
    PollRecv,
    DeferredSent,
}

pub(crate) struct NiimbotPoll {
    bundles: Vec<Vec<u8>>,
    bundle_idx: usize,
    deferred_print_end: Option<Vec<u8>>,
    phase: Phase,
    polls: u32,
    saw_activity: bool,
}

impl NiimbotPoll {
    pub(crate) fn new(
        label_bytes: &[u8],
        driver_variant: Option<&str>,
    ) -> Result<Self, DeliveryError> {
        let task = NiimbotDriver::resolve_task(driver_variant).ok_or_else(|| {
            DeliveryError::UnsupportedVariant {
                protocol: "niimbot_poll",
                variant: driver_variant.unwrap_or_default().to_string(),
            }
        })?;
        let is_b1 = matches!(task, lbl_driver_niimbot::NiimbotTask::B1);

        let (job, deferred_print_end) = if is_b1 {
            split_deferred_print_end(label_bytes)
        } else {
            (label_bytes.to_vec(), None)
        };

        // B1 bundles many tiny row frames into fewer BLE writes; other tasks
        // send the whole stream in one write and let the transport chunk it.
        let bundles = if is_b1 {
            let parts = frames(&job);
            let bundles = bundle_frames(&parts, B1_BUNDLE_MAX);
            if bundles.is_empty() {
                vec![job]
            } else {
                bundles
            }
        } else {
            vec![job]
        };

        Ok(Self {
            bundles,
            bundle_idx: 0,
            deferred_print_end,
            phase: Phase::SendingJob,
            polls: 0,
            saw_activity: false,
        })
    }

    fn poll_query(&mut self) -> DeliveryAction {
        self.phase = Phase::PollQuerySent;
        DeliveryAction::send(status_query())
    }

    fn finish(&mut self, lead: Vec<DeliveryAction>) -> Vec<DeliveryAction> {
        let mut lead = lead;
        if let Some(print_end) = self.deferred_print_end.take() {
            self.phase = Phase::DeferredSent;
            lead.push(DeliveryAction::send(print_end));
        } else {
            lead.push(DeliveryAction::Done);
        }
        lead
    }
}

impl Handshake for NiimbotPoll {
    fn start(&mut self) -> Vec<DeliveryAction> {
        self.phase = Phase::SendingJob;
        self.bundle_idx = 0;
        vec![
            DeliveryAction::progress("sending", "Sending label to printer…"),
            DeliveryAction::send(self.bundles[0].clone()),
        ]
    }

    fn advance(&mut self, event: Event) -> Vec<DeliveryAction> {
        match (self.phase, event) {
            (Phase::SendingJob, Event::SendComplete) => {
                self.bundle_idx += 1;
                if self.bundle_idx < self.bundles.len() {
                    vec![DeliveryAction::send(self.bundles[self.bundle_idx].clone())]
                } else {
                    vec![self.poll_query()]
                }
            }
            (Phase::PollQuerySent, Event::SendComplete) => {
                self.phase = Phase::PollRecv;
                vec![DeliveryAction::recv(STATUS_MIN_LEN, POLL_TIMEOUT_MS)]
            }
            (Phase::PollRecv, Event::Rx(bytes)) => {
                self.polls += 1;
                match parse_status(&bytes) {
                    Some(status) => {
                        if status.progress1 < 100 || status.progress2 < 100 {
                            self.saw_activity = true;
                        }
                        let complete =
                            status.progress1 >= 100 && status.progress2 >= 100 && self.saw_activity;
                        let note = DeliveryAction::status(PrintStatus::Niimbot(
                            NiimbotLiveStatus::from(status),
                        ));
                        if complete {
                            self.finish(vec![note])
                        } else if self.polls >= MAX_POLLS {
                            vec![note, DeliveryAction::Done]
                        } else {
                            vec![note, self.poll_query()]
                        }
                    }
                    None => {
                        if self.polls >= MAX_POLLS {
                            vec![DeliveryAction::Done]
                        } else {
                            vec![self.poll_query()]
                        }
                    }
                }
            }
            (Phase::DeferredSent, Event::SendComplete) => vec![DeliveryAction::Done],
            _ => vec![DeliveryAction::error(
                "niimbot delivery received an out-of-order transport event",
            )],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientDeliverySession, ClientHandshake};

    fn status_reply(progress1: u8, progress2: u8) -> Vec<u8> {
        // PrintStatusResponse (0xB3) with [page_hi, page_lo, progress1, progress2].
        lbl_driver_niimbot::frame_packet(0xB3, &[0x00, 0x00, progress1, progress2])
    }

    fn expect_send(action: DeliveryAction) -> Vec<u8> {
        match action {
            DeliveryAction::Send { bytes } => bytes,
            other => panic!("expected Send, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_variant() {
        let err = ClientDeliverySession::start(
            ClientHandshake::NiimbotPoll,
            Some("nope"),
            &[0x55, 0x55, 0x01, 0x01, 0x01, 0x00, 0xAA, 0xAA],
        )
        .unwrap_err();
        assert!(matches!(err, DeliveryError::UnsupportedVariant { .. }));
    }

    #[test]
    fn standard_sends_job_then_polls_until_complete() {
        let job = vec![0x55, 0x55, 0x01, 0x01, 0x01, 0x00, 0xAA, 0xAA];
        let (mut session, action) =
            ClientDeliverySession::start(ClientHandshake::NiimbotPoll, Some("standard"), &job)
                .unwrap();

        // Progress → job write.
        assert!(matches!(action, DeliveryAction::Progress { .. }));
        let action = session.tick().unwrap();
        assert_eq!(expect_send(action), job);

        // First poll query.
        let action = session.on_send_complete().unwrap();
        let query = expect_send(action);
        assert_eq!(query, status_query());

        // Poll #1: 40/0 → activity, keep polling.
        let action = session.on_send_complete().unwrap();
        assert_eq!(
            action,
            DeliveryAction::recv(STATUS_MIN_LEN, POLL_TIMEOUT_MS)
        );
        let action = session.feed_rx(&status_reply(40, 0)).unwrap();
        assert!(matches!(action, DeliveryAction::Status { .. }));
        let action = session.tick().unwrap();
        assert_eq!(expect_send(action), status_query());

        // Poll #2: 100/100 after activity → complete → done (no deferred end).
        session.on_send_complete().unwrap();
        let action = session.feed_rx(&status_reply(100, 100)).unwrap();
        assert!(matches!(action, DeliveryAction::Status { .. }));
        let action = session.tick().unwrap();
        assert_eq!(action, DeliveryAction::Done);
    }

    #[test]
    fn stale_full_progress_without_activity_keeps_polling() {
        let job = vec![0x55, 0x55, 0x01, 0x01, 0x01, 0x00, 0xAA, 0xAA];
        let (mut session, action) =
            ClientDeliverySession::start(ClientHandshake::NiimbotPoll, None, &job).unwrap();
        session.tick().unwrap(); // progress → job send
        let _ = action;
        session.on_send_complete().unwrap(); // job sent → poll query
        session.on_send_complete().unwrap(); // query sent → recv

        // Immediate 100/100 with no prior sub-100 reading: not complete yet.
        let action = session.feed_rx(&status_reply(100, 100)).unwrap();
        assert!(matches!(action, DeliveryAction::Status { .. }));
        let action = session.tick().unwrap();
        assert_eq!(expect_send(action), status_query());
    }

    #[test]
    fn b1_defers_print_end_until_after_completion() {
        use lbl_core::job::JobSpec;
        use lbl_core::media::Media;
        use lbl_core::printer::DeviceCapabilities;
        use lbl_core::units::Dpi;
        use lbl_driver_api::{Driver, EncodeContext};
        use lbl_driver_niimbot::NiimbotDriver;

        let bmp = lbl_driver_api::MonoBitmap::new(384, 2);
        let caps = DeviceCapabilities::default();
        let job = JobSpec::new(Media::fixed(12.0, 40.0, Dpi(203.0)));
        let encoded = NiimbotDriver::b1()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();
        let (expected_job, expected_end) = split_deferred_print_end(&encoded);
        let expected_end = expected_end.expect("b1 job ends with EndPrint");

        let (mut session, action) =
            ClientDeliverySession::start(ClientHandshake::NiimbotPoll, Some("b1"), &encoded)
                .unwrap();
        assert!(matches!(action, DeliveryAction::Progress { .. }));

        // Drain the bundled job writes until the first poll query appears.
        let mut action = session.tick().unwrap();
        let mut sent = Vec::new();
        loop {
            let bytes = expect_send(action);
            if bytes == status_query() {
                break;
            }
            sent.extend_from_slice(&bytes);
            action = session.on_send_complete().unwrap();
        }
        // Bundled writes reconstruct the deferred-end-stripped job exactly.
        assert_eq!(sent, expected_job);

        // Poll to completion.
        session.on_send_complete().unwrap();
        session.feed_rx(&status_reply(50, 0)).unwrap();
        let action = session.tick().unwrap();
        assert_eq!(expect_send(action), status_query());
        session.on_send_complete().unwrap();
        session.feed_rx(&status_reply(100, 100)).unwrap();

        // Completion → deferred EndPrint write → done.
        let action = session.tick().unwrap();
        assert_eq!(expect_send(action), expected_end);
        let action = session.on_send_complete().unwrap();
        assert_eq!(action, DeliveryAction::Done);
    }
}

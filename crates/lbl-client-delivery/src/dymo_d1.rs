//! DYMO LabelManager (D1) status-paced delivery.
//!
//! The job is normalized to cut-then-status trailers and split into
//! `SYN`-bounded chunks (see [`lbl_driver_dymo::d1`]). Each chunk is preceded by
//! a bare `ESC A` "ping" whose one-byte reply is drained (inter-chunk pacing),
//! then the chunk is written, then one bulk-IN reply is drained per `ESC A`
//! status query the chunk contains. Draining the trailer `ESC A` waits for the
//! cut/feed to finish so the next job's pacing poll does not race a busy
//! chassis.
//!
//! Ported from the DYMO D1 WebUSB client path; the framing math is shared with
//! native senders via [`lbl_driver_dymo::d1`].

use lbl_driver_dymo::d1;

use crate::{DeliveryAction, Event, Handshake};

const STATUS_TIMEOUT_MS: u32 = 6_000;
const JOB_COMPLETE_TIMEOUT_MS: u32 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Wrote the inter-chunk `ESC A` ping; expecting `SendComplete`.
    PingSent,
    /// Reading the ping reply; expecting `Rx`.
    PingDrain,
    /// Wrote the chunk; expecting `SendComplete`.
    ChunkSent,
    /// Draining a chunk's trailer status reply; expecting `Rx`.
    ChunkDrain,
}

pub(crate) struct DymoD1 {
    chunks: Vec<Vec<u8>>,
    idx: usize,
    phase: Phase,
    pending_drains: usize,
}

impl DymoD1 {
    pub(crate) fn new(label_bytes: &[u8]) -> Result<Self, crate::DeliveryError> {
        let job = d1::normalize_job_trailers(label_bytes);
        let chunks: Vec<Vec<u8>> = d1::split_chunks(&job, d1::SYN_CHUNK_MAX)
            .into_iter()
            .map(<[u8]>::to_vec)
            .collect();
        if chunks.is_empty() {
            return Err(crate::DeliveryError::Malformed {
                protocol: "dymo_d1",
                message: "job contains no writable data".into(),
            });
        }
        Ok(Self {
            chunks,
            idx: 0,
            phase: Phase::PingSent,
            pending_drains: 0,
        })
    }

    fn ping(&mut self) -> DeliveryAction {
        self.phase = Phase::PingSent;
        DeliveryAction::send(d1::STATUS_REQUEST.to_vec())
    }

    /// Advance to the next chunk, or finish when the stream is exhausted.
    fn next_chunk(&mut self) -> Vec<DeliveryAction> {
        self.idx += 1;
        if self.idx < self.chunks.len() {
            vec![self.ping()]
        } else {
            vec![DeliveryAction::Done]
        }
    }
}

impl Handshake for DymoD1 {
    fn start(&mut self) -> Vec<DeliveryAction> {
        vec![
            DeliveryAction::progress("sending", "Sending to printer…"),
            self.ping(),
        ]
    }

    fn advance(&mut self, event: Event) -> Vec<DeliveryAction> {
        match (self.phase, event) {
            (Phase::PingSent, Event::SendComplete) => {
                self.phase = Phase::PingDrain;
                vec![DeliveryAction::recv(d1::STATUS_READ_LEN, STATUS_TIMEOUT_MS)]
            }
            (Phase::PingDrain, Event::Rx(_)) => {
                self.phase = Phase::ChunkSent;
                vec![DeliveryAction::send(self.chunks[self.idx].clone())]
            }
            (Phase::ChunkSent, Event::SendComplete) => {
                let drains = d1::count_status_queries(&self.chunks[self.idx]);
                if drains == 0 {
                    self.next_chunk()
                } else {
                    self.pending_drains = drains;
                    self.phase = Phase::ChunkDrain;
                    vec![DeliveryAction::recv(
                        d1::STATUS_READ_LEN,
                        JOB_COMPLETE_TIMEOUT_MS,
                    )]
                }
            }
            (Phase::ChunkDrain, Event::Rx(_)) => {
                self.pending_drains -= 1;
                if self.pending_drains > 0 {
                    vec![DeliveryAction::recv(
                        d1::STATUS_READ_LEN,
                        JOB_COMPLETE_TIMEOUT_MS,
                    )]
                } else {
                    self.next_chunk()
                }
            }
            _ => vec![DeliveryAction::error(
                "dymo-d1 delivery received an out-of-order transport event",
            )],
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{ClientDeliverySession, ClientHandshake, DeliveryAction};

    const ESC: u8 = 0x1B;
    const SYN: u8 = 0x16;

    /// A tiny single-chunk job: header + one raster column + cut/status trailer.
    fn tiny_job() -> Vec<u8> {
        vec![ESC, b'C', 0, ESC, b'D', 1, SYN, 0x00, ESC, b'E', ESC, b'A']
    }

    fn drive_to_send(session: &mut ClientDeliverySession, mut action: DeliveryAction) -> Vec<u8> {
        // Consume any leading progress notifications.
        while let DeliveryAction::Progress { .. } = action {
            action = session.tick().unwrap();
        }
        match action {
            DeliveryAction::Send { bytes } => bytes,
            other => panic!("expected Send, got {other:?}"),
        }
    }

    #[test]
    fn pings_then_sends_chunk_then_drains_trailer() {
        let job = tiny_job();
        let (mut session, action) =
            ClientDeliverySession::start(ClientHandshake::DymoD1, None, &job).unwrap();

        // Ping: bare ESC A.
        let ping = drive_to_send(&mut session, action);
        assert_eq!(ping, vec![ESC, b'A']);

        // Drain the ping reply.
        let action = session.on_send_complete().unwrap();
        assert_eq!(action, DeliveryAction::recv(64, 6_000));
        let action = session.feed_rx(&[0x00]).unwrap();

        // Chunk write is the whole (single) chunk.
        let chunk = match action {
            DeliveryAction::Send { bytes } => bytes,
            other => panic!("expected chunk Send, got {other:?}"),
        };
        assert_eq!(chunk, job);

        // One trailer ESC A → one drain, then Done.
        let action = session.on_send_complete().unwrap();
        assert_eq!(action, DeliveryAction::recv(64, 60_000));
        let action = session.feed_rx(&[0x00]).unwrap();
        assert_eq!(action, DeliveryAction::Done);
        assert!(session.is_finished());
    }
}

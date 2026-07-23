//! Transport-agnostic client delivery sessions.
//!
//! Bidirectional printer protocols (DYMO LabelManager/LabelWriter status pacing,
//! NIIMBOT progress polling, LetraTag notify completion, GPGL cutter readiness)
//! are usually implemented inside a specific transport (WebUSB, Web Bluetooth,
//! native USB). That couples the handshake logic to one runtime and makes it
//! impossible to reuse the exact pacing rules from a browser client compiled to
//! WebAssembly.
//!
//! This crate factors the *handshake* out of the *transport*. A
//! [`ClientDeliverySession`] is a pure state machine: it emits
//! [`DeliveryAction`]s telling the caller what byte-level I/O to perform, and it
//! consumes the results. The caller owns the transport and simply runs a loop:
//!
//! ```text
//! let (mut session, mut action) =
//!     ClientDeliverySession::start(handshake, variant, &label_bytes)?;
//! loop {
//!     action = match action {
//!         DeliveryAction::Send { bytes }        => { transport.write(&bytes)?;         session.on_send_complete()? }
//!         DeliveryAction::Recv { min_len, timeout_ms } => { let d = transport.read(min_len, timeout_ms); session.feed_rx(&d)? }
//!         DeliveryAction::Progress { .. } | DeliveryAction::Status { .. } => { report(&action); session.tick()? }
//!         DeliveryAction::Done                  => break,
//!         DeliveryAction::Error { message }     => return Err(message.into()),
//!     };
//! }
//! ```
//!
//! ## Contract
//!
//! Every session call returns exactly one [`DeliveryAction`]. Respond to it with
//! the matching call:
//!
//! - [`DeliveryAction::Send`] → perform the write, then [`ClientDeliverySession::on_send_complete`].
//! - [`DeliveryAction::Recv`] → read up to the transport's reply (at least
//!   `min_len` bytes, honoring `timeout_ms`; a timeout yields an empty read),
//!   then [`ClientDeliverySession::feed_rx`].
//! - [`DeliveryAction::Progress`] / [`DeliveryAction::Status`] → surface it, then
//!   [`ClientDeliverySession::tick`] to obtain the next action.
//! - [`DeliveryAction::Done`] / [`DeliveryAction::Error`] → terminal; stop.
//!
//! The session never blocks and never measures wall-clock time (it is
//! `wasm32`-safe: no threads, sockets, or timers). Timeouts are advisory values
//! the caller enforces; poll loops are bounded by iteration count so a silent
//! device cannot hang the state machine forever.

mod dymo_d1;
mod dymo_lw;
mod fire_and_forget;
mod gpgl;
mod letratag;
mod niimbot;

use std::collections::VecDeque;

pub use lbl_driver_api::ClientHandshake;
pub use lbl_status::PrintStatus;

/// An instruction from the session to the transport-owning caller.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeliveryAction {
    /// Write `bytes` to the device, then call
    /// [`ClientDeliverySession::on_send_complete`].
    Send {
        /// Bytes to write to the transport.
        bytes: Vec<u8>,
    },
    /// Read from the device (at least `min_len` bytes within `timeout_ms`), then
    /// call [`ClientDeliverySession::feed_rx`] with what was read. A read that
    /// times out should be reported as an empty slice.
    Recv {
        /// Minimum useful reply length; the caller may read more.
        min_len: usize,
        /// Advisory read timeout in milliseconds (caller-enforced).
        timeout_ms: u32,
    },
    /// A machine-stable progress update; call
    /// [`ClientDeliverySession::tick`] to continue. Consumers map `phase` to
    /// display copy.
    Progress {
        /// Stable phase id (e.g. `acquiring_lock`, `sending_label`).
        phase: String,
        /// 0-based label index within a batch, when known.
        #[serde(skip_serializing_if = "Option::is_none")]
        batch_index: Option<u32>,
        /// Total labels in the batch, when known.
        #[serde(skip_serializing_if = "Option::is_none")]
        batch_total: Option<u32>,
    },
    /// A decoded print-engine status; call
    /// [`ClientDeliverySession::tick`] to continue.
    Status {
        /// Protocol-tagged status snapshot.
        status: PrintStatus,
    },
    /// Delivery finished successfully. Terminal.
    Done,
    /// Delivery failed. Terminal; `message` is a diagnostic (not UI copy).
    Error {
        /// Machine/diagnostic failure description.
        message: String,
    },
}

impl DeliveryAction {
    pub(crate) fn send(bytes: Vec<u8>) -> Self {
        Self::Send { bytes }
    }

    pub(crate) fn recv(min_len: usize, timeout_ms: u32) -> Self {
        Self::Recv {
            min_len,
            timeout_ms,
        }
    }

    pub(crate) fn progress(phase: &str) -> Self {
        Self::Progress {
            phase: phase.to_string(),
            batch_index: None,
            batch_total: None,
        }
    }

    pub(crate) fn label_progress(phase: &str, index: u32, total: u32) -> Self {
        Self::Progress {
            phase: phase.to_string(),
            batch_index: Some(index),
            batch_total: Some(total),
        }
    }

    pub(crate) fn status(status: PrintStatus) -> Self {
        Self::Status { status }
    }

    pub(crate) fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}

/// A usage or input error from constructing or driving a session.
///
/// These are caller-side faults (empty/malformed input, calling the wrong
/// response method). Device-side failures are surfaced in-band as
/// [`DeliveryAction::Error`] so the driving loop handles them uniformly.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeliveryError {
    /// `label_bytes` was empty.
    #[error("empty label payload")]
    EmptyPayload,

    /// The encoded job could not be parsed for this handshake.
    #[error("{protocol} job is malformed: {message}")]
    Malformed {
        /// Handshake id (e.g. `dymo_lw`).
        protocol: &'static str,
        /// Parser diagnostic.
        message: String,
    },

    /// The requested driver variant is not recognized for this handshake.
    #[error("unsupported driver variant {variant:?} for {protocol}")]
    UnsupportedVariant {
        /// Handshake id.
        protocol: &'static str,
        /// The rejected variant string.
        variant: String,
    },

    /// A session method was called out of order (e.g. `feed_rx` when a `Send`
    /// was pending, or any call after the session finished).
    #[error("invalid session call: {0}")]
    Usage(String),
}

/// An external event fed to a [`Handshake`] machine.
pub(crate) enum Event {
    /// The most recent [`DeliveryAction::Send`] has been written.
    SendComplete,
    /// Bytes read in response to the most recent [`DeliveryAction::Recv`].
    Rx(Vec<u8>),
}

/// A single handshake's pure state machine.
///
/// Each call returns a *batch*: zero or more notifications
/// ([`DeliveryAction::Progress`] / [`DeliveryAction::Status`]) followed by
/// exactly one terminal or I/O action ([`DeliveryAction::Send`] /
/// [`DeliveryAction::Recv`] / [`DeliveryAction::Done`] / [`DeliveryAction::Error`]).
pub(crate) trait Handshake {
    /// Produce the opening batch (called once).
    fn start(&mut self) -> Vec<DeliveryAction>;
    /// Advance the machine in response to `event`.
    fn advance(&mut self, event: Event) -> Vec<DeliveryAction>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Awaiting {
    SendComplete,
    Rx,
    Notify,
    Finished,
}

/// A transport-agnostic delivery session driving one bidirectional handshake.
///
/// See the [crate docs](crate) for the driving loop and per-action contract.
pub struct ClientDeliverySession {
    machine: Box<dyn Handshake>,
    queue: VecDeque<DeliveryAction>,
    awaiting: Awaiting,
    finished: bool,
}

impl std::fmt::Debug for ClientDeliverySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientDeliverySession")
            .field("awaiting", &self.awaiting)
            .field("queued", &self.queue.len())
            .field("finished", &self.finished)
            .finish()
    }
}

impl ClientDeliverySession {
    /// Begin delivering `label_bytes` for `handshake`.
    ///
    /// `driver_variant` is an opaque per-handshake selector (only NIIMBOT uses
    /// it, for `standard` / `v4` / `b1`). Returns the session and the first
    /// [`DeliveryAction`] to perform.
    pub fn start(
        handshake: ClientHandshake,
        driver_variant: Option<&str>,
        label_bytes: &[u8],
    ) -> Result<(Self, DeliveryAction), DeliveryError> {
        if label_bytes.is_empty() {
            return Err(DeliveryError::EmptyPayload);
        }
        let machine: Box<dyn Handshake> = match handshake {
            ClientHandshake::FireAndForget => {
                Box::new(fire_and_forget::FireAndForget::new(label_bytes))
            }
            ClientHandshake::DymoD1 => Box::new(dymo_d1::DymoD1::new(label_bytes)?),
            ClientHandshake::DymoLw => Box::new(dymo_lw::DymoLw::new(label_bytes)?),
            ClientHandshake::NiimbotPoll => {
                Box::new(niimbot::NiimbotPoll::new(label_bytes, driver_variant)?)
            }
            ClientHandshake::LetraTagNotify => Box::new(letratag::LetraTag::new(label_bytes)),
            ClientHandshake::Gpgl => Box::new(gpgl::Gpgl::new(label_bytes)),
        };
        let mut session = Self {
            machine,
            queue: VecDeque::new(),
            awaiting: Awaiting::Notify,
            finished: false,
        };
        let batch = session.machine.start();
        session.enqueue(batch)?;
        let action = session.pump();
        Ok((session, action))
    }

    /// Report that the pending [`DeliveryAction::Send`] has been written.
    pub fn on_send_complete(&mut self) -> Result<DeliveryAction, DeliveryError> {
        self.ensure_running()?;
        if self.awaiting != Awaiting::SendComplete || !self.queue.is_empty() {
            return Err(DeliveryError::Usage(
                "on_send_complete called with no Send pending".into(),
            ));
        }
        let batch = self.machine.advance(Event::SendComplete);
        self.enqueue(batch)?;
        Ok(self.pump())
    }

    /// Provide the bytes read in response to the pending [`DeliveryAction::Recv`].
    ///
    /// An empty slice signals a read timeout; poll-based handshakes treat it as
    /// "no reply yet" and continue up to their iteration bound.
    pub fn feed_rx(&mut self, bytes: &[u8]) -> Result<DeliveryAction, DeliveryError> {
        self.ensure_running()?;
        if self.awaiting != Awaiting::Rx || !self.queue.is_empty() {
            return Err(DeliveryError::Usage(
                "feed_rx called with no Recv pending".into(),
            ));
        }
        let batch = self.machine.advance(Event::Rx(bytes.to_vec()));
        self.enqueue(batch)?;
        Ok(self.pump())
    }

    /// Consume a buffered [`DeliveryAction::Progress`] / [`DeliveryAction::Status`]
    /// notification and return the next action.
    ///
    /// Only valid after a notification action; responding to a `Send`/`Recv`
    /// requires [`Self::on_send_complete`] / [`Self::feed_rx`].
    pub fn tick(&mut self) -> Result<DeliveryAction, DeliveryError> {
        self.ensure_running()?;
        if self.queue.is_empty() {
            return Err(DeliveryError::Usage(
                "tick called with no buffered notification; respond to the pending Send/Recv"
                    .into(),
            ));
        }
        Ok(self.pump())
    }

    /// Whether the session has reached a terminal action.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    fn ensure_running(&self) -> Result<(), DeliveryError> {
        if self.finished {
            return Err(DeliveryError::Usage("session already finished".into()));
        }
        Ok(())
    }

    fn enqueue(&mut self, batch: Vec<DeliveryAction>) -> Result<(), DeliveryError> {
        if batch.is_empty() {
            return Err(DeliveryError::Usage(
                "handshake produced no action (internal state machine bug)".into(),
            ));
        }
        self.queue.extend(batch);
        Ok(())
    }

    fn pump(&mut self) -> DeliveryAction {
        let action = self
            .queue
            .pop_front()
            .expect("queue is refilled before pump");
        self.awaiting = match &action {
            DeliveryAction::Send { .. } => Awaiting::SendComplete,
            DeliveryAction::Recv { .. } => Awaiting::Rx,
            DeliveryAction::Progress { .. } | DeliveryAction::Status { .. } => Awaiting::Notify,
            DeliveryAction::Done | DeliveryAction::Error { .. } => {
                self.finished = true;
                Awaiting::Finished
            }
        };
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_payload_is_rejected() {
        let err = ClientDeliverySession::start(ClientHandshake::FireAndForget, None, &[]);
        assert_eq!(err.err(), Some(DeliveryError::EmptyPayload));
    }

    #[test]
    fn action_serializes_with_type_tag() {
        let json = serde_json::to_value(DeliveryAction::recv(32, 6000)).unwrap();
        assert_eq!(json["type"], "recv");
        assert_eq!(json["min_len"], 32);
        assert_eq!(json["timeout_ms"], 6000);
    }
}

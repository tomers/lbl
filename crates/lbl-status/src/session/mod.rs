//! Transport-agnostic multi-probe status sessions.
//!
//! Bidirectional status flows (DYMO `ESC V`/`A`/`U`, NIIMBOT RFID/heartbeat,
//! GPGL `FG`/`TI` + status) are usually orchestrated in a specific transport.
//! This module factors the *probe plan* out of the *transport* the same way
//! `lbl-client-delivery` factors print handshakes.
//!
//! The session never blocks and never measures wall-clock time (`wasm32`-safe).

mod dymo;
mod gpgl;
mod niimbot;
mod single_shot;

use std::collections::VecDeque;

use lbl_core::printer::Protocol;
use lbl_driver_niimbot::live_status::NiimbotDeviceInfo;

use crate::dymo_lw::Lw550EngineVersion;
use crate::{PrintStatus, StatusError};

/// Default advisory I/O timeout for status reads (ms).
pub(crate) const STATUS_IO_TIMEOUT_MS: u32 = 6_000;

/// Optional session inputs carried across polls (cached engine / device info).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct StatusSessionContext {
    /// Opaque driver variant (e.g. NIIMBOT `"b1"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_variant: Option<String>,
    /// Previously fetched DYMO `ESC V` block for this connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_engine_version: Option<Lw550EngineVersion>,
    /// Skip `ESC V` (prior failure this session).
    #[serde(default)]
    pub skip_engine_version: bool,
    /// Cached NIIMBOT model id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_model_id: Option<u16>,
    /// Cached NIIMBOT device info.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_device_info: Option<NiimbotDeviceInfo>,
    /// Cached GPGL `FG` firmware string for this connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_gpgl_firmware: Option<String>,
    /// Skip `FG` (prior silence / failure this session).
    #[serde(default)]
    pub skip_gpgl_firmware: bool,
    /// Cached GPGL `TI` device name for this connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_gpgl_device_name: Option<String>,
    /// Skip `TI` (prior silence / failure this session).
    #[serde(default)]
    pub skip_gpgl_device_name: bool,
}

/// An instruction from the status session to the transport-owning caller.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StatusAction {
    /// Write `bytes` to the device, then call [`ClientStatusSession::on_send_complete`].
    Send {
        /// Bytes to write.
        bytes: Vec<u8>,
    },
    /// Read from the device, then call [`ClientStatusSession::feed_rx`].
    Recv {
        /// Minimum useful reply length when `match_cmds` is empty.
        min_len: usize,
        /// Advisory read timeout in milliseconds.
        timeout_ms: u32,
        /// When non-empty, treat `feed_rx` input as a NIIMBOT notify buffer and
        /// extract the first framed payload whose command is in this list.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        match_cmds: Vec<u8>,
    },
    /// Status query finished successfully.
    Done {
        /// Protocol-tagged status snapshot.
        status: Box<PrintStatus>,
        /// Updated context to persist on the connection handle.
        #[serde(skip_serializing_if = "status_context_is_default")]
        context: Box<StatusSessionContextView>,
    },
    /// Status query failed.
    Error {
        /// Diagnostic message (not UI copy).
        message: String,
    },
}

impl StatusAction {
    pub(crate) fn send(bytes: Vec<u8>) -> Self {
        Self::Send { bytes }
    }

    pub(crate) fn recv(min_len: usize, timeout_ms: u32) -> Self {
        Self::Recv {
            min_len,
            timeout_ms,
            match_cmds: Vec::new(),
        }
    }

    pub(crate) fn recv_match(timeout_ms: u32, match_cmds: Vec<u8>) -> Self {
        Self::Recv {
            min_len: 0,
            timeout_ms,
            match_cmds,
        }
    }

    pub(crate) fn done(status: PrintStatus, context: &StatusSessionContext) -> Self {
        Self::Done {
            status: Box::new(status),
            context: Box::new(StatusSessionContextView::from(context)),
        }
    }

    pub(crate) fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}

// serde `skip_serializing_if` passes `&Field`; Field is `Box<_>`.
#[allow(clippy::borrowed_box)]
fn status_context_is_default(ctx: &Box<StatusSessionContextView>) -> bool {
    ctx.engine_version.is_none()
        && ctx.model_id.is_none()
        && ctx.device_info.is_none()
        && !ctx.skip_engine_version
        && ctx.gpgl_firmware.is_none()
        && ctx.gpgl_device_name.is_none()
        && !ctx.skip_gpgl_firmware
        && !ctx.skip_gpgl_device_name
}

/// Serializable subset of [`StatusSessionContext`] returned on [`StatusAction::Done`].
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct StatusSessionContextView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_version: Option<Lw550EngineVersion>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skip_engine_version: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_info: Option<NiimbotDeviceInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpgl_firmware: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skip_gpgl_firmware: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpgl_device_name: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skip_gpgl_device_name: bool,
}

impl From<&StatusSessionContext> for StatusSessionContextView {
    fn from(ctx: &StatusSessionContext) -> Self {
        Self {
            engine_version: ctx.cached_engine_version.clone(),
            skip_engine_version: ctx.skip_engine_version,
            model_id: ctx.cached_model_id,
            device_info: ctx.cached_device_info.clone(),
            gpgl_firmware: ctx.cached_gpgl_firmware.clone(),
            skip_gpgl_firmware: ctx.skip_gpgl_firmware,
            gpgl_device_name: ctx.cached_gpgl_device_name.clone(),
            skip_gpgl_device_name: ctx.skip_gpgl_device_name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StatusSessionError {
    #[error("{0}")]
    Usage(String),
    #[error(transparent)]
    Status(#[from] StatusError),
}

/// A single protocol's pure status-probe state machine.
pub(crate) trait Probe {
    /// Produce the opening batch (called once).
    fn bootstrap(&mut self) -> Result<Vec<StatusAction>, StatusSessionError>;
    /// Advance after the pending [`StatusAction::Send`] was written.
    fn advance_after_send(
        &mut self,
        context: &mut StatusSessionContext,
    ) -> Result<Vec<StatusAction>, StatusSessionError>;
    /// Advance after bytes were read for the pending [`StatusAction::Recv`].
    fn advance_after_rx(
        &mut self,
        bytes: &[u8],
        context: &mut StatusSessionContext,
    ) -> Result<Vec<StatusAction>, StatusSessionError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Awaiting {
    SendComplete,
    Rx,
    Finished,
}

/// Pure state machine for a multi-probe status query.
pub struct ClientStatusSession {
    machine: Box<dyn Probe>,
    queue: VecDeque<StatusAction>,
    awaiting: Awaiting,
    finished: bool,
    context: StatusSessionContext,
}

impl std::fmt::Debug for ClientStatusSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientStatusSession")
            .field("awaiting", &self.awaiting)
            .field("queued", &self.queue.len())
            .field("finished", &self.finished)
            .finish()
    }
}

impl ClientStatusSession {
    /// Start a status session for `protocol`.
    pub fn start(
        protocol: Protocol,
        context: StatusSessionContext,
    ) -> Result<(Self, StatusAction), StatusSessionError> {
        if !crate::status_supported(protocol) {
            return Err(StatusError::Parse(format!(
                "status query not supported for protocol {protocol:?}"
            ))
            .into());
        }

        let machine: Box<dyn Probe> = match protocol {
            Protocol::DymoLw => Box::new(dymo::Dymo::new(&context)),
            Protocol::Niimbot => Box::new(niimbot::Niimbot::new(&context)),
            Protocol::Gpgl => Box::new(gpgl::Gpgl::new(&context)),
            other => Box::new(single_shot::SingleShot::new(other)),
        };

        let mut session = Self {
            machine,
            queue: VecDeque::new(),
            awaiting: Awaiting::Finished,
            finished: false,
            context,
        };
        let batch = session.machine.bootstrap()?;
        session.enqueue(batch)?;
        let action = session.pump();
        Ok((session, action))
    }

    pub fn on_send_complete(&mut self) -> Result<StatusAction, StatusSessionError> {
        self.require_awaiting(Awaiting::SendComplete, "on_send_complete")?;
        let batch = self.machine.advance_after_send(&mut self.context)?;
        self.enqueue(batch)?;
        Ok(self.pump())
    }

    pub fn feed_rx(&mut self, bytes: &[u8]) -> Result<StatusAction, StatusSessionError> {
        self.require_awaiting(Awaiting::Rx, "feed_rx")?;
        let batch = self.machine.advance_after_rx(bytes, &mut self.context)?;
        self.enqueue(batch)?;
        Ok(self.pump())
    }

    fn require_awaiting(&self, expected: Awaiting, method: &str) -> Result<(), StatusSessionError> {
        let ok = matches!(
            (&self.awaiting, &expected),
            (Awaiting::SendComplete, Awaiting::SendComplete) | (Awaiting::Rx, Awaiting::Rx)
        );
        if !ok || self.finished {
            return Err(StatusSessionError::Usage(format!(
                "{method} called out of order"
            )));
        }
        Ok(())
    }

    fn enqueue(&mut self, batch: Vec<StatusAction>) -> Result<(), StatusSessionError> {
        if batch.is_empty() {
            return Err(StatusSessionError::Usage(
                "status handshake produced no action".into(),
            ));
        }
        self.queue.extend(batch);
        Ok(())
    }

    fn pump(&mut self) -> StatusAction {
        let action = self.queue.pop_front().expect("queue refilled before pump");
        self.awaiting = match &action {
            StatusAction::Send { .. } => Awaiting::SendComplete,
            StatusAction::Recv { .. } => Awaiting::Rx,
            StatusAction::Done { .. } | StatusAction::Error { .. } => {
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
    use lbl_core::printer::Protocol;

    #[test]
    fn brother_ql_single_shot() {
        let mut s = [0u8; 32];
        s[0] = 0x80;
        s[1] = 0x20;
        s[2] = b'B';
        s[4] = b'A';
        s[10] = 62;
        s[11] = 0x4A;
        let (mut session, action) =
            ClientStatusSession::start(Protocol::BrotherQl, StatusSessionContext::default())
                .unwrap();
        assert!(matches!(action, StatusAction::Send { .. }));
        let action = session.on_send_complete().unwrap();
        assert!(matches!(
            action,
            StatusAction::Recv {
                min_len: crate::BROTHER_QL_STATUS_REPLY_LEN,
                ..
            }
        ));
        let action = session.feed_rx(&s).unwrap();
        match action {
            StatusAction::Done { status, .. } => match *status {
                PrintStatus::BrotherQl(st) => {
                    assert_eq!(st.media_width_mm, 62);
                }
                other => panic!("unexpected {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn dymo_skips_sku_when_bay_empty() {
        let (mut session, action) = ClientStatusSession::start(
            Protocol::DymoLw,
            StatusSessionContext {
                skip_engine_version: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(action, StatusAction::Send { .. }));
        let action = session.on_send_complete().unwrap();
        assert!(matches!(action, StatusAction::Recv { .. }));
        let mut esc_a = [0u8; 32];
        esc_a[0] = 0; // idle
        esc_a[9] = 100;
        esc_a[10] = 2; // no media
        let action = session.feed_rx(&esc_a).unwrap();
        match action {
            StatusAction::Done { status, .. } => {
                assert!(matches!(*status, PrintStatus::DymoLw(_)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn gpgl_probes_identity_then_status() {
        let (mut session, action) =
            ClientStatusSession::start(Protocol::Gpgl, StatusSessionContext::default()).unwrap();
        match action {
            StatusAction::Send { bytes } => assert_eq!(bytes, b"FG\x03"),
            other => panic!("unexpected {other:?}"),
        }
        let action = session.on_send_complete().unwrap();
        assert!(matches!(
            action,
            StatusAction::Recv {
                timeout_ms: gpgl::FIRMWARE_TIMEOUT_MS,
                ..
            }
        ));
        let action = session.feed_rx(b"CAMEO V1.10 \x03").unwrap();
        match action {
            StatusAction::Send { bytes } => assert_eq!(bytes, b"TI\x03"),
            other => panic!("unexpected {other:?}"),
        }
        let action = session.on_send_complete().unwrap();
        assert!(matches!(
            action,
            StatusAction::Recv {
                timeout_ms: gpgl::NAME_TIMEOUT_MS,
                ..
            }
        ));
        let action = session.feed_rx(b"Cameo 4\x03").unwrap();
        match action {
            StatusAction::Send { bytes } => assert_eq!(bytes, lbl_driver_gpgl::STATUS_QUERY),
            other => panic!("unexpected {other:?}"),
        }
        let action = session.on_send_complete().unwrap();
        assert!(matches!(
            action,
            StatusAction::Recv {
                timeout_ms: STATUS_IO_TIMEOUT_MS,
                ..
            }
        ));
        let action = session.feed_rx(b"0\x03").unwrap();
        match action {
            StatusAction::Done { status, context } => {
                match *status {
                    PrintStatus::Gpgl(st) => {
                        assert_eq!(st.state, crate::GpglHostStatus::Ready);
                        assert_eq!(st.firmware_version.as_deref(), Some("CAMEO V1.10"));
                        assert_eq!(st.device_name.as_deref(), Some("Cameo 4"));
                    }
                    other => panic!("unexpected {other:?}"),
                }
                assert_eq!(context.gpgl_firmware.as_deref(), Some("CAMEO V1.10"));
                assert_eq!(context.gpgl_device_name.as_deref(), Some("Cameo 4"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn gpgl_cached_identity_jumps_to_status() {
        let (mut session, action) = ClientStatusSession::start(
            Protocol::Gpgl,
            StatusSessionContext {
                cached_gpgl_firmware: Some("CAMEO V1.10".into()),
                cached_gpgl_device_name: Some("Cameo 4".into()),
                ..Default::default()
            },
        )
        .unwrap();
        match action {
            StatusAction::Send { bytes } => assert_eq!(bytes, lbl_driver_gpgl::STATUS_QUERY),
            other => panic!("unexpected {other:?}"),
        }
        let _ = session.on_send_complete().unwrap();
        let action = session.feed_rx(b"0\x03").unwrap();
        match action {
            StatusAction::Done { status, .. } => match *status {
                PrintStatus::Gpgl(st) => {
                    assert_eq!(st.firmware_version.as_deref(), Some("CAMEO V1.10"));
                    assert_eq!(st.device_name.as_deref(), Some("Cameo 4"));
                }
                other => panic!("unexpected {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn gpgl_silent_identity_still_reads_status() {
        let (mut session, _action) =
            ClientStatusSession::start(Protocol::Gpgl, StatusSessionContext::default()).unwrap();
        let _ = session.on_send_complete().unwrap();
        // Silent FG → skip, probe TI.
        let action = session.feed_rx(&[]).unwrap();
        match action {
            StatusAction::Send { bytes } => assert_eq!(bytes, b"TI\x03"),
            other => panic!("unexpected {other:?}"),
        }
        let _ = session.on_send_complete().unwrap();
        // Silent TI → skip, probe ESC E.
        let action = session.feed_rx(&[]).unwrap();
        match action {
            StatusAction::Send { bytes } => assert_eq!(bytes, lbl_driver_gpgl::STATUS_QUERY),
            other => panic!("unexpected {other:?}"),
        }
        let _ = session.on_send_complete().unwrap();
        let action = session.feed_rx(b"2\x03").unwrap();
        match action {
            StatusAction::Done { status, context } => {
                match *status {
                    PrintStatus::Gpgl(st) => {
                        assert_eq!(st.state, crate::GpglHostStatus::Unloaded);
                        assert!(st.firmware_version.is_none());
                        assert!(st.device_name.is_none());
                    }
                    other => panic!("unexpected {other:?}"),
                }
                assert!(context.skip_gpgl_firmware);
                assert!(context.skip_gpgl_device_name);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

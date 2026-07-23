//! Transport-agnostic multi-probe status sessions.
//!
//! Bidirectional status flows (DYMO `ESC V`/`A`/`U`, NIIMBOT RFID/heartbeat
//! probes) are usually orchestrated in a specific transport. This module factors
//! the *probe plan* out of the *transport* the same way
//! `lbl-client-delivery` factors print handshakes.
//!
//! The session never blocks and never measures wall-clock time (`wasm32`-safe).

use std::collections::VecDeque;

use lbl_core::printer::Protocol;
use lbl_driver_niimbot::ble::packet_payload_for_command;
use lbl_driver_niimbot::live_status::{
    self, NiimbotDeviceInfo, NiimbotHeartbeat, NiimbotPrintProgress, NiimbotRfidInfo,
};

use crate::dymo_lw::{self, apply_engine_version, apply_sku_info, Lw550EngineVersion};
use crate::{
    parse_brother_pt_status, parse_brother_ql_status, parse_status, parse_zpl_host_status,
    status_query_bytes, status_reply_len, PrintStatus, StatusError,
    DYMO_LW_ENGINE_VERSION_REPLY_LEN, DYMO_LW_SKU_INFO_REPLY_LEN, DYMO_LW_STATUS_REPLY_LEN,
};

const STATUS_IO_TIMEOUT_MS: u32 = 6_000;
const NIIMBOT_QUERY_TIMEOUT_MS: u32 = 2_500;

const RFID_INFO_RESPONSE: u8 = 0x1b;
const RFID_INFO2_RESPONSE: u8 = 0x1d;
const HEARTBEAT_RESPONSES: &[u8] = &[0xd9, 0xdd, 0xde];
const PRINT_STATUS_RESPONSE: u8 = 0xb3;
const PRINTER_MODEL_ID_RESPONSE: u8 = 0x48;
const PRINTER_SOFTWARE_VERSION_RESPONSE: u8 = 0x49;
const PRINTER_SERIAL_NUMBER_RESPONSE: u8 = 0x4b;
const PRINTER_HARDWARE_VERSION_RESPONSE: u8 = 0x4c;

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
        context: StatusSessionContextView,
    },
    /// Status query failed.
    Error {
        /// Diagnostic message (not UI copy).
        message: String,
    },
}

fn status_context_is_default(ctx: &StatusSessionContextView) -> bool {
    ctx.engine_version.is_none()
        && ctx.model_id.is_none()
        && ctx.device_info.is_none()
        && !ctx.skip_engine_version
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
}

impl From<&StatusSessionContext> for StatusSessionContextView {
    fn from(ctx: &StatusSessionContext) -> Self {
        Self {
            engine_version: ctx.cached_engine_version.clone(),
            skip_engine_version: ctx.skip_engine_version,
            model_id: ctx.cached_model_id,
            device_info: ctx.cached_device_info.clone(),
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

enum Awaiting {
    SendComplete,
    Rx,
    Finished,
}

enum Kind {
    SingleShot {
        protocol: Protocol,
    },
    Dymo {
        phase: DymoPhase,
        engine: Option<Lw550EngineVersion>,
        skip_engine: bool,
        esc_a: Option<Vec<u8>>,
    },
    Niimbot {
        phase: NiimbotPhase,
        is_b1: bool,
        model_id: Option<u16>,
        device_info: NiimbotDeviceInfo,
        rfid: Option<NiimbotRfidInfo>,
        heartbeat: Option<NiimbotHeartbeat>,
        print_status: Option<NiimbotPrintProgress>,
        heartbeat_attempts: u8,
        rfid_try_second: bool,
    },
}

#[derive(Clone, Copy)]
enum DymoPhase {
    EngineSent,
    EngineRecv,
    StatusSent,
    StatusRecv,
    SkuSent,
    SkuRecv,
}

#[derive(Clone, Copy)]
enum NiimbotPhase {
    ModelSent,
    ModelRecv,
    FwSent,
    FwRecv,
    HwSent,
    HwRecv,
    SnSent,
    SnRecv,
    RfidSent,
    RfidRecv,
    HeartbeatSent,
    HeartbeatRecv,
    PrintSent,
    PrintRecv,
}

/// Pure state machine for a multi-probe status query.
pub struct ClientStatusSession {
    kind: Kind,
    queue: VecDeque<StatusAction>,
    awaiting: Awaiting,
    finished: bool,
    context: StatusSessionContext,
    pending_match_cmds: Vec<u8>,
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

        let kind = match protocol {
            Protocol::DymoLw => Kind::Dymo {
                phase: if context.cached_engine_version.is_some() || context.skip_engine_version {
                    DymoPhase::StatusSent
                } else {
                    DymoPhase::EngineSent
                },
                engine: context.cached_engine_version.clone(),
                skip_engine: context.skip_engine_version,
                esc_a: None,
            },
            Protocol::Niimbot => {
                let is_b1 = context
                    .driver_variant
                    .as_deref()
                    .is_some_and(|v| v.eq_ignore_ascii_case("b1"));
                let device_info = context.cached_device_info.clone().unwrap_or_default();
                let have_info = device_info_has_fields(&device_info);
                let phase = if context.cached_model_id.is_none() {
                    NiimbotPhase::ModelSent
                } else if !have_info {
                    NiimbotPhase::FwSent
                } else {
                    NiimbotPhase::RfidSent
                };
                Kind::Niimbot {
                    phase,
                    is_b1,
                    model_id: context.cached_model_id,
                    device_info,
                    rfid: None,
                    heartbeat: None,
                    print_status: None,
                    heartbeat_attempts: 0,
                    rfid_try_second: false,
                }
            }
            other => Kind::SingleShot { protocol: other },
        };

        let mut session = Self {
            kind,
            queue: VecDeque::new(),
            awaiting: Awaiting::Finished,
            finished: false,
            context,
            pending_match_cmds: Vec::new(),
        };
        let batch = session.bootstrap()?;
        session.enqueue(batch)?;
        let action = session.pump();
        Ok((session, action))
    }

    pub fn on_send_complete(&mut self) -> Result<StatusAction, StatusSessionError> {
        self.require_awaiting(Awaiting::SendComplete, "on_send_complete")?;
        let batch = self.advance_after_send()?;
        self.enqueue(batch)?;
        Ok(self.pump())
    }

    pub fn feed_rx(&mut self, bytes: &[u8]) -> Result<StatusAction, StatusSessionError> {
        self.require_awaiting(Awaiting::Rx, "feed_rx")?;
        let batch = self.advance_after_rx(bytes)?;
        self.enqueue(batch)?;
        Ok(self.pump())
    }

    fn bootstrap(&mut self) -> Result<Vec<StatusAction>, StatusSessionError> {
        match &self.kind {
            Kind::SingleShot { protocol } => {
                let bytes = status_query_bytes(*protocol)?;
                Ok(vec![StatusAction::Send { bytes }])
            }
            Kind::Dymo { phase, .. } => match phase {
                DymoPhase::EngineSent => Ok(vec![StatusAction::Send {
                    bytes: dymo_lw::engine_version_request().to_vec(),
                }]),
                DymoPhase::StatusSent => Ok(vec![StatusAction::Send {
                    bytes: dymo_lw::status_request(dymo_lw::LOCK_RELEASE).to_vec(),
                }]),
                _ => Err(StatusSessionError::Usage(
                    "dymo status session bootstrap in unexpected phase".into(),
                )),
            },
            Kind::Niimbot {
                phase,
                is_b1,
                rfid_try_second,
                ..
            } => Ok(vec![StatusAction::Send {
                bytes: niimbot_query_bytes(*phase, *is_b1, *rfid_try_second),
            }]),
        }
    }

    fn advance_after_send(&mut self) -> Result<Vec<StatusAction>, StatusSessionError> {
        match &mut self.kind {
            Kind::SingleShot { protocol } => {
                let min_len = status_reply_len(*protocol).unwrap_or(1);
                Ok(vec![StatusAction::Recv {
                    min_len,
                    timeout_ms: STATUS_IO_TIMEOUT_MS,
                    match_cmds: Vec::new(),
                }])
            }
            Kind::Dymo { phase, .. } => {
                let (next, min_len) = match *phase {
                    DymoPhase::EngineSent => {
                        *phase = DymoPhase::EngineRecv;
                        (DymoPhase::EngineRecv, DYMO_LW_ENGINE_VERSION_REPLY_LEN)
                    }
                    DymoPhase::StatusSent => {
                        *phase = DymoPhase::StatusRecv;
                        (DymoPhase::StatusRecv, DYMO_LW_STATUS_REPLY_LEN)
                    }
                    DymoPhase::SkuSent => {
                        *phase = DymoPhase::SkuRecv;
                        (DymoPhase::SkuRecv, DYMO_LW_SKU_INFO_REPLY_LEN)
                    }
                    _ => {
                        return Err(StatusSessionError::Usage(
                            "dymo status send in unexpected phase".into(),
                        ))
                    }
                };
                let _ = next;
                Ok(vec![StatusAction::Recv {
                    min_len,
                    timeout_ms: STATUS_IO_TIMEOUT_MS,
                    match_cmds: Vec::new(),
                }])
            }
            Kind::Niimbot {
                phase,
                is_b1,
                rfid_try_second,
                ..
            } => {
                let cmds = niimbot_match_cmds(*phase, *rfid_try_second);
                let next = match *phase {
                    NiimbotPhase::ModelSent => NiimbotPhase::ModelRecv,
                    NiimbotPhase::FwSent => NiimbotPhase::FwRecv,
                    NiimbotPhase::HwSent => NiimbotPhase::HwRecv,
                    NiimbotPhase::SnSent => NiimbotPhase::SnRecv,
                    NiimbotPhase::RfidSent => NiimbotPhase::RfidRecv,
                    NiimbotPhase::HeartbeatSent => NiimbotPhase::HeartbeatRecv,
                    NiimbotPhase::PrintSent => NiimbotPhase::PrintRecv,
                    _ => {
                        return Err(StatusSessionError::Usage(
                            "niimbot status send in unexpected phase".into(),
                        ))
                    }
                };
                let _ = is_b1;
                *phase = next;
                Ok(vec![StatusAction::Recv {
                    min_len: 0,
                    timeout_ms: NIIMBOT_QUERY_TIMEOUT_MS,
                    match_cmds: cmds,
                }])
            }
        }
    }

    fn advance_after_rx(&mut self, bytes: &[u8]) -> Result<Vec<StatusAction>, StatusSessionError> {
        match &mut self.kind {
            Kind::SingleShot { protocol } => {
                let status = parse_single(*protocol, bytes)?;
                Ok(vec![StatusAction::Done {
                    status: Box::new(status),
                    context: StatusSessionContextView::from(&self.context),
                }])
            }
            Kind::Dymo {
                phase,
                engine,
                skip_engine,
                esc_a,
            } => match *phase {
                DymoPhase::EngineRecv => {
                    match dymo_lw::parse_engine_version(bytes) {
                        Ok(v) => {
                            *engine = Some(v.clone());
                            self.context.cached_engine_version = Some(v);
                            self.context.skip_engine_version = false;
                        }
                        Err(_) => {
                            *skip_engine = true;
                            self.context.skip_engine_version = true;
                        }
                    }
                    *phase = DymoPhase::StatusSent;
                    Ok(vec![StatusAction::Send {
                        bytes: dymo_lw::status_request(dymo_lw::LOCK_RELEASE).to_vec(),
                    }])
                }
                DymoPhase::StatusRecv => {
                    if bytes.is_empty() {
                        return Ok(vec![StatusAction::Error {
                            message: "dymo-lw status reply timed out".into(),
                        }]);
                    }
                    *esc_a = Some(bytes.to_vec());
                    let mut status = dymo_lw::parse_print_status(bytes)?;
                    if let Some(eng) = engine.clone() {
                        apply_engine_version(&mut status, &eng);
                    }
                    let bay = status.main_bay_status_code;
                    if media_likely_present(bay) {
                        *phase = DymoPhase::SkuSent;
                        Ok(vec![StatusAction::Send {
                            bytes: dymo_lw::sku_info_request().to_vec(),
                        }])
                    } else {
                        Ok(vec![StatusAction::Done {
                            status: Box::new(PrintStatus::DymoLw(status.to_view())),
                            context: StatusSessionContextView::from(&self.context),
                        }])
                    }
                }
                DymoPhase::SkuRecv => {
                    let raw = esc_a
                        .clone()
                        .ok_or_else(|| StatusSessionError::Usage("missing ESC A buffer".into()))?;
                    let mut status = dymo_lw::parse_print_status(&raw)?;
                    if let Some(eng) = engine.clone() {
                        apply_engine_version(&mut status, &eng);
                    }
                    if !bytes.is_empty() {
                        if let Ok(info) = dymo_lw::parse_sku_info(bytes) {
                            apply_sku_info(&mut status, &info);
                        }
                    }
                    Ok(vec![StatusAction::Done {
                        status: Box::new(PrintStatus::DymoLw(status.to_view())),
                        context: StatusSessionContextView::from(&self.context),
                    }])
                }
                _ => Err(StatusSessionError::Usage(
                    "dymo status rx in unexpected phase".into(),
                )),
            },
            Kind::Niimbot { .. } => self.advance_niimbot_rx(bytes),
        }
    }

    fn advance_niimbot_rx(
        &mut self,
        bytes: &[u8],
    ) -> Result<Vec<StatusAction>, StatusSessionError> {
        let pending = self.pending_match_cmds.clone();
        let (matched_cmd, payload) = if bytes.is_empty() {
            (None, None)
        } else {
            match extract_matching_payload(bytes, &pending) {
                Some((cmd, data)) => (Some(cmd), Some(data)),
                None => (None, None),
            }
        };

        let Kind::Niimbot {
            phase,
            is_b1,
            model_id,
            device_info,
            rfid,
            heartbeat,
            print_status,
            heartbeat_attempts,
            rfid_try_second,
            ..
        } = &mut self.kind
        else {
            unreachable!()
        };

        let next_phase = match *phase {
            NiimbotPhase::ModelRecv => {
                if let Some(data) = payload.as_deref() {
                    if let Some(id) = live_status::parse_model_id_payload(data) {
                        *model_id = Some(id);
                        self.context.cached_model_id = Some(id);
                    }
                }
                if device_info_has_fields(device_info) {
                    NiimbotPhase::RfidSent
                } else {
                    NiimbotPhase::FwSent
                }
            }
            NiimbotPhase::FwRecv => {
                if let Some(data) = payload.as_deref() {
                    device_info.firmware_version = live_status::parse_version_payload(data);
                }
                NiimbotPhase::HwSent
            }
            NiimbotPhase::HwRecv => {
                if let Some(data) = payload.as_deref() {
                    device_info.hardware_version = live_status::parse_version_payload(data);
                }
                NiimbotPhase::SnSent
            }
            NiimbotPhase::SnRecv => {
                if let Some(data) = payload.as_deref() {
                    device_info.serial = live_status::parse_device_serial_payload(data);
                }
                if device_info_has_fields(device_info) {
                    self.context.cached_device_info = Some(device_info.clone());
                }
                NiimbotPhase::RfidSent
            }
            NiimbotPhase::RfidRecv => {
                if let Some(data) = payload.as_deref() {
                    if let Some(parsed) = live_status::parse_rfid_payload(data) {
                        if parsed.total_len > 0 {
                            *rfid = Some(parsed);
                        } else {
                            rfid.get_or_insert(parsed);
                        }
                    }
                }
                if rfid.as_ref().is_some_and(|r| r.total_len > 0) || *rfid_try_second {
                    NiimbotPhase::HeartbeatSent
                } else {
                    *rfid_try_second = true;
                    NiimbotPhase::RfidSent
                }
            }
            NiimbotPhase::HeartbeatRecv => {
                *heartbeat_attempts += 1;
                if let (Some(data), Some(cmd)) = (payload.as_deref(), matched_cmd) {
                    let parsed = live_status::parse_heartbeat_payload(data, Some(cmd), *model_id);
                    if live_status::heartbeat_has_fields(&parsed) {
                        *heartbeat = Some(parsed);
                    }
                }
                if heartbeat.is_some() || *heartbeat_attempts >= 2 {
                    NiimbotPhase::PrintSent
                } else {
                    NiimbotPhase::HeartbeatSent
                }
            }
            NiimbotPhase::PrintRecv => {
                if let Some(data) = payload.as_deref() {
                    *print_status = live_status::parse_print_progress_payload(data);
                }
                let live = live_status::assemble_live_status(
                    rfid.clone(),
                    *heartbeat,
                    *print_status,
                    if device_info_has_fields(device_info) {
                        Some(device_info.clone())
                    } else {
                        None
                    },
                );
                return Ok(vec![StatusAction::Done {
                    status: Box::new(PrintStatus::Niimbot(live)),
                    context: StatusSessionContextView::from(&self.context),
                }]);
            }
            _ => {
                return Err(StatusSessionError::Usage(
                    "niimbot status rx in unexpected phase".into(),
                ))
            }
        };
        let is_b1 = *is_b1;
        let rfid_try_second = *rfid_try_second;
        *phase = next_phase;
        Ok(vec![StatusAction::Send {
            bytes: niimbot_query_bytes(next_phase, is_b1, rfid_try_second),
        }])
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
        if let StatusAction::Recv { match_cmds, .. } = &action {
            self.pending_match_cmds = match_cmds.clone();
        } else {
            self.pending_match_cmds.clear();
        }
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

fn niimbot_match_cmds(phase: NiimbotPhase, rfid_try_second: bool) -> Vec<u8> {
    match phase {
        NiimbotPhase::ModelSent | NiimbotPhase::ModelRecv => vec![PRINTER_MODEL_ID_RESPONSE],
        NiimbotPhase::FwSent | NiimbotPhase::FwRecv => vec![PRINTER_SOFTWARE_VERSION_RESPONSE],
        NiimbotPhase::HwSent | NiimbotPhase::HwRecv => vec![PRINTER_HARDWARE_VERSION_RESPONSE],
        NiimbotPhase::SnSent | NiimbotPhase::SnRecv => vec![PRINTER_SERIAL_NUMBER_RESPONSE],
        NiimbotPhase::RfidSent | NiimbotPhase::RfidRecv => {
            if rfid_try_second {
                vec![RFID_INFO2_RESPONSE]
            } else {
                vec![RFID_INFO_RESPONSE]
            }
        }
        NiimbotPhase::HeartbeatSent | NiimbotPhase::HeartbeatRecv => HEARTBEAT_RESPONSES.to_vec(),
        NiimbotPhase::PrintSent | NiimbotPhase::PrintRecv => vec![PRINT_STATUS_RESPONSE],
    }
}

fn niimbot_query_bytes(phase: NiimbotPhase, is_b1: bool, rfid_try_second: bool) -> Vec<u8> {
    match phase {
        NiimbotPhase::ModelSent => live_status::printer_model_id_query(),
        NiimbotPhase::FwSent => live_status::printer_software_version_query(),
        NiimbotPhase::HwSent => live_status::printer_hardware_version_query(),
        NiimbotPhase::SnSent => live_status::printer_serial_number_query(),
        NiimbotPhase::RfidSent => {
            if rfid_try_second {
                live_status::rfid_info2_query()
            } else {
                live_status::rfid_info_query()
            }
        }
        NiimbotPhase::HeartbeatSent => {
            let variant = if is_b1 { 0x04 } else { 0x01 };
            live_status::heartbeat_query(variant)
        }
        NiimbotPhase::PrintSent => live_status::print_progress_query(),
        _ => live_status::print_progress_query(),
    }
}

fn media_likely_present(bay_code: u8) -> bool {
    (4..=10).contains(&bay_code)
}

fn device_info_has_fields(info: &NiimbotDeviceInfo) -> bool {
    info.firmware_version.is_some() || info.hardware_version.is_some() || info.serial.is_some()
}

fn extract_matching_payload(buffer: &[u8], cmds: &[u8]) -> Option<(u8, Vec<u8>)> {
    for &cmd in cmds {
        if let Some((payload, _)) = packet_payload_for_command(buffer, cmd) {
            return Some((cmd, payload));
        }
    }
    None
}

fn parse_single(protocol: Protocol, bytes: &[u8]) -> Result<PrintStatus, StatusSessionError> {
    match protocol {
        Protocol::BrotherQl => Ok(PrintStatus::BrotherQl(parse_brother_ql_status(bytes)?)),
        Protocol::BrotherPt => Ok(PrintStatus::BrotherPt(parse_brother_pt_status(bytes)?)),
        Protocol::Zpl => Ok(PrintStatus::Zpl(parse_zpl_host_status(bytes)?)),
        Protocol::Gpgl => parse_status(protocol, bytes).map_err(Into::into),
        Protocol::DymoLw | Protocol::Niimbot => Err(StatusSessionError::Usage(
            "multi-probe protocol reached single-shot parser".into(),
        )),
        other => Err(StatusError::Parse(format!(
            "status parsing not supported for protocol {other:?}"
        ))
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

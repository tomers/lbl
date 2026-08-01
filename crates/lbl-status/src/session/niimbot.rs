//! NIIMBOT multi-probe live status (model / versions / RFID / heartbeat / print).

use lbl_driver_niimbot::ble::packet_payload_for_command;
use lbl_driver_niimbot::live_status::{
    self, NiimbotDeviceInfo, NiimbotHeartbeat, NiimbotPrintProgress, NiimbotRfidInfo,
};

use crate::PrintStatus;

use super::{Probe, StatusAction, StatusSessionContext, StatusSessionError};

const QUERY_TIMEOUT_MS: u32 = 2_500;

const RFID_INFO_RESPONSE: u8 = 0x1b;
const RFID_INFO2_RESPONSE: u8 = 0x1d;
const HEARTBEAT_RESPONSES: &[u8] = &[0xd9, 0xdd, 0xde];
const PRINT_STATUS_RESPONSE: u8 = 0xb3;
const PRINTER_MODEL_ID_RESPONSE: u8 = 0x48;
const PRINTER_SOFTWARE_VERSION_RESPONSE: u8 = 0x49;
const PRINTER_SERIAL_NUMBER_RESPONSE: u8 = 0x4b;
const PRINTER_HARDWARE_VERSION_RESPONSE: u8 = 0x4c;

#[derive(Clone, Copy)]
enum Phase {
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

pub(super) struct Niimbot {
    phase: Phase,
    is_b1: bool,
    model_id: Option<u16>,
    device_info: NiimbotDeviceInfo,
    rfid: Option<NiimbotRfidInfo>,
    heartbeat: Option<NiimbotHeartbeat>,
    print_status: Option<NiimbotPrintProgress>,
    heartbeat_attempts: u8,
    rfid_try_second: bool,
    pending_match_cmds: Vec<u8>,
}

impl Niimbot {
    pub(super) fn new(context: &StatusSessionContext) -> Self {
        let is_b1 = context
            .driver_variant
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("b1"));
        let device_info = context.cached_device_info.clone().unwrap_or_default();
        let have_info = device_info_has_fields(&device_info);
        let phase = if context.cached_model_id.is_none() {
            Phase::ModelSent
        } else if !have_info {
            Phase::FwSent
        } else {
            Phase::RfidSent
        };
        Self {
            phase,
            is_b1,
            model_id: context.cached_model_id,
            device_info,
            rfid: None,
            heartbeat: None,
            print_status: None,
            heartbeat_attempts: 0,
            rfid_try_second: false,
            pending_match_cmds: Vec::new(),
        }
    }
}

impl Probe for Niimbot {
    fn bootstrap(&mut self) -> Result<Vec<StatusAction>, StatusSessionError> {
        Ok(vec![StatusAction::send(query_bytes(
            self.phase,
            self.is_b1,
            self.rfid_try_second,
        ))])
    }

    fn advance_after_send(
        &mut self,
        _context: &mut StatusSessionContext,
    ) -> Result<Vec<StatusAction>, StatusSessionError> {
        let cmds = match_cmds(self.phase, self.rfid_try_second);
        let next = match self.phase {
            Phase::ModelSent => Phase::ModelRecv,
            Phase::FwSent => Phase::FwRecv,
            Phase::HwSent => Phase::HwRecv,
            Phase::SnSent => Phase::SnRecv,
            Phase::RfidSent => Phase::RfidRecv,
            Phase::HeartbeatSent => Phase::HeartbeatRecv,
            Phase::PrintSent => Phase::PrintRecv,
            _ => {
                return Err(StatusSessionError::Usage(
                    "niimbot status send in unexpected phase".into(),
                ))
            }
        };
        self.phase = next;
        self.pending_match_cmds = cmds.clone();
        Ok(vec![StatusAction::recv_match(QUERY_TIMEOUT_MS, cmds)])
    }

    fn advance_after_rx(
        &mut self,
        bytes: &[u8],
        context: &mut StatusSessionContext,
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

        let next_phase = match self.phase {
            Phase::ModelRecv => {
                if let Some(data) = payload.as_deref() {
                    if let Some(id) = live_status::parse_model_id_payload(data) {
                        self.model_id = Some(id);
                        context.cached_model_id = Some(id);
                    }
                }
                if device_info_has_fields(&self.device_info) {
                    Phase::RfidSent
                } else {
                    Phase::FwSent
                }
            }
            Phase::FwRecv => {
                if let Some(data) = payload.as_deref() {
                    self.device_info.firmware_version = live_status::parse_version_payload(data);
                }
                Phase::HwSent
            }
            Phase::HwRecv => {
                if let Some(data) = payload.as_deref() {
                    self.device_info.hardware_version = live_status::parse_version_payload(data);
                }
                Phase::SnSent
            }
            Phase::SnRecv => {
                if let Some(data) = payload.as_deref() {
                    self.device_info.serial = live_status::parse_device_serial_payload(data);
                }
                if device_info_has_fields(&self.device_info) {
                    context.cached_device_info = Some(self.device_info.clone());
                }
                Phase::RfidSent
            }
            Phase::RfidRecv => {
                if let Some(data) = payload.as_deref() {
                    if let Some(parsed) = live_status::parse_rfid_payload(data) {
                        if parsed.total_len > 0 {
                            self.rfid = Some(parsed);
                        } else {
                            self.rfid.get_or_insert(parsed);
                        }
                    }
                }
                if self.rfid.as_ref().is_some_and(|r| r.total_len > 0) || self.rfid_try_second {
                    Phase::HeartbeatSent
                } else {
                    self.rfid_try_second = true;
                    Phase::RfidSent
                }
            }
            Phase::HeartbeatRecv => {
                self.heartbeat_attempts += 1;
                if let (Some(data), Some(cmd)) = (payload.as_deref(), matched_cmd) {
                    let parsed =
                        live_status::parse_heartbeat_payload(data, Some(cmd), self.model_id);
                    if live_status::heartbeat_has_fields(&parsed) {
                        self.heartbeat = Some(parsed);
                    }
                }
                if self.heartbeat.is_some() || self.heartbeat_attempts >= 2 {
                    Phase::PrintSent
                } else {
                    Phase::HeartbeatSent
                }
            }
            Phase::PrintRecv => {
                if let Some(data) = payload.as_deref() {
                    self.print_status = live_status::parse_print_progress_payload(data);
                }
                let live = live_status::assemble_live_status(
                    self.rfid.clone(),
                    self.heartbeat,
                    self.print_status,
                    if device_info_has_fields(&self.device_info) {
                        Some(self.device_info.clone())
                    } else {
                        None
                    },
                );
                return Ok(vec![StatusAction::done(
                    PrintStatus::Niimbot(live.into()),
                    context,
                )]);
            }
            _ => {
                return Err(StatusSessionError::Usage(
                    "niimbot status rx in unexpected phase".into(),
                ))
            }
        };

        self.phase = next_phase;
        Ok(vec![StatusAction::send(query_bytes(
            next_phase,
            self.is_b1,
            self.rfid_try_second,
        ))])
    }
}

fn match_cmds(phase: Phase, rfid_try_second: bool) -> Vec<u8> {
    match phase {
        Phase::ModelSent | Phase::ModelRecv => vec![PRINTER_MODEL_ID_RESPONSE],
        Phase::FwSent | Phase::FwRecv => vec![PRINTER_SOFTWARE_VERSION_RESPONSE],
        Phase::HwSent | Phase::HwRecv => vec![PRINTER_HARDWARE_VERSION_RESPONSE],
        Phase::SnSent | Phase::SnRecv => vec![PRINTER_SERIAL_NUMBER_RESPONSE],
        Phase::RfidSent | Phase::RfidRecv => {
            if rfid_try_second {
                vec![RFID_INFO2_RESPONSE]
            } else {
                vec![RFID_INFO_RESPONSE]
            }
        }
        Phase::HeartbeatSent | Phase::HeartbeatRecv => HEARTBEAT_RESPONSES.to_vec(),
        Phase::PrintSent | Phase::PrintRecv => vec![PRINT_STATUS_RESPONSE],
    }
}

fn query_bytes(phase: Phase, is_b1: bool, rfid_try_second: bool) -> Vec<u8> {
    match phase {
        Phase::ModelSent => live_status::printer_model_id_query(),
        Phase::FwSent => live_status::printer_software_version_query(),
        Phase::HwSent => live_status::printer_hardware_version_query(),
        Phase::SnSent => live_status::printer_serial_number_query(),
        Phase::RfidSent => {
            if rfid_try_second {
                live_status::rfid_info2_query()
            } else {
                live_status::rfid_info_query()
            }
        }
        Phase::HeartbeatSent => {
            let variant = if is_b1 { 0x04 } else { 0x01 };
            live_status::heartbeat_query(variant)
        }
        Phase::PrintSent => live_status::print_progress_query(),
        _ => live_status::print_progress_query(),
    }
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

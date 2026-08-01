//! Graphtec / Silhouette GPGL identity + status probes (`FG` / `TI` / `ESC E`).

use crate::{PrintStatus, StatusError};

use super::{Probe, StatusAction, StatusSessionContext, StatusSessionError, STATUS_IO_TIMEOUT_MS};

/// `FG` firmware query — we need the reply for device info (inkscape-silhouette
/// waits up to 10s). Do **not** reuse the cut-handshake 500ms soft-drain: that
/// path discards FG content and is too short for a real read on WebUSB.
pub(super) const FIRMWARE_TIMEOUT_MS: u32 = STATUS_IO_TIMEOUT_MS;
/// Optional `TI` name query. Newer firmware replies; older stays silent — keep
/// this shorter so a missing name does not dominate the first status poll.
pub(super) const NAME_TIMEOUT_MS: u32 = 1_500;

#[derive(Clone, Copy)]
enum Phase {
    FirmwareSent,
    FirmwareRecv,
    NameSent,
    NameRecv,
    StatusSent,
    StatusRecv,
}

pub(super) struct Gpgl {
    phase: Phase,
    firmware: Option<String>,
    device_name: Option<String>,
}

impl Gpgl {
    pub(super) fn new(context: &StatusSessionContext) -> Self {
        Self {
            phase: bootstrap_phase(context),
            firmware: context.cached_gpgl_firmware.clone(),
            device_name: context.cached_gpgl_device_name.clone(),
        }
    }
}

impl Probe for Gpgl {
    fn bootstrap(&mut self) -> Result<Vec<StatusAction>, StatusSessionError> {
        Ok(vec![StatusAction::send(query_bytes(self.phase))])
    }

    fn advance_after_send(
        &mut self,
        _context: &mut StatusSessionContext,
    ) -> Result<Vec<StatusAction>, StatusSessionError> {
        let timeout_ms = match self.phase {
            Phase::FirmwareSent => {
                self.phase = Phase::FirmwareRecv;
                FIRMWARE_TIMEOUT_MS
            }
            Phase::NameSent => {
                self.phase = Phase::NameRecv;
                NAME_TIMEOUT_MS
            }
            Phase::StatusSent => {
                self.phase = Phase::StatusRecv;
                STATUS_IO_TIMEOUT_MS
            }
            _ => {
                return Err(StatusSessionError::Usage(
                    "gpgl status send in unexpected phase".into(),
                ))
            }
        };
        Ok(vec![StatusAction::recv(1, timeout_ms)])
    }

    fn advance_after_rx(
        &mut self,
        bytes: &[u8],
        context: &mut StatusSessionContext,
    ) -> Result<Vec<StatusAction>, StatusSessionError> {
        match self.phase {
            Phase::FirmwareRecv => {
                match lbl_driver_gpgl::parse_identity_reply(bytes) {
                    Some(v) => {
                        self.firmware = Some(v.clone());
                        context.cached_gpgl_firmware = Some(v);
                        context.skip_gpgl_firmware = false;
                    }
                    None => {
                        context.skip_gpgl_firmware = true;
                    }
                }
                if context.cached_gpgl_device_name.is_some() || context.skip_gpgl_device_name {
                    self.phase = Phase::StatusSent;
                    Ok(vec![StatusAction::send(
                        lbl_driver_gpgl::STATUS_QUERY.to_vec(),
                    )])
                } else {
                    self.phase = Phase::NameSent;
                    Ok(vec![StatusAction::send(
                        lbl_driver_gpgl::device_name_query(),
                    )])
                }
            }
            Phase::NameRecv => {
                match lbl_driver_gpgl::parse_identity_reply(bytes) {
                    Some(v) => {
                        self.device_name = Some(v.clone());
                        context.cached_gpgl_device_name = Some(v);
                        context.skip_gpgl_device_name = false;
                    }
                    None => {
                        context.skip_gpgl_device_name = true;
                    }
                }
                self.phase = Phase::StatusSent;
                Ok(vec![StatusAction::send(
                    lbl_driver_gpgl::STATUS_QUERY.to_vec(),
                )])
            }
            Phase::StatusRecv => {
                if bytes.is_empty() {
                    return Ok(vec![StatusAction::error("gpgl status reply timed out")]);
                }
                let state = lbl_driver_gpgl::parse_status(bytes).ok_or_else(|| {
                    StatusError::Parse("unrecognized GPGL status response".into())
                })?;
                let host = crate::GpglHostStatus::from(state);
                let view = crate::GpglStatusView {
                    state: host,
                    readiness: host.readiness(),
                    firmware_version: self.firmware.clone(),
                    device_name: self.device_name.clone(),
                };
                Ok(vec![StatusAction::done(PrintStatus::Gpgl(view), context)])
            }
            _ => Err(StatusSessionError::Usage(
                "gpgl status rx in unexpected phase".into(),
            )),
        }
    }
}

fn bootstrap_phase(context: &StatusSessionContext) -> Phase {
    if context.cached_gpgl_firmware.is_none() && !context.skip_gpgl_firmware {
        Phase::FirmwareSent
    } else if context.cached_gpgl_device_name.is_none() && !context.skip_gpgl_device_name {
        Phase::NameSent
    } else {
        Phase::StatusSent
    }
}

fn query_bytes(phase: Phase) -> Vec<u8> {
    match phase {
        Phase::FirmwareSent => lbl_driver_gpgl::firmware_query(),
        Phase::NameSent => lbl_driver_gpgl::device_name_query(),
        Phase::StatusSent => lbl_driver_gpgl::STATUS_QUERY.to_vec(),
        _ => lbl_driver_gpgl::STATUS_QUERY.to_vec(),
    }
}

//! DYMO LabelWriter multi-probe status (`ESC V` → `ESC A` → optional `ESC U`).

use crate::dymo_lw::{self, apply_engine_version, apply_sku_info, Lw550EngineVersion};
use crate::{
    PrintStatus, DYMO_LW_ENGINE_VERSION_REPLY_LEN, DYMO_LW_SKU_INFO_REPLY_LEN,
    DYMO_LW_STATUS_REPLY_LEN,
};

use super::{Probe, StatusAction, StatusSessionContext, StatusSessionError, STATUS_IO_TIMEOUT_MS};

#[derive(Clone, Copy)]
enum Phase {
    EngineSent,
    EngineRecv,
    StatusSent,
    StatusRecv,
    SkuSent,
    SkuRecv,
}

pub(super) struct Dymo {
    phase: Phase,
    engine: Option<Lw550EngineVersion>,
    esc_a: Option<Vec<u8>>,
}

impl Dymo {
    pub(super) fn new(context: &StatusSessionContext) -> Self {
        let phase = if context.cached_engine_version.is_some() || context.skip_engine_version {
            Phase::StatusSent
        } else {
            Phase::EngineSent
        };
        Self {
            phase,
            engine: context.cached_engine_version.clone(),
            esc_a: None,
        }
    }
}

impl Probe for Dymo {
    fn bootstrap(&mut self) -> Result<Vec<StatusAction>, StatusSessionError> {
        match self.phase {
            Phase::EngineSent => Ok(vec![StatusAction::send(
                dymo_lw::engine_version_request().to_vec(),
            )]),
            Phase::StatusSent => Ok(vec![StatusAction::send(
                dymo_lw::status_request(dymo_lw::LOCK_RELEASE).to_vec(),
            )]),
            _ => Err(StatusSessionError::Usage(
                "dymo status session bootstrap in unexpected phase".into(),
            )),
        }
    }

    fn advance_after_send(
        &mut self,
        _context: &mut StatusSessionContext,
    ) -> Result<Vec<StatusAction>, StatusSessionError> {
        let min_len = match self.phase {
            Phase::EngineSent => {
                self.phase = Phase::EngineRecv;
                DYMO_LW_ENGINE_VERSION_REPLY_LEN
            }
            Phase::StatusSent => {
                self.phase = Phase::StatusRecv;
                DYMO_LW_STATUS_REPLY_LEN
            }
            Phase::SkuSent => {
                self.phase = Phase::SkuRecv;
                DYMO_LW_SKU_INFO_REPLY_LEN
            }
            _ => {
                return Err(StatusSessionError::Usage(
                    "dymo status send in unexpected phase".into(),
                ))
            }
        };
        Ok(vec![StatusAction::recv(min_len, STATUS_IO_TIMEOUT_MS)])
    }

    fn advance_after_rx(
        &mut self,
        bytes: &[u8],
        context: &mut StatusSessionContext,
    ) -> Result<Vec<StatusAction>, StatusSessionError> {
        match self.phase {
            Phase::EngineRecv => {
                match dymo_lw::parse_engine_version(bytes) {
                    Ok(v) => {
                        self.engine = Some(v.clone());
                        context.cached_engine_version = Some(v);
                        context.skip_engine_version = false;
                    }
                    Err(_) => {
                        context.skip_engine_version = true;
                    }
                }
                self.phase = Phase::StatusSent;
                Ok(vec![StatusAction::send(
                    dymo_lw::status_request(dymo_lw::LOCK_RELEASE).to_vec(),
                )])
            }
            Phase::StatusRecv => {
                if bytes.is_empty() {
                    return Ok(vec![StatusAction::error("dymo-lw status reply timed out")]);
                }
                self.esc_a = Some(bytes.to_vec());
                let mut status = dymo_lw::parse_print_status(bytes)?;
                if let Some(eng) = self.engine.clone() {
                    apply_engine_version(&mut status, &eng);
                }
                let bay = status.main_bay_status;
                if dymo_lw::media_likely_present(bay) {
                    self.phase = Phase::SkuSent;
                    Ok(vec![StatusAction::send(
                        dymo_lw::sku_info_request().to_vec(),
                    )])
                } else {
                    Ok(vec![StatusAction::done(
                        PrintStatus::DymoLw(status.to_view()),
                        context,
                    )])
                }
            }
            Phase::SkuRecv => {
                let raw = self
                    .esc_a
                    .clone()
                    .ok_or_else(|| StatusSessionError::Usage("missing ESC A buffer".into()))?;
                let mut status = dymo_lw::parse_print_status(&raw)?;
                if let Some(eng) = self.engine.clone() {
                    apply_engine_version(&mut status, &eng);
                }
                if !bytes.is_empty() {
                    if let Ok(info) = dymo_lw::parse_sku_info(bytes) {
                        apply_sku_info(&mut status, &info);
                    }
                }
                Ok(vec![StatusAction::done(
                    PrintStatus::DymoLw(status.to_view()),
                    context,
                )])
            }
            _ => Err(StatusSessionError::Usage(
                "dymo status rx in unexpected phase".into(),
            )),
        }
    }
}

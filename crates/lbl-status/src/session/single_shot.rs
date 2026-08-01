//! Single request/reply status probes (Brother QL/PT, ZPL, …).

use lbl_core::printer::Protocol;

use crate::{
    parse_brother_pt_status, parse_brother_ql_status, parse_zpl_host_status, status_query_bytes,
    status_reply_len, PrintStatus, StatusError,
};

use super::{Probe, StatusAction, StatusSessionContext, StatusSessionError, STATUS_IO_TIMEOUT_MS};

pub(super) struct SingleShot {
    protocol: Protocol,
}

impl SingleShot {
    pub(super) fn new(protocol: Protocol) -> Self {
        Self { protocol }
    }
}

impl Probe for SingleShot {
    fn bootstrap(&mut self) -> Result<Vec<StatusAction>, StatusSessionError> {
        let bytes = status_query_bytes(self.protocol)?;
        Ok(vec![StatusAction::send(bytes)])
    }

    fn advance_after_send(
        &mut self,
        _context: &mut StatusSessionContext,
    ) -> Result<Vec<StatusAction>, StatusSessionError> {
        let min_len = status_reply_len(self.protocol).unwrap_or(1);
        Ok(vec![StatusAction::recv(min_len, STATUS_IO_TIMEOUT_MS)])
    }

    fn advance_after_rx(
        &mut self,
        bytes: &[u8],
        context: &mut StatusSessionContext,
    ) -> Result<Vec<StatusAction>, StatusSessionError> {
        let status = parse_single(self.protocol, bytes)?;
        Ok(vec![StatusAction::done(status, context)])
    }
}

fn parse_single(protocol: Protocol, bytes: &[u8]) -> Result<PrintStatus, StatusSessionError> {
    match protocol {
        Protocol::BrotherQl => Ok(PrintStatus::BrotherQl(parse_brother_ql_status(bytes)?)),
        Protocol::BrotherPt => Ok(PrintStatus::BrotherPt(parse_brother_pt_status(bytes)?)),
        Protocol::Zpl => Ok(PrintStatus::Zpl(parse_zpl_host_status(bytes)?)),
        Protocol::DymoLw | Protocol::Niimbot | Protocol::Gpgl => Err(StatusSessionError::Usage(
            "multi-probe protocol reached single-shot parser".into(),
        )),
        other => Err(StatusError::Parse(format!(
            "status parsing not supported for protocol {other:?}"
        ))
        .into()),
    }
}

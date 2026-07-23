//! Wasm-safe print-engine status: query bytes, reply parsing, and soft-reboot
//! commands.
//!
//! This crate is transport-agnostic — it never opens a device. It produces the
//! request byte sequences to send to a printer, parses the fixed replies into
//! typed status structs, and exposes the soft-reboot command bytes. Callers own
//! the transport (USB bulk, serial, BLE, network) and map [`StatusError`] onto
//! their own error type at the I/O boundary.

mod brother;
mod brother_pt;
mod brother_ql;
pub mod dymo_lw;
mod error;
mod session;
mod zpl;

use lbl_core::printer::Protocol;

pub use brother::{BrotherPhaseType, BrotherSeverity, BrotherStatusSummary, BrotherStatusType};
pub use brother_pt::{
    media_key_hint as brother_pt_media_key_hint, parse_status as parse_brother_pt_status,
    status_summary as brother_pt_status_summary, BrotherPtError, BrotherPtMediaType,
    BrotherPtStatus, STATUS_REPLY_LEN as BROTHER_PT_STATUS_REPLY_LEN,
    STATUS_REQUEST as BROTHER_PT_STATUS_REQUEST,
};
pub use brother_ql::{
    media_key_hint as brother_ql_media_key_hint, parse_status as parse_brother_ql_status,
    status_summary as brother_ql_status_summary, BrotherQlError, BrotherQlMediaType,
    BrotherQlStatus, STATUS_REPLY_LEN as BROTHER_QL_STATUS_REPLY_LEN,
    STATUS_REQUEST as BROTHER_QL_STATUS_REQUEST,
};
pub use dymo_lw::{
    apply_engine_version, apply_sku_info, bay_is_ok, media_likely_present, merge_dymo_lw_status,
    merge_dymo_lw_status_view, parse_engine_version, parse_print_status, parse_sku_info,
    print_job_active, soft_reboot_request as dymo_lw_soft_reboot_request, Lw550EngineVersion,
    Lw550MainBayStatus, Lw550PrintEngineStatus, Lw550PrintHeadStatus, Lw550PrintHeadVoltage,
    Lw550PrintStatus, Lw550PrintStatusView, Lw550SkuInfo,
    ENGINE_VERSION_REPLY_LEN as DYMO_LW_ENGINE_VERSION_REPLY_LEN,
    SKU_INFO_REPLY_LEN as DYMO_LW_SKU_INFO_REPLY_LEN, STATUS_REPLY_LEN as DYMO_LW_STATUS_REPLY_LEN,
};
pub use error::StatusError;
pub use session::{
    ClientStatusSession, StatusAction, StatusSessionContext, StatusSessionContextView,
    StatusSessionError,
};
pub use zpl::{parse_host_status as parse_zpl_host_status, ZplHostStatus, HOST_STATUS_CMD};

/// NIIMBOT live-status protocol module (query builders + payload parsers +
/// assembly/merge), re-exported from the driver so WASM/host callers reach it
/// through the unified status facade.
pub use lbl_driver_niimbot::live_status as niimbot;
pub use lbl_driver_niimbot::live_status::NiimbotLiveStatus;

/// Unified, protocol-tagged print-engine status for APIs and WASM JSON.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "protocol", rename_all = "kebab-case")]
pub enum PrintStatus {
    /// DYMO LabelWriter 550-series (`dymo-lw`).
    #[serde(rename = "dymo-lw")]
    DymoLw(Lw550PrintStatusView),
    /// Brother QL-series (`brother-ql`).
    #[serde(rename = "brother-ql")]
    BrotherQl(BrotherQlStatus),
    /// Brother P-touch / TZe (`brother-pt`).
    #[serde(rename = "brother-pt")]
    BrotherPt(BrotherPtStatus),
    /// Zebra ZPL (`zpl`).
    #[serde(rename = "zpl")]
    Zpl(ZplHostStatus),
    /// NIIMBOT live status (`niimbot`): RFID, heartbeat, print progress, media,
    /// and device info.
    #[serde(rename = "niimbot")]
    Niimbot(NiimbotLiveStatus),
    /// Graphtec / Silhouette GPGL cutter (`gpgl`).
    #[serde(rename = "gpgl")]
    Gpgl(GpglStatusView),
}

/// GPGL cutter status snapshot.
///
/// A struct wrapper (rather than a bare [`GpglHostStatus`]) is required because
/// [`PrintStatus`] is internally tagged (`#[serde(tag = "protocol")]`): serde
/// can only fold the tag into a variant that serializes as a map, so a variant
/// wrapping a plain string-valued enum would fail to serialize. The `state`
/// field keeps the tagged JSON well-formed (`{ "protocol": "gpgl", "state": "ready" }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct GpglStatusView {
    /// Cutter motion / load state.
    pub state: GpglHostStatus,
}

impl From<GpglHostStatus> for GpglStatusView {
    fn from(state: GpglHostStatus) -> Self {
        Self { state }
    }
}

/// Graphtec / Silhouette GPGL cutter motion / load status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GpglHostStatus {
    Ready,
    Moving,
    Unloaded,
}

impl From<lbl_driver_gpgl::GpglStatus> for GpglHostStatus {
    fn from(status: lbl_driver_gpgl::GpglStatus) -> Self {
        match status {
            lbl_driver_gpgl::GpglStatus::Ready => Self::Ready,
            lbl_driver_gpgl::GpglStatus::Moving => Self::Moving,
            lbl_driver_gpgl::GpglStatus::Unloaded => Self::Unloaded,
        }
    }
}

/// Whether `protocol` supports a print-engine status query.
pub fn status_supported(protocol: Protocol) -> bool {
    matches!(
        protocol,
        Protocol::DymoLw
            | Protocol::BrotherQl
            | Protocol::BrotherPt
            | Protocol::Zpl
            | Protocol::Niimbot
            | Protocol::Gpgl
    )
}

/// Whether `protocol` supports a host soft-reboot of the print engine.
pub fn soft_reboot_supported(protocol: Protocol) -> bool {
    matches!(protocol, Protocol::DymoLw)
}

/// Bytes to send for a primary status query (single shot).
///
/// Richer flows (e.g. DYMO's `ESC A` + `ESC V` + `ESC U`) issue several
/// queries; this returns the primary one that yields a [`PrintStatus`] via
/// [`parse_status`].
pub fn status_query_bytes(protocol: Protocol) -> Result<Vec<u8>, StatusError> {
    match protocol {
        Protocol::DymoLw => Ok(dymo_lw::status_request(dymo_lw::LOCK_RELEASE).to_vec()),
        Protocol::BrotherQl => Ok(BROTHER_QL_STATUS_REQUEST.to_vec()),
        Protocol::BrotherPt => Ok(BROTHER_PT_STATUS_REQUEST.to_vec()),
        Protocol::Zpl => Ok(HOST_STATUS_CMD.to_vec()),
        Protocol::Niimbot => Ok(lbl_driver_niimbot::status_query()),
        Protocol::Gpgl => Ok(lbl_driver_gpgl::STATUS_QUERY.to_vec()),
        other => Err(StatusError::Parse(format!(
            "status query not supported for protocol {other:?}"
        ))),
    }
}

/// Expected minimum reply length for a primary status query, when fixed.
///
/// `None` when the reply is variable-length (ZPL `~HS` framing, NIIMBOT
/// packets, GPGL ASCII status).
pub fn status_reply_len(protocol: Protocol) -> Option<usize> {
    match protocol {
        Protocol::DymoLw => Some(DYMO_LW_STATUS_REPLY_LEN),
        Protocol::BrotherQl => Some(BROTHER_QL_STATUS_REPLY_LEN),
        Protocol::BrotherPt => Some(BROTHER_PT_STATUS_REPLY_LEN),
        Protocol::Zpl | Protocol::Niimbot | Protocol::Gpgl => None,
        _ => None,
    }
}

/// Parse a primary status reply into a [`PrintStatus`].
///
/// For `dymo-lw` this parses the `ESC A` reply into a view with `label_total`
/// and engine-version fields unset; callers polling `ESC U` / `ESC V` fill
/// those in and combine with [`merge_dymo_lw_status`].
pub fn parse_status(protocol: Protocol, bytes: &[u8]) -> Result<PrintStatus, StatusError> {
    match protocol {
        Protocol::DymoLw => {
            let status = parse_print_status(bytes)?;
            Ok(PrintStatus::DymoLw(status.to_view()))
        }
        Protocol::BrotherQl => Ok(PrintStatus::BrotherQl(parse_brother_ql_status(bytes)?)),
        Protocol::BrotherPt => Ok(PrintStatus::BrotherPt(parse_brother_pt_status(bytes)?)),
        Protocol::Zpl => Ok(PrintStatus::Zpl(parse_zpl_host_status(bytes)?)),
        Protocol::Niimbot => lbl_driver_niimbot::parse_status(bytes)
            .map(|s| PrintStatus::Niimbot(NiimbotLiveStatus::from(s)))
            .ok_or_else(|| StatusError::Parse("no NIIMBOT print-status reply in buffer".into())),
        Protocol::Gpgl => lbl_driver_gpgl::parse_status(bytes)
            .map(|s| PrintStatus::Gpgl(GpglHostStatus::from(s).into()))
            .ok_or_else(|| StatusError::Parse("unrecognized GPGL status response".into())),
        other => Err(StatusError::Parse(format!(
            "status parsing not supported for protocol {other:?}"
        ))),
    }
}

/// Bytes to send to soft-reboot the print engine, when supported.
pub fn soft_reboot_bytes(protocol: Protocol) -> Result<Vec<u8>, StatusError> {
    match protocol {
        Protocol::DymoLw => Ok(dymo_lw_soft_reboot_request().to_vec()),
        other => Err(StatusError::Parse(format!(
            "soft reboot not supported for protocol {other:?}"
        ))),
    }
}

/// Best-effort media key hint from a parsed status (e.g. `62`, `12`, or a SKU).
pub fn media_key_hint(status: &PrintStatus) -> Option<String> {
    match status {
        PrintStatus::BrotherQl(s) => brother_ql_media_key_hint(s),
        PrintStatus::BrotherPt(s) => brother_pt_media_key_hint(s),
        PrintStatus::DymoLw(s) => s.sku.clone(),
        PrintStatus::Niimbot(s) => niimbot_media_key_hint(s),
        PrintStatus::Zpl(_) | PrintStatus::Gpgl(_) => None,
    }
}

/// Media key hint for a NIIMBOT live status.
///
/// Prefers the RFID barcode (a catalog `product_ids` lookup key), falling back
/// to a `WIDTHxLENGTH` string when only physical dimensions are known.
fn niimbot_media_key_hint(status: &NiimbotLiveStatus) -> Option<String> {
    if let Some(barcode) = status.media_barcode.as_deref() {
        let barcode = barcode.trim();
        if !barcode.is_empty() {
            return Some(barcode.to_string());
        }
    }
    match (status.media_width_mm, status.media_length_mm) {
        (Some(w), Some(l)) => Some(format!("{w}x{l}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn niimbot_status_serializes_with_protocol_tag_and_fields() {
        let live = niimbot::assemble_live_status(
            Some(niimbot::NiimbotRfidInfo {
                uuid: "aa".into(),
                barcode: "T50X30".into(),
                serial: "R1".into(),
                total_len: 100,
                used_len: 10,
                label_type: 1,
            }),
            Some(niimbot::NiimbotHeartbeat {
                lid_closed: Some(true),
                paper_inserted: Some(true),
                rfid_ok: Some(true),
                battery_level: Some(3),
            }),
            None,
            None,
        );
        let value = serde_json::to_value(PrintStatus::Niimbot(live)).unwrap();
        assert_eq!(value["protocol"], "niimbot");
        assert_eq!(value["media_width_mm"], 50);
        assert_eq!(value["media_length_mm"], 30);
        assert_eq!(value["rfid"]["total_len"], 100);
        assert_eq!(value["heartbeat"]["battery_level"], 3);
        assert!(value["print_status"].is_null());
        assert!(value["device_info"].is_null());
    }

    #[test]
    fn niimbot_media_key_hint_prefers_barcode_then_dimensions() {
        let with_barcode = niimbot::assemble_live_status(
            Some(niimbot::NiimbotRfidInfo {
                uuid: "aa".into(),
                barcode: "02282280".into(),
                serial: "R1".into(),
                total_len: 100,
                used_len: 10,
                label_type: 1,
            }),
            None,
            None,
            None,
        );
        assert_eq!(
            media_key_hint(&PrintStatus::Niimbot(with_barcode)).as_deref(),
            Some("02282280")
        );

        let dims_only = NiimbotLiveStatus {
            media_width_mm: Some(50),
            media_length_mm: Some(30),
            ..Default::default()
        };
        assert_eq!(
            media_key_hint(&PrintStatus::Niimbot(dims_only)).as_deref(),
            Some("50x30")
        );
    }

    #[test]
    fn gpgl_status_serializes_with_protocol_tag() {
        // Internal tagging cannot fold `protocol` into a bare string enum, so
        // the variant wraps a struct: this must serialize, not error.
        let status = PrintStatus::Gpgl(GpglHostStatus::Ready.into());
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["protocol"], "gpgl");
        assert_eq!(value["state"], "ready");
    }
}

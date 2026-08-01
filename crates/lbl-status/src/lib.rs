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
pub mod dymo_d1;
pub mod dymo_lw;
mod dymo_lw_classic;
mod error;
mod letratag;
mod readiness;
mod session;
pub mod usb_printer_id;
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
pub use dymo_d1::{
    parse_status as parse_dymo_d1_status, DymoD1Status, STATUS_READ_LEN as DYMO_D1_STATUS_READ_LEN,
    STATUS_REQUEST as DYMO_D1_STATUS_REQUEST,
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
pub use dymo_lw_classic::{
    parse_status as parse_dymo_lw_classic_status, DymoLwClassicStatus,
    STATUS_REQUEST as DYMO_LW_CLASSIC_STATUS_REQUEST,
};
pub use error::{NotReadyError, StatusError};
pub use letratag::{parse_advertising_status as parse_letratag_ad_status, LetraTagAdStatus};
pub use readiness::{ensure_ready, ensure_ready_to_print, PrintReadiness};
pub use session::{
    ClientStatusSession, StatusAction, StatusSessionContext, StatusSessionContextView,
    StatusSessionError,
};
pub use usb_printer_id::{
    meaningful_serial, parse_device_id as parse_usb_printer_device_id, UsbPrinterIdentity,
    GET_DEVICE_ID_LENGTH, GET_DEVICE_ID_REQUEST,
};
pub use zpl::{parse_host_status as parse_zpl_host_status, ZplHostStatus, HOST_STATUS_CMD};

/// NIIMBOT live-status protocol module (query builders + payload parsers +
/// assembly/merge), re-exported from the driver so WASM/host callers reach it
/// through the unified status facade.
pub use lbl_driver_niimbot::live_status as niimbot;
pub use lbl_driver_niimbot::live_status::NiimbotLiveStatus;

/// NIIMBOT live status with dispatch readiness for JSON hosts.
///
/// Flattens [`NiimbotLiveStatus`] so existing field names stay stable, and adds
/// optional [`PrintReadiness`] when heartbeat/progress is enough to decide.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NiimbotStatusView {
    #[serde(flatten)]
    pub status: NiimbotLiveStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readiness: Option<PrintReadiness>,
}

impl From<NiimbotLiveStatus> for NiimbotStatusView {
    fn from(status: NiimbotLiveStatus) -> Self {
        let readiness = Self::readiness_from_live(&status);
        Self { status, readiness }
    }
}

impl NiimbotStatusView {
    /// `None` when there is not enough heartbeat/progress data to decide.
    pub fn readiness_from_live(status: &NiimbotLiveStatus) -> Option<PrintReadiness> {
        if let Some(ps) = &status.print_status {
            if ps.progress1 > 0 && ps.progress1 < 100 {
                return Some(PrintReadiness::ready());
            }
        }
        let Some(hb) = &status.heartbeat else {
            return None;
        };
        if hb.lid_closed == Some(false) {
            return Some(PrintReadiness::not_ready("lid_open"));
        }
        if hb.paper_inserted == Some(false) {
            return Some(PrintReadiness::not_ready("no_paper"));
        }
        if hb.lid_closed.is_none() && hb.paper_inserted.is_none() {
            return None;
        }
        Some(PrintReadiness::ready())
    }
}

/// Unified, protocol-tagged print-engine status for APIs and WASM JSON.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "protocol", rename_all = "kebab-case")]
pub enum PrintStatus {
    /// DYMO LabelWriter 550-series (`dymo-lw`).
    #[serde(rename = "dymo-lw")]
    DymoLw(Lw550PrintStatusView),
    /// DYMO LabelManager / D1 tape (`dymo`).
    #[serde(rename = "dymo")]
    Dymo(DymoD1Status),
    /// DYMO LabelWriter classic 450-series (`dymolwclassic`).
    #[serde(rename = "dymolwclassic")]
    DymoLwClassic(DymoLwClassicStatus),
    /// Brother QL-series (`brother-ql`).
    #[serde(rename = "brother-ql")]
    BrotherQl(BrotherQlStatus),
    /// Brother P-touch / TZe (`brother-pt`).
    #[serde(rename = "brother-pt")]
    BrotherPt(BrotherPtStatus),
    /// Zebra ZPL (`zpl`).
    #[serde(rename = "zpl")]
    Zpl(ZplHostStatus),
    /// USB Printer Class identity (`usb-printer-id`): IEEE 1284 Device ID + USB
    /// strings. Produced when [`status_uses_usb_device_id`] is true for the
    /// profile protocol. [`PrintStatus::readiness`] is always `None`.
    #[serde(rename = "usb-printer-id")]
    UsbPrinterId(UsbPrinterIdentity),
    /// NIIMBOT live status (`niimbot`): RFID, heartbeat, print progress, media,
    /// and device info.
    #[serde(rename = "niimbot")]
    Niimbot(NiimbotStatusView),
    /// Graphtec / Silhouette GPGL cutter (`gpgl`).
    #[serde(rename = "gpgl")]
    Gpgl(GpglStatusView),
    /// DYMO LetraTag advertising-data status (`letratag`).
    #[serde(rename = "letratag")]
    LetraTag(LetraTagAdStatus),
}

impl PrintStatus {
    /// Dispatch readiness for this snapshot (`None` when incomplete/unknown).
    pub fn readiness(&self) -> Option<PrintReadiness> {
        match self {
            Self::BrotherPt(s) => Some(s.readiness.clone()),
            Self::BrotherQl(s) => Some(s.readiness.clone()),
            Self::DymoLw(s) => Some(s.readiness.clone()),
            Self::Dymo(s) => Some(s.readiness()),
            Self::DymoLwClassic(s) => Some(s.readiness()),
            Self::Zpl(s) => Some(s.readiness.clone()),
            Self::UsbPrinterId(_) => None,
            Self::Niimbot(s) => s.readiness.clone(),
            Self::Gpgl(s) => Some(s.readiness.clone()),
            Self::LetraTag(s) => Some(s.readiness()),
        }
    }
}

/// GPGL cutter status snapshot.
///
/// A struct wrapper (rather than a bare [`GpglHostStatus`]) is required because
/// [`PrintStatus`] is internally tagged (`#[serde(tag = "protocol")]`): serde
/// can only fold the tag into a variant that serializes as a map, so a variant
/// wrapping a plain string-valued enum would fail to serialize. The `state`
/// field keeps the tagged JSON well-formed (`{ "protocol": "gpgl", "state": "ready" }`).
///
/// Optional identity fields come from once-per-connection `FG` / `TI` probes
/// (see [`ClientStatusSession`]); delivery status notes leave them unset.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GpglStatusView {
    /// Cutter motion / load state.
    pub state: GpglHostStatus,
    /// Whether the cutter can accept a new cut job.
    pub readiness: PrintReadiness,
    /// Firmware string from `FG` (e.g. `"CAMEO V1.10"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware_version: Option<String>,
    /// Device name from `TI` (newer firmware).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
}

impl From<GpglHostStatus> for GpglStatusView {
    fn from(state: GpglHostStatus) -> Self {
        Self {
            state,
            readiness: state.readiness(),
            firmware_version: None,
            device_name: None,
        }
    }
}

/// Graphtec / Silhouette GPGL cutter motion / load status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GpglHostStatus {
    Ready,
    Moving,
    Unloaded,
    Paused,
    Cancelled,
}

impl GpglHostStatus {
    /// Whether the cutter can accept a new cut job.
    pub fn readiness(self) -> PrintReadiness {
        match self {
            Self::Ready | Self::Moving => PrintReadiness::ready(),
            Self::Unloaded => PrintReadiness::not_ready("unloaded"),
            Self::Paused => PrintReadiness::not_ready("paused"),
            Self::Cancelled => PrintReadiness::not_ready("cancelled"),
        }
    }
}

impl From<lbl_driver_gpgl::GpglStatus> for GpglHostStatus {
    fn from(status: lbl_driver_gpgl::GpglStatus) -> Self {
        match status {
            lbl_driver_gpgl::GpglStatus::Ready => Self::Ready,
            lbl_driver_gpgl::GpglStatus::Moving => Self::Moving,
            lbl_driver_gpgl::GpglStatus::Unloaded => Self::Unloaded,
            lbl_driver_gpgl::GpglStatus::Paused => Self::Paused,
            lbl_driver_gpgl::GpglStatus::Cancelled => Self::Cancelled,
        }
    }
}

/// Whether `protocol` supports a print-engine status query.
pub fn status_supported(protocol: Protocol) -> bool {
    matches!(
        protocol,
        Protocol::DymoLw
            | Protocol::Dymo
            | Protocol::DymoLwClassic
            | Protocol::BrotherQl
            | Protocol::BrotherPt
            | Protocol::Zpl
            | Protocol::Tspl
            | Protocol::Niimbot
            | Protocol::Gpgl
            | Protocol::LetraTag
    )
}

/// Whether status for `protocol` is USB Printer Class `GET_DEVICE_ID` identity.
///
/// When true, hosts must issue a class-control IN (not [`status_query_bytes`]
/// over bulk). The snapshot is [`PrintStatus::UsbPrinterId`].
pub fn status_uses_usb_device_id(protocol: Protocol) -> bool {
    matches!(protocol, Protocol::Tspl)
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
///
/// LetraTag status is read from BLE advertising data (no wire query); this
/// returns an empty buffer for that protocol.
pub fn status_query_bytes(protocol: Protocol) -> Result<Vec<u8>, StatusError> {
    match protocol {
        Protocol::DymoLw => Ok(dymo_lw::status_request(dymo_lw::LOCK_RELEASE).to_vec()),
        Protocol::Dymo => Ok(DYMO_D1_STATUS_REQUEST.to_vec()),
        Protocol::DymoLwClassic => Ok(DYMO_LW_CLASSIC_STATUS_REQUEST.to_vec()),
        Protocol::BrotherQl => Ok(BROTHER_QL_STATUS_REQUEST.to_vec()),
        Protocol::BrotherPt => Ok(BROTHER_PT_STATUS_REQUEST.to_vec()),
        Protocol::Zpl => Ok(HOST_STATUS_CMD.to_vec()),
        // TSPL identity uses USB Printer Class GET_DEVICE_ID (control), not bulk.
        Protocol::Tspl => Err(StatusError::Parse(
            "tspl identity uses USB GET_DEVICE_ID control transfer, not a bulk query".into(),
        )),
        Protocol::Niimbot => Ok(lbl_driver_niimbot::status_query()),
        Protocol::Gpgl => Ok(lbl_driver_gpgl::STATUS_QUERY.to_vec()),
        Protocol::LetraTag => Ok(Vec::new()),
        other => Err(StatusError::Parse(format!(
            "status query not supported for protocol {other:?}"
        ))),
    }
}

/// Expected minimum reply length for a primary status query, when fixed.
///
/// `None` when the reply is variable-length (ZPL `~HS` framing, NIIMBOT
/// packets, GPGL ASCII status). LetraTag advertising status is fixed at 3
/// bytes (no wire query; parse the manufacturer AD payload).
pub fn status_reply_len(protocol: Protocol) -> Option<usize> {
    match protocol {
        Protocol::DymoLw => Some(DYMO_LW_STATUS_REPLY_LEN),
        Protocol::Dymo => Some(1),
        Protocol::DymoLwClassic => Some(1),
        Protocol::BrotherQl => Some(BROTHER_QL_STATUS_REPLY_LEN),
        Protocol::BrotherPt => Some(BROTHER_PT_STATUS_REPLY_LEN),
        Protocol::LetraTag => Some(3),
        Protocol::Zpl | Protocol::Niimbot | Protocol::Gpgl => None,
        _ => None,
    }
}

/// Parse a primary status reply into a [`PrintStatus`].
///
/// For `dymo-lw` this parses the `ESC A` reply into a view with `label_total`
/// and engine-version fields unset; callers polling `ESC U` / `ESC V` fill
/// those in and combine with [`merge_dymo_lw_status`].
///
/// For `letratag`, `bytes` is the 3-byte advertising manufacturer payload.
pub fn parse_status(protocol: Protocol, bytes: &[u8]) -> Result<PrintStatus, StatusError> {
    match protocol {
        Protocol::DymoLw => {
            let status = parse_print_status(bytes)?;
            Ok(PrintStatus::DymoLw(status.to_view()))
        }
        Protocol::Dymo => Ok(PrintStatus::Dymo(parse_dymo_d1_status(bytes)?)),
        Protocol::DymoLwClassic => Ok(PrintStatus::DymoLwClassic(parse_dymo_lw_classic_status(
            bytes,
        )?)),
        Protocol::BrotherQl => Ok(PrintStatus::BrotherQl(parse_brother_ql_status(bytes)?)),
        Protocol::BrotherPt => Ok(PrintStatus::BrotherPt(parse_brother_pt_status(bytes)?)),
        Protocol::Zpl => Ok(PrintStatus::Zpl(parse_zpl_host_status(bytes)?)),
        Protocol::Tspl => Ok(PrintStatus::UsbPrinterId(parse_usb_printer_device_id(
            bytes,
        )?)),
        Protocol::Niimbot => lbl_driver_niimbot::parse_status(bytes)
            .map(|s| PrintStatus::Niimbot(NiimbotLiveStatus::from(s).into()))
            .ok_or_else(|| StatusError::Parse("no NIIMBOT print-status reply in buffer".into())),
        Protocol::Gpgl => lbl_driver_gpgl::parse_status(bytes)
            .map(|s| PrintStatus::Gpgl(GpglHostStatus::from(s).into()))
            .ok_or_else(|| StatusError::Parse("unrecognized GPGL status response".into())),
        Protocol::LetraTag => Ok(PrintStatus::LetraTag(parse_letratag_ad_status(bytes)?)),
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
        PrintStatus::Niimbot(s) => niimbot_media_key_hint(&s.status),
        PrintStatus::LetraTag(s) if s.cassette_id > 0 => Some(match s.cassette_id {
            1 => "6".into(),
            2 => "9".into(),
            3 => "12".into(),
            4 => "19".into(),
            5 => "24".into(),
            other => other.to_string(),
        }),
        PrintStatus::Dymo(_)
        | PrintStatus::DymoLwClassic(_)
        | PrintStatus::Zpl(_)
        | PrintStatus::UsbPrinterId(_)
        | PrintStatus::Gpgl(_)
        | PrintStatus::LetraTag(_) => None,
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
        let value = serde_json::to_value(PrintStatus::Niimbot(live.into())).unwrap();
        assert_eq!(value["protocol"], "niimbot");
        assert_eq!(value["media_width_mm"], 50);
        assert_eq!(value["media_length_mm"], 30);
        assert_eq!(value["rfid"]["total_len"], 100);
        assert_eq!(value["heartbeat"]["battery_level"], 3);
        assert!(value["print_status"].is_null());
        assert!(value["device_info"].is_null());
        assert_eq!(value["readiness"]["ready_to_print"], true);
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
            media_key_hint(&PrintStatus::Niimbot(with_barcode.into())).as_deref(),
            Some("02282280")
        );

        let dims_only = NiimbotLiveStatus {
            media_width_mm: Some(50),
            media_length_mm: Some(30),
            ..Default::default()
        };
        assert_eq!(
            media_key_hint(&PrintStatus::Niimbot(dims_only.into())).as_deref(),
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
        assert!(value.get("firmware_version").is_none());
        assert!(value.get("device_name").is_none());
    }

    #[test]
    fn gpgl_status_serializes_identity_fields() {
        let status = PrintStatus::Gpgl(GpglStatusView {
            state: GpglHostStatus::Ready,
            readiness: GpglHostStatus::Ready.readiness(),
            firmware_version: Some("CAMEO V1.10".into()),
            device_name: Some("Cameo 4".into()),
        });
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["firmware_version"], "CAMEO V1.10");
        assert_eq!(value["device_name"], "Cameo 4");
    }

    #[test]
    fn gpgl_unloaded_blocks_dispatch() {
        let r = GpglHostStatus::Unloaded.readiness();
        assert!(!r.ready_to_print);
        assert_eq!(r.reason.as_deref(), Some("unloaded"));
    }

    #[test]
    fn gpgl_paused_and_cancelled_block_dispatch() {
        assert_eq!(
            GpglHostStatus::Paused.readiness().reason.as_deref(),
            Some("paused")
        );
        assert_eq!(
            GpglHostStatus::Cancelled.readiness().reason.as_deref(),
            Some("cancelled")
        );
        let value = serde_json::to_value(PrintStatus::Gpgl(GpglHostStatus::Paused.into())).unwrap();
        assert_eq!(value["state"], "paused");
    }
}

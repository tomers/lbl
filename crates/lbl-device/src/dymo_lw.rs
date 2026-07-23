//! DYMO LabelWriter 550-series (LW5) USB print session.
//!
//! The 550 protocol is bidirectional: the host must acquire a print-engine lock,
//! send each label segment, then drain a 32-byte status reply on bulk IN before
//! the next OUT write. Streaming a whole job in one bulk transfer stalls the
//! firmware until power-cycle. See DYMO's *LabelWriter 550 Series Technical
//! Reference* and <https://thermal-label.github.io/labelwriter/protocol/lw5-raster>.
//!
//! Request byte builders, reply parsing, and status types live in
//! [`lbl_status::dymo_lw`]; this module owns the device and the mandatory lock /
//! per-label handshakes.

pub use lbl_status::dymo_lw::*;

use crate::transport::{open_usb_bulk_session, Transport, UsbBulkSession, UsbTransport};
use crate::DeviceError;

/// Query the full print-engine status without acquiring the print lock.
///
/// Issues `ESC A` for engine/remaining-count fields, `ESC V` for HW/FW/PID,
/// then `ESC U` for the full-roll total when media appears present.
pub fn query_print_status(session: &mut UsbBulkSession) -> Result<Lw550PrintStatus, DeviceError> {
    session.transfer_out(&status_request(LOCK_RELEASE))?;
    let status = session.transfer_in(STATUS_REPLY_LEN)?;
    let mut parsed = parse_print_status(&status)?;
    if let Ok(ver) = query_engine_version(session) {
        apply_engine_version(&mut parsed, &ver);
    }
    if media_likely_present(parsed.main_bay_status) {
        if let Ok(info) = query_sku_info(session) {
            if info.total_label_count > 0 {
                parsed.label_total = Some(info.total_label_count);
            }
            if parsed.sku.is_none() {
                parsed.sku = info.sku;
            }
        }
    }
    Ok(parsed)
}

/// Query the NFC consumable dump (`ESC U`) without acquiring the print lock.
pub fn query_sku_info(session: &mut UsbBulkSession) -> Result<Lw550SkuInfo, DeviceError> {
    session.transfer_out(&sku_info_request())?;
    let data = session.transfer_in(SKU_INFO_REPLY_LEN)?;
    Ok(parse_sku_info(&data)?)
}

/// Query the engine-version block (`ESC V`) without acquiring the print lock.
pub fn query_engine_version(
    session: &mut UsbBulkSession,
) -> Result<Lw550EngineVersion, DeviceError> {
    session.transfer_out(&engine_version_request())?;
    let data = session.transfer_in(ENGINE_VERSION_REPLY_LEN)?;
    Ok(parse_engine_version(&data)?)
}

/// Soft-reboot the print engine (`ESC @`). Fire-and-forget; no status reply.
pub fn soft_reboot(session: &mut UsbBulkSession) -> Result<(), DeviceError> {
    session.transfer_out(&soft_reboot_request())
}

/// Query print-engine status over USB.
pub fn query_status(usb: &UsbTransport) -> Result<Lw550PrintStatus, DeviceError> {
    let mut session = open_usb_bulk_session(usb)?;
    query_print_status(&mut session)
}

/// Soft-reboot the print engine over USB (`ESC @`).
pub fn soft_reboot_usb(usb: &UsbTransport) -> Result<(), DeviceError> {
    let mut session = open_usb_bulk_session(usb)?;
    soft_reboot(&mut session)
}

/// Query the loaded media SKU over USB.
pub fn query_loaded_media(usb: &UsbTransport) -> Result<Option<String>, DeviceError> {
    let mut session = open_usb_bulk_session(usb)?;
    query_loaded_media_sku(&mut session)
}

pub(crate) fn send_dymo_lw_job(
    session: &mut UsbBulkSession,
    payload: &[u8],
) -> Result<(), DeviceError> {
    acquire_lock(session)?;
    dispatch_job_segments(session, payload)?;
    Ok(())
}

/// USB transport for the DYMO LabelWriter 550-series (LW5) protocol.
///
/// Unlike [`UsbTransport`], this keeps the device open across jobs and performs
/// the mandatory lock acquisition and 32-byte status handshakes on bulk IN.
pub struct DymoLwUsbTransport {
    usb: UsbTransport,
    session: Option<UsbBulkSession>,
}

impl DymoLwUsbTransport {
    /// Wrap a [`UsbTransport`] configured for the target printer.
    pub fn new(usb: UsbTransport) -> Self {
        Self { usb, session: None }
    }

    fn ensure_session(&mut self) -> Result<&mut UsbBulkSession, DeviceError> {
        if self.session.is_none() {
            self.session = Some(open_usb_bulk_session(&self.usb)?);
        }
        Ok(self.session.as_mut().expect("session opened"))
    }
}

impl Transport for DymoLwUsbTransport {
    fn send(&mut self, data: &[u8]) -> Result<(), DeviceError> {
        let session = self.ensure_session()?;
        send_dymo_lw_job(session, data)
    }

    fn is_bidirectional(&self) -> bool {
        true
    }
}

fn acquire_lock(session: &mut UsbBulkSession) -> Result<(), DeviceError> {
    session.transfer_out(&status_request(LOCK_ACQUIRE))?;
    let status = session.transfer_in(STATUS_REPLY_LEN)?;
    interpret_status(&status, "acquiring print lock")
}

fn handshake(session: &mut UsbBulkSession, lock: u8) -> Result<(), DeviceError> {
    session.transfer_out(&status_request(lock))?;
    let status = session.transfer_in(STATUS_REPLY_LEN)?;
    interpret_status(&status, "label handshake")
}

fn dispatch_job_segments(session: &mut UsbBulkSession, payload: &[u8]) -> Result<(), DeviceError> {
    let mut pos = 0usize;
    require_esc_cmd(payload, &mut pos, b's')?;
    pos += 4; // job id

    skip_job_header(payload, &mut pos)?;

    let preamble_end = pos;
    let mut label_ranges = Vec::new();
    while pos + 1 < payload.len() {
        if payload[pos] == ESC && payload[pos + 1] == b'E' {
            break;
        }
        if payload[pos] == ESC && payload[pos + 1] == b'Q' {
            return Err(DeviceError::Transport(
                "dymo-lw job missing ESC E before ESC Q".into(),
            ));
        }

        let label_start = pos;
        require_esc_cmd(payload, &mut pos, b'n')?;
        pos += 2; // label index

        skip_label_data(payload, &mut pos)?;
        require_esc_cmd(payload, &mut pos, b'G')?;
        label_ranges.push((label_start, pos));
    }

    if label_ranges.is_empty() {
        return Err(DeviceError::Transport(
            "dymo-lw job missing label data".into(),
        ));
    }

    require_esc_cmd(payload, &mut pos, b'E')?;
    require_esc_cmd(payload, &mut pos, b'Q')?;
    if pos != payload.len() {
        return Err(DeviceError::Transport(format!(
            "dymo-lw job has {0} trailing bytes after ESC Q",
            payload.len() - pos
        )));
    }

    let finalize = &payload[payload.len() - 4..];

    if preamble_end > 0 {
        session.transfer_out(&payload[..preamble_end])?;
    }

    for (i, &(start, end)) in label_ranges.iter().enumerate() {
        session.transfer_out(&payload[start..end])?;
        handshake(
            session,
            if i + 1 == label_ranges.len() {
                LOCK_RELEASE
            } else {
                LOCK_INTER_LABEL
            },
        )?;
    }

    session.transfer_out(finalize)?;
    Ok(())
}

fn skip_job_header(payload: &[u8], pos: &mut usize) -> Result<(), DeviceError> {
    while *pos + 1 < payload.len() && payload[*pos] == ESC {
        match payload[*pos + 1] {
            b'L' => *pos += 6,
            b'h' | b'i' | b'e' => *pos += 2,
            b'T' | b'C' => *pos += 3,
            b'n' | b'D' => return Ok(()),
            b'Q' => {
                return Err(DeviceError::Transport(
                    "dymo-lw job missing label data".into(),
                ))
            }
            other => {
                return Err(DeviceError::Transport(format!(
                    "unexpected dymo-lw header command ESC {other:#04x}"
                )))
            }
        }
    }
    Ok(())
}

fn skip_label_data(payload: &[u8], pos: &mut usize) -> Result<(), DeviceError> {
    require_esc_cmd(payload, pos, b'D')?;
    *pos += 2; // bpp + align
    let width = read_u32_le(payload, *pos)?;
    *pos += 4;
    let height = read_u32_le(payload, *pos)?;
    *pos += 4;
    let data_len = width
        .checked_mul(height.div_ceil(8))
        .ok_or_else(|| DeviceError::Transport("dymo-lw label data length overflow".into()))?
        as usize;
    let end = pos
        .checked_add(data_len)
        .ok_or_else(|| DeviceError::Transport("dymo-lw label data length overflow".into()))?;
    if end > payload.len() {
        return Err(DeviceError::Transport(format!(
            "dymo-lw label data truncated (need {data_len} bytes, have {})",
            payload.len().saturating_sub(*pos)
        )));
    }
    *pos = end;
    Ok(())
}

fn read_u32_le(payload: &[u8], pos: usize) -> Result<u32, DeviceError> {
    let end = pos
        .checked_add(4)
        .ok_or_else(|| DeviceError::Transport("dymo-lw field truncated".into()))?;
    if end > payload.len() {
        return Err(DeviceError::Transport("dymo-lw field truncated".into()));
    }
    Ok(u32::from_le_bytes(payload[pos..end].try_into().unwrap()))
}

fn require_esc_cmd(payload: &[u8], pos: &mut usize, cmd: u8) -> Result<(), DeviceError> {
    if *pos + 1 >= payload.len() || payload[*pos] != ESC || payload[*pos + 1] != cmd {
        return Err(DeviceError::Transport(format!(
            "expected ESC {cmd} at offset {pos}, job may be malformed"
        )));
    }
    *pos += 2;
    Ok(())
}

/// Query the SKU of the roll currently loaded in the printer.
pub fn query_loaded_media_sku(session: &mut UsbBulkSession) -> Result<Option<String>, DeviceError> {
    Ok(query_print_status(session)?.sku)
}

fn interpret_status(status: &[u8], phase: &str) -> Result<(), DeviceError> {
    let parsed =
        parse_print_status(status).map_err(|e| DeviceError::Transport(format!("{phase}: {e}")))?;

    match parsed.print_status {
        Lw550PrintEngineStatus::NoLock => {
            return Err(DeviceError::Transport(format!(
                "{phase}: printer did not grant the print lock (another host may be using it)"
            )));
        }
        Lw550PrintEngineStatus::Error => {
            return Err(DeviceError::Transport(format!(
                "{phase}: printer reported an error"
            )));
        }
        _ => {}
    }

    match parsed.main_bay_status {
        Lw550MainBayStatus::MediaOk => {}
        Lw550MainBayStatus::MediaCounterfeit => {
            return Err(DeviceError::Transport(
                "printer rejected the loaded media (NFC reports non-genuine labels); \
                 LabelWriter 550 requires authentic DYMO rolls"
                    .into(),
            ));
        }
        Lw550MainBayStatus::NoMedia => {
            return Err(DeviceError::Transport(format!(
                "{phase}: no media loaded in the printer"
            )));
        }
        Lw550MainBayStatus::MediaEmpty
        | Lw550MainBayStatus::MediaCriticallyLow
        | Lw550MainBayStatus::MediaLow => {
            return Err(DeviceError::Transport(format!(
                "{phase}: media roll is empty or nearly empty ({})",
                parsed.main_bay_status.as_str()
            )));
        }
        Lw550MainBayStatus::MediaJammed => {
            return Err(DeviceError::Transport(format!(
                "{phase}: media jam reported by printer"
            )));
        }
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_single_label_job() -> Vec<u8> {
        // Minimal job: ESC s, ESC i, one 8×1-dot label, ESC E, ESC Q.
        let mut out = vec![
            ESC, b's', 1, 0, 0, 0, //
            ESC, b'i', //
            ESC, b'n', 0, 0, //
            ESC, b'D', 1, 2, //
        ];
        out.extend_from_slice(&1u32.to_le_bytes()); // width = 1 line
        out.extend_from_slice(&8u32.to_le_bytes()); // height = 8 dots
        out.push(0x80); // one line of print data
        out.extend_from_slice(&[ESC, b'G', ESC, b'E', ESC, b'Q']);
        out
    }

    #[test]
    fn interpret_status_flags_counterfeit_media() {
        let mut status = vec![0u8; STATUS_REPLY_LEN];
        status[10] = 10; // media counterfeit
        let err = interpret_status(&status, "test").unwrap_err();
        assert!(err.to_string().contains("non-genuine"));
    }

    #[test]
    fn parse_single_label_job_layout() {
        let job = sample_single_label_job();
        let mut pos = 0;
        require_esc_cmd(&job, &mut pos, b's').unwrap();
        pos += 4;
        skip_job_header(&job, &mut pos).unwrap();
        require_esc_cmd(&job, &mut pos, b'n').unwrap();
        pos += 2;
        skip_label_data(&job, &mut pos).unwrap();
        require_esc_cmd(&job, &mut pos, b'G').unwrap();
        assert_eq!(pos, 27);
        assert_eq!(&job[pos..], &[ESC, b'E', ESC, b'Q']);
    }
}

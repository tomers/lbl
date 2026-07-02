//! DYMO LabelWriter 550-series (LW5) USB print session.
//!
//! The 550 protocol is bidirectional: the host must acquire a print-engine lock,
//! send each label segment, then drain a 32-byte status reply on bulk IN before
//! the next OUT write. Streaming a whole job in one bulk transfer stalls the
//! firmware until power-cycle. See DYMO's *LabelWriter 550 Series Technical
//! Reference* and <https://thermal-label.github.io/labelwriter/protocol/lw5-raster>.

use std::time::Duration;

use crate::DeviceError;

#[cfg(feature = "usb")]
use nusb::transfer::{Buffer, Bulk, In, Out};
#[cfg(feature = "usb")]
use nusb::Interface;

/// ESC prefix byte for LW5 commands.
pub const ESC: u8 = 0x1B;

/// Length of a print-engine status reply on bulk IN.
pub const STATUS_REPLY_LEN: usize = 32;

/// Lock byte for [`status_request`]: acquire the print engine before a job.
pub const LOCK_ACQUIRE: u8 = 1;

/// Lock byte for [`status_request`]: handshake between labels in one job.
pub const LOCK_INTER_LABEL: u8 = 2;

/// Lock byte for [`status_request`]: handshake after the last label in a job.
pub const LOCK_RELEASE: u8 = 0;

/// First byte of the 12-character SKU field in a 32-byte status reply.
const STATUS_SKU_START: usize = 11;

/// Length of the SKU field in a status reply.
const STATUS_SKU_LEN: usize = 12;

/// Build an `ESC A` status request with the given lock byte.
pub fn status_request(lock: u8) -> [u8; 3] {
    [ESC, b'A', lock]
}

/// Main-bay status: media present and OK (NFC-valid genuine roll).
const BAY_MEDIA_OK: u8 = 8;

/// Main-bay status: NFC rejected the inserted roll as non-genuine.
const BAY_MEDIA_COUNTERFEIT: u8 = 10;

/// Print-engine status: reply before lock is granted to this host.
const PRINT_STATUS_NO_LOCK: u8 = 5;

#[cfg(feature = "usb")]
const USB_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg(feature = "usb")]
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);

/// A claimed USB printer interface kept open for an LW5 print session.
#[cfg(feature = "usb")]
pub struct UsbPrinterSession {
    _interface: Interface,
    ep_out: nusb::Endpoint<Bulk, Out>,
    ep_in: nusb::Endpoint<Bulk, In>,
}

#[cfg(feature = "usb")]
impl UsbPrinterSession {
    pub(crate) fn new(
        interface: Interface,
        ep_out: nusb::Endpoint<Bulk, Out>,
        ep_in: nusb::Endpoint<Bulk, In>,
    ) -> Self {
        Self {
            _interface: interface,
            ep_out,
            ep_in,
        }
    }

    pub(crate) fn transfer_out(&mut self, data: &[u8]) -> Result<(), DeviceError> {
        let completion = self
            .ep_out
            .transfer_blocking(data.to_vec().into(), USB_TIMEOUT);
        completion
            .status
            .map_err(|e| DeviceError::Transport(format!("bulk out: {e}")))
    }

    pub(crate) fn transfer_in(&mut self, len: usize) -> Result<Vec<u8>, DeviceError> {
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            let chunk = (len - out.len()).max(64);
            let completion = self
                .ep_in
                .transfer_blocking(Buffer::new(chunk), STATUS_TIMEOUT);
            completion
                .status
                .map_err(|e| DeviceError::Transport(format!("bulk in: {e}")))?;
            let buf = completion.buffer;
            let received = completion.actual_len.min(buf.len());
            if received == 0 {
                return Err(DeviceError::Transport(
                    "bulk in: printer returned no status bytes".into(),
                ));
            }
            out.extend_from_slice(&buf[..received]);
        }
        out.truncate(len);
        Ok(out)
    }
}

#[cfg(feature = "usb")]
pub(crate) fn send_dymo_lw_job(
    session: &mut UsbPrinterSession,
    payload: &[u8],
) -> Result<(), DeviceError> {
    acquire_lock(session)?;
    dispatch_job_segments(session, payload)?;
    Ok(())
}

fn acquire_lock(session: &mut UsbPrinterSession) -> Result<(), DeviceError> {
    session.transfer_out(&status_request(LOCK_ACQUIRE))?;
    let status = session.transfer_in(STATUS_REPLY_LEN)?;
    interpret_status(&status, "acquiring print lock")
}

fn handshake(session: &mut UsbPrinterSession, lock: u8) -> Result<(), DeviceError> {
    session.transfer_out(&status_request(lock))?;
    let status = session.transfer_in(STATUS_REPLY_LEN)?;
    interpret_status(&status, "label handshake")
}

fn dispatch_job_segments(
    session: &mut UsbPrinterSession,
    payload: &[u8],
) -> Result<(), DeviceError> {
    let mut pos = 0usize;
    require_esc_cmd(payload, &mut pos, b's')?;
    pos += 4; // job id

    skip_job_header(payload, &mut pos)?;

    let mut segment_start = 0usize;
    loop {
        if pos + 1 >= payload.len() {
            break;
        }
        if payload[pos] == ESC && payload[pos + 1] == b'Q' {
            session.transfer_out(&payload[pos..pos + 2])?;
            return Ok(());
        }

        require_esc_cmd(payload, &mut pos, b'n')?;
        pos += 2; // label index

        let _data_end = skip_label_data(payload, &mut pos)?;
        let feed = feed_command(payload, pos)?;
        pos += 2;

        session.transfer_out(&payload[segment_start..pos])?;
        handshake(
            session,
            if feed == b'G' {
                LOCK_INTER_LABEL
            } else {
                LOCK_RELEASE
            },
        )?;
        segment_start = pos;
    }

    Err(DeviceError::Transport(
        "dymo-lw job missing ESC Q trailer".into(),
    ))
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

fn skip_label_data(payload: &[u8], pos: &mut usize) -> Result<usize, DeviceError> {
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
    Ok(end)
}

fn feed_command(payload: &[u8], pos: usize) -> Result<u8, DeviceError> {
    if pos + 1 >= payload.len() || payload[pos] != ESC {
        return Err(DeviceError::Transport(
            "dymo-lw label missing feed command (ESC G or ESC E)".into(),
        ));
    }
    match payload[pos + 1] {
        b'G' | b'E' => Ok(payload[pos + 1]),
        other => Err(DeviceError::Transport(format!(
            "expected ESC G/E after label data, got ESC {other:#04x}"
        ))),
    }
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

/// Read the NFC-reported consumable SKU from a 32-byte LW5 status reply.
pub fn parse_status_sku(status: &[u8]) -> Option<String> {
    if status.len() < STATUS_SKU_START + 1 {
        return None;
    }
    if status.len() > 10 && status[10] == 2 {
        return None;
    }
    let end = STATUS_SKU_START + STATUS_SKU_LEN.min(status.len() - STATUS_SKU_START);
    let raw = &status[STATUS_SKU_START..end];
    let len = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    if len == 0 {
        return None;
    }
    std::str::from_utf8(&raw[..len])
        .ok()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Query the SKU of the roll currently loaded in the printer.
pub fn query_loaded_media_sku(
    session: &mut UsbPrinterSession,
) -> Result<Option<String>, DeviceError> {
    session.transfer_out(&status_request(LOCK_RELEASE))?;
    let status = session.transfer_in(STATUS_REPLY_LEN)?;
    Ok(parse_status_sku(&status))
}

fn interpret_status(status: &[u8], phase: &str) -> Result<(), DeviceError> {
    if status.len() < STATUS_REPLY_LEN {
        return Err(DeviceError::Transport(format!(
            "{phase}: short status reply ({} bytes)",
            status.len()
        )));
    }

    match status[0] {
        PRINT_STATUS_NO_LOCK => {
            return Err(DeviceError::Transport(format!(
                "{phase}: printer did not grant the print lock (another host may be using it)"
            )));
        }
        2 => {
            return Err(DeviceError::Transport(format!(
                "{phase}: printer reported an error (status byte {})",
                status[0]
            )));
        }
        _ => {}
    }

    if status.len() > 10 {
        match status[10] {
            BAY_MEDIA_OK => {}
            BAY_MEDIA_COUNTERFEIT => {
                return Err(DeviceError::Transport(
                    "printer rejected the loaded media (NFC reports non-genuine labels); \
                     LabelWriter 550 requires authentic DYMO rolls"
                        .into(),
                ));
            }
            2 => {
                return Err(DeviceError::Transport(format!(
                    "{phase}: no media loaded in the printer"
                )));
            }
            5..=7 => {
                return Err(DeviceError::Transport(format!(
                    "{phase}: media roll is empty or nearly empty (bay status {})",
                    status[10]
                )));
            }
            9 => {
                return Err(DeviceError::Transport(format!(
                    "{phase}: media jam reported by printer"
                )));
            }
            _ => {}
        }
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
        out.extend_from_slice(&[ESC, b'E', ESC, b'Q']);
        out
    }

    #[test]
    fn status_request_bytes() {
        assert_eq!(status_request(LOCK_ACQUIRE), [ESC, b'A', 1]);
        assert_eq!(status_request(LOCK_INTER_LABEL), [ESC, b'A', 2]);
    }

    #[test]
    fn interpret_status_flags_counterfeit_media() {
        let mut status = vec![0u8; STATUS_REPLY_LEN];
        status[10] = BAY_MEDIA_COUNTERFEIT;
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
        let data_end = skip_label_data(&job, &mut pos).unwrap();
        assert_eq!(feed_command(&job, pos).unwrap(), b'E');
        pos += 2;
        assert_eq!(pos, job.len() - 2);
        assert_eq!(&job[pos..], &[ESC, b'Q']);
        assert_eq!(data_end, 25);
    }
}

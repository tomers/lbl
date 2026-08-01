//! Graphtec / Silhouette GPGL USB I/O: status poll and paced cut send.

use std::time::{Duration, Instant};

use lbl_driver_gpgl::{firmware_query, parse_status, GpglStatus, INIT_CMD, STATUS_QUERY};

use crate::transport::{UsbBulkSession, UsbTransport};
use crate::DeviceError;

/// Default Cameo 4 USB ids.
pub const CAMEO4_VID: u16 = 0x0B4D;
pub const CAMEO4_PID: u16 = 0x1137;

fn read_response(session: &mut UsbBulkSession) -> Result<Vec<u8>, DeviceError> {
    // Status replies are short; read a modest bulk packet.
    match session.transfer_in(64) {
        Ok(buf) => Ok(buf),
        Err(_) => Ok(Vec::new()),
    }
}

/// Query device status until ready or timeout.
pub fn wait_until_ready(
    session: &mut UsbBulkSession,
    timeout: Duration,
) -> Result<GpglStatus, DeviceError> {
    let deadline = Instant::now() + timeout;
    loop {
        session
            .transfer_out(STATUS_QUERY)
            .map_err(|e| DeviceError::Transport(format!("gpgl status query: {e}")))?;
        let resp = read_response(session)?;
        if let Some(status) = parse_status(&resp) {
            match status {
                GpglStatus::Ready => return Ok(status),
                GpglStatus::Unloaded => {
                    return Err(DeviceError::Transport(
                        "cutter reports media unloaded (status 2)".into(),
                    ));
                }
                GpglStatus::Cancelled => {
                    return Err(DeviceError::Transport(
                        "cutter job was cancelled on the device (status 4)".into(),
                    ));
                }
                GpglStatus::Moving | GpglStatus::Paused => {
                    if Instant::now() >= deadline {
                        return Err(DeviceError::Transport(
                            "timed out waiting for cutter ready".into(),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        } else if Instant::now() >= deadline {
            return Err(DeviceError::Transport(format!(
                "unrecognized gpgl status response: {resp:?}"
            )));
        } else {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Initialize, query firmware, wait ready, send cut bytes, wait idle.
pub fn send_cut_job(usb: &UsbTransport, cut_bytes: &[u8]) -> Result<(), DeviceError> {
    let mut session = crate::transport::open_usb_bulk_session(usb)?;
    session
        .transfer_out(INIT_CMD)
        .map_err(|e| DeviceError::Transport(format!("gpgl init: {e}")))?;
    let _ = read_response(&mut session);
    wait_until_ready(&mut session, Duration::from_secs(30))?;
    session
        .transfer_out(&firmware_query())
        .map_err(|e| DeviceError::Transport(format!("gpgl FG: {e}")))?;
    let _fw = read_response(&mut session);
    wait_until_ready(&mut session, Duration::from_secs(10))?;
    session
        .transfer_out(cut_bytes)
        .map_err(|e| DeviceError::Transport(format!("gpgl cut send: {e}")))?;
    wait_until_ready(&mut session, Duration::from_secs(600))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_bytes_roundtrip() {
        assert_eq!(
            parse_status(lbl_driver_gpgl::STATUS_REPLY_READY),
            Some(GpglStatus::Ready)
        );
    }
}

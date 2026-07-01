//! Delivering encoded labels to a printer over a [`Transport`], including the
//! per-protocol completion handshake.
//!
//! Write-only protocols (DYMO, ESC/POS, ZPL, TSPL) are fire-and-forget: the
//! spooler sends the bytes and moves on. NIIMBOT is request/response, so over a
//! bidirectional transport (a serial port) we poll the printer for status after
//! each label and wait for it to finish before dispatching the next one.

use std::time::{Duration, Instant};

use lbl_core::printer::Protocol;
use lbl_device::{format_dispatch_failure, DeviceError, Transport, TransportTarget};
use lbl_spool::{SpoolReport, Spooler};

/// Enqueue every `(name, bytes)` label and dispatch it over `transport`,
/// running the protocol-specific completion handshake after each job.
pub fn dispatch_encoded<T: Transport>(
    encoded: Vec<(String, Vec<u8>)>,
    protocol: Protocol,
    transport: &mut T,
) -> SpoolReport {
    let mut spool = Spooler::new();
    for (name, bytes) in encoded {
        spool.enqueue(name, bytes, None);
    }
    spool.run_with(transport, |t| finalize_job(protocol, t))
}

/// Turn a spool [`SpoolReport`] into `Ok(())` or a user-facing error.
pub fn finish_dispatch(report: SpoolReport, target: Option<TransportTarget>) -> Result<(), String> {
    if report.disconnected {
        if let Some(err) = report.last_error.as_ref() {
            return Err(format_dispatch_failure(
                err,
                target.as_ref(),
                report.remaining,
            ));
        }
        return Err(format!(
            "device disconnected; {} job(s) not sent",
            report.remaining
        ));
    }
    Ok(())
}

/// After a job's bytes are delivered, perform any protocol-specific completion
/// handshake. NIIMBOT polls the printer's status over a bidirectional
/// transport; every other protocol is fire-and-forget.
pub fn finalize_job<T: Transport>(
    protocol: Protocol,
    transport: &mut T,
) -> Result<(), DeviceError> {
    if protocol == Protocol::Niimbot && transport.is_bidirectional() {
        wait_for_niimbot_completion(transport)
    } else {
        Ok(())
    }
}

/// Poll a NIIMBOT printer for completion after its page bytes were sent.
///
/// Returns once the printer reports the page complete, stops reporting progress
/// (its print session has ended), or a safety timeout elapses — the bytes are
/// already committed, so a quiet printer is not treated as a failure.
fn wait_for_niimbot_completion<T: Transport>(transport: &mut T) -> Result<(), DeviceError> {
    const POLL_TIMEOUT: Duration = Duration::from_millis(500);
    const POLL_INTERVAL: Duration = Duration::from_millis(200);
    const HARD_CAP: Duration = Duration::from_secs(25);

    let query = lbl_driver_niimbot::status_query();
    let deadline = Instant::now() + HARD_CAP;

    loop {
        transport.send(&query)?;
        let resp = transport.receive(POLL_TIMEOUT)?;
        if let Some(status) = lbl_driver_niimbot::parse_status(&resp) {
            if status.is_complete() {
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            tracing::warn!("niimbot: no completion status within {HARD_CAP:?}; assuming done");
            return Ok(());
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Parse a serial target string (`path` or `path:baud`) into a path and baud
/// rate, defaulting to [`lbl_device::DEFAULT_SERIAL_BAUD`].
pub fn parse_serial_target(target: &str) -> (String, u32) {
    match target.rsplit_once(':') {
        Some((path, baud)) if !baud.is_empty() && baud.chars().all(|c| c.is_ascii_digit()) => {
            let baud = baud.parse().unwrap_or(lbl_device::DEFAULT_SERIAL_BAUD);
            (path.to_string(), baud)
        }
        _ => (target.to_string(), lbl_device::DEFAULT_SERIAL_BAUD),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_serial_targets() {
        assert_eq!(
            parse_serial_target("/dev/ttyACM0"),
            ("/dev/ttyACM0".to_string(), lbl_device::DEFAULT_SERIAL_BAUD)
        );
        assert_eq!(
            parse_serial_target("/dev/ttyACM0:9600"),
            ("/dev/ttyACM0".to_string(), 9600)
        );
        // A bare COM port name with no baud falls back to the default.
        assert_eq!(
            parse_serial_target("COM3"),
            ("COM3".to_string(), lbl_device::DEFAULT_SERIAL_BAUD)
        );
    }
}

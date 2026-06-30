//! `lbl-spool` — queue encoded files and dispatch them to a printer.

use anyhow::{bail, Result};
use clap::Parser;
use lbl_device::NetworkTransport;
use lbl_spool::Spooler;

#[derive(Parser)]
#[command(
    name = "lbl-spool",
    about = "Queue encoded label files and send them to a printer with retry",
    color = clap::ColorChoice::Auto,
)]
struct Cli {
    /// Network target `host:port` (e.g. 192.168.1.50:9100).
    #[arg(long)]
    network: Option<String>,

    /// USB target `vid:pid` in hex (e.g. 0922:1001).
    #[arg(long)]
    usb: Option<String>,

    /// Serial target: a device path with optional baud (`/dev/ttyACM0` or
    /// `/dev/ttyACM0:115200`).
    #[arg(long)]
    serial: Option<String>,

    /// Encoded files to print, in order.
    #[arg(required = true)]
    files: Vec<std::path::PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut spool = Spooler::new();
    for file in &cli.files {
        let bytes = std::fs::read(file)?;
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("job");
        spool.enqueue(name, bytes, None);
    }

    let report = if let Some(target) = cli.network {
        let (host, port) = target
            .rsplit_once(':')
            .ok_or_else(|| anyhow::anyhow!("network target must be host:port"))?;
        let mut t = NetworkTransport::new(host, port.parse()?);
        spool.run(&mut t)
    } else if let Some(target) = cli.usb {
        run_usb(&mut spool, &target)?
    } else if let Some(target) = cli.serial {
        run_serial(&mut spool, &target)?
    } else {
        bail!("no target; use --network host:port, --usb vid:pid, or --serial path[:baud]")
    };

    println!(
        "completed={} failed={} remaining={} disconnected={}",
        report.completed, report.failed, report.remaining, report.disconnected
    );
    if report.disconnected {
        bail!(
            "device disconnected; {} job(s) retained in queue",
            report.remaining
        );
    }
    Ok(())
}

#[cfg(feature = "usb")]
fn run_usb(spool: &mut Spooler, target: &str) -> Result<lbl_spool::SpoolReport> {
    let (vid, pid) = target
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("usb target must be vid:pid (hex)"))?;
    let mut t = lbl_device::UsbTransport::new(
        u16::from_str_radix(vid, 16)?,
        u16::from_str_radix(pid, 16)?,
        None,
    );
    Ok(spool.run(&mut t))
}

#[cfg(not(feature = "usb"))]
fn run_usb(_spool: &mut Spooler, _target: &str) -> Result<lbl_spool::SpoolReport> {
    bail!("USB support not compiled in")
}

#[cfg(feature = "serial")]
fn run_serial(spool: &mut Spooler, target: &str) -> Result<lbl_spool::SpoolReport> {
    let (path, baud) = match target.rsplit_once(':') {
        Some((p, b)) if !b.is_empty() && b.chars().all(|c| c.is_ascii_digit()) => (
            p.to_string(),
            b.parse().unwrap_or(lbl_device::DEFAULT_SERIAL_BAUD),
        ),
        _ => (target.to_string(), lbl_device::DEFAULT_SERIAL_BAUD),
    };
    let mut t = lbl_device::SerialTransport::new(path, baud);
    Ok(spool.run(&mut t))
}

#[cfg(not(feature = "serial"))]
fn run_serial(_spool: &mut Spooler, _target: &str) -> Result<lbl_spool::SpoolReport> {
    bail!("serial support not compiled in")
}

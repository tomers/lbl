//! `lbl-spool` — queue encoded files and dispatch them to a printer.

use anyhow::{bail, Result};
use clap::Parser;
use lbl_device::NetworkTransport;
use lbl_spool::Spooler;

#[derive(Parser)]
#[command(
    name = "lbl-spool",
    about = "Queue encoded label files and send them to a printer with retry"
)]
struct Cli {
    /// Network target `host:port` (e.g. 192.168.1.50:9100).
    #[arg(long)]
    network: Option<String>,

    /// USB target `vid:pid` in hex (e.g. 0922:1001).
    #[arg(long)]
    usb: Option<String>,

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
    } else {
        bail!("no target; use --network host:port or --usb vid:pid")
    };

    println!(
        "completed={} failed={} remaining={} disconnected={}",
        report.completed, report.failed, report.remaining, report.disconnected
    );
    if report.disconnected {
        bail!("device disconnected; {} job(s) retained in queue", report.remaining);
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

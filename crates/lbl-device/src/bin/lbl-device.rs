//! `lbl-device` — discover printers and send raw bytes to them.

use std::io::Read;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use lbl_device::{discover_usb, NetworkTransport, Transport};

#[derive(Parser)]
#[command(
    name = "lbl-device",
    about = "Discover printers and send bytes to them"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List discovered (USB) printers as JSON.
    List,
    /// Send bytes (stdin or a file) to a printer.
    Send {
        /// Network target `host:port` (e.g. `192.168.1.50:9100`).
        #[arg(long)]
        network: Option<String>,

        /// USB target `vid:pid` in hex (e.g. `0922:1001`).
        #[arg(long)]
        usb: Option<String>,

        /// Serial target: a device path with optional baud (`/dev/ttyACM0` or
        /// `/dev/ttyACM0:115200`).
        #[arg(long)]
        serial: Option<String>,

        /// Input file. If omitted, read from stdin.
        input: Option<std::path::PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::List => {
            let printers = discover_usb();
            println!("{}", serde_json::to_string_pretty(&printers)?);
        }
        Command::Send {
            network,
            usb,
            serial,
            input,
        } => {
            let data = match &input {
                Some(path) => std::fs::read(path)?,
                None => {
                    let mut buf = Vec::new();
                    std::io::stdin().read_to_end(&mut buf)?;
                    buf
                }
            };
            send(network, usb, serial, &data)?;
            eprintln!("sent {} bytes", data.len());
        }
    }
    Ok(())
}

fn send(
    network: Option<String>,
    usb: Option<String>,
    serial: Option<String>,
    data: &[u8],
) -> Result<()> {
    if let Some(target) = network {
        let (host, port) = target
            .rsplit_once(':')
            .ok_or_else(|| anyhow::anyhow!("network target must be host:port"))?;
        let mut t = NetworkTransport::new(host, port.parse()?);
        t.send(data)?;
        return Ok(());
    }

    #[cfg(feature = "usb")]
    if let Some(target) = usb {
        let (vid, pid) = target
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("usb target must be vid:pid (hex)"))?;
        let vid = u16::from_str_radix(vid, 16)?;
        let pid = u16::from_str_radix(pid, 16)?;
        let mut t = lbl_device::UsbTransport::new(vid, pid, None);
        t.send(data)?;
        return Ok(());
    }
    #[cfg(not(feature = "usb"))]
    let _ = usb;

    #[cfg(feature = "serial")]
    if let Some(target) = serial {
        let (path, baud) = parse_serial(&target);
        let mut t = lbl_device::SerialTransport::new(path, baud);
        t.send(data)?;
        return Ok(());
    }
    #[cfg(not(feature = "serial"))]
    let _ = serial;

    bail!("no target given; use --network host:port, --usb vid:pid, or --serial path[:baud]")
}

/// Parse a serial target (`path` or `path:baud`), defaulting the baud rate.
#[cfg(feature = "serial")]
fn parse_serial(target: &str) -> (String, u32) {
    match target.rsplit_once(':') {
        Some((path, baud)) if !baud.is_empty() && baud.chars().all(|c| c.is_ascii_digit()) => (
            path.to_string(),
            baud.parse().unwrap_or(lbl_device::DEFAULT_SERIAL_BAUD),
        ),
        _ => (target.to_string(), lbl_device::DEFAULT_SERIAL_BAUD),
    }
}

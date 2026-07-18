//! `lbl-device` — discover printers and send raw bytes to them.

use std::io::Read;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use lbl_device::{discover, format_send_failure, NetworkTransport, Transport, TransportTarget};

#[derive(Parser)]
#[command(
    name = "lbl-device",
    about = "Discover printers and send bytes to them",
    color = clap::ColorChoice::Auto,
    styles = lbl_cli::CLAP_STYLING,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List discovered printers (USB bulk + serial ports) as JSON.
    List,
    /// Query LabelWriter 550-series print-engine status (`ESC A` reply).
    Status {
        /// USB target `vid:pid` in hex (e.g. `0922:0028`).
        #[arg(long)]
        usb: Option<String>,
    },
    /// Soft-reboot a LabelWriter 550-series print engine (`ESC @`).
    SoftReboot {
        /// USB target `vid:pid` in hex (e.g. `0922:0028`).
        #[arg(long)]
        usb: Option<String>,
    },
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

        /// Bluetooth LE target: the printer's advertised name or address
        /// (e.g. `D110`). Requires the `ble` feature. Used by NIIMBOT D-series.
        #[arg(long)]
        bluetooth: Option<String>,

        /// Input file. If omitted, read from stdin.
        input: Option<std::path::PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::List => {
            let printers = discover();
            println!("{}", serde_json::to_string_pretty(&printers)?);
        }
        Command::Status { usb } => {
            #[cfg(feature = "usb")]
            {
                let target = usb.ok_or_else(|| {
                    anyhow::anyhow!("pass --usb vid:pid (e.g. 0922:0028 for LabelWriter 550)")
                })?;
                let (vid, pid) = target
                    .split_once(':')
                    .ok_or_else(|| anyhow::anyhow!("usb target must be vid:pid (hex)"))?;
                let vendor_id = u16::from_str_radix(vid, 16)?;
                let product_id = u16::from_str_radix(pid, 16)?;
                let transport = lbl_device::UsbTransport::new(vendor_id, product_id, None);
                let status = lbl_device::query_status(&transport)?;
                println!("{}", serde_json::to_string_pretty(&status.to_view())?);
            }
            #[cfg(not(feature = "usb"))]
            {
                let _ = usb;
                bail!("USB support is not compiled in");
            }
        }
        Command::SoftReboot { usb } => {
            #[cfg(feature = "usb")]
            {
                let target = usb.ok_or_else(|| {
                    anyhow::anyhow!("pass --usb vid:pid (e.g. 0922:0028 for LabelWriter 550)")
                })?;
                let (vid, pid) = target
                    .split_once(':')
                    .ok_or_else(|| anyhow::anyhow!("usb target must be vid:pid (hex)"))?;
                let vendor_id = u16::from_str_radix(vid, 16)?;
                let product_id = u16::from_str_radix(pid, 16)?;
                let transport = lbl_device::UsbTransport::new(vendor_id, product_id, None);
                lbl_device::soft_reboot_usb(&transport)?;
                eprintln!("soft-rebooted print engine at USB {target}");
            }
            #[cfg(not(feature = "usb"))]
            {
                let _ = usb;
                bail!("USB support is not compiled in");
            }
        }
        Command::Send {
            network,
            usb,
            serial,
            bluetooth,
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
            send(network, usb, serial, bluetooth, &data)?;
            eprintln!("sent {} bytes", data.len());
        }
    }
    Ok(())
}

fn send(
    network: Option<String>,
    usb: Option<String>,
    serial: Option<String>,
    bluetooth: Option<String>,
    data: &[u8],
) -> Result<()> {
    if let Some(target) = network {
        let (host, port) = target
            .rsplit_once(':')
            .ok_or_else(|| anyhow::anyhow!("network target must be host:port"))?;
        let mut t = NetworkTransport::new(host, port.parse()?);
        t.send(data).context("send failed")?;
        return Ok(());
    }

    #[cfg(feature = "usb")]
    if let Some(target) = usb {
        let (vid, pid) = target
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("usb target must be vid:pid (hex)"))?;
        let vendor_id = u16::from_str_radix(vid, 16)?;
        let product_id = u16::from_str_radix(pid, 16)?;
        let mut t = lbl_device::UsbTransport::new(vendor_id, product_id, None);
        let transport_target = TransportTarget::Usb {
            vendor_id,
            product_id,
        };
        if let Err(err) = t.send(data) {
            bail!(format_send_failure(&err, Some(&transport_target)));
        }
        return Ok(());
    }
    #[cfg(not(feature = "usb"))]
    let _ = usb;

    #[cfg(feature = "serial")]
    if let Some(target) = serial {
        let (path, baud) = parse_serial(&target);
        let transport_target = TransportTarget::Serial { path: path.clone() };
        let mut t = lbl_device::SerialTransport::new(path, baud);
        if let Err(err) = t.send(data) {
            bail!(format_send_failure(&err, Some(&transport_target)));
        }
        return Ok(());
    }
    #[cfg(not(feature = "serial"))]
    let _ = serial;

    #[cfg(feature = "ble")]
    if let Some(target) = bluetooth {
        let mut t = lbl_device::BleTransport::new(target);
        t.send(data)?;
        return Ok(());
    }
    #[cfg(not(feature = "ble"))]
    if bluetooth.is_some() {
        bail!(
            "Bluetooth LE support is not compiled in; rebuild with `--features ble` \
             (e.g. `cargo build -p lbl-device --features ble`)"
        );
    }
    #[cfg(not(feature = "ble"))]
    let _ = bluetooth;

    bail!("no target given; use --network host:port, --usb vid:pid, --serial path[:baud], or --bluetooth name")
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

//! User-facing diagnostics for common device access failures.

use std::fmt::Write as _;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};

use crate::DeviceError;

const RESET: &str = "\x1b[0m";
const TITLE: &str = "\x1b[1;31m";
const ERROR: &str = "\x1b[31m";
const HEADING: &str = "\x1b[1;33m";
const DIM: &str = "\x1b[2m";
const CMD: &str = "\x1b[36m";
const BULLET: &str = "\x1b[38;5;245m";
const LINK: &str = "\x1b[36m";
#[cfg(target_os = "linux")]
const CUPS_ADMIN_URL: &str = "http://localhost:631/";

struct Style {
    color: bool,
}

impl Style {
    fn stderr() -> Self {
        Self {
            color: io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("{code}{text}{RESET}")
        } else {
            text.to_string()
        }
    }

    fn title(&self, text: &str) -> String {
        self.wrap(TITLE, text)
    }

    fn error_detail(&self, text: &str) -> String {
        self.wrap(ERROR, text)
    }

    fn heading(&self, text: &str) -> String {
        self.wrap(HEADING, text)
    }

    fn dim(&self, text: &str) -> String {
        self.wrap(DIM, text)
    }

    fn command(&self, text: &str) -> String {
        self.wrap(CMD, text)
    }

    fn bullet_prefix(&self) -> String {
        if self.color {
            format!("{BULLET}  •{RESET} ")
        } else {
            "  • ".into()
        }
    }

    /// OSC-8 hyperlink when color is enabled; plain `label (url)` otherwise.
    fn link(&self, url: &str, label: &str) -> String {
        if self.color {
            format!("\x1b]8;;{url}\x1b\\{LINK}{label}{RESET}\x1b]8;;\x1b\\")
        } else {
            format!("{label} ({url})")
        }
    }
}

/// The transport target that failed, used to tailor troubleshooting hints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportTarget {
    /// USB bulk device identified by vendor and product id.
    Usb {
        vendor_id: u16,
        product_id: u16,
    },
    /// USB CDC-ACM or other serial port.
    Serial {
        path: String,
    },
}

impl DeviceError {
    /// Whether the error looks like an OS permission denial.
    pub fn is_permission_denied(&self) -> bool {
        match self {
            DeviceError::Transport(msg) => transport_message_matches(msg, &[
                "permission denied",
                "access denied",
                "errno 13",
                "operation not permitted",
                "eacces",
            ]),
            DeviceError::NotFound(_) => false,
        }
    }

    /// Whether another process or driver already holds the device/interface.
    pub fn is_device_busy(&self) -> bool {
        match self {
            DeviceError::Transport(msg) => transport_message_matches(msg, &[
                "interface is busy",
                "device busy",
                "resource busy",
                "errno 16",
                "ebusy",
            ]),
            DeviceError::NotFound(_) => false,
        }
    }
}

fn transport_message_matches(msg: &str, needles: &[&str]) -> bool {
    let lower = msg.to_lowercase();
    needles.iter().any(|needle| lower.contains(needle))
}

/// Build a user-facing failure message for a spool/dispatch run that aborted.
pub fn format_dispatch_failure(
    error: &DeviceError,
    target: Option<&TransportTarget>,
    remaining: usize,
) -> String {
    let style = Style::stderr();
    let mut out = format!(
        "{}: {}",
        style.title("print failed"),
        style.error_detail(&error.to_string())
    );
    if remaining > 1 {
        let _ = write!(
            out,
            " {}",
            style.dim(&format!(
                "({remaining} labels were not sent; {remaining} still queued)"
            ))
        );
    } else if remaining == 1 {
        let _ = write!(out, " {}", style.dim("(1 label was not sent)"));
    }

    if error.is_permission_denied() {
        if let Some(target) = target {
            if let Some(section) = permission_troubleshooting(target, &style) {
                out.push_str("\n\n");
                out.push_str(&section);
            }
        }
    } else if error.is_device_busy() {
        if let Some(target) = target {
            if let Some(section) = device_busy_troubleshooting(target, &style) {
                out.push_str("\n\n");
                out.push_str(&section);
            }
        }
    }

    out
}

/// Build a user-facing failure message for a single send attempt.
pub fn format_send_failure(error: &DeviceError, target: Option<&TransportTarget>) -> String {
    let style = Style::stderr();
    let mut out = format!(
        "{}: {}",
        style.title("send failed"),
        style.error_detail(&error.to_string())
    );
    if error.is_permission_denied() {
        if let Some(target) = target {
            if let Some(section) = permission_troubleshooting(target, &style) {
                out.push_str("\n\n");
                out.push_str(&section);
            }
        }
    } else if error.is_device_busy() {
        if let Some(target) = target {
            if let Some(section) = device_busy_troubleshooting(target, &style) {
                out.push_str("\n\n");
                out.push_str(&section);
            }
        }
    }
    out
}

fn device_busy_troubleshooting(target: &TransportTarget, style: &Style) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        Some(linux_device_busy_troubleshooting(target, style))
    }
    #[cfg(target_os = "macos")]
    {
        let _ = style;
        Some(match target {
            TransportTarget::Usb { .. } => {
                "Another program is using the USB printer. Quit DYMO Connect or other label \
                 software, remove the printer from macOS Printers & Scanners if it is set up \
                 there, then unplug and replug the device."
                    .into()
            }
            TransportTarget::Serial { path } => format!(
                "Serial port {path} is in use. Close other apps that may have it open, then \
                 retry."
            ),
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (target, style);
        Some(
            "The device is in use by another program. Close other software that may be \
             connected to the printer, then retry."
                .into(),
        )
    }
}

fn permission_troubleshooting(target: &TransportTarget, style: &Style) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        Some(linux_permission_troubleshooting(target, style))
    }
    #[cfg(target_os = "macos")]
    {
        Some(macos_permission_troubleshooting(target, style))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (target, style);
        Some(
            "Device access was denied by the operating system. Try running from an \
             administrator account or check your system's USB/serial permissions."
                .into(),
        )
    }
}

#[cfg(target_os = "linux")]
fn linux_permission_troubleshooting(target: &TransportTarget, style: &Style) -> String {
    match target {
        TransportTarget::Usb {
            vendor_id,
            product_id,
        } => linux_usb_permission_troubleshooting(*vendor_id, *product_id, style),
        TransportTarget::Serial { path } => {
            linux_serial_permission_troubleshooting(path, style)
        }
    }
}

#[cfg(target_os = "linux")]
struct UsbNodeInfo {
    path: PathBuf,
    mode: Option<String>,
    owner: Option<String>,
    kernel_driver: Option<String>,
    /// Sysfs node name (e.g. `3-1.2.3`) when a printer interface has `usblp` bound.
    usblp_interface: Option<String>,
}

#[cfg(target_os = "linux")]
fn linux_usb_permission_troubleshooting(vendor_id: u16, product_id: u16, style: &Style) -> String {
    let group = "plugdev";
    let groups = current_group_names();
    let in_group = groups.iter().any(|g| g == group);
    let enumerated = usb_device_enumerated(vendor_id, product_id);
    let node_info = find_usb_device_node(vendor_id, product_id);
    let connected = enumerated || node_info.is_some();
    let udev_access_rule = udev_access_rule_for_vendor(vendor_id);

    let mut out = format!(
        "{}\n",
        style.heading("Linux denied access to the USB printer.")
    );
    push_bullet(
        &mut out,
        style,
        &format!(
            "Target device {vendor_id:04x}:{product_id:04x} {}",
            if enumerated {
                "is connected (visible to USB enumeration)"
            } else if node_info.is_some() {
                "is connected (visible in sysfs)"
            } else {
                "was not found — check that it is plugged in"
            }
        ),
    );

    if let Some(info) = node_info {
        push_bullet(
            &mut out,
            style,
            &match (&info.mode, &info.owner) {
                (Some(mode), Some(owner)) => format!(
                    "Device node {} is {mode} owned by {owner}",
                    info.path.display()
                ),
                _ => format!(
                    "Device node {} exists but its permissions could not be read",
                    info.path.display()
                ),
            },
        );
    } else if connected {
        push_bullet(
            &mut out,
            style,
            "Could not resolve the /dev/bus/usb device node (permissions may still be wrong)",
        );
    }

    if in_group {
        push_bullet(
            &mut out,
            style,
            &format!("Your user is already in the `{group}` group"),
        );
    } else {
        push_bullet(
            &mut out,
            style,
            &format!("Your user is not in the `{group}` group"),
        );
    }

    if let Some(path) = &udev_access_rule {
        push_bullet(
            &mut out,
            style,
            &format!(
                "Found a udev rule that grants device access: {}",
                path.display()
            ),
        );
    } else {
        push_bullet(
            &mut out,
            style,
            &format!("No udev rule grants `{group}` access to vendor {vendor_id:04x}"),
        );
    }

    out.push('\n');
    out.push_str(&style.heading("Try:"));
    let mut step = 1;

    if !in_group {
        if let Some(user) = current_username() {
            push_step(
                &mut out,
                style,
                step,
                &format!("Add yourself to `{group}`:"),
                &format!("sudo usermod -aG {group} {user}"),
                "",
            );
            step += 1;
            push_step(
                &mut out,
                style,
                step,
                "Log out and back in (or run",
                &format!("newgrp {group}"),
                "in this shell)",
            );
            step += 1;
        }
    }

    if udev_access_rule.is_none() {
        push_step(
            &mut out,
            style,
            step,
            &format!(
                "Install a udev rule granting `{group}` access (then reload udev and replug \
                 the printer):"
            ),
            "",
            "",
        );
        step += 1;
        let udev_rule_line = format!(
            "SUBSYSTEM==\"usb\", ATTR{{idVendor}}==\"{vendor_id:04x}\", GROUP=\"{group}\", MODE=\"0660\""
        );
        push_heredoc_block(
            &mut out,
            style,
            &[
                "sudo tee /etc/udev/rules.d/99-lbl-printer.rules <<'EOF'",
                &udev_rule_line,
                "EOF",
                "sudo udevadm control --reload-rules && sudo udevadm trigger",
            ],
        );
    } else {
        push_step(
            &mut out,
            style,
            step,
            "Unplug and replug the printer (or run",
            "sudo udevadm trigger",
            "so udev permissions apply)",
        );
        step += 1;
        if in_group {
            push_step(
                &mut out,
                style,
                step,
                &format!(
                    "If `{group}` membership or the udev rule changed recently, log out and \
                     back in"
                ),
                "",
                "",
            );
            step += 1;
        }
    }

    push_step(
        &mut out,
        style,
        step,
        "Quit DYMO Connect or other printer software that may hold the device",
        "",
        "",
    );

    out
}

#[cfg(target_os = "linux")]
fn linux_device_busy_troubleshooting(target: &TransportTarget, style: &Style) -> String {
    match target {
        TransportTarget::Usb {
            vendor_id,
            product_id,
        } => linux_usb_device_busy_troubleshooting(*vendor_id, *product_id, style),
        TransportTarget::Serial { path } => {
            let mut out = format!(
                "{}\n",
                style.heading("Serial port is already in use.")
            );
            push_bullet(
                &mut out,
                style,
                &format!("Target port {path} could not be opened exclusively"),
            );
            out.push('\n');
            out.push_str(&style.heading("Try:"));
            push_step(
                &mut out,
                style,
                1,
                "Close other programs that may be using",
                path,
                "",
            );
            push_step(
                &mut out,
                style,
                2,
                "Check which process holds the port:",
                &format!("fuser -v {path}"),
                "",
            );
            out
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_usb_device_busy_troubleshooting(vendor_id: u16, product_id: u16, style: &Style) -> String {
    let cups_running = cups_is_running();
    let cups_queues = if cups_running {
        cups_usb_queues(vendor_id, product_id)
    } else {
        Vec::new()
    };
    linux_usb_device_busy_troubleshooting_with(
        vendor_id,
        product_id,
        style,
        cups_running,
        &cups_queues,
    )
}

#[cfg(target_os = "linux")]
fn push_cups_usb_auto_add_bullet(out: &mut String, style: &Style) {
    push_bullet(
        out,
        style,
        "CUPS automatically adds USB printers when they are plugged in (udev \
         `configure-printer` / `udev-add-printer`) — if you already deleted the queue, udev \
         may have re-added it on plug-in",
    );
    push_bullet(
        out,
        style,
        "Exclude this printer from CUPS with a per-device USB quirk — other USB printers on \
         the system can keep using CUPS normally",
    );
    push_bullet(
        out,
        style,
        "Check `lpstat -v`, not only `lpstat -p` — a USB binding can appear in `-v` even when \
         `-p` looks empty",
    );
}

#[cfg(target_os = "linux")]
fn push_exclude_printer_from_cups_steps(
    out: &mut String,
    style: &Style,
    step: &mut usize,
    vendor_id: u16,
    product_id: u16,
) {
    let quirk_line = format!("0x{vendor_id:04x} 0x{product_id:04x} blacklist");
    push_step(
        out,
        style,
        *step,
        "Tell CUPS not to auto-add this printer (creates a separate quirks file; does not \
         change udev rules for other printers):",
        "",
        "",
    );
    *step += 1;
    push_heredoc_block(
        out,
        style,
        &[
            "sudo tee /usr/share/cups/usb/lbl-exclude.usb-quirks <<'EOF'",
            "# lbl: exclude this USB printer from CUPS auto-discovery",
            quirk_line.as_str(),
            "EOF",
        ],
    );
}

#[cfg(target_os = "linux")]
fn push_unbind_usblp_steps(
    out: &mut String,
    style: &Style,
    step: &mut usize,
    usblp_interface: &str,
    vendor_id: u16,
    product_id: u16,
) {
    push_step(
        out,
        style,
        *step,
        "Release the printer interface from the kernel `usblp` driver (CUPS uses this driver \
         even when no queue is listed — the CUPS USB quirk alone does not unbind it):",
        &format!("echo -n '{usblp_interface}' | sudo tee /sys/bus/usb/drivers/usblp/unbind"),
        "",
    );
    *step += 1;
    push_step(
        out,
        style,
        *step,
        "Keep `usblp` from reclaiming this printer on plug-in (other USB printers are unaffected):",
        "",
        "",
    );
    *step += 1;
    let rule = format!(
        "SUBSYSTEM==\"usb\", ATTR{{idVendor}}==\"{vendor_id:04x}\", ATTR{{idProduct}}==\"{product_id:04x}\", ATTR{{bInterfaceClass}}==\"07\", DRIVER==\"usblp\", RUN+=\"/bin/sh -c 'echo -n %k > /sys/bus/usb/drivers/usblp/unbind'\""
    );
    push_heredoc_block(
        out,
        style,
        &[
            "sudo tee /etc/udev/rules.d/99-lbl-printer-usblp.rules <<'EOF'",
            "# lbl: unbind usblp from this printer so lbl can use raw USB",
            rule.as_str(),
            "EOF",
            "sudo udevadm control --reload-rules && sudo udevadm trigger",
        ],
    );
}

#[cfg(target_os = "linux")]
fn linux_usb_device_busy_troubleshooting_with(
    vendor_id: u16,
    product_id: u16,
    style: &Style,
    cups_running: bool,
    cups_queues: &[String],
) -> String {
    let node_info = find_usb_device_node(vendor_id, product_id);
    let kernel_driver = node_info
        .as_ref()
        .and_then(|info| info.kernel_driver.as_deref());
    let usblp_interface = node_info
        .as_ref()
        .and_then(|info| info.usblp_interface.as_deref());
    let usblp_bound = usblp_interface.is_some()
        || kernel_driver.is_some_and(|driver| driver == "usblp");
    let has_cups_queue = !cups_queues.is_empty();

    let mut out = format!(
        "{}\n",
        style.heading("Another program is using the USB printer.")
    );
    push_bullet(
        &mut out,
        style,
        &format!(
            "Target device {vendor_id:04x}:{product_id:04x} is connected, but its USB interface \
             is already claimed"
        ),
    );
    push_bullet(
        &mut out,
        style,
        "`lbl` needs exclusive raw USB access — it cannot share the interface with CUPS, DYMO \
         Connect, or other printer software",
    );
    push_cups_usb_auto_add_bullet(&mut out, style);
    if has_cups_queue {
        push_bullet(
            &mut out,
            style,
            "CUPS has a print queue for this printer — delete it (Administration → Delete \
             Printer); pausing the queue or changing the driver is not enough",
        );
        for queue in cups_queues {
            push_bullet(
                &mut out,
                style,
                &format!("CUPS queue `{queue}` is bound to this printer (likely re-added by udev)"),
            );
        }
    } else if cups_running {
        push_bullet(
            &mut out,
            style,
            "CUPS is running but `lpstat -v` shows no USB queue for this printer right now — \
             other CUPS printers on the system do not block `lbl`",
        );
    }
    if cups_running {
        push_bullet(
            &mut out,
            style,
            &format!(
                "CUPS web admin: {}",
                style.link(CUPS_ADMIN_URL, CUPS_ADMIN_URL)
            ),
        );
    }
    if usblp_bound && !has_cups_queue {
        push_bullet(
            &mut out,
            style,
            "The kernel `usblp` driver is bound to this printer's USB interface — this blocks \
             raw USB access even when CUPS shows no queue; unbind `usblp` or replug after adding \
             the udev rule below",
        );
        if let Some(iface) = usblp_interface {
            push_bullet(
                &mut out,
                style,
                &format!("`usblp` is bound at `{iface}`"),
            );
        }
    } else if usblp_bound && has_cups_queue {
        push_bullet(
            &mut out,
            style,
            "The CUPS `usblp` driver has bound this device through the queue above",
        );
    } else if let Some(driver) = kernel_driver.filter(|&driver| driver != "usblp" && driver != "usb")
    {
        push_bullet(
            &mut out,
            style,
            &format!("Kernel driver `{driver}` is bound to the device"),
        );
    } else if !cups_running && !usblp_bound {
        push_bullet(
            &mut out,
            style,
            "CUPS and `usblp` do not appear to be holding this device right now — another \
             program is likely responsible",
        );
    }

    out.push('\n');
    out.push_str(&style.heading("Try:"));
    let mut step = 1;
    push_step(
        &mut out,
        style,
        step,
        "Quit DYMO Connect and any other label or printer software",
        "",
        "",
    );
    step += 1;
    push_step(
        &mut out,
        style,
        step,
        "Check whether CUPS has a USB binding for this printer:",
        "lpstat -v",
        "",
    );
    step += 1;
    if has_cups_queue {
        if cups_running {
            push_step(
                &mut out,
                style,
                step,
                "Open the CUPS web admin and delete this printer (Administration → Delete Printer):",
                CUPS_ADMIN_URL,
                "",
            );
            step += 1;
        }
        for queue in cups_queues {
            push_step(
                &mut out,
                style,
                step,
                "Delete the CUPS queue:",
                &format!("lpadmin -x {queue}"),
                "",
            );
            step += 1;
        }
    }
    push_exclude_printer_from_cups_steps(&mut out, style, &mut step, vendor_id, product_id);
    if let Some(iface) = usblp_interface {
        push_unbind_usblp_steps(&mut out, style, &mut step, iface, vendor_id, product_id);
    }
    if let Some(info) = &node_info {
        push_step(
            &mut out,
            style,
            step,
            "See which process is using the device node:",
            &format!("fuser -v {}", info.path.display()),
            "",
        );
    } else {
        push_step(
            &mut out,
            style,
            step,
            "Find the device node with",
            "lsusb",
            "and `ls -l /dev/bus/usb/*/*`, then run `fuser -v` on that path",
        );
    }
    step += 1;
    push_step(
        &mut out,
        style,
        step,
        "After applying the steps above, retry `lbl print` (unplug and replug only if the \
         interface is still busy)",
        "",
        "",
    );
    out
}

#[cfg(target_os = "linux")]
fn cups_usb_queues(vendor_id: u16, product_id: u16) -> Vec<String> {
    let Ok(output) = std::process::Command::new("lpstat").arg("-v").output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("device for ")?;
            let (queue, uri) = rest.split_once(": ")?;
            if !uri.starts_with("usb://") {
                return None;
            }
            usb_uri_matches_printer(uri, vendor_id, product_id).then_some(queue.to_string())
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn usb_uri_matches_printer(uri: &str, vendor_id: u16, product_id: u16) -> bool {
    let lower = uri.to_lowercase();
    let vid = format!("{vendor_id:04x}");
    let pid = format!("{product_id:04x}");
    if lower.contains(&vid) || lower.contains(&pid) {
        return true;
    }
    match vendor_id {
        0x0922 => lower.contains("dymo") || lower.contains("labelwriter"),
        _ => false,
    }
}

#[cfg(target_os = "linux")]
fn cups_is_running() -> bool {
    if Path::new("/run/cups/cups.sock").exists() {
        return true;
    }
    std::process::Command::new("systemctl")
        .args(["is-active", "--quiet", "cups"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn linux_serial_permission_troubleshooting(path: &str, style: &Style) -> String {
    let group = "dialout";
    let groups = current_group_names();
    let in_group = groups.iter().any(|g| g == group);
    let node = Path::new(path);
    let node_meta = node.exists().then(|| node_metadata(path)).flatten();

    let mut out = format!(
        "{}\n",
        style.heading("Linux denied access to the serial port.")
    );
    push_bullet(
        &mut out,
        style,
        &format!(
            "Target port {} {}",
            path,
            if node.exists() {
                "exists"
            } else {
                "was not found — check the path from `lbl device list`"
            }
        ),
    );

    if let Some((mode, owner)) = node_meta {
        push_bullet(
            &mut out,
            style,
            &format!("Device node is {mode} owned by {owner}"),
        );
    }

    if in_group {
        push_bullet(
            &mut out,
            style,
            &format!("Your user is already in the `{group}` group"),
        );
    } else {
        push_bullet(
            &mut out,
            style,
            &format!("Your user is not in the `{group}` group"),
        );
    }

    out.push('\n');
    out.push_str(&style.heading("Try:"));
    let mut step = 1;

    if !in_group {
        if let Some(user) = current_username() {
            push_step(
                &mut out,
                style,
                step,
                &format!("Add yourself to `{group}`:"),
                &format!("sudo usermod -aG {group} {user}"),
                "",
            );
            step += 1;
            push_step(
                &mut out,
                style,
                step,
                "Log out and back in (or run",
                &format!("newgrp {group}"),
                "in this shell)",
            );
            step += 1;
        }
    } else {
        push_step(
            &mut out,
            style,
            step,
            &format!(
                "If `{group}` was added recently, log out and back in so the new membership \
                 takes effect"
            ),
            "",
            "",
        );
        step += 1;
    }

    push_step(
        &mut out,
        style,
        step,
        "Verify the port path — use",
        "/dev/ttyACM0",
        "not `/dev/tty/ttyACM0`",
    );

    out
}

#[cfg(target_os = "macos")]
fn macos_permission_troubleshooting(target: &TransportTarget, style: &Style) -> String {
    match target {
        TransportTarget::Usb {
            vendor_id,
            product_id,
        } => format!(
            "{} Grant access in System Settings if prompted, and quit DYMO Connect or other \
             software that may be using the printer.",
            style.heading(&format!(
                "macOS denied access to USB device {vendor_id:04x}:{product_id:04x}."
            ))
        ),
        TransportTarget::Serial { path } => format!(
            "{} Check Privacy & Security settings and that no other app has the port open.",
            style.heading(&format!("macOS denied access to serial port {path}."))
        ),
    }
}

fn push_bullet(out: &mut String, style: &Style, line: &str) {
    out.push('\n');
    out.push_str(&style.bullet_prefix());
    out.push_str(line);
}

fn push_step(out: &mut String, style: &Style, step: usize, before: &str, command: &str, after: &str) {
    let _ = write!(out, "\n  {}. ", style.heading(&step.to_string()));
    out.push_str(before);
    if !command.is_empty() {
        out.push(' ');
        out.push_str(&style.command(&format!("`{command}`")));
    }
    if !after.is_empty() {
        out.push(' ');
        out.push_str(after);
    }
}

fn push_heredoc_block(out: &mut String, style: &Style, lines: &[&str]) {
    for line in lines {
        out.push('\n');
        out.push_str(&style.command(line));
    }
}

#[cfg(target_os = "linux")]
fn current_username() -> Option<String> {
    std::env::var("USER")
        .ok()
        .filter(|u| !u.is_empty())
        .or_else(|| std::env::var("LOGNAME").ok().filter(|u| !u.is_empty()))
}

#[cfg(target_os = "linux")]
fn current_group_names() -> Vec<String> {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return Vec::new();
    };
    let Some(gids) = status
        .lines()
        .find_map(|line| line.strip_prefix("Groups:\t"))
    else {
        return Vec::new();
    };

    let gid_names = group_names_by_gid();
    gids.split_whitespace()
        .filter_map(|gid| gid_names.get(gid).cloned())
        .collect()
}

#[cfg(target_os = "linux")]
fn group_names_by_gid() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Ok(group_file) = std::fs::read_to_string("/etc/group") else {
        return map;
    };
    for line in group_file.lines() {
        let mut parts = line.split(':');
        let Some(name) = parts.next() else { continue };
        let _ = parts.next();
        let Some(gid) = parts.next() else { continue };
        map.insert(gid.to_string(), name.to_string());
    }
    map
}

#[cfg(all(target_os = "linux", feature = "usb"))]
fn usb_device_enumerated(vendor_id: u16, product_id: u16) -> bool {
    use nusb::MaybeFuture;

    nusb::list_devices()
        .wait()
        .ok()
        .is_some_and(|mut devices| {
            devices.any(|d| d.vendor_id() == vendor_id && d.product_id() == product_id)
        })
}

#[cfg(all(target_os = "linux", not(feature = "usb")))]
fn usb_device_enumerated(_vendor_id: u16, _product_id: u16) -> bool {
    false
}

#[cfg(target_os = "linux")]
fn find_usblp_interface(sysfs: &Path, device_name: &str) -> Option<String> {
    let prefix = format!("{device_name}:");
    let entries = std::fs::read_dir(sysfs).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(&prefix) {
            continue;
        }
        if read_kernel_driver_name(&entry.path()).as_deref() == Some("usblp") {
            return Some(name);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn find_usb_device_node(vendor_id: u16, product_id: u16) -> Option<UsbNodeInfo> {
    let sys = Path::new("/sys/bus/usb/devices");
    let entries = std::fs::read_dir(sys).ok()?;
    for entry in entries.flatten() {
        let base = entry.path();
        let device_name = entry.file_name().to_string_lossy().into_owned();
        if device_name.contains(':') {
            continue;
        }
        let vid = read_trimmed(&base.join("idVendor"))?;
        let pid = read_trimmed(&base.join("idProduct"))?;
        if u16::from_str_radix(&vid, 16).ok()? != vendor_id {
            continue;
        }
        if u16::from_str_radix(&pid, 16).ok()? != product_id {
            continue;
        }
        let bus = read_trimmed(&base.join("busnum"))?.parse::<u16>().ok()?;
        let dev = read_trimmed(&base.join("devnum"))?.parse::<u16>().ok()?;
        let path = PathBuf::from(format!("/dev/bus/usb/{bus:03}/{dev:03}"));
        let perms = path.to_str().and_then(node_metadata);
        let kernel_driver = read_kernel_driver_name(&base);
        let usblp_interface = find_usblp_interface(sys, &device_name);
        return Some(UsbNodeInfo {
            path,
            mode: perms.as_ref().map(|(mode, _)| mode.clone()),
            owner: perms.as_ref().map(|(_, owner)| owner.clone()),
            kernel_driver,
            usblp_interface,
        });
    }
    None
}

#[cfg(target_os = "linux")]
fn read_kernel_driver_name(sysfs_device: &Path) -> Option<String> {
    let driver = sysfs_device.join("driver");
    if !driver.exists() {
        return None;
    }
    std::fs::read_link(&driver).ok().and_then(|target| {
        target
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
    })
}

#[cfg(target_os = "linux")]
fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(target_os = "linux")]
fn node_metadata(path: &str) -> Option<(String, String)> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    let mode = format!("{:o}", meta.mode() & 0o777);
    let uid = meta.uid();
    let gid = meta.gid();
    let names = group_names_by_gid();
    let owner = format!(
        "{}:{}",
        user_name(uid).unwrap_or_else(|| uid.to_string()),
        names
            .iter()
            .find_map(|(gid_s, name)| (gid_s.parse::<u32>().ok()? == gid).then_some(name.as_str()))
            .unwrap_or(&gid.to_string())
    );
    Some((mode, owner))
}

#[cfg(target_os = "linux")]
fn user_name(uid: u32) -> Option<String> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in passwd.lines() {
        let mut parts = line.split(':');
        let name = parts.next()?;
        let line_uid = parts.next()?.parse::<u32>().ok()?;
        if line_uid == uid {
            return Some(name.to_string());
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn udev_line_grants_access(line: &str, vendor_id: u16) -> bool {
    if line.trim_start().starts_with('#') {
        return false;
    }
    let lower = line.to_lowercase();
    let needle = format!("{vendor_id:04x}");
    if !lower.contains("idvendor") || !lower.contains(&needle) {
        return false;
    }
    lower.contains("group=") || lower.contains("mode=") || lower.contains("uaccess")
}

#[cfg(target_os = "linux")]
fn udev_access_rule_for_vendor(vendor_id: u16) -> Option<PathBuf> {
    for dir in ["/etc/udev/rules.d", "/usr/lib/udev/rules.d", "/lib/udev/rules.d"] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rules") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            if content
                .lines()
                .any(|line| udev_line_grants_access(line, vendor_id))
            {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_permission_denied_errors() {
        assert!(DeviceError::Transport("opening device: permission denied (errno 13)".into())
            .is_permission_denied());
        assert!(DeviceError::Transport("access denied".into()).is_permission_denied());
        assert!(!DeviceError::NotFound("gone".into()).is_permission_denied());
        assert!(!DeviceError::Transport("device busy".into()).is_permission_denied());
    }

    #[test]
    fn detects_device_busy_errors() {
        assert!(DeviceError::Transport("claiming interface: interface is busy (errno 16)".into())
            .is_device_busy());
        assert!(!DeviceError::Transport("permission denied (errno 13)".into()).is_device_busy());
    }

    #[test]
    fn dispatch_failure_includes_busy_troubleshooting() {
        let err = DeviceError::Transport("claiming interface: interface is busy (errno 16)".into());
        let msg = format_dispatch_failure(
            &err,
            Some(&TransportTarget::Usb {
                vendor_id: 0x0922,
                product_id: 0x0028,
            }),
            1,
        );
        assert!(msg.contains("print failed:"));
        assert!(msg.contains("interface is busy"));
        assert!(msg.contains("Another program is using the USB printer"));
        assert!(msg.contains("exclusive raw USB access"));
        assert!(msg.contains("DYMO Connect"));
        assert!(msg.contains("udev-add-printer"));
        assert!(msg.contains("lpstat -v"));
        assert!(msg.contains("lbl-exclude.usb-quirks"));
        assert!(msg.contains("0x0922 0x0028 blacklist"));
        assert!(!msg.contains("70-printers.rules"));
    }

    #[test]
    fn link_renders_plain_url_without_color() {
        let style = Style { color: false };
        assert_eq!(
            style.link("http://localhost:631/", "CUPS admin"),
            "CUPS admin (http://localhost:631/)"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn usb_uri_matches_dymo_printer() {
        assert!(usb_uri_matches_printer(
            "usb://DYMO/LabelWriter%20550?serial=04121002436300",
            0x0922,
            0x0028
        ));
        assert!(!usb_uri_matches_printer(
            "implicitclass://EPSON_L6190_Series/",
            0x0922,
            0x0028
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn busy_troubleshooting_lists_cups_queue_to_delete() {
        let msg = linux_usb_device_busy_troubleshooting_with(
            0x0922,
            0x0028,
            &Style { color: false },
            true,
            &["LabelWriter-550".to_string()],
        );
        assert!(msg.contains("CUPS queue `LabelWriter-550`"));
        assert!(msg.contains("lpadmin -x LabelWriter-550"));
        assert!(msg.contains("Delete Printer"));
        assert!(msg.contains("udev-add-printer"));
        assert!(msg.contains("lbl-exclude.usb-quirks"));
        assert!(msg.contains("0x0922 0x0028 blacklist"));
        assert!(!msg.contains("70-printers.rules"));
        assert!(!msg.contains("no USB queue for this printer right now"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn busy_troubleshooting_skips_cups_delete_when_no_queue() {
        let msg = linux_usb_device_busy_troubleshooting_with(
            0x0922,
            0x0028,
            &Style { color: false },
            true,
            &[],
        );
        assert!(msg.contains("no USB queue for this printer right now"));
        assert!(msg.contains("other CUPS printers"));
        assert!(msg.contains("udev-add-printer"));
        assert!(msg.contains("lbl-exclude.usb-quirks"));
        assert!(msg.contains("0x0922 0x0028 blacklist"));
        assert!(!msg.contains("70-printers.rules"));
        assert!(!msg.contains("lpadmin -x LabelWriter"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn busy_troubleshooting_omits_cups_when_not_running() {
        let msg = linux_usb_device_busy_troubleshooting_with(
            0x0922,
            0x0028,
            &Style { color: false },
            false,
            &[],
        );
        assert!(!msg.contains("no USB queue for this printer right now"));
        assert!(!msg.contains("Administration → Delete Printer"));
        assert!(msg.contains("udev-add-printer"));
        assert!(msg.contains("lbl-exclude.usb-quirks"));
        assert!(!msg.contains("70-printers.rules"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn find_usb_device_node_uses_zero_padded_dev_path() {
        if find_usb_device_node(0x0922, 0x0028).is_some_and(|info| {
            info.path.to_string_lossy().contains("/dev/bus/usb/")
                && info.path.components().count() >= 5
        }) {
            let info = find_usb_device_node(0x0922, 0x0028).unwrap();
            let path = info.path.to_string_lossy();
            assert!(
                path.starts_with("/dev/bus/usb/"),
                "unexpected path: {path}"
            );
            assert!(std::path::Path::new(path.as_ref()).exists(), "path missing: {path}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn busy_troubleshooting_includes_usblp_unbind_when_bound() {
        let Some(info) = find_usb_device_node(0x0922, 0x0028) else {
            return;
        };
        let Some(iface) = info.usblp_interface else {
            return;
        };
        let msg = linux_usb_device_busy_troubleshooting_with(
            0x0922,
            0x0028,
            &Style { color: false },
            true,
            &[],
        );
        assert!(msg.contains("usblp"));
        assert!(msg.contains(&format!("echo -n '{iface}'")));
        assert!(msg.contains("99-lbl-printer-usblp.rules"));
    }

    #[test]
    fn dispatch_failure_includes_error_without_debug() {
        let err = DeviceError::Transport("opening device: permission denied (errno 13)".into());
        let msg = format_dispatch_failure(&err, None, 4);
        assert!(msg.contains("print failed:"));
        assert!(msg.contains("permission denied"));
        assert!(msg.contains("4 labels were not sent"));
    }

    #[test]
    fn colored_failure_uses_ansi_when_enabled() {
        let err = DeviceError::Transport("permission denied".into());
        let style = Style { color: true };
        let out = format!(
            "{}: {}",
            style.title("print failed"),
            style.error_detail(&err.to_string())
        );
        assert!(out.contains("\x1b[1;31m"));
        assert!(out.contains("print failed"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn udev_line_grants_access_requires_group_or_mode() {
        assert!(udev_line_grants_access(
            r#"SUBSYSTEM=="usb", ATTR{idVendor}=="0922", GROUP="plugdev", MODE="0660""#,
            0x0922
        ));
        assert!(!udev_line_grants_access(
            r#"ATTR{idVendor}=="0922", ATTR{idProduct}=="1001", RUN+="usb_modeswitch '/%k'""#,
            0x0922
        ));
        assert!(!udev_line_grants_access(
            r#"# ATTR{idVendor}=="0922", GROUP="plugdev""#,
            0x0922
        ));
    }

    #[test]
    fn heredoc_block_starts_at_column_zero() {
        let mut out = String::new();
        push_heredoc_block(
            &mut out,
            &Style { color: false },
            &["sudo tee file <<'EOF'", "rule", "EOF"],
        );
        assert!(out.contains("\nsudo tee file <<'EOF'\nrule\nEOF"));
        assert!(!out.contains("\n     EOF"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn usb_troubleshooting_skips_group_step_when_already_member() {
        let msg = linux_usb_permission_troubleshooting(0x0922, 0x0028, &Style { color: false });
        if current_group_names().iter().any(|g| g == "plugdev") {
            assert!(msg.contains("already in the `plugdev` group"));
            assert!(!msg.contains("sudo usermod -aG plugdev"));
            assert!(
                msg.contains("Install a udev rule granting `plugdev` access")
                    || msg.contains("Unplug and replug the printer")
            );
        } else {
            assert!(msg.contains("not in the `plugdev` group"));
            assert!(msg.contains("sudo usermod -aG plugdev"));
        }
    }
}

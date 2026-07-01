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
            DeviceError::Transport(msg) => {
                let lower = msg.to_lowercase();
                lower.contains("permission denied")
                    || lower.contains("access denied")
                    || lower.contains("errno 13")
                    || lower.contains("operation not permitted")
                    || lower.contains("eacces")
            }
            DeviceError::NotFound(_) => false,
        }
    }
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
    }
    out
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
        push_shell_block(
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

fn push_shell_block(out: &mut String, style: &Style, lines: &[&str]) {
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
fn find_usb_device_node(vendor_id: u16, product_id: u16) -> Option<UsbNodeInfo> {
    let sys = Path::new("/sys/bus/usb/devices");
    let entries = std::fs::read_dir(sys).ok()?;
    for entry in entries.flatten() {
        let base = entry.path();
        let vid = read_trimmed(&base.join("idVendor"))?;
        let pid = read_trimmed(&base.join("idProduct"))?;
        if u16::from_str_radix(&vid, 16).ok()? != vendor_id {
            continue;
        }
        if u16::from_str_radix(&pid, 16).ok()? != product_id {
            continue;
        }
        let bus = read_trimmed(&base.join("busnum"))?;
        let dev = read_trimmed(&base.join("devnum"))?;
        let path = PathBuf::from(format!("/dev/bus/usb/{bus}/{dev}"));
        let perms = path.to_str().and_then(node_metadata);
        return Some(UsbNodeInfo {
            path,
            mode: perms.as_ref().map(|(mode, _)| mode.clone()),
            owner: perms.as_ref().map(|(_, owner)| owner.clone()),
        });
    }
    None
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
    fn shell_block_starts_at_column_zero() {
        let mut out = String::new();
        push_shell_block(
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

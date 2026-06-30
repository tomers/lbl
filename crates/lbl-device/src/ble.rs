//! Shared Bluetooth LE helpers for NIIMBOT printer discovery and transport.
//!
//! NIIMBOT D-series printers advertise a vendor GATT service documented at
//! <https://printers.niim.blue/interfacing/connecting/> and used by NiimBlue,
//! niimprint, and other community clients.

use uuid::Uuid;

/// NIIMBOT vendor GATT service (128-bit).
pub const NIIMBOT_SERVICE: Uuid = Uuid::from_u128(0xe7810a71_73ae_499d_8c15_faa9aef0c3f2);

/// NIIMBOT print/notify characteristic (128-bit). Supports write-without-response
/// and notify on the same handle.
pub const NIIMBOT_CHAR: Uuid = Uuid::from_u128(0xbef8d6c9_9c21_4c9e_b632_bd58c1009f9f);

/// Initial GATT connection packet sent before the framed print stream.
///
/// Documented by NiimBlue / niimbluelib; arms the printer for a print session.
pub const NIIMBOT_BLE_CONNECT: [u8; 9] = [0x03, 0x55, 0x55, 0xC1, 0x01, 0x01, 0xC1, 0xAA, 0xAA];

/// Does an advertised local name look like a NIIMBOT label printer?
pub fn name_looks_like_niimbot(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    if n.contains("niimbot") {
        return true;
    }
    // Common model prefixes; pocket D-series advertise as "D110-XXXXXXXX".
    const PREFIXES: &[&str] = &[
        "d110", "d101", "d11", "b203", "b21", "b18", "b1", "b3s", "z401",
    ];
    PREFIXES.iter().any(|p| n.starts_with(p))
}

/// Whether advertisement data identifies a NIIMBOT printer (by name or service).
#[cfg(feature = "ble")]
pub fn props_look_like_niimbot(props: &btleplug::api::PeripheralProperties, address: &str) -> bool {
    if props
        .local_name
        .as_deref()
        .is_some_and(name_looks_like_niimbot)
    {
        return true;
    }
    if props.services.iter().any(|u| *u == NIIMBOT_SERVICE) {
        return true;
    }
    // Some firmwares only expose the service in service_data keys.
    if props.service_data.contains_key(&NIIMBOT_SERVICE) {
        return true;
    }
    // NIIMBOT BLE addresses often use a recognizable pattern (e.g. xx:03:03:…).
    let _ = address;
    false
}

/// Human-readable label for a peripheral: `"D110-ABC (26:03:03:C3:F9:11)"` or just
/// the address when no name was advertised.
#[cfg(feature = "ble")]
pub async fn peripheral_label(p: &btleplug::platform::Peripheral) -> String {
    use btleplug::api::Peripheral as _;
    let addr = p.address().to_string();
    match p.properties().await {
        Ok(Some(props)) if props.local_name.as_ref().is_some_and(|n| !n.is_empty()) => {
            format!("{} ({addr})", props.local_name.as_ref().unwrap())
        }
        _ => addr,
    }
}

/// Whether `target` matches this peripheral. An empty `target` matches any
/// NIIMBOT-looking device; otherwise the advertised name or BLE address must
/// contain `target` (case-insensitive).
#[cfg(feature = "ble")]
pub async fn peripheral_matches_target(p: &btleplug::platform::Peripheral, target: &str) -> bool {
    use btleplug::api::Peripheral as _;

    let needle = target.to_ascii_lowercase();
    let addr = p.address().to_string().to_ascii_lowercase();

    let props = match p.properties().await {
        Ok(Some(p)) => p,
        _ => {
            if needle.is_empty() {
                return false;
            }
            return addr.contains(&needle);
        }
    };

    if needle.is_empty() {
        return props_look_like_niimbot(&props, &addr);
    }

    if addr.contains(&needle) {
        return true;
    }
    if props
        .local_name
        .as_ref()
        .is_some_and(|n| n.to_ascii_lowercase().contains(&needle))
    {
        return true;
    }

    // Allow `--bluetooth niimbot` to pick the sole NIIMBOT-service device even
    // when it advertises no local name.
    if needle == "niimbot" && props_look_like_niimbot(&props, &addr) {
        return true;
    }

    false
}

//! Shared Bluetooth LE helpers for label-printer discovery and transport.
//!
//! Covers NIIMBOT D-series (vendor GATT service documented at
//! <https://printers.niim.blue/interfacing/connecting/>) and DYMO LetraTag
//! LT-200B (service UUID prefix `be3dd650-…`).

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

/// LetraTag LT-200B primary GATT service (canonical body; prefix is stable).
pub const LETRATAG_SERVICE: Uuid = Uuid::from_u128(0xbe3dd650_2b3d_42f1_99c1_f0f749dd0678);

/// LetraTag print-request characteristic (write-without-response).
pub const LETRATAG_WRITE: Uuid = Uuid::from_u128(0xbe3dd651_2b3d_42f1_99c1_f0f749dd0678);

/// LetraTag print-reply characteristic (notify).
pub const LETRATAG_NOTIFY: Uuid = Uuid::from_u128(0xbe3dd652_2b3d_42f1_99c1_f0f749dd0678);

/// LetraTag short-command characteristic (set-cassette).
pub const LETRATAG_SHORT: Uuid = Uuid::from_u128(0xbe3dd653_2b3d_42f1_99c1_f0f749dd0678);

/// Whether two UUIDs share the same first 8 hex digits (LetraTag firmware may
/// vary the UUID body while keeping the `be3dd65x` prefix).
pub fn uuid_prefix_eq(a: Uuid, b: Uuid) -> bool {
    (a.as_u128() >> 96) == (b.as_u128() >> 96)
}

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

/// Does an advertised local name look like a DYMO LetraTag LT-200B?
pub fn name_looks_like_letratag(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("letratag")
        || n.contains("letra-tag")
        || n.contains("letra tag")
        || n.starts_with("dymo lt-200")
        || n.starts_with("dymo lt200")
        || n.starts_with("lt-200")
        || n.starts_with("lt200")
        || n.starts_with("lt20")
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
    let _ = address;
    false
}

/// Whether advertisement data identifies a LetraTag printer (by name or service).
#[cfg(feature = "ble")]
pub fn props_look_like_letratag(
    props: &btleplug::api::PeripheralProperties,
    address: &str,
) -> bool {
    if props
        .local_name
        .as_deref()
        .is_some_and(name_looks_like_letratag)
    {
        return true;
    }
    if props
        .services
        .iter()
        .any(|u| uuid_prefix_eq(*u, LETRATAG_SERVICE))
    {
        return true;
    }
    if props
        .service_data
        .keys()
        .any(|u| uuid_prefix_eq(*u, LETRATAG_SERVICE))
    {
        return true;
    }
    let _ = address;
    false
}

/// Any known label-printer BLE advertisement (NIIMBOT or LetraTag).
#[cfg(feature = "ble")]
pub fn props_look_like_label_printer(
    props: &btleplug::api::PeripheralProperties,
    address: &str,
) -> bool {
    props_look_like_niimbot(props, address) || props_look_like_letratag(props, address)
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
/// known label printer; otherwise the advertised name or BLE address must
/// contain `target` (case-insensitive). Tokens `niimbot` / `letratag` match by
/// family when the device has no useful local name.
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
        return props_look_like_label_printer(&props, &addr);
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

    if (needle == "niimbot" || needle == "d110" || needle == "d11")
        && props_look_like_niimbot(&props, &addr)
    {
        return true;
    }
    if (needle == "letratag"
        || needle == "letra-tag"
        || needle == "lt200b"
        || needle == "lt-200b"
        || needle == "lt20")
        && props_look_like_letratag(&props, &addr)
    {
        return true;
    }

    false
}

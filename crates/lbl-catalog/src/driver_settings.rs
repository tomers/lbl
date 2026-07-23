//! Protocol-specific driver options described as JSON Schema.
//!
//! Each device exposes its tunable print/cut options as a JSON Schema so a
//! front-end can render a settings form generically, instead of hard-coding a
//! form per protocol. The values a user produces are merged into a job's
//! `driver` options bag, where each driver reads only the sub-object it owns
//! (e.g. `dymo` for LabelWriter, `silhouette` for GPGL cutters).
//!
//! Schemas are machine-stable only (types, enums, defaults, bounds). Display
//! titles and descriptions belong to the consuming UI (i18n).
//!
//! The helpers alongside the schema builder ([`media_noun`],
//! [`supports_orientation`], [`supports_high_speed`]) describe capabilities
//! derived from a device's protocol/model rather than stored in the catalog
//! data.

use lbl_core::printer::Protocol;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::model::{DeviceEntry, DeviceRole};

/// Machine-stable noun for the device's loaded consumable.
///
/// Consumers map these tokens to localized UI copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaNoun {
    /// Generic consumable (default).
    Media,
    /// Continuous tape (e.g. Brother P-touch TZe).
    Tape,
    /// Die-cut or continuous paper (e.g. Brother QL).
    PaperType,
}

/// JSON Schema for a device's protocol-specific driver options, or `None` when
/// the protocol exposes no tunable options.
///
/// The schema's top-level `properties` are keyed by the driver that consumes
/// them (`dymo`, `silhouette`, …) so several protocols can coexist in one
/// `driver` bag without clashing. No `title` / `description` fields are
/// emitted — those are presentation concerns.
pub fn schema_for(device: &DeviceEntry) -> Option<Value> {
    if device.role == DeviceRole::Cutter || device.protocol == Protocol::Gpgl {
        return Some(silhouette_schema());
    }
    match device.protocol {
        Protocol::DymoLw => Some(dymo_lw_schema(device)),
        Protocol::DymoLwClassic => Some(dymo_lw_classic_schema()),
        _ => None,
    }
}

fn dymo_lw_classic_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "dymo": {
                "type": "object",
                "properties": {
                    "output_mode": {
                        "type": "string",
                        "enum": ["text", "graphics"],
                        "default": "text"
                    },
                    "roll": {
                        "type": "string",
                        "enum": ["auto", "left", "right"],
                        "default": "auto"
                    }
                },
                "additionalProperties": false
            }
        }
    })
}

fn dymo_lw_schema(device: &DeviceEntry) -> Value {
    let speed = if supports_high_speed(device) {
        json!({
            "type": "string",
            "enum": ["normal", "high"],
            "default": "normal"
        })
    } else {
        json!({
            "type": "string",
            "enum": ["normal"],
            "default": "normal"
        })
    };
    json!({
        "type": "object",
        "properties": {
            "dymo": {
                "type": "object",
                "properties": {
                    "output_mode": {
                        "type": "string",
                        "enum": ["text", "graphics"],
                        "default": "text"
                    },
                    "speed": speed
                },
                "additionalProperties": false
            }
        }
    })
}

fn silhouette_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "silhouette": {
                "type": "object",
                "properties": {
                    "force": {
                        "type": "number",
                        "minimum": 1,
                        "maximum": 33,
                        "default": 10
                    },
                    "speed": {
                        "type": "number",
                        "minimum": 1,
                        "maximum": 10,
                        "default": 5
                    },
                    "mat": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 2,
                        "default": 1
                    }
                },
                "additionalProperties": false
            }
        }
    })
}

/// Noun token for the device's loaded consumable.
///
/// Brother P-touch runs continuous TZe tape; Brother QL runs die-cut/roll
/// paper; everything else is generic media.
pub fn media_noun(device: &DeviceEntry) -> MediaNoun {
    match device.protocol {
        Protocol::BrotherPt => MediaNoun::Tape,
        Protocol::BrotherQl => MediaNoun::PaperType,
        _ => MediaNoun::Media,
    }
}

/// Whether the user can choose a reading-frame orientation for the device.
///
/// DYMO LabelManager (D1) and Brother P-touch print tape with a fixed vertical
/// head, so orientation follows the media rather than a user toggle.
pub fn supports_orientation(device: &DeviceEntry) -> bool {
    !matches!(device.protocol, Protocol::Dymo | Protocol::BrotherPt)
}

/// Whether the protocol's bitmap width runs along the feed axis (not the head).
///
/// DYMO D1 tape and LetraTag encode this way; most other drivers use head-width
/// as bitmap width.
pub fn bitmap_width_is_feed(device: &DeviceEntry) -> bool {
    matches!(device.protocol, Protocol::Dymo | Protocol::LetraTag)
}

/// Whether the device offers a user-selectable high print speed.
///
/// DYMO LabelWriter 550 / 550 Turbo do; the LabelWriter 5XL chassis does not.
pub fn supports_high_speed(device: &DeviceEntry) -> bool {
    device.protocol == Protocol::DymoLw && !is_5xl(device)
}

/// Whether the protocol commonly supports raw network (TCP/9100) printing.
pub fn supports_network(device: &DeviceEntry) -> bool {
    matches!(
        device.protocol,
        Protocol::Zpl
            | Protocol::Ezpl
            | Protocol::Tspl
            | Protocol::Tpcl
            | Protocol::Sbpl
            | Protocol::Slcs
            | Protocol::Dpl
            | Protocol::EscPos
            | Protocol::BrotherQl
            | Protocol::BrotherPt
            | Protocol::Gpgl
    ) || device
        .connections
        .iter()
        .any(|c| matches!(c, crate::model::ConnectionHint::Network { .. }))
}

/// Default browser transport API for a protocol when no connection hints exist.
pub fn default_browser_api(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Niimbot => "web_serial",
        Protocol::LetraTag => "web_bluetooth",
        Protocol::Phomemo
        | Protocol::PhomemoM02x
        | Protocol::PhomemoM110
        | Protocol::PhomemoD30 => "web_bluetooth",
        Protocol::Dymo | Protocol::DymoLw | Protocol::DymoLwClassic => "webusb",
        Protocol::BrotherQl | Protocol::BrotherPt => "webusb",
        _ => "webusb",
    }
}

/// Silhouette / GPGL mat index → cuttable sheet size in millimeters.
///
/// Indices align with the `silhouette.mat` driver setting and catalog mat media.
pub fn cutter_mat_sheet_mm(mat: u32) -> (f64, f64) {
    match mat {
        0 | 1 => (304.8, 304.8),
        2 => (304.8, 609.6),
        3 => (381.0, 381.0),
        4 => (609.6, 609.6),
        _ => (304.8, 304.8),
    }
}

fn is_5xl(device: &DeviceEntry) -> bool {
    device
        .keys
        .iter()
        .any(|k| k.to_ascii_uppercase().contains("5XL"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Catalog;

    fn device<'a>(catalog: &'a Catalog, key: &str) -> &'a DeviceEntry {
        catalog.lookup_device(key).expect("device in catalog")
    }

    fn assert_no_presentation_keys(value: &Value) {
        match value {
            Value::Object(map) => {
                assert!(
                    !map.contains_key("title"),
                    "schema must not embed title: {value}"
                );
                assert!(
                    !map.contains_key("description"),
                    "schema must not embed description: {value}"
                );
                for child in map.values() {
                    assert_no_presentation_keys(child);
                }
            }
            Value::Array(items) => {
                for child in items {
                    assert_no_presentation_keys(child);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn dymo_lw_550_offers_high_speed_in_schema() {
        let catalog = Catalog::bundled().unwrap();
        let schema = schema_for(device(&catalog, "LabelWriter 550")).unwrap();
        assert_no_presentation_keys(&schema);
        let speed = &schema["properties"]["dymo"]["properties"]["speed"];
        assert_eq!(speed["enum"], json!(["normal", "high"]));
        let output = &schema["properties"]["dymo"]["properties"]["output_mode"];
        assert_eq!(output["default"], json!("text"));
        assert_eq!(output["enum"], json!(["text", "graphics"]));
    }

    #[test]
    fn dymo_lw_5xl_drops_high_speed_from_schema() {
        let catalog = Catalog::bundled().unwrap();
        let schema = schema_for(device(&catalog, "LabelWriter 5XL")).unwrap();
        assert_no_presentation_keys(&schema);
        let speed = &schema["properties"]["dymo"]["properties"]["speed"];
        assert_eq!(speed["enum"], json!(["normal"]));
        assert!(speed.get("oneOf").is_none());
    }

    #[test]
    fn cutter_exposes_silhouette_schema() {
        let catalog = Catalog::bundled().unwrap();
        let schema = schema_for(device(&catalog, "cameo4")).unwrap();
        assert_no_presentation_keys(&schema);
        let silhouette = &schema["properties"]["silhouette"]["properties"];
        assert_eq!(silhouette["force"]["maximum"], json!(33));
        assert_eq!(silhouette["speed"]["maximum"], json!(10));
        assert_eq!(silhouette["mat"]["type"], json!("integer"));
        assert_eq!(silhouette["mat"]["maximum"], json!(2));
    }

    #[test]
    fn non_tunable_protocols_have_no_schema() {
        let catalog = Catalog::bundled().unwrap();
        assert!(schema_for(device(&catalog, "QL-820NWBc")).is_none());
        assert!(schema_for(device(&catalog, "LabelManager 280")).is_none());
        assert!(schema_for(device(&catalog, "D110")).is_none());
    }

    #[test]
    fn dymo_lw_classic_offers_roll_and_output_mode() {
        let catalog = Catalog::bundled().unwrap();
        let schema = schema_for(device(&catalog, "LabelWriter 450")).unwrap();
        assert_no_presentation_keys(&schema);
        let dymo = &schema["properties"]["dymo"]["properties"];
        assert_eq!(dymo["roll"]["enum"], json!(["auto", "left", "right"]));
        assert_eq!(dymo["output_mode"]["enum"], json!(["text", "graphics"]));
    }

    #[test]
    fn media_noun_follows_protocol() {
        let catalog = Catalog::bundled().unwrap();
        assert_eq!(media_noun(device(&catalog, "PT-E550W")), MediaNoun::Tape);
        assert_eq!(
            media_noun(device(&catalog, "QL-820NWBc")),
            MediaNoun::PaperType
        );
        assert_eq!(
            media_noun(device(&catalog, "LabelWriter 550")),
            MediaNoun::Media
        );
        assert_eq!(media_noun(device(&catalog, "D110")), MediaNoun::Media);
        assert_eq!(
            serde_json::to_value(MediaNoun::PaperType).unwrap(),
            json!("paper_type")
        );
    }

    #[test]
    fn orientation_is_fixed_for_vertical_tape_heads() {
        let catalog = Catalog::bundled().unwrap();
        assert!(!supports_orientation(device(&catalog, "LabelManager 280")));
        assert!(!supports_orientation(device(&catalog, "PT-E550W")));
        assert!(supports_orientation(device(&catalog, "LabelWriter 550")));
        assert!(supports_orientation(device(&catalog, "QL-820NWBc")));
        assert!(supports_orientation(device(&catalog, "D110")));
    }

    #[test]
    fn high_speed_is_dymo_lw_except_5xl() {
        let catalog = Catalog::bundled().unwrap();
        assert!(supports_high_speed(device(&catalog, "LabelWriter 550")));
        assert!(supports_high_speed(device(
            &catalog,
            "LabelWriter 550 Turbo"
        )));
        assert!(!supports_high_speed(device(&catalog, "LabelWriter 5XL")));
        assert!(!supports_high_speed(device(&catalog, "QL-820NWBc")));
    }
}

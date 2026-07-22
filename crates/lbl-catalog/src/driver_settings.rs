//! Protocol-specific driver options described as JSON Schema.
//!
//! Each device exposes its tunable print/cut options as a JSON Schema so a
//! front-end can render a settings form generically, instead of hard-coding a
//! form per protocol. The values a user produces are merged into a job's
//! `driver` options bag, where each driver reads only the sub-object it owns
//! (e.g. `dymo` for LabelWriter, `silhouette` for GPGL cutters).
//!
//! The helpers alongside the schema builder ([`media_noun`],
//! [`supports_orientation`], [`supports_high_speed`]) describe UI-facing
//! capabilities that are derived from a device's protocol/model rather than
//! stored in the catalog data.

use lbl_core::printer::Protocol;
use serde_json::{json, Value};

use crate::model::{DeviceEntry, DeviceRole};

/// JSON Schema for a device's protocol-specific driver options, or `None` when
/// the protocol exposes no tunable options.
///
/// The schema's top-level `properties` are keyed by the driver that consumes
/// them (`dymo`, `silhouette`, …) so several protocols can coexist in one
/// `driver` bag without clashing.
pub fn schema_for(device: &DeviceEntry) -> Option<Value> {
    if device.role == DeviceRole::Cutter || device.protocol == Protocol::Gpgl {
        return Some(silhouette_schema());
    }
    match device.protocol {
        Protocol::DymoLw => Some(dymo_lw_schema(device)),
        _ => None,
    }
}

fn dymo_lw_schema(device: &DeviceEntry) -> Value {
    let speed = if supports_high_speed(device) {
        json!({
            "type": "string",
            "enum": ["normal", "high"],
            "title": "Print speed",
            "default": "normal",
            "oneOf": [
                { "const": "normal", "title": "Normal" },
                { "const": "high", "title": "High" }
            ]
        })
    } else {
        json!({
            "type": "string",
            "enum": ["normal"],
            "title": "Print speed",
            "default": "normal",
            "oneOf": [
                { "const": "normal", "title": "Normal" }
            ]
        })
    };
    json!({
        "type": "object",
        "properties": {
            "dymo": {
                "type": "object",
                "title": "LabelWriter",
                "properties": {
                    "output_mode": {
                        "type": "string",
                        "enum": ["text", "graphics"],
                        "title": "Print mode",
                        "default": "text",
                        "oneOf": [
                            { "const": "text", "title": "Text" },
                            { "const": "graphics", "title": "Graphics & barcodes" }
                        ]
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
                "title": "Cutter",
                "properties": {
                    "force": {
                        "type": "number",
                        "title": "Force",
                        "description": "Blade down-force (1–33).",
                        "minimum": 1,
                        "maximum": 33,
                        "default": 10
                    },
                    "speed": {
                        "type": "number",
                        "title": "Speed",
                        "description": "Feed speed (1–10).",
                        "minimum": 1,
                        "maximum": 10,
                        "default": 5
                    },
                    "mat": {
                        "type": "integer",
                        "title": "Mat",
                        "description": "Cutting mat preset (0 = none, 1 = 12×12 in, 2 = 12×24 in).",
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

/// Noun for the device's loaded consumable in UI copy.
///
/// Brother P-touch runs continuous TZe *tape*; Brother QL runs die-cut/roll
/// *paper*; everything else is generic *media*.
pub fn media_noun(device: &DeviceEntry) -> &'static str {
    match device.protocol {
        Protocol::BrotherPt => "tape",
        Protocol::BrotherQl => "paper type",
        _ => "media",
    }
}

/// Whether the user can choose a reading-frame orientation for the device.
///
/// DYMO LabelManager (D1) and Brother P-touch print tape with a fixed vertical
/// head, so orientation follows the media rather than a user toggle.
pub fn supports_orientation(device: &DeviceEntry) -> bool {
    !matches!(device.protocol, Protocol::Dymo | Protocol::BrotherPt)
}

/// Whether the device offers a user-selectable high print speed.
///
/// DYMO LabelWriter 550 / 550 Turbo do; the LabelWriter 5XL chassis does not.
pub fn supports_high_speed(device: &DeviceEntry) -> bool {
    device.protocol == Protocol::DymoLw && !is_5xl(device)
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

    #[test]
    fn dymo_lw_550_offers_high_speed_in_schema() {
        let catalog = Catalog::bundled().unwrap();
        let schema = schema_for(device(&catalog, "LabelWriter 550")).unwrap();
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
        let speed = &schema["properties"]["dymo"]["properties"]["speed"];
        assert_eq!(speed["enum"], json!(["normal"]));
        assert_eq!(
            speed["oneOf"],
            json!([{ "const": "normal", "title": "Normal" }])
        );
    }

    #[test]
    fn cutter_exposes_silhouette_schema() {
        let catalog = Catalog::bundled().unwrap();
        let schema = schema_for(device(&catalog, "cameo4")).unwrap();
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
    fn media_noun_follows_protocol() {
        let catalog = Catalog::bundled().unwrap();
        assert_eq!(media_noun(device(&catalog, "PT-E550W")), "tape");
        assert_eq!(media_noun(device(&catalog, "QL-820NWBc")), "paper type");
        assert_eq!(media_noun(device(&catalog, "LabelWriter 550")), "media");
        assert_eq!(media_noun(device(&catalog, "D110")), "media");
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

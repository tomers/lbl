//! Media (label / tape) description.
//!
//! A [`Media`] is the physical substrate a job is printed on. It is resolved
//! either from device auto-detection, an explicit user override, or a catalog
//! entry (see the `lbl-catalog` crate, which maps SKUs like `11352` to a
//! [`Media`] profile).

use serde::{Deserialize, Serialize};

use crate::geometry::Margins;
use crate::units::{Dots, Dpi, Millimeters};

/// The length dimension of a media, which may be a fixed die-cut label or a
/// continuous roll/tape.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "mm")]
pub enum MediaLength {
    /// A die-cut label of a fixed length in millimeters.
    Fixed(f64),
    /// A continuous roll/tape with no fixed length.
    Continuous,
}

/// Physical material of the media.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Material {
    /// Standard thermal paper.
    #[default]
    Paper,
    /// Polypropylene / plastic (durable).
    Polypropylene,
    /// Vinyl.
    Vinyl,
    /// Nylon.
    Nylon,
    /// Other / unknown.
    Other,
}

/// Adhesive backing type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Adhesive {
    /// Permanent adhesive.
    #[default]
    Permanent,
    /// Removable adhesive.
    Removable,
    /// Extra-strong adhesive.
    Strong,
    /// No adhesive (e.g. continuous receipt paper).
    None,
}

/// Base color of the media.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MediaColor {
    /// White.
    #[default]
    White,
    /// Transparent.
    Transparent,
    /// Yellow.
    Yellow,
    /// Red.
    Red,
    /// Other / unspecified.
    Other,
}

/// A fully-resolved media profile, sufficient to size a render and drive a
/// printer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Media {
    /// Printable width across the head, in millimeters.
    pub width_mm: f64,
    /// Length dimension (fixed label or continuous).
    pub length: MediaLength,
    /// Device resolution this media is printed at.
    pub dpi: Dpi,
    /// Unprintable margins.
    #[serde(default)]
    pub margins: Margins,
    /// Material.
    #[serde(default)]
    pub material: Material,
    /// Adhesive.
    #[serde(default)]
    pub adhesive: Adhesive,
    /// Base color.
    #[serde(default)]
    pub color: MediaColor,
}

impl Media {
    /// A continuous-roll media of the given width at the given dpi.
    pub fn continuous(width_mm: f64, dpi: Dpi) -> Self {
        Self {
            width_mm,
            length: MediaLength::Continuous,
            dpi,
            margins: Margins::default(),
            material: Material::default(),
            adhesive: Adhesive::default(),
            color: MediaColor::default(),
        }
    }

    /// A fixed die-cut label of `width_mm` x `length_mm` at the given dpi.
    pub fn fixed(width_mm: f64, length_mm: f64, dpi: Dpi) -> Self {
        Self {
            width_mm,
            length: MediaLength::Fixed(length_mm),
            dpi,
            margins: Margins::default(),
            material: Material::default(),
            adhesive: Adhesive::default(),
            color: MediaColor::default(),
        }
    }

    /// Printable width in device dots.
    pub fn width_dots(&self) -> Dots {
        Millimeters(self.width_mm).to_dots(self.dpi)
    }

    /// Printable length in device dots, if the media has a fixed length.
    pub fn length_dots(&self) -> Option<Dots> {
        match self.length {
            MediaLength::Fixed(mm) => Some(Millimeters(mm).to_dots(self.dpi)),
            MediaLength::Continuous => None,
        }
    }

    /// Fixed feed length in millimeters, if the media is die-cut.
    pub fn fixed_length_mm(&self) -> Option<f64> {
        match self.length {
            MediaLength::Fixed(mm) => Some(mm),
            MediaLength::Continuous => None,
        }
    }
}

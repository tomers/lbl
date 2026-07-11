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

/// How the printer senses label boundaries along the feed.
///
/// Industrial dialects (TSPL / ZPL) emit different setup commands based on this
/// metadata. When unset on a catalog row, drivers infer a sensible default from
/// [`MediaLength`] (gap for die-cut, none for continuous).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MediaSense {
    /// Inter-label gap sensing (TSPL `GAP`, ZPL `^MNY`).
    Gap {
        /// Gap height along the feed, in millimeters.
        #[serde(default = "default_gap_mm")]
        gap_mm: f64,
        /// Offset from the gap edge to the print start, in millimeters.
        #[serde(default)]
        offset_mm: f64,
    },
    /// Black-mark sensing on the liner or label back (TSPL `BLINE`, ZPL `^MNM`).
    BlackMark {
        /// Mark height along the feed, in millimeters.
        #[serde(default = "default_mark_mm")]
        mark_mm: f64,
        /// Offset from the mark to the print start, in millimeters.
        #[serde(default)]
        offset_mm: f64,
    },
    /// Continuous stock with no inter-label sensor mark (TSPL `GAP 0`, ZPL `^MNN`).
    Continuous,
}

fn default_gap_mm() -> f64 {
    3.0
}

fn default_mark_mm() -> f64 {
    3.0
}

impl Default for MediaSense {
    fn default() -> Self {
        Self::Gap {
            gap_mm: default_gap_mm(),
            offset_mm: 0.0,
        }
    }
}

impl MediaSense {
    /// Infer sensing from length when a catalog row omits an explicit sense.
    pub fn inferred_from_length(length: MediaLength) -> Self {
        match length {
            MediaLength::Fixed(_) => Self::default(),
            MediaLength::Continuous => Self::Continuous,
        }
    }
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
    /// Whether this consumable needs a dual-ink (two-color) encode path.
    ///
    /// When set, the pipeline supplies a secondary ink plane alongside the
    /// primary bitmap; drivers that support dual-color media consume both.
    #[serde(default)]
    pub two_color: bool,
    /// Label-boundary sensing for industrial dialects (TSPL / ZPL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sense: Option<MediaSense>,
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
            two_color: false,
            sense: Some(MediaSense::Continuous),
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
            two_color: false,
            sense: Some(MediaSense::default()),
        }
    }

    /// Effective boundary sensing, falling back from [`MediaLength`] when unset.
    pub fn sense_or_inferred(&self) -> MediaSense {
        self.sense
            .unwrap_or_else(|| MediaSense::inferred_from_length(self.length))
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

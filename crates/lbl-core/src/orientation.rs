//! Label orientation and quarter-turn rotation.
//!
//! Printers feed media in one fixed direction: the print head spans a fixed
//! *width* (across the head) and the media advances along its *length* (the
//! feed). Media is also marketed by those physical dimensions (e.g. `12x40`),
//! so the head width and feed length are fixed facts about the hardware and
//! tape — not something to swap.
//!
//! What *is* a choice is how label content is laid out relative to that feed:
//!
//! - [`Orientation::Portrait`] renders content upright in the media's natural
//!   `width × length` frame (text runs across the head).
//! - [`Orientation::Landscape`] renders content in the transposed
//!   `length × width` frame and turns it a quarter onto the head, so text runs
//!   along the (usually longer) feed direction. Stripe labels are rarely
//!   square and people generally print along the long dimension, so landscape
//!   is the default.
//!
//! Some die-cut SKUs (notably NIIMBOT B1 labels such as `50x30`) list the
//! wider dimension first even though it spans the print head, so the head width
//! exceeds the feed length. [`Orientation::for_media`] inverts portrait and
//! landscape for those profiles so the UI icons still match the preview aspect
//! ratio.
//!
//! Orientation, plus any extra [`Rotation`] quarter-turns, resolves to a single
//! [`Rotation`] that the pipeline applies to the *rendered raster* after laying
//! the content out in the chosen frame (see [`Rotation::for_print`]).

use serde::{Deserialize, Serialize};

use crate::media::Media;
use crate::printer::Protocol;

/// How label content is laid out relative to the media feed direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    /// Content is rendered in the media's natural `width × length` frame; text
    /// runs across the print head.
    Portrait,
    /// Content is rendered in the transposed `length × width` frame and turned
    /// a quarter onto the head; text runs along the feed direction. This is the
    /// default because stripe labels are usually printed along their longer
    /// dimension.
    #[default]
    Landscape,
}

impl Orientation {
    /// Map a user-facing portrait/landscape choice to the print orientation for
    /// the given media. When the head width exceeds the feed length the natural
    /// reading frame is wider than tall, which inverts the usual semantics.
    pub fn for_media(self, media: &Media) -> Self {
        match media.fixed_length_mm() {
            Some(len) if media.width_mm > len => match self {
                Orientation::Portrait => Orientation::Landscape,
                Orientation::Landscape => Orientation::Portrait,
            },
            _ => self,
        }
    }
}

/// A rotation in 90° steps, expressed as clockwise quarter-turns and applied to
/// the rendered raster before it is dithered and encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rotation {
    /// No rotation.
    #[default]
    None,
    /// 90° clockwise.
    Cw90,
    /// 180°.
    Cw180,
    /// 270° clockwise (i.e. 90° counter-clockwise).
    Cw270,
}

impl Rotation {
    /// Normalize a signed number of clockwise quarter-turns into a [`Rotation`].
    pub fn from_quarter_turns_cw(turns: i32) -> Self {
        match turns.rem_euclid(4) {
            0 => Rotation::None,
            1 => Rotation::Cw90,
            2 => Rotation::Cw180,
            _ => Rotation::Cw270,
        }
    }

    /// The number of clockwise quarter-turns this rotation represents (`0..=3`).
    pub fn quarter_turns_cw(self) -> u8 {
        match self {
            Rotation::None => 0,
            Rotation::Cw90 => 1,
            Rotation::Cw180 => 2,
            Rotation::Cw270 => 3,
        }
    }

    /// Whether applying this rotation swaps the width and height axes (true for
    /// the 90°/270° quarter-turns).
    pub fn swaps_axes(self) -> bool {
        matches!(self, Rotation::Cw90 | Rotation::Cw270)
    }

    /// Whether the nominal reading-frame feed-start edge lands at the trailing
    /// side of the physical feed after this quarter-turn.
    ///
    /// Nominal start is left when [`swaps_axes`] is true, otherwise top. Half
    /// turns ([`Cw180`] / [`Cw270`], e.g. landscape + 180° extra) reverse that
    /// edge, so virtual gap / lead mapping must swap start↔end sides.
    pub fn reverses_feed_start(self) -> bool {
        matches!(self, Rotation::Cw180 | Rotation::Cw270)
    }

    /// Resolve the net rotation for a print from a base [`Orientation`] plus any
    /// additional clockwise / counter-clockwise quarter-turns the user
    /// requested.
    ///
    /// Landscape contributes one clockwise quarter-turn; portrait contributes
    /// none. `extra_cw` / `extra_ccw` then nudge the result in 90° steps, and
    /// the total is normalized to one of the four [`Rotation`] values.
    pub fn for_print(orientation: Orientation, extra_cw: u32, extra_ccw: u32) -> Self {
        let base = match orientation {
            Orientation::Portrait => 0,
            Orientation::Landscape => 1,
        };
        let net = base + extra_cw as i32 - extra_ccw as i32;
        Rotation::from_quarter_turns_cw(net)
    }

    /// Like [`Self::for_print`], but applies [`Orientation::for_media`] first.
    pub fn for_print_with_media(
        orientation: Orientation,
        media: &Media,
        extra_cw: u32,
        extra_ccw: u32,
    ) -> Self {
        Self::for_print(orientation.for_media(media), extra_cw, extra_ccw)
    }

    /// Quarter-turn applied to the rendered raster before encode.
    ///
    /// Row-oriented drivers (ZPL, ESC/POS, LabelWriter, …) use the same mapping
    /// as [`Self::for_print_with_media`]. Feed-oriented drivers (LabelManager
    /// tape, LetraTag) consume bitmaps with width = feed and height = head, so
    /// portrait and landscape swap their base quarter-turns.
    pub fn for_head_with_media(
        orientation: Orientation,
        media: &Media,
        extra_cw: u32,
        extra_ccw: u32,
        protocol: Protocol,
    ) -> Self {
        let oriented = orientation.for_media(media);
        let base = match (oriented, protocol.bitmap_width_is_feed()) {
            (Orientation::Portrait, false) | (Orientation::Landscape, true) => 0,
            (Orientation::Landscape, false) | (Orientation::Portrait, true) => 1,
        };
        let net = base + extra_cw as i32 - extra_ccw as i32;
        Rotation::from_quarter_turns_cw(net)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_defaults_to_landscape() {
        assert_eq!(Orientation::default(), Orientation::Landscape);
    }

    #[test]
    fn portrait_is_identity_rotation() {
        assert_eq!(
            Rotation::for_print(Orientation::Portrait, 0, 0),
            Rotation::None
        );
    }

    #[test]
    fn landscape_is_a_single_clockwise_quarter_turn() {
        assert_eq!(
            Rotation::for_print(Orientation::Landscape, 0, 0),
            Rotation::Cw90
        );
    }

    #[test]
    fn extra_turns_compose_and_wrap() {
        // Landscape (90) + one more cw = 180.
        assert_eq!(
            Rotation::for_print(Orientation::Landscape, 1, 0),
            Rotation::Cw180
        );
        // Landscape (90) - one ccw = 0.
        assert_eq!(
            Rotation::for_print(Orientation::Landscape, 0, 1),
            Rotation::None
        );
        // Portrait - one ccw wraps to 270.
        assert_eq!(
            Rotation::for_print(Orientation::Portrait, 0, 1),
            Rotation::Cw270
        );
        // Four cw turns wrap back to the base.
        assert_eq!(
            Rotation::for_print(Orientation::Portrait, 4, 0),
            Rotation::None
        );
    }

    #[test]
    fn only_quarter_turns_swap_axes() {
        assert!(!Rotation::None.swaps_axes());
        assert!(Rotation::Cw90.swaps_axes());
        assert!(!Rotation::Cw180.swaps_axes());
        assert!(Rotation::Cw270.swaps_axes());
    }

    #[test]
    fn half_turns_reverse_feed_start() {
        assert!(!Rotation::None.reverses_feed_start());
        assert!(!Rotation::Cw90.reverses_feed_start());
        assert!(Rotation::Cw180.reverses_feed_start());
        assert!(Rotation::Cw270.reverses_feed_start());
        // Landscape + 180° extra → Cw270.
        assert!(Rotation::for_print(Orientation::Landscape, 2, 0).reverses_feed_start());
        assert!(!Rotation::for_print(Orientation::Landscape, 0, 0).reverses_feed_start());
    }

    #[test]
    fn wide_first_media_inverts_orientation() {
        use crate::media::Media;
        use crate::units::Dpi;

        let tape = Media::fixed(12.0, 40.0, Dpi(203.0));
        assert_eq!(
            Orientation::Portrait.for_media(&tape),
            Orientation::Portrait
        );
        assert_eq!(
            Orientation::Landscape.for_media(&tape),
            Orientation::Landscape
        );

        let wide = Media::fixed(48.0, 30.0, Dpi(203.0));
        assert_eq!(
            Orientation::Portrait.for_media(&wide),
            Orientation::Landscape
        );
        assert_eq!(
            Orientation::Landscape.for_media(&wide),
            Orientation::Portrait
        );
    }

    #[test]
    fn wide_first_media_maps_landscape_ui_to_wide_preview() {
        use crate::media::Media;
        use crate::units::Dpi;

        let wide = Media::fixed(48.0, 30.0, Dpi(203.0));
        assert_eq!(
            Rotation::for_print_with_media(Orientation::Landscape, &wide, 0, 0),
            Rotation::None
        );
        assert_eq!(
            Rotation::for_print_with_media(Orientation::Portrait, &wide, 0, 0),
            Rotation::Cw90
        );
    }

    #[test]
    fn dymo_head_rotation_inverts_landscape_and_portrait() {
        use crate::media::Media;
        use crate::printer::Protocol;
        use crate::units::Dpi;

        let tape = Media::continuous(12.0, Dpi(180.0));
        assert_eq!(
            Rotation::for_head_with_media(Orientation::Landscape, &tape, 0, 0, Protocol::Dymo),
            Rotation::None
        );
        assert_eq!(
            Rotation::for_head_with_media(Orientation::Portrait, &tape, 0, 0, Protocol::Dymo),
            Rotation::Cw90
        );
        assert_eq!(
            Rotation::for_head_with_media(Orientation::Landscape, &tape, 0, 0, Protocol::Zpl),
            Rotation::Cw90
        );
        assert_eq!(
            Rotation::for_head_with_media(Orientation::Landscape, &tape, 0, 0, Protocol::DymoLw),
            Rotation::Cw90
        );
    }
}

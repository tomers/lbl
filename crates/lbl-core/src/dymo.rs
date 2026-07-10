//! DYMO LabelManager tape geometry (printable band vs protocol column height).
//!
//! D1 tape wider than the physical print head carries clear laminate on each
//! edge; only the central band (~8.2 mm) accepts ink. The protocol column
//! height is fixed per tape width (64 dots for 12 mm), independent of render DPI.

/// Physical height of the DYMO LabelManager print head, in millimeters.
pub const HEAD_PRINTABLE_MM: f64 = 8.2;

/// Printable head width for a tape width (full tape when narrower than the head).
pub fn printable_head_mm(tape_width_mm: f64) -> f64 {
    tape_width_mm.min(HEAD_PRINTABLE_MM)
}

/// Protocol column height in dots for a tape width (e.g. 64 for 12 mm).
pub fn protocol_head_dots(tape_width_mm: f64) -> u32 {
    let bytes = (8.0 * tape_width_mm / 12.0).floor() as u32;
    bytes.max(1) * 8
}

/// Dead-zone margin at each edge of the protocol column, in dots.
pub fn protocol_vertical_margin_dots(tape_width_mm: f64) -> u32 {
    let margin_mm = ((tape_width_mm - HEAD_PRINTABLE_MM) / 2.0).max(0.0);
    if margin_mm <= f64::EPSILON {
        return 0;
    }
    let dots_per_mm = protocol_head_dots(tape_width_mm) as f64 / tape_width_mm;
    (margin_mm * dots_per_mm).round() as u32
}

/// Inkable rows inside a protocol column.
pub fn protocol_printable_dots(tape_width_mm: f64) -> u32 {
    let protocol_h = protocol_head_dots(tape_width_mm);
    let margin = protocol_vertical_margin_dots(tape_width_mm);
    protocol_h.saturating_sub(2 * margin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twelve_mm_protocol_geometry_matches_labelle() {
        assert_eq!(protocol_head_dots(12.0), 64);
        assert_eq!(protocol_vertical_margin_dots(12.0), 10);
        assert_eq!(protocol_printable_dots(12.0), 44);
        assert!((printable_head_mm(12.0) - 8.2).abs() < f64::EPSILON);
    }

    #[test]
    fn narrow_tape_uses_full_width() {
        assert_eq!(protocol_head_dots(6.0), 32);
        assert_eq!(protocol_vertical_margin_dots(6.0), 0);
        assert_eq!(protocol_printable_dots(6.0), 32);
        assert!((printable_head_mm(6.0) - 6.0).abs() < f64::EPSILON);
    }

    #[test]
    fn nine_mm_tape() {
        assert_eq!(protocol_head_dots(9.0), 48);
        assert_eq!(protocol_vertical_margin_dots(9.0), 2);
        assert_eq!(protocol_printable_dots(9.0), 44);
    }
}

//! Sample cubic / quadratic Béziers into polylines for GPGL.

/// Segments per cubic/quadratic when flattening outlines.
pub const CURVE_SEGMENTS: usize = 8;

/// Quadratic Bézier point.
pub fn quad_point(p0: (f64, f64), p1: (f64, f64), p: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let x_out = u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p.0;
    let y_out = u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p.1;
    (x_out, y_out)
}

/// Cubic Bézier point.
pub fn cubic_point(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p: (f64, f64),
    t: f64,
) -> (f64, f64) {
    let u = 1.0 - t;
    let x_out =
        u * u * u * p0.0 + 3.0 * u * u * t * p1.0 + 3.0 * u * t * t * p2.0 + t * t * t * p.0;
    let y_out =
        u * u * u * p0.1 + 3.0 * u * u * t * p1.1 + 3.0 * u * t * t * p2.1 + t * t * t * p.1;
    (x_out, y_out)
}

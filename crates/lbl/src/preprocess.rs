//! Heuristic preprocessing cost for label jobs (render + dither + encode).
//!
//! Work units correlate with Chromium paint time, which scales roughly with the
//! high-resolution pixel count: `width × height × supersample²`, times label
//! count. [`machine_capacity_factor`] adjusts for CPU core count and installed
//! RAM (via `sysinfo`) so the same job ranks heavier on weaker machines.

use std::sync::OnceLock;
use std::time::Duration;

use lbl_core::media::Media;
use lbl_core::Rotation;

/// On a reference machine (4 cores, 8 GiB RAM), warn before starting when the
/// adjusted weight reaches this level (~8–15 s of preprocessing for a typical job).
pub const WARN_WEIGHT_THRESHOLD: f64 = 25_000_000.0;

/// During batch preprocessing, repeat guidance every this much accumulated
/// render time.
pub const BATCH_WARN_INTERVAL: Duration = Duration::from_secs(10);

/// High-res pixels processed per second on the reference machine (used to turn
/// weight into a rough ETA).
const REFERENCE_PIXELS_PER_SEC: f64 = 3_000_000.0;

/// Sidecar (Node/Playwright) is typically slower than in-process Chromium.
const SIDECAR_SLOWDOWN: f64 = 1.3;

/// Inputs that determine preprocessing cost (printing time is excluded).
#[derive(Debug, Clone, PartialEq)]
pub struct JobPreprocessInput {
    pub label_count: usize,
    pub width_dots: u32,
    pub height_dots: u32,
    pub supersample: u32,
    pub sidecar_backend: bool,
}

/// Estimated preprocessing cost for a job.
#[derive(Debug, Clone, PartialEq)]
pub struct PreprocessEstimate {
    /// Raw work units (high-res pixels × labels, backend-adjusted).
    pub work_units: f64,
    /// Work units divided by [`machine_capacity_factor`].
    pub adjusted_weight: f64,
    /// Rough preprocessing duration on this machine.
    pub estimated_seconds: f64,
    pub exceeds_threshold: bool,
}

/// Build preprocessing inputs from resolved pipeline parameters.
pub fn job_input(
    label_count: usize,
    media: &Media,
    rotation: Rotation,
    supersample: u32,
    sidecar_backend: bool,
) -> JobPreprocessInput {
    let (width_dots, height_dots) = estimate_render_dimensions(media, rotation);
    JobPreprocessInput {
        label_count: label_count.max(1),
        width_dots,
        height_dots,
        supersample,
        sidecar_backend,
    }
}

/// Estimate device dot dimensions used for the render pass, filling in typical
/// extents for continuous media on auto-sized axes.
pub fn estimate_render_dimensions(media: &Media, rotation: Rotation) -> (u32, u32) {
    let head_dots = media.width_dots().0;
    let feed_dots = media.length_dots().map(|d| d.0);
    let (w_opt, h_opt) = if rotation.swaps_axes() {
        (feed_dots, Some(head_dots))
    } else {
        (Some(head_dots), feed_dots)
    };
    let w = w_opt.unwrap_or_else(|| estimate_continuous_extent_dots(head_dots));
    let h = h_opt.unwrap_or_else(|| estimate_continuous_extent_dots(head_dots));
    (w.max(1), h.max(1))
}

/// Typical feed length for continuous media when the extent is content-determined.
fn estimate_continuous_extent_dots(head_dots: u32) -> u32 {
    ((head_dots as f64) * 2.5).clamp(96.0, 1200.0) as u32
}

/// High-resolution pixel count for one label (the Chromium first pass).
pub fn hires_pixels_per_label(width_dots: u32, height_dots: u32, supersample: u32) -> f64 {
    let ss = supersample.max(1) as f64;
    width_dots as f64 * height_dots as f64 * ss * ss
}

/// Relative machine capability vs a 4-core / 8 GiB reference (higher = faster).
pub fn machine_capacity_factor() -> f64 {
    static FACTOR: OnceLock<f64> = OnceLock::new();
    *FACTOR.get_or_init(read_machine_capacity_factor)
}

fn read_machine_capacity_factor() -> f64 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let cpus = sys
        .physical_core_count()
        .unwrap_or_else(|| sys.cpus().len())
        .max(1);
    let mem_gb = (sys.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0)).clamp(0.5, 64.0);
    let cpu_factor = cpus as f64 / 4.0;
    let mem_factor = mem_gb / 8.0;
    (cpu_factor * mem_factor).sqrt().clamp(0.2, 4.0)
}

/// Estimate preprocessing cost for a job.
pub fn estimate_job(input: &JobPreprocessInput) -> PreprocessEstimate {
    let per_label = hires_pixels_per_label(input.width_dots, input.height_dots, input.supersample);
    let backend = if input.sidecar_backend {
        SIDECAR_SLOWDOWN
    } else {
        1.0
    };
    let work_units = per_label * input.label_count as f64 * backend;
    let machine = machine_capacity_factor();
    let adjusted_weight = work_units / machine;
    let estimated_seconds = adjusted_weight / REFERENCE_PIXELS_PER_SEC;
    PreprocessEstimate {
        work_units,
        adjusted_weight,
        estimated_seconds,
        exceeds_threshold: adjusted_weight >= WARN_WEIGHT_THRESHOLD,
    }
}

/// A lower supersample that usually preserves quality while cutting cost.
pub fn suggest_supersample(current: u32) -> Option<u32> {
    if current > 4 {
        Some(4)
    } else if current > 3 {
        Some(3)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::units::Dpi;

    #[test]
    fn hires_pixels_scales_with_supersample_squared() {
        let at_2 = hires_pixels_per_label(100, 100, 2);
        let at_4 = hires_pixels_per_label(100, 100, 4);
        assert!((at_4 / at_2 - 4.0).abs() < 0.01);
    }

    #[test]
    fn large_batch_at_high_supersample_exceeds_threshold() {
        let input = JobPreprocessInput {
            label_count: 50,
            width_dots: 638,
            height_dots: 295,
            supersample: 8,
            sidecar_backend: false,
        };
        let est = estimate_job(&input);
        assert!(est.exceeds_threshold);
        assert!(est.estimated_seconds > 5.0);
    }

    #[test]
    fn single_typical_label_at_default_supersample_is_light() {
        let input = JobPreprocessInput {
            label_count: 1,
            width_dots: 638,
            height_dots: 295,
            supersample: 4,
            sidecar_backend: false,
        };
        let est = estimate_job(&input);
        assert!(!est.exceeds_threshold);
    }

    #[test]
    fn fixed_media_dimensions_from_catalog_sku() {
        let media = Media::fixed(25.0, 54.0, Dpi(300.0));
        let (w, h) = estimate_render_dimensions(&media, Rotation::None);
        assert_eq!(w, media.width_dots().0);
        assert_eq!(h, media.length_dots().unwrap().0);
    }

    #[test]
    fn suggest_supersample_steps_down() {
        assert_eq!(suggest_supersample(8), Some(4));
        assert_eq!(suggest_supersample(4), Some(3));
        assert_eq!(suggest_supersample(3), None);
    }
}

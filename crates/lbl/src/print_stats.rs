//! Timing and throughput statistics for hardware print runs.

use std::time::Duration;

use lbl_core::media::Media;
use lbl_core::printer::Protocol;

use crate::debug::LabelTrace;
use crate::preprocess::{estimate_render_dimensions, JobPreprocessInput};

/// Preprocessing and spooling durations for one print run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrintRunTimings {
    pub preprocess: Duration,
    pub print: Duration,
}

impl PrintRunTimings {
    pub fn total(self) -> Duration {
        self.preprocess + self.print
    }

    /// Fraction of wall time spent printing (`print ÷ (preprocess + print)`).
    pub fn efficiency(self) -> f64 {
        let total = self.total().as_secs_f64();
        if total <= f64::EPSILON {
            return 1.0;
        }
        self.print.as_secs_f64() / total
    }
}

/// Feed extent along the media advance axis, in device dots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelFeedDots(pub u32);

/// Inputs for the end-of-run summary line.
#[derive(Debug, Clone)]
pub struct PrintSummaryInput<'a> {
    pub timings: PrintRunTimings,
    pub label_count: usize,
    pub copies: u32,
    pub feed_dots: &'a [LabelFeedDots],
    pub media: &'a Media,
    pub rotation: lbl_core::Rotation,
    pub protocol: Protocol,
    pub preprocess: &'a JobPreprocessInput,
    pub efficiency_warn_below: f64,
}

/// Feed dimension in device dots for one encoded label.
pub fn feed_dots_for_trace(trace: &LabelTrace, protocol: Protocol) -> u32 {
    if protocol.bitmap_width_is_feed() {
        trace.dithered.width
    } else {
        trace.dithered.height
    }
    .max(1)
}

/// Total feed length advanced during printing, in millimetres.
pub fn total_feed_mm(
    feed_dots: &[LabelFeedDots],
    label_count: usize,
    media: &Media,
    rotation: lbl_core::Rotation,
    copies: u32,
    dpi: f64,
) -> f64 {
    let copies = copies.max(1) as f64;
    let dpi = dpi.max(1.0);
    let per_pass: f64 = if feed_dots.is_empty() {
        let (_, h) = estimate_render_dimensions(media, rotation);
        dots_to_mm(h, dpi) * label_count.max(1) as f64
    } else {
        feed_dots
            .iter()
            .map(|LabelFeedDots(d)| dots_to_mm(*d, dpi))
            .sum()
    };
    per_pass * copies
}

fn dots_to_mm(dots: u32, dpi: f64) -> f64 {
    dots as f64 / dpi * 25.4
}

/// Pick a human-friendly throughput string for the given feed and print time.
pub fn format_throughput(feed_mm: f64, print_time: Duration) -> String {
    let secs = print_time.as_secs_f64();
    if secs <= f64::EPSILON || feed_mm <= f64::EPSILON {
        return "—".to_string();
    }
    let mm_per_sec = feed_mm / secs;
    if mm_per_sec >= 1.0 {
        if mm_per_sec >= 1000.0 {
            format!("{:.2} m/s", mm_per_sec / 1000.0)
        } else {
            format!("{:.1} mm/s", mm_per_sec)
        }
    } else if mm_per_sec >= 0.1 {
        format!("{:.1} cm/s", mm_per_sec * 10.0)
    } else {
        format!("{:.0} mm/min", mm_per_sec * 60.0)
    }
}

pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 1.0 {
        format!("{:.0}ms", secs * 1000.0)
    } else if secs < 60.0 {
        format!("{:.1}s", secs)
    } else {
        let mins = (secs / 60.0).floor() as u32;
        let rem = secs - f64::from(mins * 60);
        if rem < 0.05 {
            format!("{mins}m")
        } else {
            format!("{mins}m {rem:.0}s")
        }
    }
}

pub fn format_efficiency(efficiency: f64) -> String {
    format!("{:.0}%", (efficiency * 100.0).round())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::units::Dpi;

    #[test]
    fn efficiency_is_print_over_total() {
        let t = PrintRunTimings {
            preprocess: Duration::from_secs(9),
            print: Duration::from_secs(1),
        };
        assert!((t.efficiency() - 0.1).abs() < 0.001);
    }

    #[test]
    fn throughput_uses_mm_per_sec_when_fast_enough() {
        let s = format_throughput(54.0, Duration::from_secs(2));
        assert!(s.contains("mm/s"));
        assert!(s.starts_with("27"));
    }

    #[test]
    fn throughput_uses_cm_per_sec_when_slow() {
        let s = format_throughput(5.0, Duration::from_secs(10));
        assert!(s.contains("cm/s"));
    }

    #[test]
    fn throughput_uses_mm_per_min_when_very_slow() {
        let s = format_throughput(1.0, Duration::from_secs(60));
        assert!(s.contains("mm/min"));
    }

    #[test]
    fn total_feed_mm_from_fixed_media_fallback() {
        let media = Media::fixed(25.0, 54.0, Dpi(300.0));
        let mm = total_feed_mm(&[], 2, &media, lbl_core::Rotation::None, 1, 300.0);
        assert!((mm - 108.0).abs() < 0.5);
    }
}

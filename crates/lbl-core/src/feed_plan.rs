//! Padding-driven feed / pre-cut policy shared by encode and preview.
//!
//! Drivers honor [`FeedPlan::precut`] only — they must not infer pre-cut from
//! lead padding alone. See `docs/src/plans/precut-feed-padding.md`.

use serde::{Deserialize, Serialize};

use crate::job::JobSpec;
use crate::printer::DeviceCapabilities;

/// Resolved feed margins and whether to emit a pre-cut prologue.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FeedPlan {
    /// Blank before content on the kept label (mm).
    pub lead_mm: f64,
    /// Blank after content before cut (mm).
    pub end_mm: f64,
    /// Emit protocol pre-cut (eject ≈ [`Self::cutter_gap_mm`] scrap) before content.
    pub precut: bool,
    /// Head-to-cutter distance \(D_x\) in mm (0 when unknown).
    pub cutter_gap_mm: f64,
}

impl Default for FeedPlan {
    fn default() -> Self {
        Self {
            lead_mm: 0.0,
            end_mm: 0.0,
            precut: false,
            cutter_gap_mm: 0.0,
        }
    }
}

/// Stable error token for UIs (`engine-labels`) and CLI.
pub const LEAD_PADDING_BELOW_CUTTER_GAP: &str = "lead_padding_below_cutter_gap";

/// Stable error token when lead is below the catalog protocol floor.
pub const LEAD_PADDING_BELOW_MIN: &str = "lead_padding_below_min";

/// Feed / pre-cut policy failure (no device I/O).
#[derive(Debug, Clone, PartialEq)]
pub enum FeedPlanError {
    /// Requested lead is below the head-to-cutter gap and pre-cut is not allowed.
    LeadPaddingBelowCutterGap {
        requested_mm: f64,
        cutter_gap_mm: f64,
        precut_supported: bool,
    },
    /// Requested lead is below the catalog / protocol minimum.
    LeadPaddingBelowMin { requested_mm: f64, min_mm: f64 },
}

impl FeedPlanError {
    /// Machine-readable token for clients.
    pub fn token(&self) -> &'static str {
        match self {
            Self::LeadPaddingBelowCutterGap { .. } => LEAD_PADDING_BELOW_CUTTER_GAP,
            Self::LeadPaddingBelowMin { .. } => LEAD_PADDING_BELOW_MIN,
        }
    }
}

impl std::fmt::Display for FeedPlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LeadPaddingBelowCutterGap {
                requested_mm,
                cutter_gap_mm,
                precut_supported,
            } => {
                write!(
                    f,
                    "Lead padding ({requested_mm} mm) is below this printer's cutter gap \
                     ({cutter_gap_mm} mm). Increase padding to at least {cutter_gap_mm} mm"
                )?;
                if *precut_supported {
                    write!(
                        f,
                        ", or enable pre-cut (ejects ~{cutter_gap_mm} mm of empty tape as scrap, \
                         then prints with your smaller margin)"
                    )?;
                }
                write!(f, ".")
            }
            Self::LeadPaddingBelowMin {
                requested_mm,
                min_mm,
            } => write!(
                f,
                "Lead padding ({requested_mm} mm) is below this printer's minimum ({min_mm} mm)."
            ),
        }
    }
}

impl std::error::Error for FeedPlanError {}

/// Resolve feed margins and pre-cut from job + capabilities.
///
/// Unset lead → \(D_x\) when known, else `caps.feed_lead_mm`, else `0`.
/// Unset end → `0`. Unset `job.precut` → `caps.precut_default`.
///
/// Never turns pre-cut on solely because padding is small.
pub fn resolve_feed_plan(
    caps: &DeviceCapabilities,
    job: &JobSpec,
) -> Result<FeedPlan, FeedPlanError> {
    let cutter_gap_mm = caps
        .feed_trail_mm
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or(0.0);

    let lead_mm = match job.feed_lead_mm {
        Some(v) if v.is_finite() && v >= 0.0 => v,
        _ if cutter_gap_mm > 0.0 => cutter_gap_mm,
        _ => caps
            .feed_lead_mm
            .filter(|d| d.is_finite() && *d >= 0.0)
            .unwrap_or(0.0),
    };

    let end_mm = job
        .feed_end_mm
        .filter(|d| d.is_finite() && *d >= 0.0)
        .unwrap_or(0.0);

    let precut_enabled = job.precut.unwrap_or(caps.precut_default);
    let will_cut = caps.supports_cut && job.cut_mode.requests_cut();

    let precut = if !will_cut || cutter_gap_mm <= 0.0 || lead_mm + f64::EPSILON >= cutter_gap_mm {
        false
    } else if caps.supports_precut && precut_enabled {
        true
    } else {
        return Err(FeedPlanError::LeadPaddingBelowCutterGap {
            requested_mm: lead_mm,
            cutter_gap_mm,
            precut_supported: caps.supports_precut,
        });
    };

    if let Some(min) = caps.feed_lead_min_mm {
        if min.is_finite() && min > 0.0 && lead_mm + f64::EPSILON < min {
            return Err(FeedPlanError::LeadPaddingBelowMin {
                requested_mm: lead_mm,
                min_mm: min,
            });
        }
    }

    Ok(FeedPlan {
        lead_mm,
        end_mm,
        precut,
        cutter_gap_mm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::{CutMode, JobSpec};
    use crate::media::Media;
    use crate::printer::DeviceCapabilities;
    use crate::units::Dpi;

    fn media() -> Media {
        Media::continuous(12.0, Dpi(180.0))
    }

    fn pt_caps() -> DeviceCapabilities {
        DeviceCapabilities {
            dpi: Dpi(180.0),
            max_width_mm: 24.0,
            supports_cut: true,
            feed_trail_mm: Some(24.0),
            feed_lead_mm: Some(2.0),
            supports_precut: true,
            precut_default: false,
            ..Default::default()
        }
    }

    fn job_with(cut: CutMode, lead: Option<f64>, precut: Option<bool>) -> JobSpec {
        let mut job = JobSpec::new(media());
        job.cut_mode = cut;
        job.feed_lead_mm = lead;
        job.precut = precut;
        job
    }

    #[test]
    fn unset_lead_uses_cutter_gap_no_precut() {
        let plan = resolve_feed_plan(&pt_caps(), &job_with(CutMode::Every, None, None)).unwrap();
        assert!((plan.lead_mm - 24.0).abs() < 1e-9);
        assert!(!plan.precut);
        assert!((plan.cutter_gap_mm - 24.0).abs() < 1e-9);
    }

    #[test]
    fn small_lead_precut_on() {
        let plan = resolve_feed_plan(&pt_caps(), &job_with(CutMode::Every, Some(2.0), Some(true)))
            .unwrap();
        assert!((plan.lead_mm - 2.0).abs() < 1e-9);
        assert!(plan.precut);
    }

    #[test]
    fn small_lead_precut_off_rejects() {
        let err = resolve_feed_plan(
            &pt_caps(),
            &job_with(CutMode::Every, Some(2.0), Some(false)),
        )
        .unwrap_err();
        match err {
            FeedPlanError::LeadPaddingBelowCutterGap {
                requested_mm,
                cutter_gap_mm,
                precut_supported,
            } => {
                assert!((requested_mm - 2.0).abs() < 1e-9);
                assert!((cutter_gap_mm - 24.0).abs() < 1e-9);
                assert!(precut_supported);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(err.token(), LEAD_PADDING_BELOW_CUTTER_GAP);
        let msg = err.to_string();
        assert!(msg.contains("enable pre-cut"));
        assert!(msg.contains("scrap"));
    }

    #[test]
    fn lead_at_or_above_gap_no_precut() {
        let plan = resolve_feed_plan(
            &pt_caps(),
            &job_with(CutMode::Every, Some(24.0), Some(true)),
        )
        .unwrap();
        assert!(!plan.precut);
        let plan2 = resolve_feed_plan(
            &pt_caps(),
            &job_with(CutMode::Every, Some(30.0), Some(true)),
        )
        .unwrap();
        assert!(!plan2.precut);
    }

    #[test]
    fn no_cut_skips_precut_and_allows_small_lead() {
        let plan = resolve_feed_plan(&pt_caps(), &job_with(CutMode::None, Some(2.0), Some(false)))
            .unwrap();
        assert!(!plan.precut);
        assert!((plan.lead_mm - 2.0).abs() < 1e-9);
    }

    #[test]
    fn unsupported_precut_rejects_small_lead() {
        let mut caps = pt_caps();
        caps.supports_precut = false;
        let err =
            resolve_feed_plan(&caps, &job_with(CutMode::Every, Some(4.0), Some(true))).unwrap_err();
        match err {
            FeedPlanError::LeadPaddingBelowCutterGap {
                precut_supported, ..
            } => assert!(!precut_supported),
            other => panic!("unexpected {other:?}"),
        }
        assert!(!err.to_string().contains("enable pre-cut"));
    }

    #[test]
    fn lowering_padding_does_not_imply_preference() {
        // precut_default false + unset job.precut + small lead → reject, not auto-enable.
        let err =
            resolve_feed_plan(&pt_caps(), &job_with(CutMode::Every, Some(2.0), None)).unwrap_err();
        assert_eq!(err.token(), LEAD_PADDING_BELOW_CUTTER_GAP);
    }

    #[test]
    fn lead_below_min_rejects() {
        let mut caps = pt_caps();
        caps.feed_lead_min_mm = Some(2.0);
        let err =
            resolve_feed_plan(&caps, &job_with(CutMode::Every, Some(1.0), Some(true))).unwrap_err();
        assert_eq!(err.token(), LEAD_PADDING_BELOW_MIN);
    }

    #[test]
    fn no_dx_uses_catalog_lead() {
        let caps = DeviceCapabilities {
            supports_cut: true,
            feed_lead_mm: Some(3.0),
            feed_trail_mm: None,
            ..Default::default()
        };
        let plan = resolve_feed_plan(&caps, &job_with(CutMode::Every, None, None)).unwrap();
        assert!((plan.lead_mm - 3.0).abs() < 1e-9);
        assert!(!plan.precut);
    }
}

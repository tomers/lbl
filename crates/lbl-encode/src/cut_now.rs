//! Host-initiated cut (no artwork): encode a short blank strip with cut enabled.

use lbl_core::bitmap::MonoBitmap;
use lbl_core::job::{CutKind, CutMode, JobSpec};
use lbl_core::media::Media;
use lbl_core::printer::{DeviceCapabilities, Protocol};
use lbl_core::units::{Dots, Millimeters};
use lbl_driver_api::EncodeContext;

use crate::{EncodeError, Registry};

/// Minimum blank feed length so continuous-tape cutters can engage.
const CUT_NOW_FEED_MM: f64 = 4.0;

/// Whether `protocol` can encode a host cut-now command via [`encode_cut_now`].
///
/// Craft cutters ([`Protocol::Gpgl`]) and on-screen sinks are excluded. Protocols
/// that ignore [`CutMode`] in encode are also excluded.
pub fn cut_now_supported(protocol: Protocol) -> bool {
    matches!(
        protocol,
        Protocol::BrotherPt
            | Protocol::BrotherQl
            | Protocol::EscPos
            | Protocol::EscLabel
            | Protocol::Zpl
            | Protocol::Tspl
            | Protocol::Slcs
            | Protocol::Ezpl
            | Protocol::Sbpl
            | Protocol::Dpl
            | Protocol::Tpcl
            | Protocol::Dymo
            | Protocol::LetraTag
    )
}

/// Encode a minimal blank job that requests a cut (`CutMode::Every`).
///
/// `caps.supports_cut` must be true for drivers that gate on capabilities;
/// callers should set it from catalog/profile. Half-cut requires
/// `caps.supports_half_cut`.
pub fn encode_cut_now(
    registry: &Registry,
    protocol: Protocol,
    media: &Media,
    caps: &DeviceCapabilities,
    cut_kind: CutKind,
) -> Result<Vec<u8>, EncodeError> {
    if !cut_now_supported(protocol) {
        return Err(EncodeError::UnsupportedCutNow(protocol));
    }
    if !caps.supports_cut {
        return Err(EncodeError::CutNotSupported);
    }
    if cut_kind.is_half() && !caps.supports_half_cut {
        return Err(EncodeError::HalfCutNotSupported);
    }

    let driver = registry
        .get(protocol)
        .ok_or(EncodeError::NoDriver(protocol))?;

    let bitmap = blank_cut_bitmap(protocol, media);
    let mut job = JobSpec::new(media.clone());
    job.cut_mode = CutMode::Every;
    job.cut_kind = cut_kind;
    job.copies = 1;

    let ctx = EncodeContext::new(&job, caps);
    Ok(driver.encode(&bitmap, &ctx)?)
}

fn blank_cut_bitmap(protocol: Protocol, media: &Media) -> MonoBitmap {
    let feed_dots = Millimeters(CUT_NOW_FEED_MM).to_dots(media.dpi).0.max(1);
    let head_dots = media.width_dots().0.max(1);

    if protocol.bitmap_width_is_feed() {
        // Width = feed, height = head (LabelManager / LetraTag).
        MonoBitmap::new(feed_dots, head_dots)
    } else {
        // Width = head, height = feed (Brother PT/QL, ESC/POS, ZPL, …).
        MonoBitmap::new(head_dots, feed_dots)
    }
}

/// Convenience: dots along the blank feed axis used by [`encode_cut_now`].
pub fn cut_now_feed_dots(media: &Media) -> Dots {
    Millimeters(CUT_NOW_FEED_MM).to_dots(media.dpi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::units::Dpi;

    fn caps_cut() -> DeviceCapabilities {
        DeviceCapabilities {
            supports_cut: true,
            ..DeviceCapabilities::default()
        }
    }

    #[test]
    fn gpgl_and_virtual_are_unsupported() {
        assert!(!cut_now_supported(Protocol::Gpgl));
        assert!(!cut_now_supported(Protocol::Virtual));
        assert!(!cut_now_supported(Protocol::Console));
        assert!(!cut_now_supported(Protocol::Niimbot));
    }

    #[test]
    fn brother_pt_emits_auto_cut_and_no_chain() {
        let registry = Registry::with_builtin_drivers();
        let media = Media::continuous(18.0, Dpi(180.0));
        let bytes = encode_cut_now(
            &registry,
            Protocol::BrotherPt,
            &media,
            &caps_cut(),
            CutKind::Full,
        )
        .unwrap();
        // ESC i M with bit 6 (auto-cut).
        assert!(contains_subsequence(&bytes, &[0x1B, b'i', b'M', 1 << 6]));
        // ESC i K with bit 3 (no-chain / cut-at-end).
        assert!(contains_subsequence(&bytes, &[0x1B, b'i', b'K', 1 << 3]));
        assert!(bytes.contains(&0x1A));
    }

    #[test]
    fn brother_pt_half_cut_sets_half_bit() {
        let registry = Registry::with_builtin_drivers();
        let media = Media::continuous(18.0, Dpi(180.0));
        let mut caps = caps_cut();
        caps.supports_half_cut = true;
        let bytes =
            encode_cut_now(&registry, Protocol::BrotherPt, &media, &caps, CutKind::Half).unwrap();
        assert!(contains_subsequence(
            &bytes,
            &[0x1B, b'i', b'K', (1 << 2) | (1 << 3)]
        ));
    }

    #[test]
    fn brother_ql_emits_auto_cut() {
        let registry = Registry::with_builtin_drivers();
        let media = Media::continuous(62.0, Dpi(300.0));
        let mut caps = caps_cut();
        caps.max_width_mm = 62.0;
        let bytes =
            encode_cut_now(&registry, Protocol::BrotherQl, &media, &caps, CutKind::Full).unwrap();
        assert!(contains_subsequence(&bytes, &[0x1B, b'i', b'M', 1 << 6]));
    }

    #[test]
    fn escpos_emits_gs_v_full_cut() {
        let registry = Registry::with_builtin_drivers();
        let media = Media::continuous(58.0, Dpi(203.0));
        let bytes = encode_cut_now(
            &registry,
            Protocol::EscPos,
            &media,
            &caps_cut(),
            CutKind::Full,
        )
        .unwrap();
        assert!(contains_subsequence(&bytes, &[0x1D, b'V', 0x00]));
    }

    #[test]
    fn zpl_emits_mmc() {
        let registry = Registry::with_builtin_drivers();
        let media = Media::fixed(50.0, 30.0, Dpi(203.0));
        let bytes =
            encode_cut_now(&registry, Protocol::Zpl, &media, &caps_cut(), CutKind::Full).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("^MMC"));
    }

    #[test]
    fn rejects_when_caps_lack_cut() {
        let registry = Registry::with_builtin_drivers();
        let media = Media::continuous(18.0, Dpi(180.0));
        let err = encode_cut_now(
            &registry,
            Protocol::BrotherPt,
            &media,
            &DeviceCapabilities::default(),
            CutKind::Full,
        )
        .unwrap_err();
        assert!(matches!(err, EncodeError::CutNotSupported));
    }

    #[test]
    fn rejects_half_without_cap() {
        let registry = Registry::with_builtin_drivers();
        let media = Media::continuous(18.0, Dpi(180.0));
        let err = encode_cut_now(
            &registry,
            Protocol::BrotherPt,
            &media,
            &caps_cut(),
            CutKind::Half,
        )
        .unwrap_err();
        assert!(matches!(err, EncodeError::HalfCutNotSupported));
    }

    #[test]
    fn rejects_unsupported_protocol() {
        let registry = Registry::with_builtin_drivers();
        let media = Media::continuous(18.0, Dpi(180.0));
        let err = encode_cut_now(
            &registry,
            Protocol::Gpgl,
            &media,
            &caps_cut(),
            CutKind::Full,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            EncodeError::UnsupportedCutNow(Protocol::Gpgl)
        ));
    }

    fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }
}

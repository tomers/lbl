//! Resolution of the media currently loaded in a printer.
//!
//! Most label printers do not report their loaded media electronically, so the
//! toolchain prefers auto-detection where available and falls back to an
//! explicit override (often a catalog SKU resolved by `lbl-catalog`).

use lbl_core::media::Media;

/// Where a resolved [`Media`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSource {
    /// The device reported its loaded media.
    AutoDetected,
    /// The user supplied an explicit override.
    Override,
}

/// Resolve the media for a print job.
///
/// `detected` is what (if anything) the device reported; `override_media` is an
/// explicit user choice. The override always wins when present; otherwise
/// auto-detection is used. Returns `None` if neither is available (the caller
/// must then require an explicit `--media`).
pub fn resolve_media(
    detected: Option<Media>,
    override_media: Option<Media>,
) -> Option<(Media, MediaSource)> {
    match (override_media, detected) {
        (Some(m), _) => Some((m, MediaSource::Override)),
        (None, Some(m)) => Some((m, MediaSource::AutoDetected)),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::units::Dpi;

    #[test]
    fn override_wins() {
        let detected = Media::continuous(12.0, Dpi(180.0));
        let over = Media::fixed(25.0, 54.0, Dpi(300.0));
        let (m, src) = resolve_media(Some(detected), Some(over.clone())).unwrap();
        assert_eq!(src, MediaSource::Override);
        assert_eq!(m, over);
    }

    #[test]
    fn falls_back_to_detected() {
        let detected = Media::continuous(12.0, Dpi(180.0));
        let (_, src) = resolve_media(Some(detected), None).unwrap();
        assert_eq!(src, MediaSource::AutoDetected);
    }

    #[test]
    fn none_when_unknown() {
        assert!(resolve_media(None, None).is_none());
    }
}

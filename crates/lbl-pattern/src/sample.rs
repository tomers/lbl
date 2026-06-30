//! Labelle-compatible sample (calibration) pattern generation.

use fontdue::{Font, FontSettings};
use lbl_core::bitmap::MonoBitmap;
use lbl_core::media::Media;
use lbl_core::printer::Protocol;

const FONT_BYTES: &[u8] = include_bytes!("../assets/Carlito-Regular.ttf");
const FONT_SIZE_PX: f32 = 12.0;
const STAGGER_BASE_WIDTH: i32 = 40;
const STAGGER_LINE_WIDTH: i32 = 40;

/// Resolve the print-head height (in device dots) for a sample pattern.
///
/// When `height` is [`None`], uses [`Media::width_dots`] (the printable width
/// across the head from `--media` / `--width-mm` at `--dpi`). An explicit value
/// overrides that default.
pub fn resolve_head_dots(height: Option<u32>, media: &Media) -> Result<u32, String> {
    match height {
        Some(0) => Err("sample pattern head height must be at least 1 dot".into()),
        Some(n) => Ok(n),
        None => {
            let dots = media.width_dots().0;
            if dots == 0 {
                return Err(
                    "media width resolves to 0 dots; pass an explicit --sample-pattern value"
                        .into(),
                );
            }
            Ok(dots)
        }
    }
}

/// Build a calibration pattern sized for `media`.
///
/// Head height comes from `head_dots`. When the media has a fixed length, the
/// pattern is widened along the feed (Labelle orientation) by growing the two
/// outer staggered corner-mark segments so the marks sit on the label edges.
/// The bitmap is then transposed when `protocol` expects width = head (NIIMBOT,
/// ESC/POS, ZPL, …).
pub fn sample_pattern_for_media(head_dots: u32, media: &Media, protocol: Protocol) -> MonoBitmap {
    let feed_dots = media.length_dots().map(|d| d.0);
    orient_sample_pattern(sample_pattern_sized(head_dots, feed_dots), protocol)
}

/// Build a horizontal calibration pattern at `height` device dots across the
/// print head (Labelle / DYMO orientation: bitmap width = feed, height = head).
pub fn sample_pattern(height: u32) -> MonoBitmap {
    sample_pattern_sized(height, None)
}

/// Like [`sample_pattern`], but optionally extend the feed length to
/// `feed_dots` by padding the outer staggered segments (no scaling).
pub fn sample_pattern_sized(head_dots: u32, feed_dots: Option<u32>) -> MonoBitmap {
    assert!(head_dots > 0, "pattern height must be at least 1 dot");
    let height = head_dots as i32;
    let font =
        Font::from_bytes(FONT_BYTES, FontSettings::default()).expect("embedded Carlito font");

    let core_width = STAGGER_BASE_WIDTH
        + vertical_lines(5, height).width
        + fine_checkerboard(12, height).width
        + solid_black(12, height).width
        + dyadic_checkerboard(height, &font).width
        + STAGGER_BASE_WIDTH;

    let (left_w, right_w) = stagger_segment_widths(core_width, feed_dots);

    let segments = [
        staggered_horizontal_lines(4, STAGGER_LINE_WIDTH, left_w, height, false, &font),
        vertical_lines(5, height),
        fine_checkerboard(12, height),
        solid_black(12, height),
        dyadic_checkerboard(height, &font),
        staggered_horizontal_lines(4, STAGGER_LINE_WIDTH, right_w, height, true, &font),
    ];

    let total_width: i32 = segments.iter().map(|s| s.width).sum();
    let mut out = MonoBitmap::new(total_width as u32, height as u32);
    let mut x = 0;
    for segment in &segments {
        blit(&mut out, x, 0, segment);
        x += segment.width;
    }
    out
}

/// Reorient a Labelle-layout pattern for the target driver's bitmap convention.
pub fn orient_sample_pattern(bmp: MonoBitmap, protocol: Protocol) -> MonoBitmap {
    if protocol.bitmap_width_is_feed() {
        bmp
    } else {
        transpose_bitmap(&bmp)
    }
}

fn stagger_segment_widths(core_width: i32, feed_dots: Option<u32>) -> (i32, i32) {
    let Some(target) = feed_dots.map(|d| d as i32) else {
        return (STAGGER_BASE_WIDTH, STAGGER_BASE_WIDTH);
    };
    if target <= core_width {
        return (STAGGER_BASE_WIDTH, STAGGER_BASE_WIDTH);
    }
    let extra = target - core_width;
    (
        STAGGER_BASE_WIDTH + extra / 2,
        STAGGER_BASE_WIDTH + extra - extra / 2,
    )
}

fn transpose_bitmap(bmp: &MonoBitmap) -> MonoBitmap {
    let mut out = MonoBitmap::new(bmp.height, bmp.width);
    for y in 0..bmp.height {
        for x in 0..bmp.width {
            if bmp.get(x, y) {
                out.set(y, x, true);
            }
        }
    }
    out
}

struct Segment {
    width: i32,
    height: i32,
    pixels: Vec<bool>,
}

impl Segment {
    fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            pixels: vec![false; (width * height) as usize],
        }
    }

    fn set(&mut self, x: i32, y: i32, ink: bool) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        self.pixels[(y * self.width + x) as usize] = ink;
    }

    fn get(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return false;
        }
        self.pixels[(y * self.width + x) as usize]
    }

    fn toggle(&mut self, x: i32, y: i32) {
        self.set(x, y, !self.get(x, y));
    }
}

fn blit(dst: &mut MonoBitmap, dx: i32, dy: i32, src: &Segment) {
    for y in 0..src.height {
        for x in 0..src.width {
            if src.get(x, y) {
                dst.set((dx + x) as u32, (dy + y) as u32, true);
            }
        }
    }
}

fn vertical_lines(num_lines: i32, height: i32) -> Segment {
    let width = 2 * num_lines - 1;
    let mut seg = Segment::new(width, height);
    for x in (0..width).step_by(2) {
        for y in 0..height {
            seg.set(x, y, true);
        }
    }
    seg
}

fn staggered_horizontal_lines(
    num_lines: i32,
    line_width: i32,
    segment_width: i32,
    height: i32,
    align_end: bool,
    font: &Font,
) -> Segment {
    let mut seg = Segment::new(segment_width, height);
    let x_offset = if align_end {
        (segment_width - line_width).max(0)
    } else {
        0
    };
    for y0 in (0..2 * num_lines).step_by(2) {
        for x in 0..line_width {
            let y = if x < line_width / 2 { y0 } else { y0 + 1 };
            seg.set(x_offset + x, y, true);
        }
    }
    for y0 in (height - 2 * num_lines..height).step_by(2) {
        for x in 0..line_width {
            let y = if x < line_width / 2 { y0 + 1 } else { y0 };
            seg.set(x_offset + x, y, true);
        }
    }
    let text = format!("h={height}");
    let text_height = pil_bbox_bottom(font, &text);
    let y = (height - text_height) / 2;
    draw_text(&mut seg, 3, y, &text, font);
    seg
}

fn fine_checkerboard(width: i32, height: i32) -> Segment {
    let mut seg = Segment::new(width, height);
    for x in 0..width {
        for y in 0..height {
            if (x + y) % 2 == 0 {
                seg.set(x, y, true);
            }
        }
    }
    seg
}

fn solid_black(width: i32, height: i32) -> Segment {
    let mut seg = Segment::new(width, height);
    for y in 0..height {
        for x in 0..width {
            seg.set(x, y, true);
        }
    }
    seg
}

fn dyadic_checkerboard(height: i32, font: &Font) -> Segment {
    const MARGIN_BELOW: i32 = 3;
    let font_height = pil_bbox_bottom(font, "0123456789");
    let log_font_block_size = u32::try_from(font_height + MARGIN_BELOW)
        .unwrap_or(1)
        .next_power_of_two()
        .ilog2() as i32;
    let font_block_size = 1 << log_font_block_size;

    let mut required_text_width = 0;
    for yc in (font_block_size..=height).step_by(font_block_size as usize) {
        let text = yc.to_string();
        required_text_width = required_text_width.max(pil_bbox_right(font, &text));
    }

    let text_x_offset = log_font_block_size * font_block_size;
    let image_width = text_x_offset + required_text_width + 2;
    let mut seg = Segment::new(image_width, height);

    for yc in (font_block_size..=height).step_by(font_block_size as usize) {
        let text = yc.to_string();
        let y = height - yc - 1;
        draw_text(&mut seg, text_x_offset + 1, y + 2, &text, font);
    }

    for yc in 0..height {
        let y = height - yc - 1;
        for x in 0..image_width {
            let x0 = (x / font_block_size).min(log_font_block_size);
            if (yc >> x0) & 1 == 0 {
                seg.toggle(x, y);
            }
        }
    }
    seg
}

/// Match Pillow `ImageFont.FreeTypeFont.getbbox` width metric (the `right` value).
fn pil_bbox_right(font: &Font, text: &str) -> i32 {
    let mut cursor_x = 0.0f32;
    let mut right = 0i32;
    for ch in text.chars() {
        let (metrics, _) = font.rasterize(ch, FONT_SIZE_PX);
        let glyph_right = cursor_x as i32 + metrics.xmin + metrics.width as i32;
        right = right.max(glyph_right);
        cursor_x += metrics.advance_width;
    }
    right
}

/// Match Pillow `ImageFont.FreeTypeFont.getbbox` height metric (the `bottom` value).
fn pil_bbox_bottom(font: &Font, text: &str) -> i32 {
    let mut bottom = 0i32;
    let mut cursor_x = 0.0f32;
    for ch in text.chars() {
        let (metrics, _) = font.rasterize(ch, FONT_SIZE_PX);
        let glyph_bottom = metrics.ymin + metrics.height as i32;
        bottom = bottom.max(glyph_bottom);
        cursor_x += metrics.advance_width;
        let _ = cursor_x;
    }
    bottom
}

fn draw_text(seg: &mut Segment, x: i32, y_top: i32, text: &str, font: &Font) {
    let mut cursor_x = x as f32;
    for ch in text.chars() {
        let (metrics, bitmap) = font.rasterize(ch, FONT_SIZE_PX);
        let base_x = cursor_x as i32 + metrics.xmin;
        let base_y = y_top + metrics.ymin;
        for (idx, alpha) in bitmap.iter().enumerate() {
            if *alpha > 127 {
                let gx = base_x + (idx % metrics.width) as i32;
                let gy = base_y + (idx / metrics.width) as i32;
                seg.set(gx, gy, true);
            }
        }
        cursor_x += metrics.advance_width;
    }
}

#[cfg(test)]
mod tests {
    use lbl_core::media::Media;
    use lbl_core::units::Dpi;

    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn raster_hash(bmp: &MonoBitmap) -> u64 {
        let mut hasher = DefaultHasher::new();
        bmp.width.hash(&mut hasher);
        bmp.height.hash(&mut hasher);
        bmp.data.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn resolve_head_dots_uses_media_width() {
        let media = Media::fixed(12.0, 30.0, Dpi(203.0));
        assert_eq!(resolve_head_dots(None, &media).unwrap(), 96);
    }

    /// Dimensions verified against Labelle's `SamplePatternRenderEngine`
    /// (Carlito 12 px) when feed length is not pinned.
    #[test]
    fn matches_labelle_dimensions() {
        let cases = [(15, 179), (64, 191), (65, 191), (100, 191), (256, 197)];
        for (height, width) in cases {
            let bmp = sample_pattern(height);
            assert_eq!(bmp.width, width, "width for h={height}");
            assert_eq!(bmp.height, height, "height for h={height}");
        }
    }

    #[test]
    fn extends_feed_for_fixed_media() {
        let media = Media::fixed(12.0, 30.0, Dpi(203.0));
        let bmp = sample_pattern_sized(96, media.length_dots().map(|d| d.0));
        assert_eq!(bmp.width, 240);
        assert_eq!(bmp.height, 96);
    }

    #[test]
    fn niimbot_orientation_is_head_by_feed() {
        let media = Media::fixed(12.0, 30.0, Dpi(203.0));
        let bmp = sample_pattern_for_media(96, &media, Protocol::Niimbot);
        assert_eq!(bmp.width, 96);
        assert_eq!(bmp.height, 240);
    }

    #[test]
    fn stable_raster_for_64_dot_head() {
        let bmp = sample_pattern(64);
        // Guard against accidental drift; update when intentionally changing fonts/layout.
        assert_eq!(raster_hash(&bmp), 4819436873265637314);
    }
}

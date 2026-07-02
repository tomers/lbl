//! Console (terminal) "printer": renders the dithered [`MonoBitmap`] as text.
//!
//! Like the virtual file driver, this is not a hardware protocol — it presents
//! a label to a human instead of a device. The raster is drawn with Unicode
//! half-block characters (`▀`), so each character cell carries two vertical
//! pixels and the preview is half as tall as the bitmap.
//!
//! Two looks are available via [`TerminalOptions::color`]:
//!
//! * **Plain** (default, deterministic): ink → block glyphs (`█ ▀ ▄`), blank
//!   media → space. Safe to redirect to a file or pipe.
//! * **Color**: each half-cell is painted with ANSI foreground/background so the
//!   label appears as black ink on white media, the way it prints.
//!
//! The same renderer backs `lbl`'s `--protocol console` output, `--preview`,
//! the `--confirm` preview, and the `--debug` dithered-raster dump, so all
//! agree.

use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

/// How to render a [`MonoBitmap`] to the terminal.
#[derive(Debug, Clone, Copy)]
pub struct TerminalOptions {
    /// Maximum width, in terminal columns. Wider rasters are box-downsampled to
    /// fit (aspect-preserving); narrower rasters are never upscaled.
    pub max_width: usize,
    /// Draw a light box-drawing frame around the label.
    pub frame: bool,
    /// Emit ANSI colors (black ink on white media). When false, plain block
    /// glyphs are used — appropriate for files and pipes.
    pub color: bool,
}

impl Default for TerminalOptions {
    fn default() -> Self {
        Self {
            max_width: 100,
            frame: true,
            color: false,
        }
    }
}

/// Render a [`MonoBitmap`] as terminal art per [`TerminalOptions`].
///
/// Set bits are ink; clear bits are blank media. The result is newline
/// terminated and ready to write to a stream.
pub fn render_terminal(bitmap: &MonoBitmap, opts: &TerminalOptions) -> String {
    if bitmap.width == 0 || bitmap.height == 0 {
        return "(empty raster)\n".to_string();
    }

    let src_w = bitmap.width as usize;
    let src_h = bitmap.height as usize;
    let target_w = src_w.min(opts.max_width.max(1));
    let target_h = (((src_h * target_w) as f64 / src_w as f64).round() as usize).max(1);

    // Box-downsample into a boolean grid: a target cell is ink when at least
    // half of the source pixels it covers are ink.
    let mut grid = vec![false; target_w * target_h];
    for ty in 0..target_h {
        let sy0 = ty * src_h / target_h;
        let sy1 = (((ty + 1) * src_h / target_h).max(sy0 + 1)).min(src_h);
        for tx in 0..target_w {
            let sx0 = tx * src_w / target_w;
            let sx1 = (((tx + 1) * src_w / target_w).max(sx0 + 1)).min(src_w);
            let mut ink = 0usize;
            let mut total = 0usize;
            for sy in sy0..sy1 {
                for sx in sx0..sx1 {
                    total += 1;
                    if bitmap.get(sx as u32, sy as u32) {
                        ink += 1;
                    }
                }
            }
            grid[ty * target_w + tx] = total > 0 && ink * 2 >= total;
        }
    }
    let ink_at = |x: usize, y: usize| -> bool { y < target_h && grid[y * target_w + x] };

    let mut out = String::new();
    if opts.frame {
        out.push('┌');
        out.extend(std::iter::repeat_n('─', target_w));
        out.push_str("┐\n");
    }

    let mut ty = 0;
    while ty < target_h {
        if opts.frame {
            out.push('│');
        }
        for tx in 0..target_w {
            let top = ink_at(tx, ty);
            let bot = ink_at(tx, ty + 1);
            if opts.color {
                // '▀' paints its upper half with the foreground color and its
                // lower half with the background color. Ink is black (30/40),
                // media is white (97/107).
                let fg = if top { 30 } else { 97 };
                let bg = if bot { 40 } else { 107 };
                out.push_str(&format!("\x1b[{fg};{bg}m▀"));
            } else {
                out.push(match (top, bot) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => ' ',
                });
            }
        }
        if opts.color {
            out.push_str("\x1b[0m");
        }
        if opts.frame {
            out.push('│');
        }
        out.push('\n');
        ty += 2;
    }

    if opts.frame {
        out.push('└');
        out.extend(std::iter::repeat_n('─', target_w));
        out.push_str("┘\n");
    }
    out
}

/// The console preview "printer": encodes the bitmap as plain (file-safe)
/// terminal art.
#[derive(Debug, Default, Clone, Copy)]
pub struct ConsoleDriver;

impl ConsoleDriver {
    /// Create a console driver.
    pub fn new() -> Self {
        Self
    }
}

impl Driver for ConsoleDriver {
    fn protocol(&self) -> Protocol {
        Protocol::Console
    }

    fn name(&self) -> &'static str {
        "console"
    }

    fn encode(&self, bitmap: &MonoBitmap, _ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.width == 0 || bitmap.height == 0 {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }
        Ok(render_terminal(bitmap, &TerminalOptions::default()).into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::job::JobSpec;
    use lbl_core::media::Media;
    use lbl_core::printer::PrinterCapabilities;
    use lbl_core::units::Dpi;

    fn checker(w: u32, h: u32) -> MonoBitmap {
        let mut bmp = MonoBitmap::new(w, h);
        for y in 0..h {
            for x in 0..w {
                bmp.set(x, y, (x + y) % 2 == 0);
            }
        }
        bmp
    }

    #[test]
    fn plain_uses_block_glyphs_and_is_half_height() {
        let bmp = checker(6, 4);
        let art = render_terminal(
            &bmp,
            &TerminalOptions {
                frame: false,
                color: false,
                ..Default::default()
            },
        );
        let rows: Vec<&str> = art.lines().collect();
        // 4 pixel rows pack into 2 character rows.
        assert_eq!(rows.len(), 2);
        assert!(art.chars().any(|c| matches!(c, '█' | '▀' | '▄')));
        // No ANSI escapes in plain mode.
        assert!(!art.contains('\x1b'));
    }

    #[test]
    fn color_mode_emits_ansi_and_half_blocks() {
        let bmp = checker(4, 4);
        let art = render_terminal(
            &bmp,
            &TerminalOptions {
                frame: false,
                color: true,
                ..Default::default()
            },
        );
        assert!(art.contains('\x1b'));
        assert!(art.contains('▀'));
        assert!(art.contains("\x1b[0m"));
    }

    #[test]
    fn frame_wraps_content() {
        let bmp = checker(3, 2);
        let art = render_terminal(
            &bmp,
            &TerminalOptions {
                frame: true,
                color: false,
                ..Default::default()
            },
        );
        assert!(art.contains('┌') && art.contains('┐'));
        assert!(art.contains('└') && art.contains('┘'));
    }

    #[test]
    fn wide_raster_is_downsampled_to_max_width() {
        let bmp = checker(200, 50);
        let art = render_terminal(
            &bmp,
            &TerminalOptions {
                max_width: 40,
                frame: false,
                color: false,
            },
        );
        // Each content row must be no wider than max_width characters.
        for line in art.lines() {
            assert!(line.chars().count() <= 40, "line too wide: {line:?}");
        }
    }

    #[test]
    fn driver_encodes_plain_art() {
        let bmp = checker(8, 4);
        let job = JobSpec::new(Media::fixed(25.0, 54.0, Dpi(300.0)));
        let caps = PrinterCapabilities::default();
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = ConsoleDriver::new().encode(&bmp, &ctx).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains('\x1b'));
        assert!(text.contains('┌'));
    }

    #[test]
    fn empty_bitmap_is_unsupported() {
        let bmp = MonoBitmap {
            width: 0,
            height: 0,
            data: vec![],
        };
        let job = JobSpec::new(Media::fixed(25.0, 54.0, Dpi(300.0)));
        let caps = PrinterCapabilities::default();
        let ctx = EncodeContext::new(&job, &caps);
        assert!(ConsoleDriver::new().encode(&bmp, &ctx).is_err());
    }
}

# 0004 — Two-pass render + photo-aware dither

- Status: Accepted

## Context

Thermal label heads are 1-bit. Rendering directly at device resolution and
thresholding produces aliased text and destroys photographs.

## Decision

Render in two passes: rasterize at `supersample×` the device resolution, then
downscale to the exact device dots with a Lanczos3 filter. Then dither to 1-bit
with a photo-aware Floyd-Steinberg: near-pure source pixels (text/line art) are
hard-thresholded and excluded from error diffusion to stay crisp, while
mid-tones (photos) are diffused. Ordered and plain threshold modes are also
available.

## Consequences

- Noticeably better quality for mixed text/photo labels.
- Higher render cost (larger first-pass image); tunable via `supersample`.
- The downscale and dither steps are pure functions, easy to test.

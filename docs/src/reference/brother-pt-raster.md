# Brother P-touch (PT) raster protocol

Wire notes for the `brother-pt` encoder (`lbl-driver-brother-pt`). This page
captures decisions that are easy to lose when Brother PDFs disagree with each
other or with QL docs.

`lbl` is not affiliated with Brother.

## Job skeleton

```text
N × 0x00                invalidate (200 default; override via caps)
ESC @                   initialize
ESC i a 0x01            switch to raster mode
per page:
  ESC i z …             print information (TZe width + raster line count)
  ESC i M …             various mode (auto-cut bit)
  ESC i A …             cut every N (when auto-cut)
  ESC i K …             advanced mode (no-chain / high-res)
  ESC i d …             margin / feed amount
  M 0x00 | M 0x02       compression off or TIFF PackBits
  per row:
    G n_lo n_hi <n B>   raster graphics transfer
    or Z                blank row (PackBits mode only)
  0x0C | 0x1A           print (more pages follow) | print with feed (last)
```

Multi-page batches must **not** wrap each label as a full job. Emit invalidate +
init once, use page index in `ESC i z`, set no-chain only where the page role
requires it, end intermediate pages with `0x0C`, and end the job with `0x1A`.
Wrapping every label in invalidate + no-chain + `0x1A` feeds an empty leader
cut before each real label (head-to-cutter scrap).

## Head-to-cutter leader (~23–25 mm)

The print head sits roughly one inch **before** the cutter so TZe can laminate.
Brother documents this as unavoidable waste on a fully cut job:

- **Large lead** (\(p \ge D_x\)): blank nose stays **on** the kept label; no pre-cut.
- **Small lead** + **opt-in pre-cut** (`FeedPlan.precut`): encoder emits a
  zero-raster page with auto-cut + no-chain + `0x1A` (nbuchwitz/ptouch-style)
  before the first content page, ejecting ≈ \(D_x\) as scrap; content uses
  `ESC i d` from `feed_plan.lead_mm`.
- **Small lead** without pre-cut: encode rejects (`lead_padding_below_cutter_gap`).
- **End clearance:** after the last inked row (still at the head), print-with-feed
  and cut leave ≈ \(D_x\) blank **after** content on the kept sticker.
  `resolve_feed_plan` floors `end_mm` to \(D_x\) when cutting so preview matches;
  this driver emits trailing blank rasters only for surplus above \(D_x\) (firmware
  `0x1A` already advances the clearance).
- **Multi-label batches:** one pre-cut for the job (`batch_first` only); do **not**
  end every page with no-chain/`0x1A` as its own job.
- Unset job lead defaults to \(D_x\) (no surprise scrap). See
  [padding-driven pre-cut](../plans/precut-feed-padding.md).

## Pre-cut prologue (when `feed_plan.precut`)

```text
(after invalidate + ESC @ + ESC i a)
ESC i z …   raster line count = 0
ESC i M …   auto-cut
ESC i A 01
ESC i K …   no-chain
ESC i d …   margin (same dots as content lead)
M …
0x1A        print-and-feed → eject scrap
(then normal content page(s))
```

## `ESC i z` media type (`n2`)

| Chassis | Laminated TZe `n2` | Notes |
|---------|-------------------|--------|
| P700 / Cube (128-dot) | `0x01` | P710BT family PDF: `0x00` = no media, `0x01` = laminated |
| P900 (560-dot) | `0x00` | P900 PDF: lam/non-lam; high-res laminated uses `0x09` |

## Raster opcode: `G` (0x47), not QL `g`

| Opcode | ASCII | Length framing | Use |
|--------|-------|----------------|-----|
| `0x47` | `G` | `n1 + 256·n2` (u16 little-endian) | **PT path used by this driver** |
| `0x67` | `g` | Often `n2` only, or QL `67 00 n` (u8) | QL / other chassis — **not** PT u16 LE |

Brother manuals conflict by family (P710BT / Cube docs list `G`/`47`; some P700
tables list `g`/`67`; P900 TOC/detail can disagree). Community reverse
engineering treats `G` and `g` as **different commands with different length
rules**, not as a single typo.

**Hardware check (PT-P710BT Cube Plus, USB `04f9:20af`):**

- `0x47` + u16 LE — bulk OUT completes; prints.
- `0x67` + u16 LE — mid-job bulk stall; status often `error` with empty fault
  masks (“unspecified printer error” in UIs).

Do **not** port QL’s `g 00 <u8 length>` onto PT. Mixing QL’s opcode with PT’s
two-byte length is the usual desync pattern.

## Compression (`M`) and PackBits

| Mode | Meaning |
|------|---------|
| `M 0x00` | Uncompressed: `G` + u16 + raw head bytes |
| `M 0x02` | TIFF PackBits: blank rows as `Z`; otherwise `G` + u16 + **PackBits payload only** |

Under `M 0x02`, never fall back to raw head bytes when PackBits does not shrink
the row. Short Cube heads (16 bytes/row) hit that case often. Raw bytes are
misread as PackBits run headers → parser desync → job may “complete” with **no
ink**. Emit a valid PackBits encoding instead (literal-only is fine and may be
one byte longer than the raw row). Shared helper: `lbl_driver_api::packbits_encode`.

The same `M 02` invariant applies to `brother-ql` (`g 00 n` + PackBits payload).

## Heads and geometry

| Class | Head | Bytes/row | DPI | Selected when |
|-------|-----:|----------:|----:|---------------|
| P700 / Cube | 128-dot | 16 | 180 | `max_width_mm` ≤ 24 |
| P900 | 560-dot | 70 | 360 | `max_width_mm` > 24 |

Printable band and left offset depend on TZe width (see driver
`resolve_media`). Bit polarity: `MonoBitmap` `1` = ink; each row is mirrored
before packing.

High-resolution (capability-gated) duplicates each raster line, doubles feed
margin, and uses print-info media type `0x09` for laminated high-res jobs.

## Status

USB models answer `ESC i S` with a 32-byte status block (parsed in
`lbl-status` / `brother_pt`). Tape width mapping to TZe SKUs is catalog-driven;
color/finish is not unique from status alone.

## References (external)

Prefer these when Brother PDFs conflict:

1. [Undocumented Printing (Stecman archive)](https://archive.stecman.co.nz/files/datasheets/P-Touch-Cube/brother-p-touch-undocprint.html)
   — dual `G`/`g` length rules.
2. [philpem/printer-driver-ptouch `rastertoptch.c`](https://github.com/philpem/printer-driver-ptouch/blob/master/rastertoptch.c)
   — PT emits `G` + LE length; QL emits `g`.
3. Brother *Raster Command Reference* for PT-E550W / P750W / **P710BT** (lists `G`/`47`).
4. [nbuchwitz/ptouch](https://github.com/nbuchwitz/ptouch) — `0x47` + u16 LE; multi-page / cut behaviour notes.
5. [ogelpre/labelprinterkit](https://github.com/ogelpre/labelprinterkit) — PT `G`; notes some Brother docs wrongly say `g`.

QL-oriented writeups (`g 00 NN`, thermal-label QL pages, many blog posts) are
correct for QL and wrong when copied onto PT length framing.

## Related code

- Encoder: `crates/drivers/lbl-driver-brother-pt`
- PackBits: `crates/drivers/lbl-driver-api` (`packbits`)
- Status parse: `crates/lbl-status` (`brother_pt`)
- Device transport helpers: `crates/lbl-device` (`brother_pt`)
- Setup guide: [Brother P-touch / TZe setup](../guides/brother-pt.md)
- QL contrast: [Brother QL setup](../guides/brother-ql.md) (different row opcode)

# DYMO LabelWriter 550 (LW5) command coverage

Opcode vocabulary for the LabelWriter 550 / 550 Turbo / 5XL wire protocol
(`dymo-lw`). Authoritative layout: DYMO *LabelWriter 550 Series Technical
Reference*; community summary at
[thermal-label LW5 raster](https://thermal-label.github.io/labelwriter/protocol/lw5-raster).

This page tracks which opcodes `lbl` implements today and whether each gap is
useful for a current or future product feature. Prefer implementing a command
here when a feature needs it — do not invent host-side workarounds that diverge
from the printer’s NFC/engine state.

| Status | Meaning |
|--------|---------|
| **Implemented** | Issued and parsed (or encoded into jobs) in `lbl-device` / `lbl-driver-dymo` / browser WebUSB. |
| **Partial** | Used in a limited way; richer fields unused. |
| **Unimplemented** | Not sent or parsed anywhere in this tree. |

## Opcode table

| Opcode | Bytes | Status | Role | Feature usefulness |
|--------|-------|--------|------|--------------------|
| `ESC @` | `1B 40` | **Unimplemented** | Restart / soft-reboot the print engine. | **Useful.** Recovery when the engine is wedged after a failed job or stuck lock (alternative to power-cycle). Studio “wake / reset printer” and CLI troubleshoot flows. |
| `ESC *` | `1B 24`¹ | **Unimplemented** | Restore factory settings. | **Low.** Rare; risky on shared devices. Only if we add an explicit advanced “factory reset” action with strong confirmation. |
| `ESC A` | `1B 41 nn` | **Implemented** | Print-engine status (**32**-byte reply); lock acquire / inter-label / release. | **Core.** Idle polling, print handshakes, remaining label count, bay/SKU/error/voltage. |
| `ESC C` | `1B 43 nn` | **Implemented** | Set print density (duty %). | **Core.** Job density encoding in the LW550 driver. |
| `ESC D` | `1B 44 …` | **Implemented** | Start of label raster + payload. | **Core.** Every print job. |
| `ESC e` | `1B 65` | **Unimplemented** | Reset print density to 100%. | **Low.** Density is set per job via `ESC C`; a standalone reset is only useful for interactive tuning / recovery after a bad density write. |
| `ESC E` | `1B 45` | **Implemented** | Feed to tear position (job trailer). | **Core.** End of every multi/single-label job. |
| `ESC G` | `1B 47` | **Implemented** | Short feed to print head (per-label footer). | **Core.** Inter-label advance + handshake pacing. |
| `ESC h` | `1B 68` | **Implemented** | Select text output mode (300 dpi). | **Core.** Default job header mode; engine settings tuned for text. |
| `ESC i` | `1B 69` | **Implemented** | Select graphics output mode (300 dpi). | **Useful.** Studio advanced print mode for barcodes/graphics; same raster DPI, engine slows for cleaner dots. (LW450’s 300×600 feed mode is a different protocol.) |
| `ESC L` | `1B 4C …` | **Implemented** | Set maximum label length (continuous stock). | **Core** for continuous media; unused for die-cut (length comes from NFC). |
| `ESC n` | `1B 6E NN` | **Implemented** | Set label index in the current job. | **Core.** Multi-copy jobs + status echo of current label. |
| `ESC o` | `1B 6F nn` | **Unimplemented** | Override on-printer remaining label count. | **Avoid for product features.** Would desync the NFC counter from physical stock; at most a diagnostics/test hook. Prefer reading the real counter (`ESC A` / NFC). |
| `ESC Q` | `1B 51` | **Implemented** | End of print job (releases lock). | **Core.** Job trailer + soft recovery (send alone to drop a held lock). |
| `ESC s` | `1B 73 NNNN` | **Implemented** | Start of print job (job id). | **Core.** Job header; id echoed in status replies. |
| `ESC T` | `1B 74 nn` | **Implemented** | Content type / speed mode (`0x10` normal, `0x20` high). | **Useful.** Faster throughput on supported rolls; print-quality vs speed setting in Studio. Not all media/chassis support high speed (5XL does not; driver clamps to normal). |
| `ESC U` | `1B 55` | **Implemented** | Get SKU / NFC dump (63-byte reply), including **total label count**. | **Core for roll UI.** Full-roll total for depleting progress bars; richer geometry/material fields still unused (see below). |
| `ESC V` | `1B 56` | **Implemented** | Get engine version (HW/FW/PID block, 34 bytes). | **Useful.** Printer detail “About” (firmware/hardware), support diagnostics, and compatibility checks before enabling speed modes or new job features. |

¹ Tech Ref mnemonic is `ESC *` but the documented wire byte is `0x24` (`$`). Hex is authoritative.

## `ESC U` fields beyond what we use

We parse magic, SKU, and **total label count** (bytes 50–51) and fold the total
into status as `label_total`. Remaining count still comes from `ESC A`.

Still unused from the same dump (candidates for later media auto-config):

| Field (approx.) | Why it may matter |
|-----------------|-------------------|
| Material / label / colour bytes | Richer media identity when catalog SKU is missing or ambiguous. |
| Marker geometry + label W/H (deci-mm) | Cross-check or fill media size without catalog; continuous vs die-cut. |
| Printable-area offsets / liner width | Tighter layout margins for unknown SKUs. |
| Total length, counter margin / strategy | Diagnostics for NFC counter behaviour; continuous roll length UI. |
| Production date/time | Support / authenticity breadcrumbs only. |

## Intentionally not mirrored

- **Host lock policy** — we acquire/release via `ESC A` lock bytes around jobs; no separate opcode.
- **Catalog pack quantity** — not a substitute for NFC `total_label_count` (pack sizes vary; NFC is authoritative).

## Related code

- Device / status: `lbl-device` `dymo_lw` (`ESC A`, `ESC U`, `ESC V`)
- Job encode: `lbl-driver-dymo` `lw550` (`ESC s/L/h|i/T/C/n/D/G/E/Q`)
- Browser WebUSB: `apps/frontend/lib/client-dispatch/dymo-lw.ts`
- Status UI row: shared `labelRemainingProgressRow` in the frontend printer-status layer

# Plan: padding-driven pre-cut (opt-in)

Status: **proposed** (not implemented).
Scope: generic engine capability in `lbl`; first concrete driver: Brother PT.
Studio consumes catalog + job fields only — no vendor cut logic in the UI.

Companion: [Brother PT raster](../reference/brother-pt-raster.md) (head-to-cutter
notes). Layering: engine owns feed/pre-cut **policy** and driver prologues;
clients only expose padding + capability fields from the catalog (no
vendor-specific cut branches in the UI).

---

## 1. Problem

Many tape printers have a fixed **head-to-cutter distance** \(D_x\) (catalog
[`feed_trail_mm`](../../crates/lbl-core/src/printer.rs)): the blade sits downstream
of the print head so laminate / stock can seal. Consequences:

| User request | Pre-cut off | Pre-cut on (user opted in) |
| --- | --- | --- |
| Lead padding \(p \ge D_x\) | One piece: blank lead \(p\) + content; cut after | Same (pre-cut unused) |
| Lead padding \(p < D_x\) | **Reject** — cannot honor small lead without ejecting the gap | Eject \(\sim D_x\) as an empty scrap, then print with lead \(p\) |

Brother documents \(\sim 23\)–\(25\,\mathrm{mm}\) for TZe; DYMO LM-class uses a
smaller \(D_x\) (\(\sim 8.1\,\mathrm{mm}\)). The **policy** is the same; only
\(D_x\) and the wire prologue differ per driver.

Today we document the gap but do not model pre-cut. Users who want “small
margin” either get a surprise empty scrap (firmware / leftover state) or a fat
lead they did not ask for.

---

## 2. Goals

1. **User specifies padding** (lead / end along feed), not vendor cut opcodes.
2. **Pre-cut is opt-in.** The UI/engine **suggests** enabling it when padding
   is below \(D_x\); it must **not** flip the setting on automatically when the
   user shrinks padding.
3. Once the user (or profile) has **enabled** pre-cut, the engine **applies** it
   when \(p < D_x\) and a cut will fire — no per-print confirmation beyond the
   standing preference.
4. **`supports_precut`** is a catalog capability (device *can* do it). Default
   **job/profile preference is off** (`precut_default = false`) so small-margin
   prints require an explicit enable (after the user accepts the scrap tradeoff).
5. **Pre-cut off** (or unsupported) **rejects** \(p < D_x\) — see §5.1. Do not
   silently inflate padding or surprise-cut.
6. **Generic across protocols**; **one pre-cut per job** when applicable, not
   per chained page.

Non-goals for this delivery:

- Half-cut vs full-cut policy (separate).
- Inventing padding UI chrome beyond catalog/schema-driven fields.
- Changing PackBits / raster opcode framing.
- Auto-toggling the user’s pre-cut preference from padding changes.

---

## 2.1 Tradeoffs (why opt-in, not silent)

Pre-cut does **not** save tape versus a large lead. \(D_x\) of stock is still
consumed either way; only **where** the blank goes changes.

| Approach | What the user gets | Tape / mechanism |
| --- | --- | --- |
| **Large lead** (\(p \ge D_x\)), no pre-cut | One sticker: long blank nose + content | \(\sim D_x\) (or \(p\)) blank stays **on the label** |
| **Small lead** + **pre-cut on** | Short empty scrap drops first, then a short-lead label | \(\sim D_x\) blank ejected as **scrap**, then content with small \(p\) |
| **Small lead** + **pre-cut off** | Invalid — Print disabled / encode error | Nothing sent |

So yes: the main tradeoff **is** that an empty piece falls off (waste you throw
away, easy to mistake for a failed print, extra cut noise/time). The upside is
a **shorter kept label** with the margin the user actually asked for.

Mention this in:

- The enable-pre-cut control help text.
- Hints that suggest enabling pre-cut (§5.1).
- Preview when `feed_plan.precut` (scrap segment labeled as ejected).

Secondary costs (brief): one extra cut cycle per job that needs it; scrap litter
next to the printer. Not a reason to hide the feature — a reason to **ask**
before enabling.

---

## 3. Definitions

| Symbol | Meaning | Source |
| --- | --- | --- |
| \(D_x\) | Head-to-cutter distance (mm) | `DeviceCapabilities::feed_trail_mm` (required when `supports_precut`) |
| \(p_{\mathrm{lead}}\) | Requested blank before content (mm) | Job / print settings (`feed_lead_mm` or explicit padding field — see §5) |
| \(p_{\mathrm{end}}\) | Requested blank after content before cut (mm) | Job / settings |
| \(p_{\min}\) | Minimum lead the chassis can honor after a pre-cut (mm) | Catalog optional `feed_lead_min_mm`; default `0` or driver floor (e.g. Brother `ESC i d` clamp) |
| Pre-cut | Zero-content (or protocol-equivalent) feed to cutter + cut, ejecting \(\approx D_x\) scrap | Driver `precut` prologue |

**Decision rule (when cut will fire after this label / job):**

```text
if !will_cut:
    no precut
else if p_lead >= Dx:
    no precut   # lead absorbs the gap (or equals it)
else if supports_precut && precut_enabled:
    emit precut, then encode with p_lead
else:
    reject: suggest increase padding and/or enable precut (do not auto-enable)
```

“Pre-cut enabled” = job/profile `precut: true`. Catalog
`supports_precut` only means the control is offered; **`precut_default` is
false** so new sessions do not eject scrap until the user opts in.

**Never:** set `precut: true` as a side effect of the user lowering padding.
Only suggest.

---

## 4. Catalog / capabilities

Add to device catalog + `DeviceCapabilities` (names illustrative; keep serde
stable snake_case):

| Field | Type | Meaning |
| --- | --- | --- |
| `feed_trail_mm` | `f64` (existing) | \(D_x\). **Required** if `supports_precut` is true. |
| `supports_precut` | `bool`, default `false` | Device can emit a pre-cut prologue. **True** on PT Cube / P700-class once implemented; false when no safe empty feed-cut. |
| `feed_lead_min_mm` | `Option<f64>` | Floor for \(p_{\mathrm{lead}}\) after pre-cut (protocol clamp). |
| `precut_default` | `bool`, default **`false`** | Initial job/profile preference. User must enable to allow \(p < D_x\). |

Validation at catalog load: `supports_precut => feed_trail_mm.is_some_and(|d| d > 0)`.

Populate:

- Brother PT with cut + known \(D_x\) (e.g. P710BT `feed_trail_mm = 24`).
- Later: other tape cutters where empty feed-and-cut is defined (DYMO D1-style,
  etc.) — each driver implements the prologue; policy stays shared.

---

## 5. Job / API surface (generic)

Prefer **padding as the user-facing knob**; pre-cut as an explicit preference:

```text
JobSpec / print options:
  feed_lead_mm: Option<f64>     # requested lead padding (content-side)
  feed_end_mm: Option<f64>      # optional trailing padding before cut
  precut: Option<bool>          # None = use device precut_default (false)
  cut_mode: CutMode             # existing
```

CLI / HTTP / WASM:

- Accept the same fields.
- On reject, return a structured error, e.g.
  `lead_padding_below_cutter_gap { requested_mm, cutter_gap_mm, precut_supported }`
  so UIs can offer “enable pre-cut” or “increase margin” without hardcoding PT.

Do **not** expose a Brother-only `ESC i …` toggle in Studio. If a power-user
override is needed, it is `precut: bool` on the job.

### 5.1 What “reject” means (surfacing)

**Reject = validation failure before any print I/O.** The engine does not send
USB/serial bytes, does not emit a pre-cut, does not flip `precut` on, and does
not quietly bump padding up to \(D_x\). Callers get a structured error (token +
numeric fields), e.g.
`lead_padding_below_cutter_gap { requested_mm, cutter_gap_mm, precut_supported }`.

Surfacing is **two layers** (same pattern as today’s “media probe pending” /
`canPrint` gating):

| Layer | Behavior |
| --- | --- |
| **Proactive UI** | While lead \(p < D_x\) and pre-cut is off (or unsupported), **Print is disabled** and an inline hint explains why (map the engine token via `engine-labels`). Highlight the padding and (when relevant) pre-cut controls. |
| **Hard stop on encode** | CLI / API / WASM encode still runs `resolve_feed_plan` and returns the same error if a client bypasses the UI. Studio shows that message (toast or print-status line) if a race slips through. |

**Hint copy must offer the fix(es), not only the constraint — and must not
auto-apply them:**

| Condition | Suggested actions in the message |
| --- | --- |
| `precut_supported` (capability on; preference still off) | **Increase lead padding** to at least \(D_x\) **mm**, **or enable pre-cut** (user action). When suggesting pre-cut, **mention the empty scrap** (\(\approx D_x\) mm drops off before the real label). |
| Pre-cut not supported on this device | **Increase lead padding** to at least \(D_x\) **mm** only. |

Example tone (final strings live in `engine-labels`): *“Lead padding (4 mm) is
below this printer’s cutter gap (24 mm). Increase padding to 24 mm, or enable
pre-cut (ejects ~24 mm of empty tape as scrap, then prints with your smaller
margin).”*

UI may deep-link focus to the pre-cut toggle / padding field; it must **not**
set `precut: true` without an explicit user gesture (click the toggle / confirm).

CLI/server: print commands exit non-zero with the structured error message
(including both remedies when `precut_supported`); no device write.

---

## 6. Pipeline policy (single place)

Implement one pure function in `lbl-core` or `lbl` pipeline (unit-tested):

```text
resolve_feed_plan(caps, job) -> FeedPlan {
  lead, end, precut: bool, cutter_gap
}
```

Rules:

1. Resolve \(p_{\mathrm{lead}}\), \(p_{\mathrm{end}}\) from job, else caps
   `feed_lead_mm` / symmetric trail defaults (keep today’s preview symmetry
   helpers aligned).
2. Apply `feed_lead_min_mm` clamp only when documenting floors — **do not**
   silently raise \(p_{\mathrm{lead}}\) above the user’s request to avoid
   pre-cut; that would hide the tradeoff. Either pre-cut (if enabled) or reject.
3. Set `precut` from §3 (preference ∧ need ∧ capability) — never invent
   preference from padding alone.
4. Pass `FeedPlan` into encode context so drivers do not re-derive policy.

Batch / copies:

- **Pre-cut once** at the start of a job that will cut and needs \(p < D_x\)
  **and** preference is on.
- Multi-page with `0x0C` chaining: still one pre-cut for the job, not per page.
- `CutMode::Every` as separate physical jobs (if ever split): each job may
  pre-cut — prefer coalesced Brother batch so scrap is paid once per batch.

---

## 7. Driver contract

Extend `EncodeContext` (or adjacent) with:

```text
feed_plan: FeedPlan  # lead/end mm, precut: bool, cutter_gap_mm
```

Drivers that `supports_precut`:

1. If `feed_plan.precut`, emit protocol **pre-cut prologue** before the first
   content page (after invalidate/init as required by that protocol).
2. Encode content with margins matching `feed_plan.lead` / `end` (dots via
   DPI), not with an implicit extra \(D_x\) lead.
3. If `precut` is false, never emit the prologue.

**Brother PT (first implementer):** zero raster lines (or empty page control
block) + auto-cut + no-chain + `0x1A`, matching nbuchwitz/ptouch `precut()` —
documented in the PT raster reference. Do not invent a second cut mechanism.

**Other drivers:** same `FeedPlan`; different bytes (or no-op if capability
false).

Drivers **must not** decide “user asked for 4 mm so I’ll pre-cut” on their
own — only honor `feed_plan.precut`.

---

## 8. Preview

Align `pad_preview_encode_feed` / layout CSS with `FeedPlan`:

- When `precut`: show a discarded scrap marker of width \(D_x\) (dashed /
  labeled “ejected” / scrap), then the kept label with lead \(p_{\mathrm{lead}}\).
- When `!precut` and \(p \ge D_x\): single strip, lead \(p\).
- When validation would reject: preview/editor surfaces the same error token
  before print (with enable / increase suggestions).

Preview must not invent vendor opcodes.

---

## 9. Frontend / Studio (generic client)

- Read `supports_precut`, `feed_trail_mm`, `precut_default` (false), min lead
  from catalog / device capabilities JSON Schema (or existing caps DTO).
- Print settings: numeric **lead/end padding**; **“Allow pre-cut”** (or similar)
  bound to `precut`, default off; help text states the empty-scrap tradeoff (§2.1).
- If user sets lead \(< D_x\) and pre-cut off → **disable Print** + hint that
  offers **increase padding to \(D_x\)** and, when supported, **enable pre-cut**
  (user must toggle — do not auto-enable) (§5.1); encode-path rejection as
  backstop.
- No `brother-pt` branches for this feature.

---

## 10. Implementation order

1. **Core:** `FeedPlan` + `resolve_feed_plan` + error type; catalog fields;
   serde on caps.
2. **Tests:** table-driven policy (precut on/off, \(p < D_x\), \(p \ge D_x\),
   no cut, missing \(D_x\); assert lowering padding does not imply preference).
3. **Brother PT driver:** emit precut prologue; wire `ESC i d` / row padding to
   `feed_plan`; update PT raster doc.
4. **Pipeline / CLI / server / WASM:** plumb job fields; reject path.
5. **Preview** markers for ejected scrap.
6. **Catalog:** `supports_precut = true`, `precut_default = false` on PT cutters
   with known \(D_x\); leave others false.
7. **Studio:** schema-driven padding + precut toggle (opt-in) + scrap copy;
   hard-refresh wasm.

---

## 11. Test plan

| Layer | Cases |
| --- | --- |
| Unit (`resolve_feed_plan`) | \(p < D_x\) + precut on → precut; \(p < D_x\) + precut off → err; \(p \ge D_x\) → no precut; `CutMode::None` → no precut; `supports_precut` false + \(p < D_x\) → err |
| Unit (PT encode) | `feed_plan.precut` → control block with **zero** raster lines then `0x1A` before content job; content job margins match lead; multi-page → single precut |
| Integration / CLI | Encode-only fixture diffs; optional on-device: small lead with precut → scrap then short-lead label; precut off + small lead → error, no USB write |
| Preview | Snapshot or assertion on scrap + content geometry |
| Frontend | Caps-driven: cannot submit invalid combo; hint suggests enable (does not auto-toggle); tokens mention scrap |

Done = all of the above green for PT; at least one non-PT driver documents
`supports_precut = false` and inherits reject behavior without code changes.

---

## 12. Open questions (resolve before coding)

1. **Default lead when unset:** use \(D_x\) (no precut, “large margin” on the
   label) or a small catalog default that requires opt-in precut? Proposal:
   unset lead means \(D_x\) (no surprise scrap); explicit small lead + user
   enables precut.
2. **End padding vs cutter:** after pre-cut, is \(p_{\mathrm{end}}\) independent
   of \(D_x\), or does no-chain still force \(\sim D_x\) trail on the last cut?
   Measure on P710BT; encode trail accordingly.
3. **Profile vs job:** does `printers.toml` store a `precut` override, or only
   per-print? Proposal: catalog `precut_default = false`; optional profile
   override; job wins.
4. **Naming:** `feed_lead_mm` vs `padding_lead_mm` — prefer extending existing
   feed_* vocabulary for consistency with preview.

---

## 13. Summary decision

**Padding is the user control; pre-cut is an opt-in preference.** Shrinking
padding below \(D_x\) **suggests** enabling pre-cut (and mentions the empty
scrap) or increasing padding — it does **not** turn pre-cut on by itself. Once
enabled, the engine applies pre-cut when needed. Capability (`supports_precut`)
is catalog; preference defaults **off**. All policy lives in `lbl`; drivers only
emit the empty feed-and-cut their protocol defines.

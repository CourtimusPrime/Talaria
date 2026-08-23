---
phase: 05-distributed-mode
plan: 02
subsystem: infra
tags: [servo, opengl, glreadpixels, readback, png, tile-diff, benchmark, spike, llvmpipe, xvfb]

# Dependency graph
requires:
  - phase: 01-servo-shell
    provides: "`capture_now`, the per-tab `OffscreenRenderingContext`, and the show/hide invariant the lease shape had to survive"
  - phase: 04-authenticated-remote-transport-v2
    provides: "the throwaway-cargo-example spike form (04-02) and its leaves-no-source-behind discipline"
provides:
  - "`05-02-SPIKE.md` — assumption A1 settled CONFIRMED with a measured distribution over 900 ticks, and A8 settled REFUTED with the divergences named"
  - "The measured cost of `read_to_image` at a 30 ms cadence on this tree's real Servo: 1.19–1.56 ms mean, p95 never above 1.86 ms, 0 failures"
  - "T-05-12's loop-starvation ratio as a number: the pump costs 5.8 % of a 30 ms tick on a settled page and 21 % while scrolling, and slows the local chrome's own frame by 9–18 %"
  - "The discovery that `encode_screenshot` never ran at `png::Compression::Default` — png 0.17's `Info::default()` is `Fast`+`Sub` — which collapses D-05-05's three-line sibling encoder to one line and voids the 21 ms / 175 ms premise"
  - "A written instruction 05-08 is planned against, rather than an absence: request 30 ms unconditionally, never environment-gate the cadence, and let 05-10's ladder lower the delivered rate"
  - "A measured keyframe threshold — 90 of 260 tiles at 1280×800 — and the ceiling 05-10's fastest rung may claim"
  - "Two `deferred-items.md` entries: the `png` premise correction, and the absence of any hardware-GL path on this machine as a `VERIFICATION.md` manual item"
affects: [05-08, 05-09, 05-10, 05-11, phase-05.1]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "A spike that measures a future code path measures that path's *shape*, not the shape of the existing call site it will replace — here a held-open lease with unconditional paint, not `capture_now`'s show/wait/paint/read/hide"
    - "A performance claim is reported as a distribution (mean, median, p95, max, min) over 300 ticks per shape, never as a mean, because a p95 that blows the budget is what a user feels"
    - "A software-rendered figure is labelled as one, and the absence of a hardware figure is evidenced (GL_RENDERER, the DRI3 failure, the Xwrapper restriction) rather than asserted"
    - "A benchmark's *premise* is re-derived from the dependency's source before its numbers are trusted — the encoder configuration was read out of `png 0.17.16`'s `Info::default()`, not assumed from the call site"

key-files:
  created:
    - .planning/phases/05-distributed-mode/05-02-SPIKE.md
  modified:
    - .planning/phases/05-distributed-mode/deferred-items.md

key-decisions:
  - "A1 is CONFIRMED: `read_to_image` costs 1.19–1.56 ms mean and never more than 1.86 ms at p95 across three page shapes, so the phase's capture model stands and 05-08 may be executed as planned"
  - "A8 is REFUTED, and the honest verdict is worth more than a convenient one: the synthetic frames predicted encode *time* well and encode *bytes* badly (3.2× on the photo case), and the dirty-tile scan costs 2.4× its synthetic figure"
  - "`encode_screenshot` has always run at `Compression::Fast` + `FilterType::Sub`, because that is `png 0.17`'s `Info::default()` — proven by byte-identical output against an explicitly-configured encoder, with an explicit `Compression::Default` landing at 21.275 ms, within 1.4 % of the research's 20.98 ms"
  - "One number, not two: this machine has no non-llvmpipe GL path, so the Xvfb figure is reported alone with the limitation stated, rather than a second figure being fabricated"
  - "The Xvfb readback figure is called out as plausibly *optimistic* rather than conservative — under software rendering the framebuffer is already in system memory, so the readback is near a memcpy while `paint()` carries the cost; on a GPU that inverts"
  - "05-08 requests a 30 ms cadence unconditionally and must not environment-gate it; if a cadence ever cannot be sustained, 05-10's ladder lowers the delivered rate and the suite asserts the *requested* cadence plus the ladder's *response*"
  - "05-10's fastest rung stays 30 ms full-res; the measured production floor beneath it is ≈13 ms in the worst shape, and none of that headroom is yet spent on the wire"
  - "The e2e suite was run on display `:95`, not the `:98` the workflow file names, because `:98` belongs to this machine's self-hosted CI runner and two runs on one display SIGKILL each other"

patterns-established:
  - "Pattern: measure the local chrome's own frame in the same run, with and without the pump attached, so a cost claim is a ratio rather than an absolute nobody can interpret"
  - "Pattern: count the failure case explicitly over a sustained loop — a pump that silently drops a fraction of frames is a different product from one that does not"
  - "Pattern: prove an invariant rather than cite it — the displayed tab's own framebuffer was checksummed before and after the lease, in all three runs"

requirements-completed: []

coverage:
  - id: D1
    description: "`read_to_image` at a 30 ms cadence is a measured distribution on this machine rather than the phase's largest unknown — 900 ticks over three page shapes, with the local chrome's cost beside it"
    requirement: "DIST-02"
    verification:
      - kind: other
        ref: "crates/talaria-shell/examples/readback_spike.rs (deleted in Task 2); results transcribed into .planning/phases/05-distributed-mode/05-02-SPIKE.md § A1"
        status: pass
    human_judgment: false
  - id: D2
    description: "The codec numbers D-05-05 rests on are re-measured against real Servo readback, and the divergence from the synthetic figures is stated"
    requirement: "DIST-02"
    verification:
      - kind: other
        ref: ".planning/phases/05-distributed-mode/05-02-SPIKE.md § A8 — synthetic-vs-real table, wire table, keyframe threshold"
        status: pass
    human_judgment: false
  - id: D3
    description: "The spike leaves no source behind and no production source changed"
    requirement: "DIST-02"
    verification:
      - kind: other
        ref: "test ! -f crates/talaria-shell/examples/readback_spike.rs; git status --porcelain crates/ tests/ empty; git diff --stat crates/talaria-shell/src/app.rs empty"
        status: pass
    human_judgment: false
  - id: D4
    description: "Nothing in the tree changed: 296 unit tests and e2e 22/22 both at baseline after the deletion"
    verification:
      - kind: unit
        ref: "cargo test --locked — 291 + 3 + 2 = 296 passed, 0 failed"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/run_all.py on TALARIA_E2E_DISPLAY=:95 — 22/22, failed: none"
        status: pass
    human_judgment: false
  - id: D5
    description: "The real-hardware readback figure, which this repository cannot produce"
    verification: []
    human_judgment: true
    rationale: "Every X display on the build machine is an Xvfb on llvmpipe; reaching either GPU needs a compositor that is not installed or an Xorg holding DRM master from the console session. Confirming SC 2 on hardware requires the two-machine manual check, not the harness."

# Metrics
duration: 45min
completed: 2026-08-23
status: complete
---

# Phase 5 Plan 02: GL Readback and Frame-Encode Spike Summary

**The phase's largest unknown is now a measured distribution — `read_to_image` costs 1.19–1.56 ms
mean and never more than 1.86 ms at p95 over 900 sustained ticks, so A1 is CONFIRMED — and the
re-measurement against real Servo output refuted A8 by finding that `encode_screenshot` never ran at
`Compression::Default` at all, which voids the 21 ms figure the whole encoder argument was built on
while leaving D-05-05's actual choice standing.**

## Performance

- **Duration:** 45 min
- **Started:** 2026-08-23T11:05:00Z
- **Completed:** 2026-08-23T11:50:00Z
- **Tasks:** 2 of 2
- **Files modified:** 2 committed (`05-02-SPIKE.md` created, `deferred-items.md` appended); one
  throwaway created and deleted within the plan

## Accomplishments

- **A1: CONFIRMED.** `OffscreenRenderingContext::read_to_image`, driven the way the frame pump will
  drive it — leased tab held `show()`n, unconditional `paint()` on a 30 ms tick, full-surface
  readback, chrome still rendering — cost **1.555 / 1.194 / 1.543 ms mean** on a static text page, a
  scrolling page and a photographic page, with **p95 of 1.860 / 1.790 / 1.856 ms** and a worst single
  sample of 3.80 ms over all 900 ticks. The 30 ms cadence held in every shape, with exactly one
  overrun per 300 ticks, and that overrun was always the first frame after the lease was taken.
- **The readback is content-independent, and now it is understood why.** Reading
  `servo-paint-api-0.4.0`'s `Framebuffer::read_framebuffer_to_image` shows a 4 MB `glReadPixels`
  followed by a full `clone()` of those 4 MB and a row-by-row vertical flip — about 12 MB of memory
  traffic per call regardless of what the page contains. That is why the figure barely moves between
  a blank page and a photograph, and it is the part that will not get cheaper on a GPU.
- **`read_to_image` never returned its failure case** — 0 of 900 ticks across three shapes.
  **T-05-12-B is closed as not-observed** under software rendering.
- **T-05-12 is quantified rather than argued.** The pump's own main-thread cost is **5.8 % of a 30 ms
  tick** on a settled page and **21 % while scrolling**; the local chrome's own frame slowed from
  2.419 → 2.639 ms (+9 %), 2.093 → 2.434 ms (+16 %) and 2.165 → 2.557 ms (+18 %) with a viewer
  attached. `05-RESEARCH.md` estimated 10 %–25 %; the measurement lands inside it.
- **The per-tab-framebuffer invariant is proven, not cited.** An FNV-1a checksum over the *locally
  displayed* tab's own framebuffer, taken before the lease and again after release, was identical in
  all three runs.
- **A8: REFUTED, with the divergences named.** Real pages bracketed the synthetic cases on encode
  *time* almost exactly, and on *bytes* not at all — the synthetic photo-like frame (gradient plus
  noise) overstates a real photograph by **3.2×** (3.45 MB against 1.08 MB), while the synthetic
  page-like frame *understates* a real text page. The whole-frame dirty-tile scan costs **0.330 ms**
  against the synthetic 0.14 ms, because a static page is the scan's worst case: nothing
  short-circuits, so all 260 tiles are compared row by row.
- **The finding that changes 05-08: `encode_screenshot` was never running at
  `png::Compression::Default`.** `png 0.17.16`'s `Info::default()` sets `compression:
  Compression::Fast`, and `set_filter`'s own doc says the default filter is `Sub` — so the shipped
  encoder has been running at exactly the configuration D-05-05 chose for the *frame* path. Proven by
  encoding the same real frame both ways and getting **byte-identical output** (465,662 B; 1.685 ms
  as shipped against 1.847 ms explicitly configured), while an explicitly-set `Compression::Default`
  on the same buffer produced 183,030 B in **21.275 ms** — within 1.4 % of `05-RESEARCH.md`'s
  20.98 ms. The benchmark was run correctly; it measured a configuration this codebase never executes.
- **05-08 is handed an instruction rather than an absence.** The pump requests 30 ms unconditionally
  on every machine including under Xvfb; no environment check, no Xvfb-specific cadence, no
  `TALARIA_*` knob that lowers it. If a cadence ever cannot be sustained, 05-08 does not lower the
  *requested* rate — 05-10's ladder lowers the *delivered* one, and the suite asserts the requested
  cadence plus the ladder's response at a rung the measuring machine can sustain.
- **A measured keyframe threshold and a ladder ceiling.** Keyframe above **90 of 260 tiles** at
  1280×800, where the sum of per-tile encode overhead (~0.02 ms each) meets a whole-frame keyframe
  (1.5–1.9 ms) and the byte saving stops favouring tiles. 05-10's fastest rung stays **30 ms
  full-res**; the measured production floor beneath it is **≈13 ms** in the worst shape (scrolling).
- **The off-thread precondition is confirmed in this tree.** `assert_send::<servo::RgbaImage>()`
  compiled and ran, so the diff and the encode can be moved to a worker thread.

## The Xvfb figure and the hardware figure

**There is one number, and it is software-rendered.** Every X display on this machine is an Xvfb on
llvmpipe — `:20` and `:21` (Sunshine), `:95` (this spike), `:98` (CI). The spike printed
`GL_RENDERER = llvmpipe (LLVM 20.1.2, 256 bits)` on every run, preceded by
`libEGL warning: DRI3 error: Could not get DRI3 device`, and reported the same renderer when run on
`:21` as a control. Reaching either of the machine's two real GPUs would need a Wayland compositor
(none installed) or an Xorg holding DRM master, and `/etc/X11/Xwrapper.config` restricts that to
`allowed_users=console`. No second figure was fabricated.

**And the direction of the error matters more than its absence.** Software rendering does not scale
the pipeline down uniformly — it moves cost between the halves in opposite directions. `paint()` is
*pessimistic* under llvmpipe (5.1 ms mean while scrolling; a GPU does that in the compositor), while
`read_to_image` is very likely *optimistic*, because a software framebuffer already lives in system
memory and `glReadPixels` is close to a `memcpy`. **The Xvfb readback figure is therefore not a
conservative upper bound — it is plausibly a lower one**, which is precisely why the hardware
re-measure is recorded as a `VERIFICATION.md` manual item rather than waved off. The number that
could still break this phase's capture model is a hardware `read_to_image` *slower* than 1.5 ms, not
a `paint()` that is faster.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 — missing critical handoff] Two `deferred-items.md` entries added**

- **Found during:** Task 2
- **Issue:** The plan's `files_modified` named only `05-02-SPIKE.md`, but two findings are
  cross-plan corrections that a reader of `05-RESEARCH.md` or `05-CONTEXT.md` would otherwise never
  see: the `png::Compression::Default` premise is false for this tree, and no hardware-GL path exists
  on this machine. `deferred-items.md` is the phase's register and 05-01 opened it with exactly this
  kind of inherited-error correction.
- **Fix:** Appended two entries — "Correction: `encode_screenshot` was never running at
  `Compression::Default`" and "Manual item for `VERIFICATION.md`: no hardware GL number exists on
  this machine".
- **Files modified:** `.planning/phases/05-distributed-mode/deferred-items.md`
- **Commit:** 22e2e2b

**2. [Rule 3 — blocking] The spike deleted its own engine profile and crashed Servo's ResourceManager**

- **Found during:** Task 1 (smoke run)
- **Issue:** The example removed its temporary Servo `config_dir` at the end of `run()`, which races
  the still-running `ResourceManager` thread — it panicked writing `auth_cache.json` into a directory
  that had just been unlinked. Harmless to the measurement, but it made every run end in a panic
  trace that would have polluted the transcript the findings were read from.
- **Fix:** Left the profile on disk and reaped `${TMPDIR}/talaria-readback-spike-*` from the calling
  shell instead, which is also where the plan's "any page fixture the run wrote outside the
  repository" obligation is discharged.
- **Files modified:** the throwaway (since deleted)
- **Commit:** 24fd40c

**3. [Rule 1 — bug in the measurement, not the tree] The first codec run reported Fast and Default
as byte-identical, which looked like a broken benchmark**

- **Found during:** Task 2
- **Issue:** `Compression::Fast` + `Sub` and the shipped encoder produced identical byte counts to
  the byte, which is not a coincidence any encoder pair produces by accident. Reporting it without
  investigating would have shipped either a false "the encoders are the same" or a false "the
  benchmark is broken".
- **Fix:** Read `png 0.17.16`'s `Info::default()` and found `compression: Compression::Fast`, then
  added explicit `Compression::Default` and `Compression::Best` rows to the example so the divergence
  is demonstrated rather than argued. The explicit-`Default` row reproduces `05-RESEARCH.md`'s
  20.98 ms to within 1.4 %, which is what turns this from a suspicion into a finding.
- **Files modified:** the throwaway (since deleted)
- **Commit:** 22e2e2b

### Display choice

The e2e suite was run on `TALARIA_E2E_DISPLAY=:95`, not the `:98` that `e2e.yml` pins, per the
phase's own `deferred-items.md` entry: `harness.start_xvfb` `pkill`s stale Xvfb on its display and
`kill_shells_on_display` reaps shells there, neither under a lock, so two concurrent runs on one
display SIGKILL each other. A CI run *was* live on `:98` during this plan's first hour.

## Verification

| Check | Result |
|-------|--------|
| `cargo build --release --locked` | exit 0 |
| `cargo clippy --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --locked` | **296 passed**, 0 failed (291 + 3 + 2) |
| `python3 tests/e2e/run_all.py` (`:95`) | **22/22 PASS**, `failed: none` |
| `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'` | **1** |
| `test ! -f crates/talaria-shell/examples/readback_spike.rs` | exit 0 |
| `git status --porcelain crates/ tests/` | empty |
| `git diff --stat crates/talaria-shell/src/app.rs` | empty |
| `grep -cE '^A1: (CONFIRMED\|REFUTED)' 05-02-SPIKE.md` | 1 |
| `grep -cE '^A8: (CONFIRMED\|REFUTED)' 05-02-SPIKE.md` | 1 |

Baseline held exactly: 296 unit tests, e2e 22/22. No dependency changed, no lockfile line moved.

## What the next plans inherit

- **05-08** — the frame pump: a confirmed capture model, a 30 ms cadence to request unconditionally,
  a first-frame-after-attach outlier to keep out of the rate controller's EWMA (53–112 ms measured
  against a 0.19–5.1 ms steady state), a keyframe threshold of 90 of 260 tiles, and an instruction
  **not** to describe the frame encoder as a three-line divergence or to repeat the 21 ms / 175 ms
  justification in a comment.
- **05-10** — the rate controller: 30 ms full-res is the fastest rung it may claim, with a measured
  ≈13 ms production floor beneath it and ~2.3× headroom in the worst shape, none of it yet spent on
  the wire. Also the one row that moved against the plan: a photographic keyframe is 1.08 MB, or
  288 Mbit/s at 30 ms, which is inside a direct WireGuard path's measured 342–447 Mbit/s but not
  comfortably — so the ladder must be reachable from the interactive path, not only from a relay
  detection.
- **05-11** — `SECURITY.md` / `VERIFICATION.md`: the hardware readback re-measure is a manual item,
  and SC 2's evidence must come from the two-machine check rather than from the harness.

## Self-Check: PASSED

- `.planning/phases/05-distributed-mode/05-02-SPIKE.md` — FOUND
- `.planning/phases/05-distributed-mode/deferred-items.md` — FOUND
- `crates/talaria-shell/examples/readback_spike.rs` — correctly ABSENT
- commit `24fd40c` — FOUND
- commit `22e2e2b` — FOUND

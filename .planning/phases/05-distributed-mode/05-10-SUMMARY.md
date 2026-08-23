---
phase: 05-distributed-mode
plan: 10
subsystem: core
tags: [rate-control, hysteresis, ewma, cadence, degrade-ladder, tailscale, link-shim, e2e, servo]

# Dependency graph
requires:
  - phase: 05-distributed-mode
    provides: "05-02's measured readback distribution, its ≈13 ms production floor, its keyframe threshold and its written instruction to request 30 ms unconditionally — the numbers the ladder's fastest rung is bounded by rather than re-derived"
  - phase: 05-distributed-mode
    provides: "05-03's `FrameHeader`, its `scale_denominator` and its `last_applied_input` echo — the field the whole latency estimate is built on"
  - phase: 05-distributed-mode
    provides: "05-08's frame pump: the hold, the tick, the two cadences, the idle threshold and its boundary rule, and the encoder thread the reduction now runs on"
  - phase: 05-distributed-mode
    provides: "05-09's client: `Presenter::last_applied_input`, `Capture`'s monotonic sequence, the fit that already honours a denominator above one, and the recorded-control geometry hook the suite reads"
provides:
  - "`talaria_protocol::wire::RUNG_LADDER` and `Rung` — five rungs, fastest first, each carrying an interval in whole milliseconds and a scale denominator, with the ordering as a unit-tested invariant"
  - "`ClientView::Cadence { rung }` — a cadence request that names an enumerated rung instead of carrying a raw interval, with an unknown name decoding to `None` rather than ending the connection"
  - "`talaria_protocol::wire::is_driving`, `parse_view_idle_ms`, `DEFAULT_VIEW_IDLE_MS`, `VIEW_IDLE_ENV` — the passive/driven transition as one function both ends call"
  - "`view::reduce` — nearest-neighbour reduction with round-up destination dimensions, on the encoder thread"
  - "Server-side rung honouring: per-connection rung, the interval and denominator it implies, and a forced keyframe when a rung change resizes the surface"
  - "`crates/talaria-client/src/rate.rs` — the sampling, the smoothed estimate, the hysteretic ladder walk, the reported state and the direct-or-relayed path lookup"
  - "Client controls `link.full` / `link.degraded` / `link.relayed` / `link.measurement`, and the readings `reading.rung`, `reading.rung_changes`, `reading.input_to_photon_ms`"
  - "`tests/e2e/link_shim.py` — an unprivileged stdlib forwarder with delay, rate cap and kill, defaulting to this tailnet's measured relayed profile"
  - "`tests/e2e/remote_latency_test.py` — the cadence transition proved from frame timestamps in both directions and at the idle boundary, and the ladder's degrade-and-report under the shim"
  - "The measured fact that a script-animated page repaints about once a second on a background webview while a CSS-animated one repaints at the pump's own rate"
affects: [05-11, phase-05.1]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "A ladder's rungs live in one ordered table both ends read, and the ordering between them is a walked test rather than a convention — the shell's timeout-ordering discipline, applied to a second kind of ordered thing"
    - "Replace a number on the wire with an enumerated value when the receiver would otherwise have to decide whether to honour it: the decision is removed rather than bounded, and with it the clamp and the disagreement about its edges"
    - "An anti-oscillation margin is derived from the table it guards rather than tuned, and the derivation is a test that walks the table — so a rung inserted later that breaks it fails rather than flapping in the field"
    - "Split the smoothed reading from the decision input: the average is what a human reads, the raw samples are what the controller moves on, because an average carries one spike for a dozen samples"
    - "An interface control whose recorded *name* carries its state, so an end-to-end suite asserts which report is drawn without reading a pixel or matching copy that is free to be reworded"
    - "A forward-compatible enumerated wire field decodes an unknown name to `None` and is answered with the one reasonless refusal, rather than ending the connection — so adding a value later is not a breaking change"

key-files:
  created:
    - crates/talaria-client/src/rate.rs
    - tests/e2e/link_shim.py
    - tests/e2e/remote_latency_test.py
  modified:
    - crates/talaria-protocol/src/wire.rs
    - crates/talaria-shell/src/view.rs
    - crates/talaria-client/src/chrome.rs
    - crates/talaria-client/src/main.rs
    - tests/e2e/run_all.py
    - CHANGELOG.md

key-decisions:
  - "The passive rung is half resolution, not full: the ladder's floor is the rung that has to survive the worst link, and 05-RESEARCH measured a half-resolution passive frame at 138 KB / 5.5 Mbit/s against a full one's 522 KB, which is inside a relayed path's 13 Mbit/s where a full one is not"
  - "An attachment that names no rung starts at the fastest one, so every hand-composed viewer in the e2e suite gets exactly the behaviour 05-08 shipped — full resolution, 30 ms driving, 250 ms idle — and nothing about the frame pump's default moved"
  - "A rung names the *driven* interval and the denominator; the passive interval is always the ladder's slowest rung's, whichever rung the connection is on. That is what lets one table replace 05-08's two cadence constants without a second concept"
  - "The rung lives on the *session* rather than on the attachment, because `ClientView::Cadence` names no tab: what a client has learned is a fact about the link, and a second attachment inherits it rather than rediscovering it"
  - "The frame header carries the **reduced** dimensions plus the denominator, not the source dimensions: the client multiplies them back, which is two declared facts rather than a scale inferred from a ratio, and it is what 05-09's `SurfaceSize::page_width` already expects"
  - "The reduction rounds **up**. A floor would drop the rightmost column and the bottom row on any viewport the denominator does not divide, which reads as a rendering fault rather than as an arithmetic choice"
  - "The ladder moves on raw whole-millisecond samples and the EWMA is only what the human reads. An average driven by one spike steps down and then needs a dozen good frames to notice, which contradicts the stated behaviour that one overrun followed by a good sample moves nothing"
  - "The recovery margin is two fifths, derived rather than tuned: a sample good enough to step up must never be an overrun on the rung it steps up to, which bounds the margin by the smallest ratio between adjacent intervals (120/250 = 0.48). Seven tenths, the first draft, fails that on every doubling step"
  - "The client's driven cadence starts on a *sent* input where the server's starts on an *accepted* one — a one-transmission difference — but both call `talaria_protocol::wire::is_driving` with the same threshold from the same environment variable, so the two cannot come to disagree about the rule itself"
  - "The path lookup runs on its own thread and is never waited for, and its parsing is a pure function over one `tailscale status --json` document, so what it decides is assertable without a daemon and the window never stalls on a subprocess"
  - "The suite advertises the shell at the shim's origin rather than teaching the shim to rewrite `Host` headers: a byte forwarder that rewrote HTTP would stop being an honest model of a link"
  - "The e2e fixture animates in CSS rather than from script, because measurement showed a script-animated background webview repaints about once a second however often the pump asks"

patterns-established:
  - "Pattern: when a plan's stated constant turns out to violate the property it exists to provide, derive it from the structure it guards and make the derivation the test — the margin is now a consequence of the ladder rather than a number beside it"
  - "Pattern: measure the harness before building on it. Three ways of animating a page were compared over four seconds each before one was chosen, and the table is in the suite's own source so the next timing suite does not rediscover it"
  - "Pattern: a timing assertion states its tolerance and why that tolerance is defensible *on this hardware*, beside the assertion. An assertion whose tolerance is invisible is one that gets loosened silently the first time it flakes"
  - "Pattern: assert a transition as a ratio rather than as a wall-clock number when the measuring machine cannot be held to the number — the ratio survives a slow rasteriser scaling both sides"

requirements-completed: [DIST-02]

coverage:
  - id: D1
    description: "Every rung is named in one ordered table both ends read, and the ordering is a test rather than a convention"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "cargo test -p talaria-protocol wire::tests::the_rung_ladders_intervals_and_denominators_are_both_non_decreasing, ::every_rung_resolves_to_its_own_row_and_back, ::stepping_off_either_end_of_the_ladder_stays_on_it, ::one_step_down_and_back_up_returns_to_where_it_started"
        status: pass
      - kind: other
        ref: "grep -ci 'rung' crates/talaria-protocol/src/wire.rs is 120; grep -ci 'non-decreasing|ordering' is 4"
        status: pass
    human_judgment: false
  - id: D2
    description: "A cadence request names a rung rather than an interval, so a client cannot ask for a number the server then has to decide whether to honour (T-05-12-D)"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "wire::tests::a_cadence_request_names_a_rung_and_an_unknown_name_decodes_to_nothing, ::a_cadence_request_carrying_a_raw_interval_is_no_longer_this_wires_vocabulary; view::tests::a_cadence_request_naming_a_known_rung_takes_effect_on_the_next_tick, ::a_cadence_request_naming_an_unrecognised_rung_is_refused_with_no_reason"
        status: pass
    human_judgment: false
  - id: D3
    description: "Cadence arithmetic is whole milliseconds; the smoothed estimate is the only floating-point value and is only ever compared against an integer interval"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "wire::tests::every_rungs_interval_is_a_whole_number_of_milliseconds (the u32 binding is the assertion — a float field stops it compiling)"
        status: pass
      - kind: other
        ref: "grep -c 'f64' crates/talaria-client/src/rate.rs is 6, every one of them the EWMA; is_overrun and is_comfortable both take u32 and u64"
        status: pass
    human_judgment: false
  - id: D4
    description: "A reduced-resolution frame declares its own denominator and reduced dimensions, and the reduction rounds up so no pixel column or row is dropped (T-05-11-E)"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "view::tests::a_denominator_of_one_produces_the_source_dimensions_unchanged, ::a_denominator_above_one_halves_a_divisible_surface_exactly, ::a_source_dimension_the_denominator_does_not_divide_loses_no_column_or_row (201x137), ::a_reduced_frame_declares_its_denominator_and_its_reduced_dimensions"
        status: pass
    human_judgment: false
  - id: D5
    description: "The passive-to-driven boundary is one comparison both ends call, tested at the threshold and one millisecond either side"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "wire::tests::an_attachment_returns_to_passive_exactly_at_the_idle_threshold; view::tests::the_cadence_falls_back_to_passive_exactly_at_the_idle_threshold; rate::tests::the_driven_cadence_lapses_exactly_at_the_idle_threshold"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_latency_test.py — 'BOUNDARY': an input at 600 ms held the driven cadence at a 31 ms median; letting 800 ms elapse returned it to 252 ms"
        status: pass
    human_judgment: false
  - id: D6
    description: "A sample exactly equal to the rung's interval is not an overrun and one millisecond above it is — stated and tested at the edge and one unit either side"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "rate::tests::a_sample_exactly_equal_to_the_interval_is_not_an_overrun, ::a_sample_one_millisecond_above_the_interval_is_an_overrun, ::at_a_sixty_millisecond_rung_sixty_is_fine_and_sixty_one_is_an_overrun"
        status: pass
    human_judgment: false
  - id: D7
    description: "The ladder is hysteretic and cannot oscillate: asymmetric counts, a derived recovery margin, and the margin's own invariant walked over the whole table (T-05-12-E)"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "rate::tests::a_sample_good_enough_to_step_up_is_never_an_overrun_on_the_rung_it_reaches, ::a_long_run_of_samples_exactly_at_the_edge_moves_the_rung_in_neither_direction, ::one_overrun_followed_by_a_good_sample_moves_nothing, ::the_rung_steps_down_after_the_configured_run_of_overruns_and_not_before, ::the_rung_steps_up_after_the_configured_run_of_comfortable_samples_and_not_before, ::a_sample_under_the_interval_but_over_the_recovery_margin_does_not_step_up"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_latency_test.py — 'STEADY': over twelve seconds of driving on the shim the rung moved 0 more times"
        status: pass
    human_judgment: false
  - id: D8
    description: "The round-trip sample is an input-to-photon measurement needing no synchronised clock, taken once per sequence and never from an echo the client did not produce"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "rate::tests::a_frame_echoing_a_sequence_this_client_sent_yields_one_sample, ::a_frame_echoing_a_sequence_this_client_never_sent_yields_nothing, ::a_sequence_is_sampled_once_and_never_twice, ::an_echo_retires_every_earlier_sequence_with_it, ::the_first_sample_of_an_attachment_is_kept_out_of_the_estimate, ::the_estimate_moves_toward_a_sample_and_never_jumps_to_it"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_latency_test.py — 'DEGRADED' reports an input-to-photon estimate of 113 ms read off the client's own reading channel"
        status: pass
    human_judgment: false
  - id: D9
    description: "The frame cadence rises on takeover and falls back after, proved from frame timestamps in both directions"
    requirement: "DIST-02"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/remote_latency_test.py — 'PASSIVE' 251 ms, 'TAKEOVER' 31 ms, 'RELEASED' 251 ms"
        status: pass
    human_judgment: false
  - id: D10
    description: "Under a reproduction of this tailnet's measured relayed profile the client steps down the ladder, reports it in terms a human can act on, and still lets them click (D-05-06, T-05-06-A, T-05-06-B)"
    requirement: "DIST-02"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/remote_latency_test.py — 'DEGRADED' (walked to rung 2, drew link.degraded, no link.full) and 'STILL REACHED' (a real click through the shim navigated the agent's tab, asserted over the control socket)"
        status: pass
      - kind: other
        ref: "grep -ci 'refuse|reject|disable' crates/talaria-client/src/rate.rs is 0 — the controller has no path that stops the human driving"
        status: pass
    human_judgment: false
  - id: D11
    description: "The report names the path type when the platform can be asked, and degrades to what it knows when it cannot"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "rate::tests::a_peer_with_a_current_address_is_on_a_direct_path, ::a_peer_with_a_relay_and_no_current_address_is_on_a_relayed_path, ::anything_that_is_not_a_peer_of_this_tailnet_is_simply_unknown, ::a_degraded_report_on_a_relayed_path_names_the_relay_and_says_driving_still_works, ::a_degraded_report_still_renders_when_the_path_could_not_be_looked_up"
        status: pass
    human_judgment: false
  - id: D12
    description: "Local mode is provably unaffected (SC 3)"
    requirement: "DIST-02"
    verification:
      - kind: e2e
        ref: "takeover_test.py and panel_click_test.py both PASS inside run_all.py; git diff --stat on both is empty; git status --porcelain on crates/talaria-shell/src/app.rs and gui.rs is empty"
        status: pass
    human_judgment: false
  - id: D13
    description: "The whole tree is green: 536 unit tests and e2e 24/24"
    verification:
      - kind: unit
        ref: "cargo test --locked — 536 passed, 0 failed (409 shell + 90 client + 35 protocol + 2 mcp)"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/run_all.py on :94 — 24/24 PASS, failed: none, exit 0"
        status: pass
    human_judgment: false
  - id: D14
    description: "Success Criterion 2's ~30-60 ms takeover target on a real direct WireGuard path between two machines"
    verification: []
    human_judgment: true
    rationale: "This suite deliberately does not claim it, and says so in its own docstring. Loopback hides transmission, which is the only variable the target turns on, and every X display on this machine is an Xvfb on llvmpipe. What is asserted here is the cadence transition, the ladder's response and the reporting — all real properties of the product. The target itself is the two-machine manual check 05-VALIDATION.md lists and 05-11 turns into a script: ThinkPad server, MacBook Air client, over the real tailnet, with `tailscale status` recorded to say whether the path was direct or relayed, and the client's own input-to-photon figure written down."

# Metrics
duration: 70min
completed: 2026-08-23
status: complete
---

# Phase 5 Plan 10: The Rung Ladder, the Rate Controller and the Latency Suite Summary

**The frame cadence now rises on takeover and falls back after — proved from
frame timestamps at 251 ms passive, 31 ms driven and 251 ms again, in both
directions and at the idle boundary — and on a reproduction of this tailnet's
own measured relayed profile the client walks down a named ladder, says so in
terms a human can act on, and still lands a click, because `D-05-06` says
degrade and report and never withhold control.**

## Performance

- **Duration:** 70 min
- **Tasks:** 3 of 3, each committed on its own
- **Files created:** 3 (`rate.rs`, `link_shim.py`, `remote_latency_test.py`); modified: 6
- **Unit tests:** 505 → **536** (+50 over the 486 baseline 05-09 left; 409 shell, 90 client, 35 protocol, 2 mcp)
- **End-to-end:** **24/24 PASS**, `failed: none`, exit 0 — the target this phase set entering it

## The ladder, exactly

| Rung | Interval | Scale | What it is for |
|------|----------|-------|----------------|
| `full_resolution_fast` | 30 ms | 1 | The target. A direct WireGuard path carries it. |
| `full_resolution_steady` | 60 ms | 1 | Detail kept, rate halved — the first thing to give up, because text stays legible. |
| `half_resolution_steady` | 60 ms | 2 | Rate restored, about a quarter of the bytes. Aiming a click still works. |
| `half_resolution_slow` | 120 ms | 2 | Both halved. Driving is deliberate rather than fluent, and it still lands. |
| `passive_only` | 250 ms | 2 | The floor. Driving gets the same cadence as watching, which is what a relayed link can carry. |

**The number that bounded the top rung is 05-02's, not the requirement's.** The
spike drove this tree's real engine at a 30 ms cadence over 900 ticks in three
page shapes and recorded a **≈13 ms production floor** in the worst shape, so
30 ms is a rung the server can actually produce with roughly 2.3× headroom
rather than one it starts beneath and never notices. The test asserting it names
that figure.

**The floor is half resolution, and that is a decision rather than a
consequence.** `05-RESEARCH.md` measured a passive half-resolution frame at
138 KB / 5.5 Mbit/s against a full one's 522 KB / 139 Mbit/s. The bottom of a
ladder is the rung that has to survive the worst link, and 13 Mbit/s carries the
first and not the second. It also makes the ordering invariant hold in both
columns, which is what the walked test requires.

**A rung names the driven interval and the denominator; the passive interval is
always the slowest rung's, whichever rung the connection is on.** That is the
one idea that let a single table replace 05-08's two cadence constants without
introducing a second concept — and it is why an attachment that never sends a
cadence request behaves exactly as the frame pump shipped: it starts at
`full_resolution_fast`, so full resolution, 30 ms driving, 250 ms idle. Every
hand-composed viewer in `remote_view_test.py` is one of those, and that suite is
unchanged and green.

## The boundary rules, as implemented at both ends

**Passive to driven.** One function,
`talaria_protocol::wire::is_driving(since_last_input, idle)`, in the shared
vocabulary, called by the server's `tick_interval` and by the client's
`RateController::driving`. An attachment enters the driven cadence on the first
input and returns to passive once the elapsed time since the last input
**reaches** the threshold; an input arriving exactly at the threshold re-enters,
because a new input always does. One comparison on elapsed-since-last, no
countdown, no race between two clocks. `TALARIA_VIEW_IDLE_MS` is read by both
ends through the same parser, so a suite that lowers it lowers it once and
cannot lower it on one side only.

The plan asked for a comment saying the two ends run one rule. **They run one
function**, which is the stronger form and was cheap.

**Overrun.** A sample **strictly greater** than the current rung's interval. At
a sixty millisecond rung, sixty is fine and sixty-one is an overrun — tested at
the edge and one millisecond either side, in `rate.rs`, as its own named test.

**Movement.** Three consecutive overruns step down; twelve consecutive samples
under **two fifths** of the interval step up. A sample that is neither breaks
**both** runs, which is what leaves a link parked exactly on a rung's edge where
it is.

### The recovery margin changed while this was being written, and the reason is the point

The plan and the first draft both used **seven tenths**. Writing the end-to-end
degrade section made it obvious that seven tenths is wrong, and wrong in exactly
the way the margin exists to prevent.

Most steps of this ladder **double** the interval. A sample at 1.2× the fast
rung's interval is an overrun there (so it steps down) and is comfortably under
0.7× of the slow rung's interval (so it steps back up) — and then overruns
again. That is a ladder flapping between two rungs on a link whose rate never
changed, and every one of those moves forces a keyframe.

The property that actually has to hold is: **a sample good enough to step up
from one rung must never be an overrun on the rung it steps up to.** That bounds
the margin by the smallest ratio between adjacent intervals, which on this
ladder is 120/250 ≈ 0.48. **Two fifths** clears every step with room, and the
number is now derived from the table rather than sitting beside it —
`a_sample_good_enough_to_step_up_is_never_an_overrun_on_the_rung_it_reaches`
walks the whole ladder asserting it directly, so a rung inserted later that broke
it fails a test rather than oscillating in somebody's session.

## The estimate and the movement are two readings, deliberately

The exponentially weighted average is the **only** floating-point value in the
controller, and it is what the human is shown. The **ladder moves on the raw
whole-millisecond samples**.

That split is not an oversight and it is a divergence worth stating: an average
carries one slow frame for many samples afterwards, so a ladder driven by the
average would step down on a single spike and then need a dozen good frames to
climb back — which directly contradicts the plan's own stated behaviour that "a
single overrun followed by a good sample does not step down". Driven by the
samples, that behaviour is exactly what it looks like, and it has its own test.

The first sample after an attach is discarded entirely rather than smoothed:
05-02 measured it at 53–112 ms against a 0.19–5.1 ms steady state, and it is a
measurement of the attach rather than of the link. 05-08 handed this forward in
as many words.

## The report

Written in `04-UI-SPEC.md`'s register — the fact first, the next step second, no
exclamation, no reassurance — as a table in `rate.rs`'s own module header so it
can be reviewed as copy:

| State | Fact | Next step |
|-------|------|-----------|
| at the fastest rung | `Takeover is running at full speed: {rung}.` | — |
| stepped down, relayed | `Your connection to this server is relayed rather than direct, so frames are arriving at {rung}.` | `Clicks and keystrokes all still arrive — takeover works, it just will not feel immediate. A direct path between the two machines is what makes it feel immediate.` |
| stepped down, direct | `This link cannot carry full-speed frames, so they are arriving at {rung}.` | `Tailscale reports a direct path, so the limit is one of the two machines rather than the route between them. Clicks and keystrokes all still arrive.` |
| stepped down, unknown | as above | `Clicks and keystrokes all still arrive — takeover works, it just will not feel immediate.` |
| once measured | `Input to picture: about {estimate} ms.` | — |

`{rung}` is derived from the ladder's own numbers — "reduced detail, about 8
frames a second" — so a rung added later describes itself, and it is a rate
rather than an interval because "about eight frames a second" is a thing people
have an intuition for.

**The measured figure is surfaced, not just computed.** It is drawn as
`link.measurement` and recorded as `reading.input_to_photon_ms`, which is the
number 05-11's two-machine script asks a human to write down. The end-to-end run
recorded 113 ms through the shim.

**The path lookup degrades rather than disappearing.** `tailscale status --json`
runs on its own named thread and is never waited for; the parsing is a pure
function over the document, so what it decides is unit-tested without a daemon.
A missing binary, a non-zero exit, output that is not what it expects and a peer
this host does not name all land on `Unknown`, and the report then drops the
path clause and keeps everything else.

**`grep -ci 'refuse\|reject\|disable' crates/talaria-client/src/rate.rs` is 0.**
There is no state in the controller that stops input and no branch that could
grow into one.

## The reduction

Nearest neighbour, on the encoder thread, with the destination dimensions
**rounded up**. A floor would drop the rightmost column and the bottom row on
any viewport the denominator does not divide — which is most of them — and that
reads as a rendering fault rather than as an arithmetic choice. Tested at a
deliberately awkward 201×137, where the source's very last pixel is asserted to
survive.

The reduction runs **before** the tile comparison, so the diff, the keyframe
decision, the crop and the header all speak the coordinate space the client's
texture is actually in. The previous-frame buffer therefore holds the reduced
surface, which means a denominator change is independently a size-change
keyframe trigger on top of the one a rung change forces.

**The header carries the reduced dimensions plus the denominator, not the source
dimensions.** The plan's `<interfaces>` says "keep its source dimensions as the
source dimensions"; taken literally that would break 05-09, whose
`SurfaceSize::page_width` multiplies `frame_width` by the denominator to recover
the page's size, and would also break the wire's own tile-fits-inside-its-frame
refusal, since tile coordinates are in reduced space. Two declared facts the
client multiplies back is the reconstruction the sentence is asking for; a scale
inferred from a ratio of two sizes is what it is warning against, and that is
what is avoided. Commented at the assembly site.

One consequence worth recording: with round-up, `reduced × denominator` can
exceed the source by up to `denominator − 1`, so on an odd-width viewport the
extreme right pixel column maps to a page coordinate one past the viewport and
the server's own out-of-viewport refusal turns it away. A one-pixel strip, on
odd viewports only, refused rather than moved to the nearest valid position —
which is the discipline `input.rs` already keeps.

## The link shim

Standard library only, unprivileged, in the terse register the Python suites
use. Three knobs: an added one-way delay, a token-bucket rate cap, and a kill
that closes both halves of every live pair without closing the listener. The
default is this tailnet's own measured relayed profile — **13 Mbit/s and 40 ms**
— attributed in the docstring to `05-CONTEXT.md`'s measured facts, because a
magic constant that turns out to be somebody's real measurement is a different
kind of number.

Two threads per direction rather than one, and the comment says why: a single
thread that slept between a read and the matching write would cap throughput at
one chunk per delay, which is a *bandwidth* limit wearing a latency limit's
clothes and would make every measurement taken through it meaningless. Smoke-
tested directly: 200 KB through the default profile took 203 ms against a
predicted 123 ms of transmission plus 80 ms of round-trip delay.

**Its honest limits are in the docstring**: bandwidth and latency and nothing
else — no jitter, no loss, no reordering, no congestion response, none of the
path changes a real overlay network performs mid-session, and nothing about MTU
because it is a byte-stream shim. It is a regression harness for degrade
behaviour, not a network simulator.

**The kill knob is unused by this plan and is here anyway.** Phase 5.1's
reconnect work needs exactly it, and the docstring says so.

## What the suite proves, and what it refuses to claim

Proved, from the recorded run:

| Assertion | Measured |
|-----------|----------|
| `PASSIVE` — nobody driving | 15 frames, median **251 ms** |
| `TAKEOVER` — a human driving | 100 frames, median **31 ms** |
| `RELEASED` — they stopped | 16 frames, median **251 ms** |
| `BOUNDARY` — input at 600 ms of an 800 ms threshold | still driven, median **31 ms**; letting 800 ms elapse returned **252 ms** |
| `SHIMMED` — real client through 13 Mbit/s + 40 ms | attached, starting at rung **0** |
| `DEGRADED` | walked to rung **2**, drew `link.degraded`, no `link.full`, estimate **113 ms** |
| `STILL REACHED` | a real click navigated the agent's tab, asserted over the control socket |
| `STEADY` | **0** further rung moves over twelve seconds of driving |

**Refused, in the suite's own docstring:** it does not assert Success Criterion
2's ~30–60 ms target is met. That is a claim about a direct WireGuard path
between two machines, and everything here is one machine over loopback under
llvmpipe — which hides transmission, the only variable the target turns on.
Every timing assertion is about a **transition** or a **response**, never about
a wall-clock target, and carries its tolerance and the reason that tolerance is
defensible on this hardware beside it. The driven assertion in particular is a
**ratio** (at most half the passive median) plus a loose 150 ms ceiling, and the
comment says why it is not 30: asserting 30 would be a claim about this
machine's software rasteriser wearing a product claim's clothes.

### The tolerances, and why each is defensible

- **Passive, 150–600 ms** against DIST-02's 200–500 band. Widened 50 ms at the
  bottom and 100 ms at the top, both for one reason: the pump reschedules from
  `now` rather than from the deadline it missed, so a late tick pushes the
  *following* gap out rather than compressing it, and llvmpipe produces late
  ticks. Nothing in that band is reachable by a driven attachment, which is what
  the measurement is distinguishing it from. Measured 251.
- **Driven, at most half the passive median.** A ratio, so it survives a slow
  rasteriser scaling both sides — and the transition is a ratio, not a number.
  Measured 31 against 251.
- **Driven, at most 150 ms absolute.** A ceiling that exists only so a passive
  cadence which happened to be measured fast cannot satisfy the ratio. Five
  times the fastest rung, deliberately not 30, because 05-02 measured `paint()`
  alone at 5.1 ms mean and 7.8 ms p95 under llvmpipe while scrolling and this
  suite is animating a page and reading a socket at the same time.
- **The boundary windows are sized against the threshold** (three quarters of it
  before the second input, the whole of it plus a margin after) rather than
  against a clock, so this measures the rule and not how promptly the loop
  wakes. The exact-boundary case, where a sample lands on the threshold to the
  millisecond, is not something a wall clock can produce and is pinned by the
  unit test on `is_driving` instead.
- **The degrade budget is sixty seconds with no assertion about how fast it gets
  there.** What is asserted is the *response*; how many samples it takes is the
  step-down count, which is a unit test's business.
- **At most one further rung move over twelve steady seconds.** Not zero: the
  shim's rate is steady but llvmpipe's is not, and one genuine step in twelve
  seconds is the ladder working. Two or more would be flapping. Measured 0.

## Deviations from Plan

### Auto-fixed issues

**1. [Rule 1 — Bug] The recovery margin the plan specified permits the
oscillation it exists to prevent**

- **Found during:** Task 3, reasoning about what the shim's profile would do to
  the ladder before running it.
- **Issue:** the plan says "under a margin fraction of it"; the implementation's
  first draft, and the natural reading, was seven tenths. Most steps of this
  ladder double the interval, so a sample at 1.2× a rung's interval is an
  overrun there **and** comfortably under 0.7× of the next rung down's interval
  — it steps down, steps back up, overruns, and repeats forever on a link whose
  rate never changed. Each of those moves forces a keyframe. That is precisely
  T-05-12-E, shipped rather than mitigated, and the end-to-end run would very
  likely have sat in it, because 80–130 ms is exactly the band the shim
  produces.
- **Fix:** derived the margin from the table it guards instead of choosing it.
  The property is "a sample good enough to step up must not be an overrun on the
  rung it steps up to", which bounds the margin by the smallest adjacent-interval
  ratio (120/250 ≈ 0.48). Two fifths, with
  `a_sample_good_enough_to_step_up_is_never_an_overrun_on_the_rung_it_reaches`
  walking the whole ladder asserting the property rather than the number.
- **Files modified:** `crates/talaria-client/src/rate.rs`
- **Verification:** the new test, plus `STEADY: 0 more moves` on the real link.
- **Commit:** `e7f6174`

**2. [Rule 2 — CLAUDE.md directive] `CHANGELOG.md` entry**

- **Found during:** Task 3.
- **Issue:** the project's `CLAUDE.md` requires every change to be logged in
  `CHANGELOG.md`; the plan's `files_modified` did not name it. 05-08 and 05-09
  both landed the same obligation for their own work, and `05-VALIDATION.md`
  additionally carries constraint C-9 —
  `grep -c 'DIST-0[12]' CHANGELOG.md` at least 2, which stood at **0**.
- **Fix:** five `### Added` entries above 05-09's, covering the ladder, the
  controller, the report, the reduction and the two new test files, with the two
  headline entries tagged `(DIST-02)` and the suite entry naming the DIST-01
  client it drives. The grep now returns 2.
- **Commit:** `e7f6174`

### Investigated and deliberately reverted

**3. A `spin_event_loop` in the shell's `new_events`, tried and dropped**

The first attempt at the cadence measurement produced 3 frames in 4 seconds
where 16 were expected. The obvious suspect was the shell's loop: every winit
handler ends by spinning the engine **except** `new_events`, which is the one
reached by a wake from the pump's own `WaitUntil` deadline — so a viewer watching
a page nobody is locally touching looked like it would be reading a framebuffer
frozen since the last window or socket event. That matches 05-09's recorded
observation exactly ("one `evaluate` was enough to restart the flow").

It was implemented, built, and **measured against a control** rather than
assumed: with the spin, 16 passive / 137 driven frames per four seconds; without
it, 17 / 136. **The change fixed nothing**, so it was reverted rather than
shipped. `git diff --stat crates/talaria-shell/src/app.rs` is empty and the
local render path is untouched, which is also what SC 3 asks for.

The real cause is recorded below and in the suite's own source.

### Deliberate divergences from the plan text

**4. The margin is two fifths, not seven tenths** — see deviation 1. The plan
names no specific fraction; "a margin fraction" is satisfied, and the fraction
is now derived rather than picked.

**5. The frame header carries the reduced dimensions, not the source
dimensions**

The plan's `<interfaces>` says "keep its source dimensions as the **source**
dimensions". Taken literally that breaks two things that already exist:
05-09's `SurfaceSize::page_width` recovers the page's size by multiplying
`frame_width` by the denominator, and `FrameHeader::from_bytes` refuses a tile
that does not fit inside the frame it declares, with tile coordinates that are
necessarily in reduced space. What the sentence is *for* — "a client
reconstructs the placement exactly rather than inferring a scale from a size
ratio" — is delivered exactly: two declared facts, multiplied. Commented at the
assembly site so the choice reads as a choice.

**6. The ladder moves on raw samples, not on the smoothed estimate**

The plan says the estimate "is only ever compared against an integer interval",
which reads as the estimate driving the comparison. Implementing it that way
contradicts the plan's own behaviour list two lines later — "a single overrun
followed by a good sample does not step down" — because an average carries a
spike for many samples. The estimate is compared against the integer interval
where it is *reported*; the ladder moves on the samples. Both properties hold,
the arithmetic is integer throughout, and the split is stated in the module
header.

**7. The idle rule is a shared function, not a comment**

The plan asks for "a comment saying the client's controller uses the same
comparison and that the two are one rule". `is_driving` moved into
`talaria-protocol`'s wire module instead and both ends call it, along with the
threshold's parser and its environment variable name. A shared function cannot
drift; a comment saying two things agree can.

## Issues Encountered

- **A script-animated page repaints about once a second on a background
  webview, however often the pump asks.** This is the finding that cost the most
  time and is the one most worth carrying forward. Three ways of changing one
  small region were measured over four seconds each, with a viewer attached to a
  background agent tab:

  | How the page changes | Passive | Driven |
  |----------------------|---------|--------|
  | `setInterval` setting a style every 10 ms | 5 frames | 4 frames |
  | `requestAnimationFrame` | 16 frames | 30 frames |
  | a CSS `@keyframes` animation | 16 frames | **137 frames** |

  Only the last is the cadence — 16 in four seconds is 250 ms and 137 is 29 ms,
  which are the two rungs exactly. The engine batches a script's style mutations
  against its own refresh driver and throttles that driver for a webview nobody
  is looking at; `requestAnimationFrame` is throttled the same way. A
  declarative animation is driven by the compositor and is not. **This is a fact
  about the engine and not about the pump**, and it is written into the suite's
  own source, with the table, so the next person writing a timing suite does not
  rediscover it the slow way. It also explains 05-09's "the pump ticks when the
  loop wakes" note more precisely than that note did.

- **The shim changes the port, and the port is part of the identity the browser
  checks.** A client arriving through the shim addresses the server as
  `127.0.0.1:{shim_port}`, which the DNS-rebinding protector correctly turns away
  because it is neither the bound address nor an advertised one. Two ways out:
  teach the shim to rewrite the `Host` header of the first request, or advertise
  the shell at the shim's origin. The second was taken. A byte forwarder that
  rewrote HTTP would stop being an honest model of a link, and `advertised_url`
  with the loopback exception already exists for exactly this shape. The bound
  address stays in the allowlist either way, which is what lets the direct
  sections of the same suite connect to it.

- **`WebSocket.next_message` decodes everything one `recv` delivered before
  answering with the first of them**, so a timestamp taken at the pop is the
  time the caller got round to it rather than the time the bytes arrived. The
  suite reaches for the private `_pump` and stamps there, and says why at the
  function: it is the only seam where "the bytes just arrived" is a fact rather
  than an inference.

## Verification

| Check | Result |
|-------|--------|
| `cargo build --release --locked` | exit 0, no warnings |
| `cargo clippy --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --locked` | **536 passed**, 0 failed (409 + 90 + 35 + 2) |
| `python3 tests/e2e/remote_latency_test.py` | exit 0, `REMOTE LATENCY CHECKS PASSED` |
| `python3 tests/e2e/remote_view_test.py` | exit 0, `REMOTE VIEW CHECKS PASSED` |
| `python3 tests/e2e/run_all.py` (`:94`) | **24/24 PASS**, `failed: none`, exit 0 |
| `takeover_test` / `panel_click_test`, unmodified | both PASS; `git diff --stat` on both is empty |
| `git diff Cargo.toml Cargo.lock` | empty |
| `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'` | **1** |
| `git diff --stat crates/talaria-shell/src/app.rs` / `gui.rs` | empty / empty |
| `grep -ci 'rung' crates/talaria-protocol/src/wire.rs` | **120** |
| `grep -ci 'non-decreasing\|ordering' …/wire.rs` | **4** |
| `grep -c '05-02-SPIKE' …/wire.rs` | **3** |
| `grep -ci 'round\|ceil' crates/talaria-shell/src/view.rs` | **13** |
| `grep -q 'unwrap()' …/wire.rs …/view.rs` | no match |
| `grep -c 'D-05-06' crates/talaria-client/src/rate.rs` | **1** |
| `grep -ci 'hysteresis\|oscillat' …/rate.rs` | **7** |
| `grep -ci 'refuse\|reject\|disable' …/rate.rs` | **0** |
| `grep -c 'f64' …/rate.rs` | **6**, every one the EWMA |
| `grep -c 'unwrap()' …/rate.rs` | **0** |
| `grep -c '#[test]' …/rate.rs` | **31** |
| `grep -ci 'relayed\|direct' crates/talaria-client/src/chrome.rs` | **6** |
| `grep -rEci 'danger\|no_verify\|noverify\|accept_invalid\|insecure_skip' crates/talaria-client/src/` | **0** in every file |
| `grep -ci 'Mbit\|13_000_000\|13000000' tests/e2e/link_shim.py` | **5** |
| `grep -ci 'kill' tests/e2e/link_shim.py` | **4** |
| `grep -ci 'jitter\|loss\|not a network simulator' tests/e2e/link_shim.py` | **2** |
| `grep -ci 'does not\|refuses to claim\|not assert' tests/e2e/remote_latency_test.py` | **3** |
| `grep -ci 'tolerance' tests/e2e/remote_latency_test.py` | **7** |
| `grep -ci 'still reached\|still navig\|degrade' tests/e2e/remote_latency_test.py` | **17** |
| `grep -c 'remote_latency_test' tests/e2e/run_all.py` | **3** |
| `grep -c 'DIST-0[12]' CHANGELOG.md` | **2** (was 0) |

Display: `TALARIA_E2E_DISPLAY=:94`, with `XDG_RUNTIME_DIR=/tmp/tal-e2e-rt6`.
Not `:98`, which belongs to this machine's self-hosted CI runner, and not `:95`,
which earlier plans in this phase used and may have left a stale Xvfb on. One
display activity at a time; the diagnostic probes ran on `:93` and none was
started while the full suite was in flight.

## Commits

| Task | Commit | What |
|------|--------|------|
| 1 | `60e54d6` | The rung ladder and its ordering invariant, the cadence request naming a rung, `is_driving` and the idle parser in the shared vocabulary, the server honouring a rung, and the round-up reduction on the encoder thread |
| 2 | `3cbade3` | `rate.rs` — the sampling, the estimate, the hysteretic walk, the report and the path lookup; the client's cadence request, its idle deadline in the loop's wait, and the interface's link surface and three readings |
| 3 | `e7f6174` | `link_shim.py`, `remote_latency_test.py`, its registration, the recovery-margin correction and its invariant test, and the `CHANGELOG.md` entries |

## What the next plans inherit

- **05-11 — `VERIFICATION.md` and the two-machine script.** Success Criterion 2's
  target is now the *only* part of DIST-02 left unclosed, and it is unclosed
  deliberately (D14). The script should record three things the client already
  computes and displays: the rung it settled on (`reading.rung`), how many times
  the ladder moved (`reading.rung_changes`), and the input-to-photon figure
  (`reading.input_to_photon_ms`, drawn as `link.measurement`). Also worth
  recording beside it: `tailscale status`'s own verdict, which the client will
  already have put on screen as direct or relayed. Two manual items are
  unchanged from 05-08 — the hardware readback re-measure, and remote keyboard
  while the local human types into a different tab.
- **Phase 5.1 — reconnect and resync.** `LinkShim.kill()` exists and is unused;
  it closes both halves of every live pair while leaving the listener up, which
  is "the link went away" rather than "the far end went away". The rate
  controller already keeps what it learned across an attachment ending and
  clears only what was outstanding, which is the behaviour a resync wants.
- **Anyone writing a timing suite against this engine** — read
  `remote_latency_test.py`'s fixture comment before choosing how to make a page
  change. The measured table is there.

## User Setup Required

None. No new dependency, no new configuration file, and no new environment
variable — `TALARIA_VIEW_IDLE_MS` already existed and is now read by both ends
instead of one.

## Self-Check: PASSED

- `.planning/phases/05-distributed-mode/05-10-SUMMARY.md` — FOUND
- `crates/talaria-client/src/rate.rs` — FOUND
- `tests/e2e/link_shim.py` — FOUND
- `tests/e2e/remote_latency_test.py` — FOUND
- `crates/talaria-protocol/src/wire.rs`, `crates/talaria-shell/src/view.rs`,
  `crates/talaria-client/src/chrome.rs`, `crates/talaria-client/src/main.rs`,
  `tests/e2e/run_all.py`, `CHANGELOG.md` — FOUND
- commit `60e54d6` (Task 1) — FOUND
- commit `3cbade3` (Task 2) — FOUND
- commit `e7f6174` (Task 3) — FOUND

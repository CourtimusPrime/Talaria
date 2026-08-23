---
phase: 05-distributed-mode
plan: 08
subsystem: core
tags: [frame-pump, tile-diff, png, encoder-thread, visibility-hold, servo, winit, e2e]

# Dependency graph
requires:
  - phase: 05-distributed-mode
    provides: "05-02's measured readback distribution, its keyframe threshold, and its written instruction to request 30 ms unconditionally — this plan was executed against that instruction rather than re-deriving one"
  - phase: 05-distributed-mode
    provides: "05-03's `FrameHeader`, its fixed 51-byte layout, `FRAME_HEADER_LEN` and the six refusals its decoder makes, which this plan's assembly is asserted against rather than beside"
  - phase: 05-distributed-mode
    provides: "05-05's `/view` route, `ViewSessions`, the attachment table, the concurrent-attachment cap and `TabManager::agent_tab`"
  - phase: 05-distributed-mode
    provides: "05-06's `last_applied_input` recorded per connection — the value this plan reads at paint time and stamps — and its finding that a hidden webview answers no hit test, which is the blocker this plan closes"
  - phase: 01-servo-shell
    provides: "`capture_now`'s paint/size/rect/read sequence, the per-tab `OffscreenRenderingContext`, `sync_visibility`'s show/hide invariant and `next_capture_deadline`'s wait computation"
provides:
  - "`Tab::held_for_view` and `TabManager::{hold_for_view, release_view_hold, view_holds}` — a per-tab hold count the visibility synchronisation consults, so a viewer's tab survives a tab-set change the local human caused"
  - "`tabs::visibility_of` / `tabs::Visibility` — the three-state decision extracted from `sync_visibility` so it is assertable without a live engine"
  - "`view::capture`, `view::Surface`, `view::CaptureOutcome` — the unconditional paint and readback, off the engine's own image type and therefore `Send`"
  - "`view::{dirty_tiles, bounding_region, select_frame, crop, encode_frame, frame_message}` — the whole comparison and codec, pure and exhaustively tested at every partial-tile edge"
  - "`view::FrameEncoder` and the `talaria-frames` thread — the fourth off-thread actor, with one previous-frame buffer per attachment and a degrade-and-report spawn"
  - "`ViewSessions::{next_tick, take_due, frame_captured, require_keyframe_again, take_encoder_failure}` and `view::DueTick` — the tick, taken out of the table so the borrow is released before the readback"
  - "`Shared::process_view_frames` — the pump's main-thread half, which is the readback and nothing else"
  - "`AppEvent::FrameEncoderFailed` — an encoder that could not start, reported rather than fatal"
  - "`Command::ViewHolds` — a `TALARIA_TEST_HOOKS`-gated read of the hold count, so an e2e suite can assert a *release* rather than infer it from an absence"
  - "`TALARIA_VIEW_IDLE_MS` — the driven-to-passive threshold's override, falling back to the default and never to zero"
  - "`tests/e2e/remote_view_test.py`'s frame section — sections 24–34, including the byte-for-byte local-display assertion"
affects: [05-09, 05-10, 05-11]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Extract the *decision* out of a function whose *effects* need a live engine: `visibility_of` is a pure three-state table and `sync_visibility` is six engine calls around it, so the part worth asserting is assertable"
    - "A lease's bookkeeping is a count and its doc comment says why it is not a flag, at the field, because the two-viewer case is invisible in a diff"
    - "Clone the engine handles out under a scoped borrow and release it *before* the slow call — the deferred-drain idiom applied to a readback rather than to a queue"
    - "A test hook exists for the one assertion an e2e suite cannot make from outside: a released lease is an absence, and every other symptom of it is indistinguishable from a viewer with nothing to send"
    - "A fresh tab for a silence assertion: the previous section left a caret blinking, and a caret is a tile changing twice a second forever"
    - "A spike's written instruction overrides its own plan's prose where the two disagree, and the divergence is recorded rather than quietly reconciled"

key-files:
  created: []
  modified:
    - crates/talaria-shell/src/view.rs
    - crates/talaria-shell/src/tabs.rs
    - crates/talaria-shell/src/app.rs
    - crates/talaria-shell/src/remote_input.rs
    - crates/talaria-protocol/src/lib.rs
    - tests/e2e/remote_view_test.py
    - CHANGELOG.md

key-decisions:
  - "The readback lives in `view::capture` as a free function over two cloned handles, not as a `ViewTabs` method: a trait call would hold the tab-table borrow across the slowest step in the tick, which is exactly what the plan forbade"
  - "`CaptureOutcome` has two variants, not three — the missing-tab case is decided by the lookup in `app.rs` before `capture` is reached, so `capture` cannot be called with nothing to paint"
  - "A held tab is shown and **blurred**, not merely un-focused: the existing invariant blurred every non-displayed tab and keeping that preserves it exactly, with only hide→show changing"
  - "`ViewTabs` gained `hold_for_view`/`release_view_hold` and `ViewSessions::{message, closed}` became `&mut dyn ViewTabs`, which is what makes the fake tab table carry the hold arithmetic and the two-viewer case unit-testable"
  - "One delta per tick over the bounding region of every dirty tile, rather than one message per tile: a caret and a word of typing are adjacent tiles that were going to travel together anyway"
  - "The keyframe threshold is the fraction 9/26 and not the literal 90, so a viewport resize cannot silently re-tune it; 9/26 is exactly 90/260 at 1280x800"
  - "The previous-frame buffer is keyed on `(connection, tab)` and released by an explicit message, because the encoder thread outlives every lease and nothing else knows when one ends"
  - "An attach is refused when the encoder could not start, rather than accepted into a stream that would never produce a frame — the same refusal everything else gets, so it adds no bit"
  - "The frame encoder's comment states the divergence as **one line** and names neither the 21 ms figure nor the compression level that produced it, per 05-02's instruction 3: repeating a number this tree never executes would embed a false claim in the source"
  - "A viewport change sets a keyframe flag and does **not** resize the webview — see Deviations; the header always declares the surface actually painted"

patterns-established:
  - "Pattern: assert a boundary at the threshold and one unit either side, and say in the message which reading failed — `the threshold itself was already a keyframe, so 'exceeds' meant 'reaches'`"
  - "Pattern: a crop/row accessor bounds-checks against the row's own width, not only the buffer's length, because a horizontal overrun on any row but the last lands inside the buffer and silently reads the next scanline"
  - "Pattern: drain to quiet before asserting silence, so the assertion is about a static page and not about a page that had not finished arriving"

requirements-completed: []

coverage:
  - id: D1
    description: "An attachment holds its tab shown so Servo answers a hit test for it, closing 05-06's blocker"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "cargo test -p talaria-shell view::tests::attaching_holds_the_tab_shown, ::a_tab_the_human_owns_is_never_held; tabs::tests::the_visibility_table_shows_a_held_tab_and_hides_only_an_unheld_one"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'HELD' (view_holds is 1 on a background tab) and 'KEYFRAME ON ATTACH' (that tab paints)"
        status: pass
    human_judgment: false
  - id: D2
    description: "The hold is a count, so two viewers on one tab release it only when the last leaves, and a tab-set change does not hide it"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "view::tests::two_viewers_hold_one_tab_and_the_first_detach_does_not_release_it, ::attaching_twice_from_one_connection_holds_the_tab_once, ::detaching_a_tab_that_was_never_held_releases_nothing, ::a_connection_ending_releases_every_hold_it_had"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'INDEPENDENT' and 'CLOSED'"
        status: pass
    human_judgment: false
  - id: D3
    description: "An attachment changes nothing the local human sees: not the displayed tab, not the view mode, not focus, and not the displayed tab's pixels"
    requirement: "DIST-02"
    verification:
      - kind: other
        ref: "grep -c 'set_active\\|ViewMode\\|focus()' crates/talaria-shell/src/view.rs is 0"
        status: pass
      - kind: unit
        ref: "tabs::tests::a_hold_never_takes_focus_from_the_displayed_tab, ::a_held_tab_is_shown_without_being_focused"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'LOCAL DISPLAY UNMOVED': title, focused tabs, and the human's own tab byte-identical across a whole frame exchange"
        status: pass
    human_judgment: false
  - id: D4
    description: "The pump paints unconditionally on its own tick, uses neither the screenshot queue nor a frame-ready wait, and installs no second control-flow source"
    requirement: "DIST-02"
    verification:
      - kind: other
        ref: "grep -c 'pending_captures' / 'notify_new_frame_ready' / 'ControlFlow::' in crates/talaria-shell/src/view.rs all 0; app::next_capture_deadline names views.next_tick()"
        status: pass
      - kind: unit
        ref: "view::tests::with_no_attachment_there_is_no_tick_and_nothing_is_due, ::an_attachment_is_due_at_once_and_then_not_until_its_interval_passes, ::a_late_tick_does_not_burst_to_catch_up"
        status: pass
    human_judgment: false
  - id: D5
    description: "30 ms is requested unconditionally, with no environment check and no knob that lowers it; the passive rate is inside DIST-02's 200-500 band and the transition is defined at the threshold"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "view::tests::the_two_cadences_are_the_measured_ones, ::the_cadence_falls_back_to_passive_exactly_at_the_idle_threshold, ::an_accepted_input_moves_the_attachment_to_the_driven_cadence, ::the_idle_threshold_falls_back_to_its_default_and_never_to_zero"
        status: pass
      - kind: other
        ref: "the only environment variable the pump reads is TALARIA_VIEW_IDLE_MS (the transition), never the interval itself; DRIVEN_TICK_MS is a const with no override"
        status: pass
    human_judgment: false
  - id: D6
    description: "A keyframe on attach, deltas thereafter, and nothing at all on a static page"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "view::tests::an_unchanged_frame_yields_no_dirty_tiles_and_no_message, ::the_first_frame_of_an_attachment_selects_a_keyframe, ::adjacent_changed_tiles_become_one_bounding_region"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'KEYFRAME ON ATTACH', 'STATIC IS SILENT', 'DELTA'"
        status: pass
    human_judgment: false
  - id: D7
    description: "The tile comparison finds exactly the changed tiles, including at the right edge, the bottom edge and the corner where tiles are partial"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "view::tests::a_single_pixel_change_in_the_interior_dirties_exactly_its_tile, ::a_change_at_the_right_edge_is_found_in_the_partial_tile, ::a_change_at_the_bottom_edge_is_found_in_the_partial_tile, ::a_change_at_the_bottom_right_corner_is_found, ::every_pixel_of_a_partial_edge_tile_is_compared, ::a_cropped_region_is_exactly_its_own_pixels"
        status: pass
    human_judgment: false
  - id: D8
    description: "A scroll-sized change is one keyframe rather than hundreds of tiles, at a threshold with a measured basis"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "view::tests::the_dirty_tile_threshold_selects_a_keyframe_above_a_third_of_the_grid, ::a_surface_that_changed_size_selects_a_keyframe, ::a_forced_keyframe_overrides_an_unchanged_frame"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'THRESHOLD', 'RESIZE'"
        status: pass
    human_judgment: false
  - id: D9
    description: "The comparison and the encode are off the main thread, and the encoder degrades rather than aborting"
    requirement: "DIST-02"
    verification:
      - kind: other
        ref: "grep -c 'std::thread::Builder' crates/talaria-shell/src/view.rs is 1 and no expect(\" appears outside cfg(test); Shared::process_view_frames performs only paint + read_to_image"
        status: pass
    human_judgment: false
  - id: D10
    description: "Frame sequences are strictly increasing per attachment, each attachment has its own sequence space and its own previous-frame buffer, and the buffer is released when the attachment ends"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "view::tests::frame_sequences_increase_per_attachment_and_never_across_them, ::a_keyframe_a_failed_tick_consumed_is_put_back"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'SECOND VIEWER' (its own keyframe at sequence 1 while the first is past 63)"
        status: pass
    human_judgment: false
  - id: D11
    description: "Every frame carries the last input sequence the server had applied when it painted, in a header the wire's own decoder accepts"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "view::tests::a_composed_frame_carries_a_header_the_wire_accepts, ::a_keyframe_covers_its_whole_surface_and_still_decodes, ::a_frame_message_is_the_tag_then_the_header_then_the_payload"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'ECHOED'"
        status: pass
    human_judgment: false
  - id: D12
    description: "The screenshot encoder is untouched with its one call site intact, and the local render path is unchanged"
    requirement: "DIST-02"
    verification:
      - kind: other
        ref: "grep -c 'encode_screenshot' app.rs is 2 and view.rs is 0; git diff HEAD~3 -- app.rs | grep -c encode_screenshot is 0; git diff --stat gui.rs is empty"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/takeover_test.py and panel_click_test.py both exit 0 unmodified; git diff --stat on both is empty"
        status: pass
    human_judgment: false
  - id: D13
    description: "The readback cost on real GPU hardware, which this repository still cannot measure"
    verification: []
    human_judgment: true
    rationale: "Inherited unchanged from 05-02 D5: every X display on this machine is an Xvfb on llvmpipe. The number that could break the capture model is a hardware `read_to_image` slower than 1.5 ms, and this plan's e2e ran on the same software rasteriser. SC 2's evidence must come from the two-machine manual check."

# Metrics
duration: 95min
completed: 2026-08-23
status: complete
---

# Phase 5 Plan 08: The Frame Pump, Tile Diff and Encoder Thread Summary

**An attachment now holds its tab shown — which is what makes a remote click reach a page at all —
and pumps it on the loop's own clock, sending a keyframe on attach, only the changed region
afterwards, and nothing whatsoever while the page is static, with the comparison and the encode on
a fourth off-thread actor and the local human's own pixels proved byte-identical across the whole
exchange.**

## Performance

- **Duration:** 95 min
- **Tasks:** 3 of 3, each committed on its own
- **Files modified:** 7 (`view.rs`, `tabs.rs`, `app.rs`, `remote_input.rs`, `talaria-protocol/src/lib.rs`, `remote_view_test.py`, `CHANGELOG.md`)
- **Unit tests:** 410 → **448** (+38: 401 shell, 24 protocol, 21 client, 2 mcp)
- **End-to-end:** **23/23 PASS**, `failed: none`, exit 0

## Accomplishments

- **05-06's blocker is closed.** `Tab::held_for_view` is a count consulted by `sync_visibility`
  before it hides anything, so an attachment shows its tab and Servo will answer a hit test for it.
  The end-to-end suite attaches to a **background** agent tab while the local human sits in the Me
  view and reads `view_holds` back over the control socket: it is 1, and that tab paints. Before
  this, the input suite had to make the local human switch views first.

- **And it changes nothing the local human sees.** A held tab is shown and *not* focused; the
  displayed tab wins over held for every hold count (asserted across the whole table, not at one
  point); `grep -c 'set_active\|ViewMode\|focus()'` in `view.rs` is 0. The per-tab framebuffer
  invariant is now asserted rather than cited a second way: the suite captures the human's own
  displayed tab before and after a full frame exchange on a different tab and compares the two
  **base64 payloads byte for byte**. They are identical.

- **The tick joins the loop's existing wait computation.** `next_capture_deadline` gained one
  source, `views.next_tick()`. `ControlFlow::` appears nowhere in `view.rs`. With no viewer
  attached the pump contributes `None` and the loop waits exactly as it did.

- **The main thread spends the readback and nothing else.** `process_view_frames` takes the due
  list out of the session table under one scoped borrow, clones the two engine handles out of the
  tab table under a second, and calls `view::capture` with **both released** — the deferred-drain
  idiom applied to a readback rather than to a queue. Everything after that is `talaria-frames`.

- **The comparison is exhaustively tested where it is easiest to get wrong.** A single-pixel change
  in the interior, at the right edge, at the bottom edge and at the bottom-right corner, plus a
  walk of every corner pixel of a doubly-partial tile. The partial-tile arithmetic is the part
  whose off-by-one produces a stripe of stale pixels that reads as a rendering bug rather than as a
  protocol error.

- **The keyframe threshold is a fraction, not the literal 90.** Nine twenty-sixths, which is
  exactly 90/260 at 1280×800 and stays a third of the grid at any other size, asserted at the
  threshold and one tile either side with a message that names which reading failed.

- **`encode_screenshot` is provably untouched.** 2 occurrences in `app.rs` (definition plus its one
  call site), 0 in `view.rs`, and `git diff` over the three commits mentions it zero times.

- **The encoder degrades.** A checked spawn following the listener's shape, a recorded reason
  surfaced as `AppEvent::FrameEncoderFailed`, and an attach refused with the one refusal rather
  than accepted into a stream that would never produce a frame. One previous-frame buffer per
  *attachment* — not per tab — released on detach, on disconnect and on the tab closing.

- **A real bug found by a test that was written to pass.** `Surface::row` originally bounds-checked
  only against the buffer's length. A horizontal overrun on any row but the last lands *inside* the
  buffer: it silently reads the beginning of the next scanline. The crop test caught it, and the
  accessor now checks `x + width` against the row's own width.

### The numbers this plan chose, and what they were chosen against

| Constant | Value | Basis |
|----------|-------|-------|
| `DRIVEN_TICK_MS` | 30 | 05-02's instruction: request it **unconditionally, on every machine**, including under Xvfb. No environment check, no knob. It is also 05-10's fastest permitted rung. |
| `PASSIVE_TICK_MS` | 250 | Inside DIST-02's 200–500 band, and asserted to be by a test rather than by a comment. |
| `DEFAULT_VIEW_IDLE_MS` | 1000 | Longer than the gap between two keystrokes, shorter than a pause for thought. Overridable by `TALARIA_VIEW_IDLE_MS`; falls back to the default, never to zero, never to unbounded. |
| `TILE_SIZE` | 64 | 05-02: a 260-tile grid at 1280×800 scans in 0.33 ms static (the scan's worst case) and 0.11 ms scrolling. |
| Keyframe threshold | 9/26 of the grid | 05-02's measured crossover of ~90 of 260 tiles, where per-tile encode overhead meets a whole keyframe. |
| `FULL_SCALE_DENOMINATOR` | 1 | This plan sends full resolution; 05-10 owns the halved rung. |

### The observed cost, and what it is not

The plan asked for "the observed main-thread cost of a tick as a fraction of the local frame."
**This plan did not re-measure it, deliberately.** 05-02 measured exactly this quantity for exactly
this shape — a leased tab held shown, unconditional `paint()`, full-surface readback, chrome
rendering throughout — over 900 ticks, and reported **5.8 % of a 30 ms tick settled and 21 %
scrolling**, with the local chrome's own frame slowed 9–18 %. The shipped pump does strictly less
main-thread work than the spike's loop did, because the dirty-tile scan the spike timed on the main
thread now runs on `talaria-frames`. Producing a second, worse-instrumented figure from the e2e
harness would have added a number without adding a fact. The figure that is still missing is the
hardware one, and it is unchanged as a manual item (D13).

The per-attachment buffer bound is one full surface per live attachment: 1200×800×4 ≈ 3.8 MB each,
bounded by the concurrent-attachment cap (default 8) times the number of connections.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 — Bug] `Surface::row` bounds-checked the buffer, not the row**

- **Found during:** Task 2, by the crop test's own negative case failing.
- **Issue:** the accessor computed a flat offset and used `pixels.get(start..end)`. A request for 8
  pixels at `x = 196` on a 200-wide surface is in range for every row but the last — it reads four
  pixels of the requested row and four of the *next* one. The doc comment claimed such a request was
  refused; it was not. Left in, this would have produced a delta whose right-hand column carried
  pixels from one row lower, on every viewport whose width is not a multiple of 64 — which is most
  of them.
- **Fix:** check `x + width <= self.width` and `y < self.height` explicitly, with a comment saying
  why the buffer-length check is not sufficient.
- **Files modified:** `crates/talaria-shell/src/view.rs`
- **Commit:** `80de3bd`

**2. [Rule 2 — Missing critical functionality] `Command::ViewHolds`, a test hook for the release**

- **Found during:** Task 1, planning Task 3's assertion 8.
- **Issue:** the plan requires the suite to "read back over the control socket that it is no longer
  held". Nothing exposed the hold count, and the alternatives are all vacuous: a released tab going
  hidden and a viewer's frames stopping are both indistinguishable from a static page with nothing
  to send. Without this the release path would have been asserted only in unit tests.
- **Fix:** a `TALARIA_TEST_HOOKS=1`-gated command reusing the existing `ResultPayload::Value`, so
  the protocol gains one command and no new payload variant. Refused as *unrecognised* rather than
  as forbidden, exactly as `Command::ChromeRects` is, with a doc comment saying why a count of who
  is watching is not an agent's to read. A round-trip assertion was added to the protocol crate's
  existing wire test.
- **Files modified:** `crates/talaria-protocol/src/lib.rs`, `crates/talaria-shell/src/app.rs`,
  `tests/e2e/remote_view_test.py`
- **Commits:** `dcfdb4f`, `80de3bd`

**3. [Rule 2 — CLAUDE.md directive] `CHANGELOG.md` entry**

- **Found during:** Task 3.
- **Issue:** the project's `CLAUDE.md` requires every change to be logged in `CHANGELOG.md`; the
  plan's `files_modified` did not name it. 05-06 landed the same obligation for its own work.
- **Fix:** an `### Added` entry above 05-06's, covering the hold, the pump, the cadence and the
  encoder thread.
- **Commit:** `1e890bd`

### Deliberate divergences from the plan text

**4. The frame encoder's comment states the divergence as one line, not three, and names no
millisecond figure**

The plan's Task 2 action asks for a comment "stating the two numbers that settle it" — the default
compression measuring tens to hundreds of milliseconds against a 30 ms budget. **05-02's instruction
3 forbids exactly that**, because the premise is false for this tree: `png 0.17.16`'s
`Info::default()` already sets `Compression::Fast` and filter `Sub`, `encode_screenshot` never
called `set_compression`, and the spike proved it by encoding one real frame both ways and getting
byte-identical output. Repeating the 21 ms figure in a code comment would embed a claim about a
configuration this codebase does not execute. The prompt reinforced the instruction, and it was
followed: the comment says the divergence is **one line** — raw bytes for a binary channel — says
the settings are written out anyway because a default that happens to agree is not a choice, and
keeps D-05-05's two-encoders-for-two-jobs argument, which is the half that survived. The acceptance
criterion `grep -c 'Compression::Default'` is 0 partly for this reason.

**5. A viewport message sets a keyframe flag and does not resize the webview**

The plan's Task 2 behaviour says "A viewport size change produces a keyframe, and the header's
full-frame size reflects the new size", and Task 3 asks the suite to assert "a keyframe whose
declared full-frame size is the new one". Taken literally that requires the server to resize the
agent's webview because a remote viewer asked. **That was not built, and the reasons are specific:**

- Nothing in the plan's `<action>`, its artifact list or `D-05-05` says to resize anything, and
  05-05's own shipped comment says the opposite — "a resize is a hint about how the viewer wants
  `tab` painted, and painting is not this plan's."
- `ServerView::Attached`'s doc comment already calls its size "a *starting* size and never a
  contract", with the frame header as the authority.
- Resizing an agent's tab changes the size the *agent* is driving, and would change the local
  display the moment the human switched to the Agents view — which is the one thing this plan's
  security argument turns on.
- Server-side scaling is 05-10's, through `scale_denominator`, which is carried in the header
  precisely so this can be added there without a wire change.

What is built: a viewport message forces the next frame to be a whole keyframe (the client is about
to reallocate its texture, so every tile it holds is meaningless), and a surface that *does* change
size is independently a keyframe trigger with the new size in the header. The e2e asserts the
keyframe and that its declared size is the surface actually painted; the unit test
`a_surface_that_changed_size_selects_a_keyframe` asserts the header follows a real size change.
**This is the one place where the delivered behaviour is narrower than a literal reading of the
plan, and 05-10 is where the rest belongs.**

**6. Two acceptance criteria were written against a false baseline**

Both are recorded rather than gamed:

| Criterion | Stated | Baseline before this plan | What was done |
|-----------|--------|---------------------------|---------------|
| `grep -c 'expect("' crates/talaria-shell/src/view.rs` is 0 | 0 | **16**, every one inside `#[cfg(test)]` | The criterion's purpose is "the encoder degrades rather than aborting". Verified instead that **zero** `expect("` occurrences appear outside `#[cfg(test)]` — checked with an `awk` pass over the file. The total is now 28, all in tests. Rewriting 28 test call-sites to satisfy a literal grep would have made the tests worse for no gain. |
| `grep -ci 'latency\|cadence\|ms' tests/e2e/remote_view_test.py \| head -1` is 0 | 0 | **4** | `ms` matches inside ordinary words: three of the four are `**params` in the `rpc` helper and one is the word "claims". The count is now **3** and every match was inspected — none is a timing claim. The criterion's real intent (no cadence or latency assertion in this suite) holds: the frame section makes no timing claim of any kind, and says so in the docstring. |

## Verification

| Check | Result |
|-------|--------|
| `cargo build --release --locked` | exit 0, no warnings |
| `cargo clippy --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --locked` | **448 passed**, 0 failed (401 + 24 + 21 + 2) |
| `python3 tests/e2e/remote_view_test.py` | exit 0, `REMOTE VIEW CHECKS PASSED` |
| `python3 tests/e2e/takeover_test.py` (unmodified) | exit 0 |
| `python3 tests/e2e/panel_click_test.py` (unmodified) | exit 0 |
| `python3 tests/e2e/run_all.py` (`:95`) | **23/23 PASS**, `failed: none` |
| `git diff --stat tests/e2e/takeover_test.py tests/e2e/panel_click_test.py` | empty |
| `git diff --stat crates/talaria-shell/src/gui.rs` | empty |
| `git diff Cargo.toml Cargo.lock crates/talaria-shell/Cargo.toml` | empty |
| `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'` | **1** |
| `grep -c 'encode_screenshot' app.rs` / `view.rs` | **2** / **0** |
| `grep -c 'pending_captures' view.rs` | 0 |
| `grep -c 'notify_new_frame_ready' view.rs` | 0 |
| `grep -c 'set_active\|ViewMode\|focus()' view.rs` | 0 |
| `grep -c 'ControlFlow::' view.rs` | 0 |
| `grep -c 'Compression::Fast' / 'FilterType::Sub' / 'Compression::Default' view.rs` | 1 / 1 / **0** |
| `grep -c 'base64' view.rs` | 0 |
| `grep -c 'D-05-05' view.rs` | 1 |
| `grep -c 'std::thread::Builder' view.rs` | 1 |
| `grep -c 'unwrap()' view.rs tabs.rs app.rs http.rs` | 0 / 0 / 0 / 0 |
| `grep -ci 'held' tabs.rs` | 27 (was 0) |
| `grep -c '#[test]' view.rs` | **54** (was 19) |
| `grep -ci 'keyframe' remote_view_test.py` | 28 |

Display: `TALARIA_E2E_DISPLAY=:95`, per the phase's `deferred-items.md` entry — `:98` belongs to
this machine's self-hosted CI runner and two runs on one display SIGKILL each other. No ad-hoc run
was started while the full suite was in flight.

## Commits

| Task | Commit | What |
|------|--------|------|
| 1 | `dcfdb4f` | The hold, the visibility table, the unconditional paint and readback, the cadence, the loop's clock, the `ViewHolds` hook |
| 2 | `80de3bd` | The tile comparison, keyframe selection, the sibling encoder, the header assembly, the `talaria-frames` thread, `AppEvent::FrameEncoderFailed` |
| 3 | `1e890bd` | The e2e frame section (sections 24–34) and the `CHANGELOG.md` entry |

## Issues Encountered

- **The echo assertion's first draft broke before it forced a repaint.** A pointer move over a link
  changes no pixels, so the pump correctly sent nothing — and the loop's `next_frame(...) is None`
  branch broke out *before* reaching the `evaluate` that would have made the page repaint. The
  fixed loop forces the repaint before each read. Worth carrying forward: **on this design a test
  that waits for a frame must first make one exist**, because silence is the correct answer to a
  static page and never a symptom.

- **The input half leaves a caret blinking.** `remote_view_test.py`'s input sections click into the
  fixture's text field and type, and a focused caret dirties a tile twice a second forever. The
  frame section therefore opens a tab of its own and never clicks into it; reusing the driven tab
  would have made the static-page silence assertion a test of nothing.

## What the next plans inherit

- **05-09** — the client: the frame channel is live and its header is exactly what
  `tests/e2e/remote_view_test.py`'s `decode_frame` reads. A client must expect a keyframe first,
  composite tiles at `(tile_x, tile_y)` over what it holds, resize its texture when
  `frame_width`/`frame_height` change, and treat silence as "nothing changed" rather than as a
  stall.
- **05-10** — the rate controller: `DRIVEN_TICK_MS` is 30 and is the fastest rung; the *requested*
  cadence must not move, only the delivered one. Two things are its job and were deliberately left
  undone here — the halved-resolution rungs (`FULL_SCALE_DENOMINATOR` is a named constant so it has
  one place to change) and server-side response to `ClientView::Viewport` beyond a keyframe (see
  divergence 5). Also inherited: **keep the first frame after an attach out of the EWMA** — 05-02
  measured it at 53–112 ms against a 0.19–5.1 ms steady state, and the pump makes an attachment due
  immediately, so that outlier is the very first sample a controller would see.
- **05-11** — `VERIFICATION.md`: the hardware readback re-measure is still a manual item (D13), and
  a second one joins it — **remote keyboard input to a tab the local human is not looking at.** A
  held tab is shown and blurred, and Servo's keyboard focus is a different thing from visibility;
  the shipped e2e types only into a displayed tab, so this is untested rather than known-broken.
  Logged to `deferred-items.md`.

## User Setup Required

None. No new dependency, no new configuration file, and one new optional environment variable
(`TALARIA_VIEW_IDLE_MS`) that falls back to its default when unset.

## Self-Check: PASSED

- `.planning/phases/05-distributed-mode/05-08-SUMMARY.md` — FOUND
- `.planning/phases/05-distributed-mode/deferred-items.md` — FOUND (two entries appended)
- `crates/talaria-shell/src/view.rs`, `crates/talaria-shell/src/tabs.rs`,
  `tests/e2e/remote_view_test.py`, `CHANGELOG.md` — FOUND
- commit `dcfdb4f` (Task 1) — FOUND
- commit `80de3bd` (Task 2) — FOUND
- commit `1e890bd` (Task 3) — FOUND

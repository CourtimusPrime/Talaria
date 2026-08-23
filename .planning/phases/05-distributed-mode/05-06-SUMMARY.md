---
phase: 05-distributed-mode
plan: 06
subsystem: core
tags: [remote-input, servo, keyboard, coordinates, sequence, chrome-isolation, e2e]

# Dependency graph
requires:
  - phase: 05-distributed-mode
    provides: "05-03's `InputMessage` and its structural refusals — a missing field, an unknown kind, a non-finite coordinate and a key naming both or neither all decode to nothing, so this plan implemented no structural validation at all"
  - phase: 05-distributed-mode
    provides: "05-05's `/view` route, `ViewSessions`, the attachment table, `TabManager::agent_tab`, and the input channel already decoded structurally with its sequence mark recorded"
  - phase: 05-distributed-mode
    provides: "05-PATTERNS.md's finding that only `forward_mouse_move` subtracts the toolbar height and the other two inherit it by reading `webview_point` back — the trap this plan exists to not fall into"
  - phase: 02-mcp-agent-control
    provides: "`ChromeRect`'s doc comment, whose argument about synthetic input aimed at the credentials button is this module's direct ancestor, and the `chrome_rects` test hook the end-to-end chrome assertion aims with"
provides:
  - "`crates/talaria-shell/src/remote_input.rs` — the single wire-to-engine input path: agent-only resolution, per-connection sequence enforcement, crash refusal, conversion, and delivery to three functions and nothing else"
  - "`app::deliver_mouse_move` / `deliver_mouse_button` / `deliver_wheel` — delivery taking a webview and a point already relative to *that* webview's viewport, refusing rather than clamping"
  - "`app::WHEEL_LINE_PIXELS` — one pixels-per-line constant read by both input paths"
  - "`keyutils::keyboard_event_from_wire` plus `NAMED_KEYS`, the one name/winit/engine table both keyboard mappings read"
  - "`view::Handled` — the three-way answer that lets `ViewSessions` decode an input frame and deliberately not act on it"
  - "`ViewSessions::admit_input` and `ViewSession::last_applied_input` — the per-connection sequence rule, and the value 05-08's frame header echoes"
  - "`view::testing` — the fake tab table and viewer scaffolding, now shared by two suites rather than duplicated"
  - "`tests/e2e/remote_view_test.py`'s input half: a served fixture with decoy links, both coordinate directions, typing, replay, the chrome assertion, and a displayed Me tab refusing"
affects: [05-07, 05-08, 05-09, 05-10, 05-11]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "A signature that cannot express the bug: delivery takes a point already relative to the target's viewport, so applying a window-chrome offset remotely is not a mistake somebody can make in passing — there is no window in scope"
    - "A decoy element in an end-to-end fixture, placed exactly where the suspected bug would land the click, so a regression navigates to a **named** wrong destination instead of to nothing"
    - "A probe proved discriminating in the same run before it is relied on: a local click opens the panel, a second closes it, and only then is the remote click's `present=False` evidence"
    - "One table read by two mappings, with the agreement asserted by a test that walks the table rather than by inspection"
    - "A module whose guarantees are greps returning zero, with the *reason* written as a comment at the line somebody would otherwise add"

key-files:
  created:
    - crates/talaria-shell/src/remote_input.rs
  modified:
    - crates/talaria-shell/src/app.rs
    - crates/talaria-shell/src/keyutils.rs
    - crates/talaria-shell/src/view.rs
    - crates/talaria-shell/src/main.rs
    - tests/e2e/remote_view_test.py
    - CHANGELOG.md

key-decisions:
  - "The three delivery functions take `(&WebView, Point2D<f32, DevicePixel>, …)` and return `bool`; the toolbar subtraction moved to `forward_mouse_move`'s call site and the left-viewport transition stayed with the local caller, because it needs a previous position and the remote path deliberately keeps none"
  - "The wheel's line-to-pixel factor became `app::WHEEL_LINE_PIXELS: f32 = 76.0`, kept as `f32` so the local expression `(x * WHEEL_LINE_PIXELS) as f64` is bit-identical to the `(x * 76.0) as f64` it replaced"
  - "The keyboard table is `&[(&str, WinitNamedKey, Engine)]` — the wire name, the winit key, and the engine key — with `Engine::Character(\" \")` for Space, so the space-key exception lives in the table and neither mapping special-cases it"
  - "The winit `Super`/`Meta` fold stayed in `name_from_winit` rather than becoming a second table row spelled \"Meta\", because a duplicate name would make the *wire* lookup ambiguous"
  - "The agreement test asserts at the level of the two lookups, not by feeding a `winit::event::KeyEvent` through the public mapping — that type is not constructible outside winit, which is the same fact that keeps it off the wire"
  - "`ViewSessions::message` returns `Handled` rather than `bool`, so an input frame is handed *back*: `view.rs` owns no path to a webview, which is what makes `remote_input` the only one"
  - "`last_input_seq` became `last_applied_input`, matching `FrameHeader`'s field, and a message refused downstream still consumes its number — a number that could be reused is a message that could be replayed once the state it was refused for has changed"
  - "Ownership is asked twice on the delivery path — `ViewTabs::agent_viewport` in the engine-free `admit`, and `TabManager::agent_tab` for the webview — because the refusal must be unit-testable *and* the webview must come from the agent-only lookup"
  - "The end-to-end chrome probe is the history panel's rows, not the credentials panel's, because an empty vault draws no named rectangle; the credentials control is still a coordinate aimed at, and the discriminating half rides the history control, which is the same window-event path a bad refactor would reach"

patterns-established:
  - "Pattern: put a decoy where the bug would land, so the regression assertion names the wrong element rather than reporting an absence"
  - "Pattern: assert a negative through a probe you have just demonstrated is positive-capable in the same run"
  - "Pattern: when a producer lands one commit before its consumer under `-D warnings`, use `#[cfg_attr(not(test), expect(dead_code, …))]` — the compiler then *forces* the attribute's removal in the consumer's commit"

requirements-completed: [DIST-02]

coverage:
  - id: D1
    description: "A remote pointer press on an attached agent tab reaches that tab's page, asserted on the tab's URL over the control socket rather than over the channel under test"
    requirement: "DIST-02"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'AIMED LOWER' and 'AIMED UPPER'"
        status: pass
    human_judgment: false
  - id: D2
    description: "The click lands at the coordinate it was aimed at: the local toolbar offset is never applied remotely, and the assertion names the specific element hit in both directions, on a fixture whose decoy bands are proved reachable"
    requirement: "DIST-02"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'AIMED LOWER', 'DECOY REACHABLE', 'AIMED UPPER'"
        status: pass
      - kind: other
        ref: "grep -c 'toolbar_height_device\\|toolbar_height' crates/talaria-shell/src/remote_input.rs is 0"
        status: pass
    human_judgment: false
  - id: D3
    description: "Remote input never writes the cached local cursor position, so the two input sources cannot cross-contaminate"
    requirement: "DIST-02"
    verification:
      - kind: other
        ref: "grep -c 'webview_point' crates/talaria-shell/src/remote_input.rs is 0"
        status: pass
      - kind: unit
        ref: "cargo test -p talaria-shell remote_input::tests::a_wire_coordinate_is_carried_through_untouched"
        status: pass
    human_judgment: false
  - id: D4
    description: "Remote input reaches the engine's input entry point and nothing else — not the chrome, not the shortcut handler, not the interface-action queue — asserted by a source criterion at zero and by an end-to-end click at a real chrome rect that opens no panel"
    requirement: "DIST-02"
    verification:
      - kind: other
        ref: "grep -c 'Gui\\|UiAction\\|handle_browser_shortcut\\|apply_ui_actions\\|ChromePanel\\|set_active\\|ViewMode' crates/talaria-shell/src/remote_input.rs is 0"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'PROBE' then 'CHROME'"
        status: pass
    human_judgment: false
  - id: D5
    description: "Remote input aimed at a tab the human owns is refused server-side through a lookup that yields nothing, indistinguishably from a tab that never existed, without dropping the connection — asserted with the human's tab displayed on screen"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "cargo test -p talaria-shell remote_input::tests::a_tab_the_human_owns_is_refused, ::a_tab_that_does_not_exist_is_refused_the_same_way"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'HUMAN'S TAB'"
        status: pass
    human_judgment: false
  - id: D6
    description: "Coordinates are clamped against the target tab's own viewport, and a coordinate outside it is refused rather than clamped to an edge"
    requirement: "DIST-02"
    verification:
      - kind: other
        ref: "app::viewport_of takes the webview it is given; grep -c 'displayed()\\|displayed_mut()' crates/talaria-shell/src/remote_input.rs is 0"
        status: pass
    human_judgment: false
  - id: D7
    description: "The input sequence is strictly increasing per connection, a non-increasing value is dropped, the space is per connection, and the applied value is recorded"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "cargo test -p talaria-shell remote_input::tests::a_sequence_that_did_not_increase_is_dropped, ::the_sequence_space_is_per_connection_and_two_viewers_do_not_interfere, ::a_message_refused_downstream_still_consumes_its_sequence_number; view::tests::the_input_sequence_mark_is_per_connection_and_only_advances"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'REPLAY'"
        status: pass
    human_judgment: false
  - id: D8
    description: "Keys travel as a state plus either a character or a named key, never as a window-system event, and an unmapped name is a refusal; both keyboard mappings share one table and cannot drift"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "cargo test -p talaria-shell keyutils:: — 11 tests, including both_mappings_agree_on_every_name_in_the_shared_table"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'TYPED'"
        status: pass
    human_judgment: false
  - id: D9
    description: "A viewer sending input changes no active tab, no view mode and no window focus"
    requirement: "DIST-02"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'UNMOVED' (window title and focused-tab set unchanged across the remote input sequence)"
        status: pass
    human_judgment: false
  - id: D10
    description: "The local input path behaves exactly as it did"
    requirement: "DIST-02"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/takeover_test.py, keyboard_nav_test.py, panel_click_test.py — all pass unmodified; git diff --stat tests/e2e/takeover_test.py is empty"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/run_all.py — 23/23, failed: none"
        status: pass
    human_judgment: false

# Metrics
duration: 76min
completed: 2026-08-23
status: complete
---

# Phase 5 Plan 06: The Remote Input Path Summary

**A human on another machine can now click, scroll and type into an agent's tab — and the four things
that would have made that dangerous are absences in a module rather than checks inside one, each
asserted by a grep returning zero and, for the two a reader most wants proven, by an end-to-end
assertion that would fail on the plausible-looking regression rather than on an obvious one.**

## Performance

- **Duration:** 76 min
- **Started:** 2026-08-23T13:51Z
- **Completed:** 2026-08-23T15:07Z
- **Tasks:** 3 of 3
- **Files modified:** 6 (1 created, 5 modified)

## Accomplishments

- **The coordinate trap did not ship, and the assertion that would catch it names the wrong element.**
  `05-PATTERNS.md` found that only `forward_mouse_move` subtracts the toolbar height and that
  `forward_mouse_button` and `forward_wheel` inherit it by reading `webview_point` back — so a remote
  path reusing either would land every click some forty pixels high, with nothing erroring. The
  subtraction moved to the local call site and the delivery functions take a point *already relative
  to the target's viewport*, which makes the bug inexpressible rather than merely avoided. The
  end-to-end fixture carries a **decoy** band one toolbar-height above each real link, so the
  regression navigates to `/decoy-lower.html` — a named wrong destination — instead of hitting
  nothing and reading as "the click did not work". The decoy is asserted *reachable* in the same run,
  so "not the decoy" is a statement about where the click went rather than about a link that never
  worked.

- **Four absences, four greps at zero.** In `remote_input.rs`: `webview_point` 0, `toolbar_height`
  0, the displayed-tab accessors 0, and `Gui|UiAction|handle_browser_shortcut|apply_ui_actions|ChromePanel|set_active|ViewMode`
  0. None of them is enforced by a check — no type in the module connects to any of it. The *reason*
  for each is a comment at the line somebody would otherwise add, not a paragraph at the top of the
  file that nobody reads at the moment they need it.

- **The chrome assertion is discriminating, and it says so.** A remote pointer press is aimed at the
  credentials control's and the history control's **real** rectangles, read from the `chrome_rects`
  test hook, and no panel opens. The probe — the history panel's rows — is proved discriminating in
  the same run first: a local click on that control opens the panel and a second closes it. That
  matters because the credentials panel draws no named rectangle while the vault is empty, so a
  `present=False` on `credentials.row.0` would have been vacuously true. The suite says which half
  is which.

- **The human's tabs refuse with the tab on screen.** The Me-tab refusal is asserted while that tab
  is *displayed*, at coordinates that provably worked on the agent's copy of the same page moments
  earlier — so "nothing happened" is the server refusing rather than a hidden webview having no hit
  test to answer. It is byte-identical to naming a tab id that never existed, and the connection
  stays open.

- **One key table, and a test that walks it.** `keyutils` now holds a single
  `&[(&str, WinitNamedKey, Engine)]` table read by both mappings, with `Space` as a *character* row
  so neither mapping special-cases it. `both_mappings_agree_on_every_name_in_the_shared_table` walks
  the table rather than naming keys, so a row added later is covered the moment it is added.

- **Nothing local moved.** `takeover_test.py`, `keyboard_nav_test.py` and `panel_click_test.py` all
  pass unmodified, `git diff --stat tests/e2e/takeover_test.py` is empty, and the full suite is
  **23/23, `failed: none`**.

## Task Commits

1. **Task 1: generalise the three forwarders and share one key table** — `00aa4ea` (refactor)
2. **Task 2: the remote input entry point — one module, one path, four structural refusals** — `4debcf9` (feat)
3. **Task 3: prove it end to end** — `cc9980f` (test)
4. **CLAUDE.md-mandated changelog entry** — `13ac400` (docs)

## What landed, precisely

### The three delivery functions, as they now read

```rust
pub(crate) fn deliver_mouse_move(webview: &WebView, point: Point2D<f32, DevicePixel>) -> bool;
pub(crate) fn deliver_mouse_button(
    webview: &WebView,
    point: Point2D<f32, DevicePixel>,
    button: ServoMouseButton,
    action: MouseButtonAction,
) -> bool;
pub(crate) fn deliver_wheel(
    webview: &WebView,
    point: Point2D<f32, DevicePixel>,
    delta: WheelDelta,
) -> bool;
```

All three test containment against `viewport_of(webview)` — the **given** webview's size — and
return whether the event was delivered. A point outside is a refusal and never a clamp to the
nearest edge, because clamping turns "aim at nothing" into "aim at the nearest thing". The
`bool` is what lets `forward_mouse_move` keep its left-viewport transition at the call site, where
the previous position lives.

### The named-key table, and how it is shared

`NAMED_KEYS: &[(&str, WinitNamedKey, Engine)]` — 33 rows. `name_from_winit` reads the middle column
to find a row; `key_from_name` reads the first. `Engine` is `Named(NamedKey)` or
`Character(&'static str)`, and `("Space", WinitNamedKey::Space, Engine::Character(" "))` is the one
row of the second kind, which is `keyboard_types`' own rule. The `Super`/`Meta` fold stayed in
`name_from_winit`, because a second row spelled `"Meta"` would make the *wire* lookup ambiguous.

`both_mappings_agree_on_every_name_in_the_shared_table` asserts, for every row, that
`name_from_winit(row.1) == Some(row.0)` and `key_from_name(row.0) == Some(row.2.key())`. It asserts
at the level of the two lookups rather than through `keyboard_event_from_winit`, because
`winit::event::KeyEvent` is not constructible outside winit — the same fact that keeps it off the
wire.

### The sequence rule, exactly

```rust
pub fn admit_input(&mut self, connection: u64, tab: u64, seq: u64) -> bool {
    let Some(session) = self.session_mut(connection) else { return false };
    if seq <= session.last_applied_input {
        return false;
    }
    session.last_applied_input = seq;
    session.attached.contains(&tab)
}
```

`seq <= last_applied_input` is the comparison: equal is a replay and lower is a reorder, and both are
dropped. The mark advances *before* the attachment and ownership questions, so a message refused
downstream still consumes its number — a number that could be reused is a message that could be
replayed later, once the state it was refused for has changed. The field is on the session, not on
the attachment, so two viewers on one tab neither share a mark nor starve each other.

### The fixture's geometry, and the two-direction assertion

| Element | `top` | Height | Destination |
|---------|-------|--------|-------------|
| `#decoy-upper` | 105 px | 80 px | `/decoy-upper.html` |
| `#upper` | 200 px | 24 px | `/upper.html` |
| `#decoy-lower` | 405 px | 80 px | `/decoy-lower.html` |
| `#lower` | 500 px | 24 px | `/lower.html` |
| `#field` | 620 px | 30 px | — |

The decoy bands are 80 px tall and sit 95 px above their link, which brackets any toolbar height
between roughly 15 and 90 device pixels; the shell's is about 45. Nothing sits in the top 100 px,
because the toolbar is drawn **over** the webview rather than beside it — so the chrome assertion's
coordinate (about y = 11) lands in a genuinely empty part of the page and "the URL did not change"
means the click reached neither a panel nor a link.

The suite asserts, in both directions, that the URL became the aimed-at destination **and** that it
is not the decoy's and not the other link's. Then it aims at the decoy deliberately and asserts it
navigates, so the negative clause is not vacuous.

## Decisions Made

Recorded in full in the frontmatter's `key-decisions`. The four a later reader most needs:

1. **The delivery functions' signature is the mitigation.** They take a webview and a
   viewport-relative point, so there is no window in scope to take a chrome offset from. `D-05-01`'s
   input half is a signature, not a rule.

2. **`ViewSessions::message` hands an input frame back rather than acting on it.** `view.rs` owns no
   path to a webview and now cannot acquire one without changing its own signature, which is what
   makes `remote_input` provably the only path. The three-way `Handled` answer is what expresses
   that.

3. **A refused message still spends its sequence number.** The alternative — advancing only on
   success — leaves a captured message replayable the moment the state that refused it changes (a
   viewport resize, a later attach). Recorded in `last_applied_input`'s doc, with a test named for
   it.

4. **Ownership is asked twice, deliberately.** `admit` asks through `ViewTabs::agent_viewport` (whose
   implementation *is* `TabManager::agent_tab`) so the refusal is unit-testable without an engine;
   `apply` asks through `agent_tab` directly so the webview itself comes out of the agent-only
   lookup. Neither is a check applied after a general lookup.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] `keyboard_event_from_wire` has no caller until Task 2, and `-D warnings` refuses that**

- **Found during:** Task 1
- **Issue:** the plan makes Task 1 the producer of the wire keyboard mapping and Task 2 its only
  consumer, but Task 1's own gate is `cargo clippy --all-targets --locked -- -D warnings`, under
  which an uncalled function is `error: function is never used`. The same class 05-05 hit with
  `last_input_seq`. `--all-targets` does not help: the non-test build of the binary still warns.
- **Fix:** `#[cfg_attr(not(test), expect(dead_code, reason = "…"))]` on the function for exactly one
  commit. `expect` rather than `allow` deliberately — the moment `remote_input` called it, the
  non-test build reported `this lint expectation is unfulfilled` and the compiler *forced* the
  attribute's removal in Task 2's commit rather than leaving a stale silence behind.
- **Files modified:** `crates/talaria-shell/src/keyutils.rs`
- **Verification:** clippy exit 0 at `00aa4ea`; the attribute is absent at `4debcf9` and clippy is
  still exit 0.
- **Committed in:** `00aa4ea` (added) and `4debcf9` (removed by the compiler's own insistence)

**2. [Rule 2 - Missing critical functionality] `deliver_wheel` now performs a containment test the old `forward_wheel` did not**

- **Found during:** Task 1
- **Issue:** the plan's behaviour list requires every delivery call outside the target's viewport to
  be refused, which is the clamp a hostile remote coordinate needs. The original `forward_wheel` had
  no containment test at all — it read `webview_point` and forwarded unconditionally.
- **Fix:** the test lives in the delivery function, so all three input kinds refuse identically.
  This is a no-op for the local path in practice: the `WindowEvent::MouseWheel` arm is already
  guarded by `!over_toolbar(&state) && !chrome_replaces_page(&state)`, so the local wheel forwarder
  is only reached when the cursor is over the page.
- **Files modified:** `crates/talaria-shell/src/app.rs`
- **Verification:** `takeover_test.py`, `keyboard_nav_test.py` and `panel_click_test.py` pass
  unmodified; full e2e 23/23.
- **Committed in:** `00aa4ea`

**3. [Rule 3 - Blocking] The end-to-end input assertions need the agent's tab on screen**

- **Found during:** Task 3
- **Issue:** a probe run showed a remote click on an agent tab that is *not displayed* reaches
  nothing — Servo's hit test needs a shown webview, and until 05-08 an attachment neither shows one
  nor ticks a pump. `sync_visibility()` shows exactly the displayed tab, and `tabs_focus` sets
  `active_agent` without changing the view mode.
- **Fix:** the suite has the **local human** switch to the Agents view, with a real click on the
  real toggle via `harness.click_rect("toolbar.agents", …)`, before any measurement begins — and
  asserts through the window title that the agent's fixture tab is the one on screen. It is stated
  in the suite's docstring as an honest limit and named at the step that does it. Nothing after that
  line switches anything, which is what makes the `UNMOVED` assertion meaningful.
- **Files modified:** `tests/e2e/remote_view_test.py`
- **Verification:** 'ON SCREEN' assertion; every input assertion below it passes.
- **Committed in:** `cc9980f`

**4. [Rule 3 - Blocking] The credentials panel draws no named rectangle while the vault is empty**

- **Found during:** Task 3
- **Issue:** the plan asks the chrome assertion to use "the absent form of the rectangle wait" after
  a remote click at the credentials control. A probe showed `chrome_rects` returns exactly the same
  14 toolbar names whether the credentials panel is open or closed on an empty vault — the panel's
  only named rects are its per-entry rows. `present=False` on `credentials.row.0` would have been
  vacuously true, and the vault cannot be seeded from a Python suite (its key lives in the OS
  keychain).
- **Fix:** the remote click is aimed at the credentials control's real rectangle **and** at the
  history control's, and "no panel opened" is measured on `history.row.0` — a probe the suite proves
  discriminating first, with a local click that opens the panel and a second that closes it. The
  history control is the same window-event path a bad refactor would route remote input through, so
  the discriminating half covers the regression class; the credentials control is still the
  coordinate aimed at, because it is the one whose reachability matters most. Both the limit and the
  reason are in the suite's docstring.
- **Files modified:** `tests/e2e/remote_view_test.py`
- **Verification:** 'PROBE' then 'CHROME'.
- **Committed in:** `cc9980f`

**5. [CLAUDE.md directive] `CHANGELOG.md` entry, which the plan's `files_modified` did not list**

- **Found during:** plan close-out
- **Issue:** the global CLAUDE.md directive "Log all changes in a project CHANGELOG.md file" is a
  hard constraint and takes precedence over the plan's file list. 05-01 and 05-03 set the precedent
  within this phase.
- **Fix:** an `### Added` entry in the existing Unreleased section, in the file's established
  prose-with-reasoning style. It does not encroach on 05-11, which owns the phase's user-facing
  closeout entry and the `grep -c 'DIST-0[12]' CHANGELOG.md` assertion.
- **Files modified:** `CHANGELOG.md`
- **Committed in:** `13ac400` (separate from all three task commits)

### Not deviations, recorded for the reader

- **`grep -c 'displayed()' remote_input.rs` was 1 before it was 0.** The comment saying the module
  deliberately does not resolve the displayed tab named the accessor, which tripped the criterion
  that asserts the accessor is absent — the same shape of collision 05-05 hit with `middlewares`.
  The comment was rephrased to describe the accessor without spelling it; the reasoning is intact
  and the count is 0.
- **`grep -c 'agent_tab' remote_input.rs` is 4, not 1.** Three are comments explaining why the
  lookup is the one used; one is the call.
- **`view::input_seq` was removed** rather than left unused — it moved into `remote_input` as
  `sequence_of`, beside `target_tab`, because the consumer of both is there now.

## Files Created/Modified

- `crates/talaria-shell/src/remote_input.rs` — **created**, 17 tests. The module header states what
  it reaches (a webview's input entry point) and what it reaches by construction not at all, quotes
  `ChromeRect`'s reasoning and says it was written for agents, and cites `D-05-02`, `T-05-04` and
  `T-05-05`.
- `crates/talaria-shell/src/app.rs` — `WHEEL_LINE_PIXELS`, `viewport_of`, the three `deliver_*`
  functions, the three local callers reduced to their local concerns, `webview_point`'s expanded doc
  comment, and `view_message`'s three-way dispatch.
- `crates/talaria-shell/src/keyutils.rs` — the shared table, `name_from_winit`, `key_from_name`,
  `keyboard_event_from_wire`, a rewritten module header, and 11 tests (the file had none).
- `crates/talaria-shell/src/view.rs` — `Handled`, `admit_input`, `last_applied_input` and its doc,
  `input()` reduced to decoding, and the `testing` module the two suites share.
- `crates/talaria-shell/src/main.rs` — `mod remote_input;`.
- `tests/e2e/remote_view_test.py` — the fixture server and page, the `Input` client, the coordinate
  helpers, and sections 14–23.
- `CHANGELOG.md` — the `### Added` entry.

## Issues Encountered

- **A hidden webview answers no click.** The probe that found this cost one throwaway run and shaped
  the whole of Task 3's setup; see deviation 3. It is worth carrying forward: 05-08's attachment
  must show the webview, or the input path this plan built is only reachable for the tab the local
  human happens to be looking at.
- **The first captured `run_all.py` log lost its first four lines** to the background-task output
  file being reopened. The run was re-executed cleanly for the record: 23 `[PASS]` lines,
  `failed: none`, exit 0.

## User Setup Required

None — no external service configuration required, no new environment variable, no new dependency.

## Verification Evidence

| Check | Result |
|-------|--------|
| `cargo build --release --locked` | exit 0 |
| `cargo clippy --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --locked` | **389 passed** (363 shell + 24 protocol + 2 mcp), 0 failed — baseline 360 |
| `cargo test -p talaria-shell --locked remote_input::` | 17 passed |
| `cargo test -p talaria-shell --locked keyutils::` | 11 passed |
| `python3 tests/e2e/remote_view_test.py` | exit 0, `REMOTE VIEW CHECKS PASSED` |
| `python3 tests/e2e/run_all.py` (display `:95`) | **23/23, `failed: none`**, exit 0 |
| `python3 tests/e2e/takeover_test.py` / `keyboard_nav_test.py` / `panel_click_test.py` | all pass **unmodified** |
| `git diff --stat tests/e2e/takeover_test.py` | empty |
| `git diff --stat Cargo.toml Cargo.lock crates/talaria-shell/Cargo.toml` | empty |
| `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'` | 1 |
| `grep -c 'fn deliver_' app.rs` | 3 |
| `grep -c 'toolbar_height_device' app.rs` | 2 |
| `grep -c 'from_wire' keyutils.rs` | 11 |
| `grep -c 'winit' crates/talaria-protocol/src/wire.rs` | 0 |
| `grep -c '#[test]' keyutils.rs` | 11 (plan asked ≥ 6) |
| `grep -c '#[test]' remote_input.rs` | 17 (plan asked ≥ 13) |
| `grep -c 'webview_point' remote_input.rs` | **0** |
| `grep -c 'toolbar_height_device\|toolbar_height' remote_input.rs` | **0** |
| `grep -c 'displayed()\|displayed_mut()' remote_input.rs` | **0** |
| `grep -c 'Gui\|UiAction\|handle_browser_shortcut\|apply_ui_actions\|ChromePanel\|set_active\|ViewMode' remote_input.rs` | **0** |
| `grep -c 'agent_tab' remote_input.rs` | 4 |
| `! grep -q 'remote_input' crates/talaria-mcp/src/tools.rs crates/talaria-protocol/src/lib.rs` | holds |
| `grep -c 'T-05-04'` / `'T-05-05'` in remote_input.rs | 4 / 2 |
| `grep -c 'unwrap()'` across `remote_input.rs`, `view.rs`, `app.rs`, `keyutils.rs` | 0 |
| `grep -ci 'chrome_rects'` / `'credentials'` / `'upper\|lower'` / `'replay\|sequence'` in remote_view_test.py | 3 / 7 / 40 / 16 |
| `grep -c 'present=False'` in remote_view_test.py | 3 |

## Threat Mitigations Landed

| Threat ID | Severity | How it was mitigated here |
|-----------|----------|---------------------------|
| T-05-04 | high | The remote path lives in one module that calls only the three delivery functions; the chrome types, the shortcut handler and the interface-action queue appear in it **zero** times. Proven end to end by aiming at a real toolbar control's rectangle and asserting no panel opened, with the probe demonstrated discriminating first |
| T-05-05 | high | Resolution goes through `TabManager::agent_tab`, so a human-owned tab is unrepresentable rather than refused downstream, and takes the same exit a nonexistent tab takes. Asserted at unit level and end to end **with the human's tab displayed**, connection staying open |
| T-05-04-A | high | The offset moved to the local call site; the delivery functions take an already-viewport-relative point. The regression assertion names the specific element hit, in both directions, on a fixture with decoy bands proved reachable |
| T-05-04-B | high | The remote path never writes `webview_point`; the grep is 0, and the wire carries coordinates on every pointer message precisely so it needs no cached previous position |
| T-05-04-C | medium | The remote path never resolves the displayed tab and never touches the active-tab or view-mode state; the greps are 0 and the end-to-end suite asserts the window title and the focused-tab set are unchanged across the whole remote input sequence |
| T-05-11 | medium | Structural refusal is 05-03's; semantic refusal is this module's — an unknown connection, a non-increasing sequence, an unattached tab, a human-owned tab, a nonexistent tab, a crashed tab, an unmapped key name, and a coordinate outside the target's viewport are each an explicit refusal, and a coordinate outside is **refused, not clamped** |
| T-05-15 | low | Strictly-increasing per connection, per-connection isolation unit-tested, a refused message still spending its number, and a verbatim replay asserted to have no effect end to end |
| T-05-SC | n/a | Accepted. No dependency change; the lockfile is byte-unchanged and `primeorder` is still pinned at `0.14.0-rc.14` |

## Threat Flags

None. This plan adds no network endpoint, no auth path, no file access and no schema at a trust
boundary. The one surface it opens — the wire-to-engine input path — is the subject of the threat
register above, and every change outside `remote_input.rs` narrows rather than widens.

## Known Stubs

None. Every function this plan added is reachable and asserted.

Two things are deliberately **absent** rather than stubbed, and both are named in the code or the
suite:

- **An attachment still does not show a webview.** 05-08 owns that. Until then a remote click only
  reaches a tab the local human happens to be displaying, which is why the e2e switches the local
  view itself and says so.
- **`last_applied_input` is recorded and echoed nowhere yet.** 05-08's frame header is what carries
  it to a client.

## What the Next Plans Inherit

- **05-07** (the client) composes input messages against this wire: coordinates in the target tab's
  own device pixels, a strictly increasing `seq` per connection, and a key naming exactly one of a
  character or a name from `keyutils::NAMED_KEYS`.
- **08** must show the attached webview, or this input path is reachable only for the displayed tab —
  and must carry `last_applied_input` into the frame header, or 05-10's rate controller has no
  latency estimate.
- **05-10**'s degrade ladder measures input-to-photon latency off that one field, with no clock
  shared between the two machines.

## Self-Check: PASSED

- `crates/talaria-shell/src/remote_input.rs` — FOUND
- `crates/talaria-shell/src/app.rs` — FOUND (modified)
- `crates/talaria-shell/src/keyutils.rs` — FOUND (modified)
- `crates/talaria-shell/src/view.rs` — FOUND (modified)
- `crates/talaria-shell/src/main.rs` — FOUND (modified)
- `tests/e2e/remote_view_test.py` — FOUND (modified)
- `CHANGELOG.md` — FOUND (modified)
- `.planning/phases/05-distributed-mode/05-06-SUMMARY.md` — FOUND
- Commit `00aa4ea` — FOUND
- Commit `4debcf9` — FOUND
- Commit `cc9980f` — FOUND
- Commit `13ac400` — FOUND

---
phase: 03-table-stakes-browsing
plan: 03
subsystem: ui
tags: [rust, egui, servo, persistence, json, settings, search, security]

requires:
  - phase: 01-foundation
    provides: the egui-on-winit shell, resolve_location and its address-bar heuristic
  - phase: 02-harden-the-agent-surface
    provides: parse_agent_url's scheme allowlist and SECURITY.md's two-trust-roots rule
  - phase: 03-table-stakes-browsing
    plan: 01
    provides: ChromePanel, UiAction::SetPanel, Gui::set_panel's view-state reset, the hand-mapped serde_json idiom
  - phase: 03-table-stakes-browsing
    plan: 02
    provides: bookmarks.rs's atomic .tmp-sibling + fs::rename save, and its TempPath test fixture
provides:
  - "crates/talaria-shell/src/settings.rs — the project's first config file, a single-object store holding the configured search engine"
  - "resolve_location(input, engine) — the address bar's search fallback is configurable at all three call sites"
  - "settings::is_valid_template — one definition of a valid template, shared by load-time validation and the panel's Save gate"
  - "ChromePanel::Settings, the GEAR toolbar button and the two-field Settings form"
  - "UiAction::SaveSearchEngine — the write lands in apply_ui_actions, never in the egui closure"
  - "app.rs's first #[cfg(test)] mod tests, pinning resolve_location including its data:-URL posture"
affects: [03-04-downloads]

tech-stack:
  added: []
  patterns:
    - "Single-object store: one JSON object, validated at load rather than merely parsed, degrading whole rather than half-applied"
    - "Validate-at-entry: the Save button is disabled until the input is valid, so no error state has to be designed for afterwards"
    - "A draft form pre-filled from the store on the frame the panel opens, guarded by a stale flag so typing is never overwritten"

key-files:
  created:
    - crates/talaria-shell/src/settings.rs
  modified:
    - crates/talaria-shell/src/app.rs
    - crates/talaria-shell/src/gui.rs
    - crates/talaria-shell/src/main.rs
    - tests/e2e/vault_ui_test.py
    - .planning/phases/03-table-stakes-browsing/deferred-items.md

key-decisions:
  - "SearchEngine has no id: exactly one engine is configured at a time, so the {name, url_template} pair is the whole identity — no engine list, no default-vs-active distinction, honoring the plan's assumption-delta note"
  - "A data: URL pasted into the address bar stays a search, not a navigation — the human/agent asymmetry runs the right way and is now a tested decision rather than an accident"
  - "An invalid template on disk discards the whole engine including its name, rather than half-applying it — a panel reading 'Kagi' while searching DuckDuckGo is worse than one reading the truth"
  - "is_valid_template is a free function, so the panel's Save gate and the store's load-time check cannot drift apart"
  - "main.rs's CLI-argument fallback passes SearchEngine::default() explicitly rather than loading config.json before the event loop exists"
  - "serde derives were replaced with hand-mapped serde_json::json!/Value, inheriting 03-01's finding that serde is not a dependency of talaria-shell"
  - "resolve_location keeps a second, call-time fallback below Settings::load's validation, so a SearchEngine built in code cannot panic the navigation path"

patterns-established:
  - "Validation shared as a free function between a store's load path and the form that writes it"
  - "A UI-SPEC backstop item resolved with a headless egui::__run_test_ui layout measurement rather than a screenshot"

requirements-completed: [BROWSE-03]

coverage:
  - id: D1
    description: "Typing a non-URL into the address bar searches the configured engine once one has been saved"
    requirement: "BROWSE-03"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/app.rs#a_configured_engine_is_what_a_search_actually_uses"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/settings.rs#a_saved_engine_survives_a_reload"
        status: pass
    human_judgment: false
  - id: D2
    description: "A fresh install with no config.json behaves byte-for-byte identically to the pre-BROWSE-03 hardcoded DuckDuckGo behavior"
    requirement: "BROWSE-03"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/app.rs#the_default_engine_reproduces_the_old_hardcoded_search_url, crates/talaria-shell/src/settings.rs#the_default_engine_is_todays_hardcoded_duckduckgo, #a_missing_file_loads_as_the_default_engine"
        status: pass
      - kind: e2e
        ref: "tests/e2e/run_all.py — 17/17 PASS with no config.json present in any suite's home"
        status: pass
    human_judgment: false
  - id: D3
    description: "A malformed config.json — unparseable, wrong-shaped, or carrying a template without {query} exactly once — degrades to the default engine at load, never panics, never blocks startup"
    requirement: "BROWSE-03"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/settings.rs#unparseable_json_loads_as_the_default_engine_without_panicking, #a_file_of_the_wrong_shape_loads_as_the_default_engine, #an_invalid_template_on_disk_loads_as_the_whole_default_engine"
        status: pass
    human_judgment: false
  - id: D4
    description: "resolve_location never panics on a malformed template that bypassed load-time validation"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/app.rs#a_malformed_template_still_yields_a_url_rather_than_a_panic"
        status: pass
    human_judgment: false
  - id: D5
    description: "The engine parameter affects only the search fallback — every URL-shaped input resolves identically regardless of which engine is passed"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/app.rs#url_shaped_input_is_unaffected_by_the_engine"
        status: pass
    human_judgment: false
  - id: D6
    description: "The Settings panel's Save button is disabled whenever the template does not contain {query} exactly once, so an invalid engine cannot be committed"
    requirement: "BROWSE-03"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/settings.rs#a_template_with_no_placeholder_is_invalid, #a_template_with_two_placeholders_is_invalid, #a_template_with_one_placeholder_is_valid_wherever_it_sits (the gate's own predicate, shared with gui.rs)"
        status: pass
    human_judgment: true
    rationale: "The predicate behind the gate is unit-tested and gui.rs calls that exact free function (`grep -c is_valid_template crates/talaria-shell/src/gui.rs` is 2), but nothing automated opens the panel, types an invalid template and observes the button greyed out. This inherits 03-01's D7 and 03-02's D8 rationale — same egui-closure-rendering gap, and 03-VALIDATION.md already scopes the visual check to a human."
  - id: D7
    description: "The target config.json only ever changes through a single fs::rename, so a crash mid-save cannot leave it half-written (T-03-03-01)"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/settings.rs#a_save_stages_through_a_temp_file_and_leaves_none_behind, #a_stale_staging_file_does_not_block_the_next_save"
        status: pass
    human_judgment: false
  - id: D8
    description: "A settings write that cannot reach the disk still applies to the running session"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/settings.rs#an_unwritable_path_still_updates_the_running_session"
        status: pass
    human_judgment: false
  - id: D9
    description: "parse_agent_url and agent_scheme_allowed are provably unchanged — the two trust roots stay separate (T-03-03-02)"
    verification:
      - kind: other
        ref: "both function bodies diffed against the pre-plan revision and reported IDENTICAL; tests/e2e/scheme_refusal_test.py PASS"
        status: pass
    human_judgment: false
  - id: D10
    description: "No MCP tool surface changed — the configured engine is not agent-reachable (T-03-03-03)"
    verification:
      - kind: other
        ref: "git diff --stat crates/talaria-mcp reports no change across this plan; grep for search_engine/config tool names returns 0"
        status: pass
    human_judgment: false
  - id: D11
    description: "A pathologically long URL template scrolls inside the 320pt field rather than widening it (03-UI-SPEC's one backstop item)"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/gui.rs#a_long_template_scrolls_inside_the_field_rather_than_widening_it"
        status: pass
    human_judgment: false
  - id: D12
    description: "A data: URL in the human address bar is searched for rather than navigated to — a decision, not an accident"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/app.rs#a_data_url_is_searched_for_rather_than_opened"
        status: pass
    human_judgment: false

duration: 25 min
completed: 2026-08-20
status: complete
---

# Phase 03 Plan 03: Configurable Search Engine Summary

**`resolve_location` — the load-bearing function behind every "type something and hit Enter" in the human's own toolbar — takes a `&SearchEngine` instead of a hardcoded DuckDuckGo literal, backed by the project's first config file, a two-field Settings panel whose Save button refuses to commit an invalid template, and app.rs's first-ever unit tests.**

## Performance

- **Duration:** 25 min
- **Started:** 2026-08-20T08:15:34Z
- **Completed:** 2026-08-20T08:40:10Z
- **Tasks:** 3
- **Files modified:** 5 (1 created, 4 modified)

## Accomplishments

- **BROWSE-03 delivered with zero behavioural regression.** A fresh install with no `config.json` produces byte-for-byte the URL the hardcoded literal produced — asserted directly (`https://duckduckgo.com/?q=hello+world`), not assumed.
- **`app.rs` has unit tests for the first time.** 2,150 lines, and the function every address-bar keystroke funnels through had never been tested. It now has five, including the one that pins the trust-root boundary this plan was most at risk of eroding.
- **`parse_agent_url` is provably untouched**, verified by extracting and diffing both agent-path function bodies against the pre-plan revision rather than by eyeballing a diff whose hunk header misleadingly names `parse_agent_url` (git labels a hunk with the *preceding* function).
- **The `data:`-URL question, open since 03-01, is closed as a decision** — not widened, with the reasoning committed as a test doc comment so the next reader finds the argument before rediscovering the symptom.
- **`03-UI-SPEC.md`'s only `backstop` item is resolved with a number**, not a promise: a 4,000-character template lays out at exactly 320.0 points, identical to a 33-character one.
- **The e2e suite stayed at 17/17**, and `cargo test` went 47 → 67.

## Task Commits

1. **Task 1 (RED): failing tests for the settings store** — `54eaaae` (test)
2. **Task 1 (GREEN): the settings store** — `92284e7` (feat)
3. **Task 2 (RED): app.rs's first tests + the signature change** — `ecd3c88` (test)
4. **Task 2 (GREEN): resolve_location honors the configured engine** — `d45ec85` (feat)
5. **Task 3: the Settings panel, the GEAR button and the Save gate** — `e45d528` (feat)

_Tasks 1 and 2 are `tdd="true"`, so each carries a test → feat pair. Task 1 RED: 7 failing / 7 passing (a stub that always returns the default happens to satisfy the degrade cases, which is honest — those tests guard a property the stub already had). Task 2 RED: 1 failing / 4 passing, the failure being the only assertion the unwired parameter could affect. No refactor commit was needed for either._

## Files Created/Modified

- `crates/talaria-shell/src/settings.rs` (new, 460 lines) — `SearchEngine` (+ `Default`, `to_json`/`from_json`), `Settings` (`load`/`load_from`/`save`), the free `is_valid_template`, a private `config_dir()`, and 14 unit tests.
- `crates/talaria-shell/src/app.rs` — `Shared::settings`, `resolve_location`'s new parameter and templated fallback, both in-crate call sites, the `SaveSearchEngine` arm of `apply_ui_actions`, and the file's first `#[cfg(test)] mod tests` (5 tests).
- `crates/talaria-shell/src/gui.rs` — `ChromePanel::Settings`, `UiAction::SaveSearchEngine`, three `Gui` draft fields, the `GEAR` toolbar button, the Settings panel body, and the backstop layout test.
- `crates/talaria-shell/src/main.rs` — `mod settings;` and the CLI-argument fallback's explicit `SearchEngine::default()`.
- `tests/e2e/vault_ui_test.py` — `CREDENTIALS_BUTTON` moved 341 → 370 (see deviations).
- `.planning/phases/03-table-stakes-browsing/deferred-items.md` — the `data:` entry marked decided with its reasoning; the toolbar-coordinate entry updated with the third datapoint and the ~29-points-per-button rate.

## Decisions Made

- **A `data:` URL pasted into the address bar stays a search. Deliberately.**
  This was the deferred item routed to this plan, and the answer is *not* to
  make the human path match the agent path. "The human typed it, so trust it"
  is at its weakest exactly here: the paste-this-into-your-address-bar attack
  makes the human a courier for someone else's payload rather than its author,
  which is why `resolve_location`'s own doc comment calls the human the trust
  *root* and not the trust *source*. `parse_agent_url` admits `data:` because
  an agent builds its own data URLs and Phase 2's SEC-01 hardened that path on
  its own terms; `SECURITY.md` says outright that the two paths are
  deliberately different code and that collapsing them is a trust-model change,
  not a refactor. Parity was never the goal, and the asymmetry runs in the safe
  direction. Independently, this plan's own must-have required byte-for-byte
  behavioural identity with the pre-BROWSE-03 address bar, so changing `data:`
  handling here would have violated the plan regardless of the security call.
  The decision is now `a_data_url_is_searched_for_rather_than_opened` with the
  argument in its doc comment — a test, so the day someone changes it, they
  change it on purpose. One residual is **accepted, not fixed**: the pasted
  URL's payload does reach the search engine, which is the same exposure any
  non-URL text typed into an omnibox has and is not specific to `data:`.
- **`SearchEngine` carries no id.** Honoring the plan's assumption-delta note:
  exactly one engine is configured at a time, so the `{name, url_template}`
  pair *is* the identity. No engine list, no default-vs-active flag, nothing
  to keep in sync. A future picker among presets wraps this shape rather than
  changing it.
- **An invalid template discards the whole engine, name included.** The
  tempting half-measure is to keep the name and swap the template. A panel
  reading "Kagi" while every search goes to DuckDuckGo is a worse failure than
  one reading "DuckDuckGo" honestly, so `load` returns
  `SearchEngine::default()` entire. A test asserts the name specifically, not
  just the pair.
- **`is_valid_template` is a free function, not a method.** It has two callers
  that must never disagree: `Settings::load`'s validation and the panel's Save
  gate. As a method it would have pulled `gui.rs` into constructing a
  `Settings` just to ask a question about a string.
- **Exactly one `{query}`, not at least one.** `replacen(.., 1)` substitutes
  the first occurrence only, so a two-placeholder template would put a literal
  `{query}` on the wire — a broken search that looks like a working one. That
  is why the check counts rather than tests for presence, and there is a test
  for each direction.
- **`resolve_location` keeps a second fallback below the load-time check.**
  `Settings::load` already refuses an invalid template, so the `unwrap_or_else`
  in `resolve_location` is unreachable through the normal path — which is the
  point. A `SearchEngine` built in code (a test, a future caller, a hand-edit
  to a file this process already had open) must not be able to panic the
  navigation path. Four malformed templates are tested through it.
- **`main.rs` passes `SearchEngine::default()` explicitly.** That call site runs
  before the event loop and before any `Shared` exists. Loading `config.json`
  early just for it would duplicate the load logic for a best-effort path that
  only fires when the first CLI argument is neither a URL nor a host. The
  comment at the call site says so, so the next reader does not "fix" it.
- **Serialisation is hand-mapped, again.** The plan's `<action>` specified
  `#[derive(Serialize, Deserialize)]`, but `serde` is still not a dependency of
  `talaria-shell` and the phase's prohibitions forbid adding one. `to_json` /
  `from_json` through `serde_json::json!` and `Value`, sitting next to each
  other, exactly as `history.rs` and `bookmarks.rs` do. Unlike those two, both
  fields here are load-bearing: an engine with no template cannot search and
  one with no name cannot be shown, so a half-present object is discarded
  rather than patched.
- **The Settings draft is pre-filled and refilled, not cleared.** The
  credentials form clears on save because it is an *add* form; a settings form
  shows what is configured. `settings_draft_stale` is set by `set_panel` and
  cleared on the first frame the panel draws, so reopening always shows what is
  actually saved rather than what someone abandoned mid-edit — and typing is
  never overwritten by the next frame's prefill.
- **The template is not trimmed on save; the name is.** A leading space in a
  URL template is a typo that `Url::parse` will catch and the fallback will
  degrade; silently editing what someone typed into a URL is worse than
  refusing it. A name is display text, where trimming is unambiguous.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] The plan's `Serialize`/`Deserialize` derives cannot compile**
- **Found during:** Task 1
- **Issue:** `03-03-PLAN.md`'s `<action>` specifies
  `#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]` on both new
  structs. `serde` is not a dependency of `talaria-shell` (only `serde_json`
  is), and the phase prohibits adding one or touching `Cargo.lock`. This is
  03-01's finding, restated by 03-02, and the plan's own `<interfaces>` section
  did not carry it forward.
- **Fix:** Hand-mapped `to_json`/`from_json` through `serde_json::json!` and
  `Value`, matching `history.rs` and `bookmarks.rs` verbatim in shape and in
  the doc comment explaining why. `PartialEq`/`Eq` were kept — they are std
  derives and the tests lean on them.
- **Files modified:** `crates/talaria-shell/src/settings.rs`
- **Verification:** `git diff --stat Cargo.toml Cargo.lock` is empty; 14 unit
  tests pass.
- **Committed in:** `92284e7`

---

**2. [Rule 1 - Bug] The GEAR button broke `vault_ui_test` again**
- **Found during:** Task 3
- **Issue:** `tests/e2e/vault_ui_test.py` opens the credentials panel by
  clicking a hardcoded logical point. Inserting the GEAR button ahead of the
  credentials button moves it, so the old point (`341`) would have landed on
  GEAR and opened the Settings panel instead. A **known, documented cost**,
  recorded in `deferred-items.md` by 03-01 and re-flagged by 03-02.
- **Fix:** Measured, not guessed, exactly as `deferred-items.md` prescribes:
  temporarily bound the credentials button's `Response` in `gui.rs`,
  `log::warn!`ed its `.rect` under Xvfb, which reported
  `[[359.3 2.0] - [380.3 20.0]]`; took the centre of the x span; removed the
  instrumentation and confirmed with `grep -c MEASURE` that none survived.
  Updated the constant to `("370", "11")` and its comment with the third move
  and the per-button rate (~29 points) so 03-04 can check its own arithmetic
  before running the suite.
- **Files modified:** `tests/e2e/vault_ui_test.py`
- **Verification:** `vault_ui_test` PASS in the full run (42s).
- **Committed in:** `e45d528`

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 bug)
**Impact on plan:** None on scope. One repairs a regression this plan caused in
an unrelated suite and was anticipated in writing beforehand; the other is a
known constraint the plan's prose had not inherited. No architectural decision
was needed and no Rule 4 checkpoint was reached.

## Issues Encountered

- **Task 2's clippy gate could not pass at its own boundary.** Task 2's
  `<verify>` includes `cargo clippy --all-targets -- -D warnings`, but after
  Task 2 `Settings::save`, `SearchEngine::to_json` and `Settings::path` had no
  caller — the Settings panel that calls them is Task 3. Three `dead_code`
  errors, all on `settings.rs` items. This is 03-01's recorded shape exactly.
  Resolved by finishing Task 3 rather than papering over it: **no
  `#[allow(dead_code)]` was added at any point**, and clippy is clean from
  `e45d528` onward. Recorded rather than silently skipped.
- **`git diff`'s hunk header names the wrong function.** The hunk containing
  `resolve_location`'s signature change is labelled
  `@@ ... fn parse_agent_url(input: &str) -> Result<Url, String> {`, because
  git labels a hunk with the nearest preceding function definition. Reading
  that as "the diff touches `parse_agent_url`" would have been a false alarm on
  this plan's single load-bearing constraint. The check that actually answers
  it is extracting both function bodies with `awk` from the working tree and
  from `HEAD~2` and diffing them — both reported IDENTICAL.
- **Measuring the toolbar rect needed a foreground `timeout` run.** The first
  attempt backgrounded the shell and used `pkill` to clean up, which killed the
  measuring shell itself (exit 144) before the log was read. `setsid` for Xvfb
  plus `timeout 12` on the binary in the foreground works, and is what
  `deferred-items.md` should say if this is done a fourth time.

## Verification

| Check | Result |
|---|---|
| `cargo build --release` | exit 0 |
| `cargo clippy --all-targets -- -D warnings` | exit 0 |
| `cargo test` | 67 passed, 0 failed (14 settings, 14 bookmarks, 17 history, 7 gui, 12 vault, 5 app) + 2 protocol |
| `cargo test -p talaria-shell settings::tests` | 14 passed, 0 failed |
| `cargo test -p talaria-shell app::tests` | 5 passed, 0 failed |
| `python3 tests/e2e/run_all.py` | 17/17 PASS, `failed: none` |
| `git diff --stat Cargo.toml Cargo.lock` | no change |
| `git diff --stat crates/talaria-mcp` | no change |
| `grep -c '"search_engine\|"config' crates/talaria-mcp/src/tools.rs` | 0 |
| `parse_agent_url` / `agent_scheme_allowed` bodies vs. pre-plan | IDENTICAL (both) |
| `grep -c 'unwrap()' crates/talaria-shell/src/settings.rs` | 0 |
| `grep -c 'unwrap()' crates/talaria-shell/src/app.rs` | 0 (unchanged from baseline) |
| `grep -c 'MEASURE' crates/talaria-shell/src/gui.rs` | 0 (instrumentation removed) |

The full e2e run used a private runtime dir, as CI does:
`XDG_RUNTIME_DIR=/tmp/tal-e2e-rt TALARIA_E2E_DISPLAY=:98 TALARIA_E2E_OUT=/tmp/tal-e2e-out`.
No `dbus-run-session` wrapper was needed.

### The UI-SPEC backstop item, resolved with evidence

`03-UI-SPEC.md`'s `## UI Considerations` carried exactly one `🧪 backstop` row
and it belonged to this plan: long custom template text in the Settings field.
`TextEdit::singleline` was *expected* to scroll its content horizontally rather
than grow the 320pt field, but the spec recorded that nothing had rendered a
pathological input.

Now measured, not assumed. `a_long_template_scrolls_inside_the_field_rather_than_widening_it`
lays out the real field shape — `TextEdit::singleline(..).desired_width(320.0)`
inside the same `ui.horizontal` the panel uses — through `egui::__run_test_ui`,
egui's own headless harness, with a 4,000-character no-whitespace template and
an ordinary 33-character one as a control:

| Input | Field width |
|---|---|
| `https://duckduckgo.com/?q={query}` (33 chars) | 320.0 pt |
| `https://example.com/search?q={query}&filter=aaa…` (4,036 chars) | 320.0 pt |
| Available canvas | 10,000 pt |

Identical to the decimal, with 9,680 points of room the field declined to take.
Content length does not reach the layout at all. The test asserts both the
equality and the literal 320.0 (rather than a `<= available` bound, which the
harness's deliberately huge canvas would make near-vacuous), so a future egui
upgrade that changed this would fail rather than pass quietly. **This item can
be marked resolved in `03-UI-SPEC.md` — with a number, not a promise.**

## Threat Model Outcomes

| Threat ID | Disposition | Outcome |
|---|---|---|
| T-03-03-01 (hand-edited or corrupted `config.json`, incl. a hostile `url_template`) | mitigated | Two layers. `Settings::load` validates the template and discards the whole engine on any failure — unparseable JSON, wrong shape, missing fields, or a template without `{query}` exactly once — with one `log::warn!` and no panic; `resolve_location` keeps an independent `unwrap_or_else` for a template that never passed through `load`. Seven unit tests across the two layers. A `javascript:`- or `file:`-scheme template does not survive `Url::parse` of the substituted string in any form the address bar would navigate |
| T-03-03-02 (`resolve_location` and `parse_agent_url` collapsed into one path) | mitigated | Both agent-path function bodies extracted and diffed against the pre-plan revision: IDENTICAL. No new parameter, no call into `settings`, no call site changed. `scheme_refusal_test` PASS |
| T-03-03-03 (a future MCP tool exposing search-engine configuration) | mitigated | `crates/talaria-mcp/` is untouched by this plan's diff; the negative grep returns 0. The engine is reachable only from the toolbar button — no control-socket command, no keyboard shortcut, no tool |
| Information disclosure — `config.json` reveals the chosen engine | accepted | Unchanged from the plan's stated posture, and the same posture `history.jsonl` and `bookmarks.json` already have |
| T-03-03-SC (package installs) | accepted | No dependency added; `Cargo.toml`/`Cargo.lock` diff is empty |

## Threat Flags

None. This plan adds no network endpoint, no auth path and no schema at a trust
boundary. `config.json` is a new local file under the existing config directory,
which T-03-03-01 already covers.

One thing worth naming rather than flagging: `CLAUDE.md`'s Configuration
section states "No `.env` files, no config file format. All configuration is
environment variables plus CLI argv." **That is no longer true** — this plan
introduces `dirs::config_dir()/talaria/config.json`. The crossing is
deliberate (a search engine is a preference a person sets once and expects to
persist, which an environment variable cannot be), the file is kept to the
smallest possible shape, and `settings.rs`'s module header names the boundary
so it does not read as an oversight. `CLAUDE.md` should be updated when the
phase closes.

## Known Stubs

None. The panel reads the live store on open, the Save gate calls the same
predicate the loader does, and `resolve_location` reads `Shared::settings` at
navigation time.

## User Setup Required

None — no external service configuration required. An existing install picks up
the default engine with no migration and no `config.json` on disk until the
first save.

## Next Phase Readiness

Ready for `03-04` (downloads). What it inherits:

- `ChromePanel` now has five variants and the mechanism is still unchanged
  across three consecutive plans. Adding `Downloads` remains one variant, one
  toolbar button and one `else if` arm.
- `settings.rs` is the worked example for a **single-object** store; `bookmarks.rs`
  remains the one for a whole-array store. `downloads.rs` wants the latter.
- **Expect to move `vault_ui_test.py`'s `CREDENTIALS_BUTTON` one last time.**
  It is now `370`, its comment carries all three moves and both measured rects,
  and the observed rate is ~29 points per icon button — so `399` is the number
  to check a measurement against, not to assume. `deferred-items.md` still holds
  the two candidate permanent fixes, neither of which got cheaper.
- The `data:`-URL item is **closed**, with its reasoning in
  `deferred-items.md` and in a test. It should not be rediscovered a fourth
  time.
- `03-UI-SPEC.md`'s backstop count can go from `1 backstop` to `0` — the
  measurement is in `gui.rs`.

No blockers.

---
*Phase: 03-table-stakes-browsing*
*Completed: 2026-08-20*

## Self-Check: PASSED

Both claimed key files exist on disk, and all five claimed commits
(`54eaaae`, `92284e7`, `ecd3c88`, `d45ec85`, `e45d528`) are reachable in git
history.

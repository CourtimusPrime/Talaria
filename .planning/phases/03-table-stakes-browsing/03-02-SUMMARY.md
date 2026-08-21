---
phase: 03-table-stakes-browsing
plan: 02
subsystem: ui
tags: [rust, egui, servo, persistence, json, bookmarks]

requires:
  - phase: 01-foundation
    provides: the egui-on-winit shell, the tab table and its Me/Agent ownership split
  - phase: 02-harden-the-agent-surface
    provides: vault.rs's whole-array CRUD shape and its degrade-never-abort load posture
  - phase: 03-table-stakes-browsing
    plan: 01
    provides: ChromePanel, UiAction::SetPanel, Gui::panel/panel_open/set_panel, truncate_chars, now_ms, the hand-mapped serde_json idiom
provides:
  - "crates/talaria-shell/src/bookmarks.rs — a whole-array (url, title, created_at_ms) store whose every mutation is an atomic temp+rename write"
  - "ChromePanel::Bookmarks and the Bookmarks list panel"
  - "UiAction::ToggleBookmark / UiAction::RemoveBookmark — the add-or-remove decision resolved in apply_ui_actions, never in the egui closure"
  - "The toolbar bookmark star (Ctrl+D) and Bookmarks-list button (Ctrl+B)"
  - "tests/e2e/bookmarks_test.py — xdotool-driven toggle and restart survival"
affects: [03-03-search-engine, 03-04-downloads]

tech-stack:
  added: []
  patterns:
    - "Whole-array store with an atomic write: every mutation re-serialises the list into a .tmp sibling and fs::renames it over the target"
    - "A store method that deliberately refuses to be a toggle, so the add-or-remove decision lives at the single call site that owns the state"

key-files:
  created:
    - crates/talaria-shell/src/bookmarks.rs
    - tests/e2e/bookmarks_test.py
  modified:
    - crates/talaria-shell/src/app.rs
    - crates/talaria-shell/src/gui.rs
    - crates/talaria-shell/src/main.rs
    - tests/e2e/run_all.py
    - tests/e2e/vault_ui_test.py

key-decisions:
  - "Bookmark serialisation is hand-mapped through serde_json::json!/Value, inheriting 03-01's finding that serde is not a dependency of talaria-shell and the phase's prohibitions forbid adding one"
  - "Bookmarks::upsert is an add-if-absent, never an overwrite: it refuses to be the toggle so a re-press cannot silently rewrite the title of a bookmark the user meant to remove"
  - "Every mutation goes through .tmp + fs::rename — unlike history.rs, which reserves that for its prune, because here every write is already a whole-file rewrite"
  - "The star emits the same UiAction::ToggleBookmark in both states; apply_ui_actions resolves add vs remove against the live store, so the decision is never made from a one-frame-old read inside an egui closure"
  - "The panel reverses for display only; the store keeps insertion order, so two bookmarks made in the same millisecond are not reordered"
  - "vault_ui_test.py's CREDENTIALS_BUTTON was measured (283 -> 341) from the button's own rect rather than guessed"

patterns-established:
  - "Atomic whole-array store: serialise, stage beside the target, rename; a stale .tmp is never read as data"
  - "A toolbar toggle whose .selected() state reads a store every frame through a shared borrow, while the write stays in apply_ui_actions"

requirements-completed: [BROWSE-02]

coverage:
  - id: D1
    description: "Ctrl+D (and the toolbar star, same UiAction) bookmarks the displayed page with its URL, title and a timestamp"
    requirement: "BROWSE-02"
    verification:
      - kind: e2e
        ref: "tests/e2e/bookmarks_test.py#BOOKMARKED"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/bookmarks.rs#an_upserted_bookmark_survives_a_reload"
        status: pass
    human_judgment: false
  - id: D2
    description: "The toggle is a true toggle: a second activation removes, a third re-adds, and the same URL is never stored twice"
    requirement: "BROWSE-02"
    verification:
      - kind: e2e
        ref: "tests/e2e/bookmarks_test.py#TOGGLED off / TOGGLED back on"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/bookmarks.rs#upserting_the_same_url_twice_leaves_exactly_one_entry, #is_bookmarked_follows_upsert_and_remove"
        status: pass
    human_judgment: false
  - id: D3
    description: "A bookmark set from the keyboard survives a full shell restart against the same config directory"
    requirement: "BROWSE-02"
    verification:
      - kind: e2e
        ref: "tests/e2e/bookmarks_test.py#RESTART the bookmark survived"
        status: pass
    human_judgment: false
  - id: D4
    description: "An empty, missing, corrupted or wrong-shaped bookmarks.json degrades to an empty list with a warning, never a panic or a blocked startup"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/bookmarks.rs#a_missing_file_loads_as_an_empty_store, #an_empty_file_loads_as_an_empty_store, #a_corrupt_file_loads_as_an_empty_store_without_panicking, #a_file_of_the_wrong_shape_loads_as_an_empty_store, #an_entry_with_no_url_is_skipped_and_the_good_ones_survive"
        status: pass
    human_judgment: false
  - id: D5
    description: "The target file only ever changes through a single fs::rename, so a crash mid-save cannot leave it half-written (T-03-02-01)"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/bookmarks.rs#a_save_stages_through_a_temp_file_and_leaves_none_behind, #a_stale_staging_file_does_not_block_the_next_save, #a_save_replaces_the_whole_array_rather_than_appending_to_it"
        status: pass
    human_judgment: false
  - id: D6
    description: "Removal reports whether it removed anything and writes only when it did"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/bookmarks.rs#removing_an_absent_url_reports_false_and_writes_nothing, #removing_a_present_url_reports_true_and_does_not_survive_a_reload"
        status: pass
    human_judgment: false
  - id: D7
    description: "Ctrl+B opens the Bookmarks panel without crashing or wedging the shell"
    verification:
      - kind: e2e
        ref: "tests/e2e/bookmarks_test.py#PANEL Ctrl+B"
        status: pass
    human_judgment: false
  - id: D8
    description: "The Bookmarks panel lists newest-first, truncates long text with the URL on hover, navigates on a row click and closes, and removes per row with no confirmation"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/gui.rs#truncation_counts_characters_not_bytes (the row-label mechanism, shared with the History panel)"
        status: pass
    human_judgment: true
    rationale: "The panel's rendered layout, the newest-first order on screen, the click-to-navigate round trip and the Trash button are driven entirely from the chrome. The e2e suite proves the panel opens and does not wedge the shell, and the store behind it is unit- and e2e-tested, but nothing automated opens the panel and clicks a row. This inherits 03-01's D7 rationale exactly — same mechanism, same gap, and 03-VALIDATION.md already scopes the visual check to a human."
  - id: D9
    description: "No MCP tool surface changed — bookmarks are not reachable by an agent (T-03-02-03)"
    verification:
      - kind: other
        ref: "git diff --stat crates/talaria-mcp reports no change; grep for bookmark tool names returns 0"
        status: pass
    human_judgment: false

duration: 22 min
completed: 2026-08-20
status: complete
---

# Phase 03 Plan 02: Bookmarks Summary

**A plaintext `bookmarks.json` whose every mutation lands through a temp file and an `fs::rename`, driven by a toolbar star (Ctrl+D) whose add-or-remove decision is resolved in `apply_ui_actions` rather than in the egui closure that drew it, and listed in a fourth `ChromePanel` variant.**

## Performance

- **Duration:** 22 min
- **Started:** 2026-08-20T07:48:15Z
- **Completed:** 2026-08-20T08:10:16Z
- **Tasks:** 3
- **Files modified:** 7 (2 created, 5 modified)

## Accomplishments

- A page can be bookmarked and un-bookmarked from the toolbar star or Ctrl+D, never duplicating an entry, and the bookmark survives a full restart — BROWSE-02's literal success criterion, proven through the real binary driven by `xdotool`.
- `bookmarks.rs` is the first store in this codebase where **every** write is atomic: the whole array is staged into a `.tmp` sibling and renamed over the target, so T-03-02-01 (a crash mid-rewrite) cannot leave a half-written file.
- `ChromePanel` grew its third real variant with no change to the mechanism 03-01 built — no new panel plumbing, no new state-reset hook, no change to `panel_open()`'s mouse-routing gate. The marginal cost of a panel is now genuinely one enum variant, one toolbar button and one `else if` arm.
- The e2e suite grew from 16 registered suites to 17, all green.
- The MCP surface is untouched: `git diff --stat crates/talaria-mcp` is empty across this plan.

## Task Commits

1. **Task 1 (RED): failing tests for the bookmarks store** — `963c91b` (test)
2. **Task 1 (GREEN): the atomic whole-array bookmarks store** — `0e70903` (feat)
3. **Task 2: the star, the panel, `ChromePanel::Bookmarks`, Ctrl+D and Ctrl+B** — `27fc85a` (feat)
4. **Task 3: the xdotool e2e proof and its registration** — `969d3a5` (test)

_Task 1 is `tdd="true"`, so it carries the test → feat pair (RED: 14 failing, 0 passing; GREEN: 14 passing). No refactor commit was needed — the GREEN implementation needed no cleanup pass._

## Files Created/Modified

- `crates/talaria-shell/src/bookmarks.rs` (new, 452 lines) — `Bookmark`, `Bookmarks` (`load`/`is_bookmarked`/`upsert`/`remove`/`entries`), the private `config_dir()`, the atomic `save()`, and 14 unit tests.
- `crates/talaria-shell/src/app.rs` — `Shared::bookmarks`, the `Bookmarks::load()` call site, the `ToggleBookmark`/`RemoveBookmark` arms of `apply_ui_actions`, and the `"b"`/`"d"` arms of `handle_browser_shortcut`.
- `crates/talaria-shell/src/gui.rs` — `ChromePanel::Bookmarks`, `UiAction::ToggleBookmark`, `UiAction::RemoveBookmark`, the `current_page` local, the `STAR` and `BOOKMARKS` toolbar buttons, and the Bookmarks panel body.
- `crates/talaria-shell/src/main.rs` — `mod bookmarks;`.
- `tests/e2e/bookmarks_test.py` (new, 251 lines) — the toggle / panel-shortcut / restart suite.
- `tests/e2e/run_all.py` — `bookmarks_test` registered in the standalone-shell tuple.
- `tests/e2e/vault_ui_test.py` — `CREDENTIALS_BUTTON` moved 283 → 341 (see deviations).

## Decisions Made

- **`upsert` deliberately refuses to be the toggle.** It adds when absent and
  does nothing at all when present — it does not overwrite. That is what makes
  the caller's `is_bookmarked` → `remove`-or-`upsert` branch in
  `apply_ui_actions` the single place the toggle exists. An `upsert` that
  overwrote would silently rewrite the title of a bookmark the user was in the
  middle of removing, and it would make the store's behaviour depend on which
  of two call sites reached it first.
- **The toggle is resolved at apply time, not at draw time.** The star draws
  from a read the frame *before* the click, so deciding "this is a remove" in
  the egui closure would both break the no-inline-mutation rule and act on a
  stale answer. The button emits the same `ToggleBookmark(url, title)` in
  either state; `apply_ui_actions` re-reads the store and decides.
- **Every mutation is atomic, which is the deliberate divergence from
  `history.rs`.** 03-01 argued *against* uniform temp+rename because its write
  was a one-line append that a rename would have turned into an O(n) rewrite.
  Bookmarks has no such tension: every mutation is already a whole-array
  rewrite, so the rename costs nothing and buys the T-03-02-01 mitigation.
  Both modules' doc comments now say why they differ, so neither reads as an
  oversight.
- **The staging file is a sibling of the target, not a file in the temp
  directory.** `fs::rename` is only atomic within one filesystem, and a config
  directory on a different mount than `/tmp` is an ordinary setup. A test
  (`a_stale_staging_file_does_not_block_the_next_save`) pins that a leftover
  `.tmp` from a killed process is never read as data and never blocks the next
  save.
- **Serialisation is hand-mapped, inherited from 03-01.** `serde` is still not
  a dependency of `talaria-shell` and the phase's prohibitions still forbid
  changing `Cargo.lock`, so `Bookmark::to_json`/`from_json` map through
  `serde_json::json!` and `Value`, sitting next to each other. Only `url` is
  load-bearing: an entry with no URL names no page and is skipped; a missing
  `title`/`created_at_ms` degrades rather than discarding a real bookmark.
- **The panel reverses for display only.** Same call 03-01 made: the store
  stays in insertion order, so two bookmarks created in the same millisecond
  keep the order the human made them in, which a sort on the timestamp could
  not guarantee.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] The two new toolbar buttons broke `vault_ui_test` again**
- **Found during:** Task 2
- **Issue:** `tests/e2e/vault_ui_test.py` opens the credentials panel by
  clicking a hardcoded logical point, `CREDENTIALS_BUTTON`. This plan inserts
  the `STAR` and `BOOKMARKS` buttons ahead of the credentials button, so the
  old point (`283`) now lands on the bookmark star — which would have
  bookmarked `about:blank` and then failed the suite's first assertion. This
  was a **known, documented cost**, recorded in `deferred-items.md` by 03-01
  rather than rediscovered here.
- **Fix:** Measured rather than guessed, exactly as `deferred-items.md`
  prescribes: temporarily bound the credentials button's `Response` in
  `gui.rs` and `log::warn!`ed its `.rect` under Xvfb, which reported
  `[[330.3 2.0] - [351.3 20.0]]`; took the centre of the x span; removed the
  instrumentation. Updated the constant to `("341", "11")` and rewrote its
  comment to record the new control order, both moves (239 → 283 → 341), and
  the exact rect the 341 came from so the next plan can check its own
  arithmetic against a worked example.
- **Files modified:** `tests/e2e/vault_ui_test.py`
- **Verification:** `vault_ui_test` PASS standalone and in the full run.
- **Committed in:** `27fc85a`

---

**Total deviations:** 1 auto-fixed (1 bug)
**Impact on plan:** None on scope. The one deviation repairs a regression this
plan caused in an unrelated suite, and it was anticipated in writing before
execution started. No architectural decision was needed and no Rule 4
checkpoint was reached.

## Issues Encountered

- **No clippy sequencing problem this time.** 03-01 recorded that its Task 2
  could not pass `-D warnings` at its own boundary, because a store method had
  no caller until the panel task landed. The same shape existed here — after
  Task 1, every method on `Bookmarks` was dead code — but Task 1's gate is
  `cargo build --release && cargo test` (warnings, not errors), so it passed
  cleanly, and clippy was green from Task 2 onward. The plan's own task
  sequencing avoided the problem rather than papering over it; no
  `#[allow(dead_code)]` was added at any point.
- **`cargo test`'s total is 47, not the 49 a naive sum predicts.** 03-01's
  summary reported 35 pre-existing tests ("17 history, 6 gui, 12 vault"); the
  suite actually reports 33 in the binary plus 2 in `talaria-protocol`'s own
  target. 33 + 14 = 47 in the shell binary, plus the protocol's 2. A counting
  artifact in the earlier summary, not a lost test.

## Verification

| Check | Result |
|---|---|
| `cargo build --release` | exit 0 |
| `cargo clippy --all-targets -- -D warnings` | exit 0 |
| `cargo test` | 47 passed, 0 failed (14 bookmarks, 17 history, 6 gui, 12 vault) + 2 protocol |
| `cargo test -p talaria-shell bookmarks::tests` | 14 passed, 0 failed |
| `python3 tests/e2e/bookmarks_test.py` | exit 0, `BOOKMARKS CHECKS PASSED` |
| `python3 tests/e2e/run_all.py` | 17/17 PASS, `failed: none` |
| `git diff --stat Cargo.toml Cargo.lock` | no change |
| `git diff --stat crates/talaria-mcp` | no change |
| `grep -c 'downloads_open\|downloads_list\|"bookmark' crates/talaria-mcp/src/tools.rs` | 0 |

The full run used a private runtime dir, as CI does:
`XDG_RUNTIME_DIR=/tmp/tal-e2e-rt TALARIA_E2E_DISPLAY=:98 TALARIA_E2E_OUT=/tmp/tal-e2e-out`.
No `dbus-run-session` wrapper was needed.

## Threat Model Outcomes

| Threat ID | Disposition | Outcome |
|---|---|---|
| T-03-02-01 (crash mid-`save()` corrupting `bookmarks.json`) | mitigated | Every mutation stages into a `.tmp` sibling and lands with one `fs::rename`; three unit tests pin the staging file's absence after a save, a stale one being ignored, and the whole-array replacement |
| T-03-02-02 (hand-edited or corrupted `bookmarks.json`) | mitigated | Unparseable, wrong-shaped, and per-entry-broken files all degrade to what is readable with one `log::warn!`; four unit tests, no panic, no blocked startup |
| T-03-02-03 (a future MCP tool exposing bookmarks to an agent) | mitigated | `crates/talaria-mcp/` is untouched by this plan's diff; the negative grep returns 0. The star and the panel are reachable only from the toolbar and two keystrokes — no control-socket command reaches either |
| Plaintext bookmarks on disk (low) | accepted | Unchanged from the plan's stated posture, and the same posture history already has |
| T-03-02-SC (package installs) | accepted | No dependency added; `Cargo.toml`/`Cargo.lock` diff is empty |

## Threat Flags

None. This plan adds no network endpoint, no auth path and no schema at a
trust boundary. `bookmarks.json` is a new local file under the existing config
directory, which the register above already covers.

## Known Stubs

None. The star reads the live store every frame, the panel lists real entries,
and the e2e suite drives the real binary end to end.

## User Setup Required

None — no external service configuration required.

## Next Phase Readiness

Ready for `03-03` (search engine). What it inherits:

- `ChromePanel` now has four variants and the mechanism is unchanged; adding
  `Settings` is one variant, one toolbar button and one `else if` arm.
- `bookmarks.rs` is the worked example of the **atomic whole-array store** —
  the shape `downloads.rs` and `settings.rs` should both copy, rather than
  `vault.rs`'s plain `fs::write`.
- **Expect to move `vault_ui_test.py`'s `CREDENTIALS_BUTTON` again.** 03-03 and
  03-04 each add a button ahead of Credentials. The constant is now `341` and
  its comment carries the measured rect that produced it, so the next move can
  be checked rather than guessed. `deferred-items.md` still holds the two
  candidate permanent fixes.
- `resolve_location`'s `data:`-URL mishandling is still open and is 03-03's to
  decide, per `deferred-items.md`.

No blockers.

---
*Phase: 03-table-stakes-browsing*
*Completed: 2026-08-20*

## Self-Check: PASSED

Both claimed key files exist on disk, and all four claimed commits
(`963c91b`, `0e70903`, `27fc85a`, `969d3a5`) are reachable in git history.

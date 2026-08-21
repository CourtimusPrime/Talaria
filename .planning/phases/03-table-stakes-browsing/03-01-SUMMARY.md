---
phase: 03-table-stakes-browsing
plan: 01
subsystem: ui
tags: [rust, egui, servo, persistence, jsonl, history]

requires:
  - phase: 01-foundation
    provides: the egui-on-winit shell, the tab table and its Me/Agent ownership split
  - phase: 02-harden-the-agent-surface
    provides: the credentials panel, whose credentials_open bool this plan generalised, and vault.rs's degrade-never-abort store shape
provides:
  - "crates/talaria-shell/src/history.rs — an append-only (url, title, timestamp) store with a retention cap and atomic prune"
  - "pending_history_writes: the deferred-queue idiom applied to a Servo delegate callback that needs to reach a plain-file store"
  - "ChromePanel: the panel-selection enum that replaces the single credentials_open bool, extended one variant per plan for the rest of Phase 3"
  - "UiAction::SetPanel / UiAction::ClearHistory"
  - "tests/e2e/history_test.py — restart survival, the Me-only filter, and the standing pushState probe"
affects: [03-02-bookmarks, 03-03-search-engine, 03-04-downloads]

tech-stack:
  added: []
  patterns:
    - "Append-log store (one JSON line per write) as the alternative to vault.rs's whole-array rewrite, for any store written per-navigation"
    - "Temp-file-plus-fs::rename for the one whole-file write a store still needs"
    - "ChromePanel + UiAction::SetPanel as the panel mechanism every later chrome surface extends"

key-files:
  created:
    - crates/talaria-shell/src/history.rs
    - tests/e2e/history_test.py
    - .planning/phases/03-table-stakes-browsing/deferred-items.md
  modified:
    - crates/talaria-shell/src/app.rs
    - crates/talaria-shell/src/gui.rs
    - crates/talaria-shell/src/main.rs
    - tests/e2e/run_all.py
    - tests/e2e/vault_ui_test.py

key-decisions:
  - "History serialisation is hand-mapped through serde_json::json! rather than a Serialize derive, because serde is not a dependency of talaria-shell and this plan's prohibitions forbid adding one"
  - "Servo 0.4.0 does not re-fire LoadStatus::Complete for an in-page pushState — measured, not assumed; SPA route changes are the accepted v1 gap"
  - "History::append writes one line with no temp+rename; only the retention prune rewrites the file, at most once per startup"
  - "Row labels truncate by character, not by String::truncate's byte index, which panics mid-character on a multibyte title"
  - "The History panel reverses only for display; the store stays in append order, so two visits sharing a millisecond keep their real order"

patterns-established:
  - "Append-log store: History::append writes exactly one JSON line; load() skips unparseable lines and warns once with the count"
  - "Panel-local view state resets in Gui::set_panel on every switch, so no half-pressed confirm survives a panel close"
  - "An e2e step whose answer is unknown reports rather than asserts, and keeps reporting on every run"

requirements-completed: [BROWSE-01]

coverage:
  - id: D1
    description: "A completed navigation in a Me-owned tab is recorded with URL, title and timestamp"
    requirement: "BROWSE-01"
    verification:
      - kind: e2e
        ref: "tests/e2e/history_test.py#RECORDED"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/history.rs#an_appended_entry_survives_a_reload"
        status: pass
    human_judgment: false
  - id: D2
    description: "Recorded history survives a full shell restart against the same config directory"
    requirement: "BROWSE-01"
    verification:
      - kind: e2e
        ref: "tests/e2e/history_test.py#RESTART both rows survived"
        status: pass
    human_judgment: false
  - id: D3
    description: "An agent-owned tab's navigation produces no history row"
    requirement: "BROWSE-01"
    verification:
      - kind: e2e
        ref: "tests/e2e/history_test.py#AGENT tab recorded nothing"
        status: pass
    human_judgment: false
  - id: D4
    description: "A revisit appends a second row rather than merging into the first"
    requirement: "BROWSE-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/history.rs#the_same_url_twice_is_two_entries_not_one_updated_one"
        status: pass
      - kind: e2e
        ref: "tests/e2e/history_test.py#APPENDED second row, in visit order"
        status: pass
    human_judgment: false
  - id: D5
    description: "An empty, missing or corrupted history.jsonl degrades to an empty list with a warning, never a panic or a blocked startup"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/history.rs#a_missing_file_loads_as_an_empty_store, #a_corrupt_line_is_skipped_and_the_good_ones_survive, #a_truncated_trailing_line_costs_only_that_line, #an_entirely_unreadable_file_loads_as_an_empty_store"
        status: pass
    human_judgment: false
  - id: D6
    description: "Entries past the retention cap (default 5,000, TALARIA_HISTORY_MAX_ENTRIES override) are pruned from disk at load through a temp file plus fs::rename"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/history.rs#a_store_over_the_cap_keeps_only_the_most_recent_entries, #the_cap_prune_shrinks_the_file_too, #a_store_under_the_cap_is_left_alone, #an_unparseable_cap_falls_back_to_the_default_not_to_zero"
        status: pass
    human_judgment: false
  - id: D7
    description: "The History panel opens from the toolbar and Ctrl+H, lists newest-first, navigates on click and closes, truncates long text with the URL on hover, and clears only on a second confirming click"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/gui.rs#truncation_counts_characters_not_bytes, #older_visits_read_in_the_largest_unit_that_fits"
        status: pass
    human_judgment: true
    rationale: "The panel's rendered layout, the newest-first order on screen, the click-to-navigate round trip and the two-stage clear are driven entirely from the chrome; no automated check in this plan opens the panel and exercises them. The helper functions behind the row label and timestamp are unit-tested, the store behind it is e2e-tested, but the panel itself needs a human to look at it."
  - id: D8
    description: "No MCP tool surface changed — history is not reachable by an agent"
    verification:
      - kind: other
        ref: "git diff --stat crates/talaria-mcp/src/tools.rs reports no change; grep for history/bookmarks/downloads tool names returns 0"
        status: pass
    human_judgment: false
  - id: D9
    description: "Servo 0.4.0's actual pushState/same-document callback behaviour, measured under the e2e harness (03-RESEARCH.md Open Question 1)"
    verification:
      - kind: e2e
        ref: "tests/e2e/history_test.py#PUSHSTATE PROBE"
        status: pass
    human_judgment: true
    rationale: "The probe is an observation, not an assertion — by design, since the answer was unknown. It reports a finding on every run and asserts only that no row was lost and the shell still answers. A human should read the finding (recorded below) rather than treat a green suite as agreement with any particular outcome."

duration: 34 min
completed: 2026-08-20
status: complete
---

# Phase 03 Plan 01: Local Browsing History Summary

**An append-only `history.jsonl` store fed from a deferred `notify_load_status_changed` queue, filtered to the human's own tabs, browsable from a new History panel behind the `ChromePanel` generalisation that the rest of Phase 3 extends.**

## Performance

- **Duration:** 34 min
- **Started:** 2026-08-20T07:08:04Z
- **Completed:** 2026-08-20T07:42:58Z
- **Tasks:** 3
- **Files modified:** 8 (3 created, 5 modified)

## Accomplishments

- Every completed navigation in a Me-owned tab is recorded as a `(url, title, visited_at_ms)` row and survives a restart — BROWSE-01's literal success criterion, proven end to end through the real binary.
- An agent-owned tab's navigation records nothing, asserted directly by a negative e2e step rather than inferred from silence.
- The single `credentials_open: bool` became a real `ChromePanel` enum with a `UiAction::SetPanel` round trip, which is the substrate 03-02/03-03/03-04 each extend by one variant.
- `03-RESEARCH.md`'s Open Question 1 was answered by measurement: Servo 0.4.0 does **not** re-fire `LoadStatus::Complete` for an in-page `pushState`.
- The e2e suite grew from 15 registered suites to 16, all green.

## Task Commits

1. **Task 1 (RED): failing tests for the history store** — `54baaa3` (test)
2. **Task 1 (GREEN): the append-only history store** — `d9c89f5` (feat)
3. **Task 2: deferred capture queue wired into `notify_load_status_changed`** — `9c7a8f6` (feat)
4. **Task 3: ChromePanel, the History panel, Ctrl+H, Clear history, and the e2e proof** — `8e08c46` (feat)

_Task 1 is `tdd="true"`, so it carries the test → feat pair. No refactor commit was needed: the GREEN implementation needed no cleanup pass._

## Files Created/Modified

- `crates/talaria-shell/src/history.rs` (new, 491 lines) — `HistoryEntry`, `History` (`load`/`append`/`clear`/`entries`), private `config_dir()` and `history_max_entries()`, the atomic cap prune, and 17 unit tests.
- `crates/talaria-shell/src/app.rs` — `Shared::history`, `Shared::pending_history_writes`, `HistoryWrite`, `now_ms()`, `process_pending_history_writes()` and its three drain call sites, the `Complete`-arm push, `toggle_panel()`, the `ChromePanel` handling in `chrome_replaces_page` / `handle_browser_shortcut` / `apply_ui_actions`.
- `crates/talaria-shell/src/gui.rs` — `ChromePanel`, `UiAction::SetPanel`, `UiAction::ClearHistory`, `Gui::panel`/`panel_open`/`set_panel`, the History toolbar button, the History panel body, `truncate_chars`, `relative_time`, and a new `#[cfg(test)] mod tests` (6 tests).
- `crates/talaria-shell/src/main.rs` — `mod history;`.
- `tests/e2e/history_test.py` (new, 246 lines) — the capture/append/filter/restart/probe suite.
- `tests/e2e/run_all.py` — `history_test` registered in the standalone-shell tuple.
- `tests/e2e/vault_ui_test.py` — `CREDENTIALS_BUTTON` moved 239 → 283 (see deviations).
- `.planning/phases/03-table-stakes-browsing/deferred-items.md` (new) — three out-of-scope findings.

## The pushState finding (RESEARCH.md Open Question 1 — RESOLVED by measurement)

`tests/e2e/history_test.py`'s final step opens a Me-owned tab on a local page,
records the history row count, runs `history.pushState(null, '', '#probed')`
through the raw `evaluate` command, waits, and counts again. The measured
result, on Servo 0.4.0, on this Linux baseline:

```
PUSHSTATE evaluate returned: #probed
PUSHSTATE PROBE: 3 -> 3 rows
PUSHSTATE PROBE: no new row — same-document navigation is not captured
```

**The `pushState` succeeded** — `location.hash` came back as `#probed`, so the
document's URL really did change — **and no history row appeared.** Servo 0.4.0
does not re-fire `LoadStatus::Complete` (nor anything else this shell listens
to that reaches the capture point) for a same-document navigation.

Consequences, all of them already anticipated by `03-RESEARCH.md` Pitfall 4:

- The `Complete`-gated capture design in Task 2 is **correct as built** for full
  document loads, and it does **not** get SPA navigation for free.
- In-app route changes in a single-page app produce no history rows in v1. This
  is the accepted gap Pitfall 4 named; closing it needs a signal libservo does
  not expose to embedders today. **No follow-up task is required this phase**,
  per the plan's own instruction.
- The probe is not a one-off: it stays in the suite and prints its finding on
  every run, so if a future Servo upgrade changes this, the output says so.

`03-RESEARCH.md` Open Question 2 (retention cap) needed no change: the default
was fixed at 5,000 entries with a `TALARIA_HISTORY_MAX_ENTRIES` override during
planning, and that is exactly what shipped.

## Decisions Made

- **Serialisation is hand-mapped, not derived.** The plan specified
  `#[derive(Serialize, Deserialize)]` on `HistoryEntry`, but `serde` is not a
  dependency of `talaria-shell` (only `serde_json` is), and the plan's own
  prohibition block forbids any change to `Cargo.toml`/`Cargo.lock` — adding
  `serde` to the crate's manifest would have changed the lockfile's
  `talaria-shell` package entry. `HistoryEntry::to_json_line` /
  `from_json_line` do the mapping through `serde_json::json!` and `Value`
  instead. They sit next to each other so a field added to one is visibly
  missing from the other.
- **`append` deliberately does *not* use temp+rename**, unlike the prune.
  `03-PATTERNS.md` suggested applying the atomic write uniformly; doing so per
  navigation would reintroduce exactly the O(n) whole-file rewrite Pitfall 1
  exists to avoid. An interrupted single-line append can corrupt only its own
  trailing line, which `load()` already tolerates — and there is now a unit test
  (`a_truncated_trailing_line_costs_only_that_line`) pinning that.
- **A cap of exactly `0` is honoured**, not treated as a typo. Only an
  *unparseable* value falls back to the default. "Keep no history" is a coherent
  thing for someone to ask for, and `vault.rs`'s `> 0` filter exists because a
  zero *timeout* is meaningless, which is not the same case.
- **Row labels truncate by character.** `gui.rs`'s existing tab-strip labels use
  `String::truncate(28)`, which panics when the byte index lands inside a
  multibyte character. The History panel does not copy that; `truncate_chars`
  is character-counted and unit-tested against a CJK title.
- **The store stays in append order; only the panel reverses.** Reversing at
  display time means two visits sharing a millisecond keep their real order,
  which a sort on the timestamp could not guarantee.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] `serde` derive is unavailable in `talaria-shell`**
- **Found during:** Task 1
- **Issue:** The plan's `<action>` specifies
  `#[derive(Debug, Clone, Serialize, Deserialize)]` on `HistoryEntry`, but
  `crates/talaria-shell/Cargo.toml` does not depend on `serde` — only on
  `serde_json`. Adding it would have changed `Cargo.lock`, which the plan's
  prohibition block explicitly forbids (`git diff --stat Cargo.toml Cargo.lock`
  must report no change).
- **Fix:** Hand-mapped `HistoryEntry` to and from one JSON line via
  `serde_json::json!` and `serde_json::Value`, documented in place with the
  reason. Only `url` is load-bearing; a missing `title`/`visited_at_ms`
  degrades to empty/epoch rather than discarding a row that still names a page.
- **Files modified:** `crates/talaria-shell/src/history.rs`
- **Verification:** `git diff --stat Cargo.toml Cargo.lock` reports no change;
  round-trip covered by `an_appended_entry_survives_a_reload` and the corrupt-
  line tests.
- **Committed in:** `d9c89f5`

**2. [Rule 1 - Bug] The new toolbar button broke `vault_ui_test`**
- **Found during:** Task 3 (full e2e regression)
- **Issue:** `tests/e2e/vault_ui_test.py` opens the credentials panel by
  clicking a hardcoded logical point, `CREDENTIALS_BUTTON = ("239", "11")`.
  Task 3 inserts a separator and the History button ahead of the credentials
  button, so that click now lands on History; the suite typed a credential into
  a panel that was not there and failed at `assert len(entries) == 1`. That
  suite's own comment warned this would happen.
- **Fix:** Measured the button's new position rather than guessing — temporarily
  bound the button's `Response` in `gui.rs` and logged its `.rect`, which
  reported `[[272.3 2.0] - [293.3 20.0]]`, then removed the instrumentation.
  Updated the constant to `("283", "11")` and rewrote its comment to name the
  new control order, the 239 → 283 move, and how to measure the next one.
- **Files modified:** `tests/e2e/vault_ui_test.py`
- **Verification:** `vault_ui_test` PASS standalone and in the full run.
- **Committed in:** `8e08c46`

**3. [Rule 2 - Missing Critical] Character-safe truncation for panel row labels**
- **Found during:** Task 3
- **Issue:** The plan says row labels "truncate to 60 characters", and the
  nearest analog in the file is the tab strip's `label.truncate(28)`.
  `String::truncate` takes a **byte** index and panics when it splits a
  multibyte character — a page title in Japanese, Greek or with an emoji would
  have panicked the chrome. Copying the analog would have copied a latent panic
  into a new surface, against CLAUDE.md's no-panic rule.
- **Fix:** Added `truncate_chars`, which counts characters and appends an
  ellipsis, plus a `#[cfg(test)] mod tests` in `gui.rs` covering it (including
  a 40-character CJK title) and `relative_time`.
- **Files modified:** `crates/talaria-shell/src/gui.rs`
- **Verification:** `cargo test` — 6 new gui tests pass, including
  `truncation_counts_characters_not_bytes`.
- **Committed in:** `8e08c46`
- **Note:** the pre-existing `String::truncate` calls in the tab strip were
  **not** touched — they are outside this task's scope. They remain a latent
  panic and are worth a separate quick fix.

---

**Total deviations:** 3 auto-fixed (1 blocking, 1 bug, 1 missing critical)
**Impact on plan:** All three were necessary. None expanded scope: the serde
workaround exists precisely to honour a prohibition, the coordinate fix repairs
a regression this plan caused, and the truncation fix avoids shipping a panic
into a brand-new surface. No architectural decision was needed and no Rule 4
checkpoint was reached.

## Issues Encountered

- **Task 2's `cargo clippy --all-targets -- -D warnings` gate could not pass at
  its own task boundary.** After Task 2, `History::clear` and `History::entries`
  existed but had no caller — the History panel that consumes them is Task 3 —
  so `-D dead-code` failed with exactly that one error. `cargo build --release`
  and `cargo test` were green at that point, and clippy went green as soon as
  Task 3 landed. This is a plan-sequencing artifact, not a defect: adding a
  throwaway `#[allow(dead_code)]` would have left a stale attribute behind, and
  merging the two tasks would have lost the atomic commit boundary. Recorded
  here rather than silently skipped. The gate is green at the plan tip.
- **`grep -c 'pub struct History'` returns 2, not the 1 the criterion states.**
  `pub struct HistoryEntry` contains that substring. Both structs are specified
  by the plan itself, so this is a wording artifact in the criterion, not a
  finding. The criterion's intent — the store type exists — holds.

## Verification

| Check | Result |
|---|---|
| `cargo build --release` | exit 0 |
| `cargo clippy --all-targets -- -D warnings` | exit 0 |
| `cargo test` | 35 passed, 0 failed (17 history, 6 gui, 12 vault) |
| `python3 tests/e2e/history_test.py` | exit 0, `HISTORY CHECKS PASSED` |
| `python3 tests/e2e/run_all.py` | 16/16 PASS, `failed: none` |
| `git diff --stat Cargo.toml Cargo.lock crates/talaria-mcp/src/tools.rs` | no change |
| `git diff --stat crates/talaria-shell/src/vault.rs` | no change |

The full run used a private runtime dir, as CI does:
`XDG_RUNTIME_DIR=/tmp/tal-e2e-rt TALARIA_E2E_DISPLAY=:98 TALARIA_E2E_OUT=/tmp/tal-e2e-out`.
No `dbus-run-session` wrapper was needed, confirming in passing that the vault's
D-Bus startup hang stays closed.

## Threat Model Outcomes

| Threat ID | Disposition | Outcome |
|---|---|---|
| T-03-01-01 (agent navigation leaking into the human's history) | mitigated | `process_pending_history_writes` filters on `TabOwner::Me`; `history_test.py`'s agent step asserts the row count is unchanged |
| T-03-01-02 (truncated trailing line after a kill) | mitigated | `load()` skips only the unparseable line; `a_truncated_trailing_line_costs_only_that_line` pins it |
| T-03-01-03 (hand-edited or corrupted file) | mitigated | Degrades to the parseable rows, one `log::warn!` with the skipped count, never a panic or a blocked startup |
| Unbounded growth (DoS, low) | mitigated | 5,000-entry cap, pruned via temp+`fs::rename` at load |
| Plaintext history on disk (low) | accepted | Unchanged from the plan's stated posture |
| T-03-01-04 (a future MCP tool exposing history) | mitigated | `crates/talaria-mcp/src/tools.rs` diff is empty; negative grep returns 0 |
| T-03-01-SC (package installs) | accepted | No dependency added; `Cargo.toml`/`Cargo.lock` diff is empty |

No new threat surface was introduced beyond the register: no network endpoint,
no auth path, no schema at a trust boundary. `history.jsonl` is a new local file
under the existing config directory, which the register already covers.

## Known Stubs

None. Every surface this plan ships is wired to real data: the panel reads the
live store, the store is fed by real navigations, and the e2e suite drives the
real binary end to end.

## User Setup Required

None — no external service configuration required.

## Next Phase Readiness

Ready for `03-02` (bookmarks). What it inherits:

- `ChromePanel` needs one new variant (`Bookmarks`) plus its toolbar button; the
  `UiAction::SetPanel` round trip, the `panel_open()` mouse-routing gate and the
  `Gui::set_panel` state-reset hook all already exist and need no change.
- `vault.rs`'s whole-array shape is the right analog for bookmarks (low churn),
  but `03-PATTERNS.md`'s temp+rename upgrade should be taken — `history.rs`'s
  `prune_to` is the worked example of it in this codebase.
- **Expect to move `vault_ui_test.py`'s `CREDENTIALS_BUTTON` again.** 03-02 adds
  two buttons (`STAR`, `BOOKMARKS`) ahead of Credentials. `deferred-items.md`
  records how to measure the new value in one build.
- `resolve_location` mishandles a pasted `data:` URL (it becomes a search query).
  Out of scope here; noted in `deferred-items.md` for 03-03, which owns that
  function.

No blockers.

---
*Phase: 03-table-stakes-browsing*
*Completed: 2026-08-20*

## Self-Check: PASSED

All four claimed key files exist on disk, and all four claimed commits
(`54baaa3`, `d9c89f5`, `9c7a8f6`, `8e08c46`) are reachable in git history.

---
phase: 03-table-stakes-browsing
plan: 04
subsystem: ui
tags: [rust, egui, servo, persistence, json, downloads, threading, security]

requires:
  - phase: 02-harden-the-agent-surface
    provides: the bounded `download` command, `create_unique`'s non-clobbering exclusive create, and SEC-01/SEC-02's narrowed agent surface
  - phase: 03-table-stakes-browsing
    plan: 01
    provides: ChromePanel, UiAction::SetPanel, truncate_chars, relative_time, now_ms, the hand-mapped serde_json idiom
  - phase: 03-table-stakes-browsing
    plan: 02
    provides: bookmarks.rs's atomic .tmp-sibling + fs::rename save, and its temp-path test fixture
  - phase: 03-table-stakes-browsing
    plan: 03
    provides: the five-variant ChromePanel chain, the measured ~29pt-per-toolbar-button rate, and the awk function-body diff technique
provides:
  - "crates/talaria-shell/src/downloads.rs — a whole-array store recording every completed download regardless of who asked for it"
  - "Shared::event_proxy — the first EventLoopProxy on Shared, and the codebase's only route from a background thread back onto the main loop"
  - "AppEvent::DownloadCompleted { path, filename, url, bytes } — gated on Outcome::Ok, carrying the actually-written path"
  - "ChromePanel::Downloads, the DOWNLOAD toolbar button and Ctrl+J"
  - "UiAction::OpenDownload — the codebase's only process-spawning action, reachable from exactly one button"
  - "gui.rs::human_bytes — byte counts in words, no crate added"
  - "tests/e2e/downloads_list_test.py — a real fetch proving the uniquified path is what gets recorded"
affects: []

tech-stack:
  added: []
  patterns:
    - "Cross-thread completion notice: clone an EventLoopProxy before std::thread::spawn, send an AppEvent, apply it only in user_event — the only shape available when Shared is Rc-based and not Send"
    - "A store that structurally cannot do the dangerous thing: downloads.rs calls nothing that deletes a file, so `remove` cannot grow into `delete` by accident"
    - "A one-shot chrome notice living on Gui rather than Shared, cleared by both its own Dismiss intent and by set_panel"

key-files:
  created:
    - crates/talaria-shell/src/downloads.rs
    - tests/e2e/downloads_list_test.py
  modified:
    - crates/talaria-shell/src/app.rs
    - crates/talaria-shell/src/gui.rs
    - crates/talaria-shell/src/main.rs
    - tests/e2e/run_all.py
    - tests/e2e/vault_ui_test.py
    - CHANGELOG.md

key-decisions:
  - "AppEvent::DownloadCompleted carries no owning-session field: Pitfall 5 requires recording every download regardless of owner, so RESEARCH.md's drafted session_owner would have been a field nothing ever reads"
  - "downloads.rs calls nothing that deletes a file — not even bookmarks.rs's stale-staging cleanup — so the lever that could delete a downloaded file does not exist in the file to be reached for by mistake"
  - "The AppEvent send is gated on Outcome::Ok by matching on a reference, so the outcome is still intact for the reply and a capped/cancelled/failed download produces no row"
  - "The Open error notice lives on Gui, not Shared, and is cleared by set_panel as well as by Dismiss, so it cannot outlive the panel session that raised it"
  - "human_bytes is hand-rolled at powers of 1024, matching every browser's own downloads list, rather than adding a formatting crate the phase prohibits"
  - "The row's tooltip shows the actual written path, not the requested filename, because those differ exactly when it matters most"

patterns-established:
  - "Enforcing a security boundary by what a module does not import, then pinning it with a source grep as an acceptance criterion"
  - "Proving a data-flow chain end to end with an e2e assertion that the stored string appears verbatim in the file, not merely that it parses equal"

requirements-completed: [BROWSE-04]

coverage:
  - id: D1
    description: "A completed download appends one entry carrying its exact written path, filename, source url and byte count"
    requirement: "BROWSE-04"
    verification:
      - kind: e2e
        ref: "tests/e2e/downloads_list_test.py — FIRST row asserts filename, path == the shell's own reported path, url, bytes, completed_at_ms > 0, and that the path names a file of exactly the served length"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/downloads.rs#an_appended_download_survives_a_reload, #a_round_trip_preserves_every_field_exactly"
        status: pass
    human_judgment: false
  - id: D2
    description: "A completed download appears in the Downloads panel and can be opened from it (BROWSE-04's literal success criterion)"
    requirement: "BROWSE-04"
    verification:
      - kind: e2e
        ref: "tests/e2e/downloads_list_test.py proves the row exists and names the written file; the panel reads shared.downloads.borrow().entries() directly"
        status: pass
      - kind: other
        ref: "the Open button's construction site (gui.rs:1215) and its only consumer (app.rs:1376) traced by grep; `grep -c 'xdg-open' crates/talaria-shell/src/app.rs` is 1"
        status: pass
    human_judgment: true
    rationale: "Nothing automated opens the panel, clicks Open and observes a PDF viewer launch — spawning a real external handler is not something an Xvfb suite should do, and 03-VALIDATION.md scopes the visual check to a human. The data path into the launch call is fully pinned (D3), and the panel-render gap is the same egui-closure limitation 03-01's D7, 03-02's D8 and 03-03's D6 each recorded."
  - id: D3
    description: "Two downloads of the same requested filename each get their own row, keyed on the actual uniquified written path, neither overwriting the other"
    requirement: "BROWSE-04"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/downloads.rs#two_downloads_of_the_same_filename_are_two_distinct_entries"
        status: pass
      - kind: e2e
        ref: "tests/e2e/downloads_list_test.py SECOND row — asserts the second path contains '(1)', differs from the first, equals the shell's own reported path, and that the first row's path and byte count are unchanged"
        status: pass
    human_judgment: false
  - id: D4
    description: "An empty or missing downloads.json loads as an empty list without panicking"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/downloads.rs#a_missing_file_loads_as_an_empty_store, #an_empty_file_loads_as_an_empty_store, #a_corrupt_file_loads_as_an_empty_store_without_panicking, #a_file_of_the_wrong_shape_loads_as_an_empty_store, #an_entry_with_no_path_is_skipped_and_the_good_ones_survive"
        status: pass
      - kind: e2e
        ref: "tests/e2e/downloads_list_test.py asserts read_store() == [] against the isolated home before any download"
        status: pass
    human_judgment: false
  - id: D5
    description: "Downloads render most-recently-completed first; entries with identical timestamps preserve append order rather than being re-sorted unstably"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/downloads.rs#entries_keep_their_append_order_across_a_reload — all four entries claim the same completion millisecond, so an unstable sort would be free to reorder them"
        status: pass
      - kind: other
        ref: "gui.rs's Downloads arm iterates `entries.iter().rev()`; the store has no sort call at all — `grep -n 'sort' crates/talaria-shell/src/downloads.rs` returns exactly one line (453), and it is a test comment, not a call"
        status: pass
    human_judgment: false
  - id: D6
    description: "Open launches exactly the stored path, never one re-derived from the requested filename, and never agent-supplied input (T-03-04-02)"
    requirement: "BROWSE-04"
    verification:
      - kind: other
        ref: "the whole chain traced: create_unique's return -> download()'s Outcome -> the closure's AppEvent -> Downloads::append -> entry.path.clone() (gui.rs, 2 sites) -> UiAction::OpenDownload -> xdg-open. create_unique and download both diffed byte-for-byte against the pre-plan revision: IDENTICAL"
        status: pass
      - kind: e2e
        ref: "tests/e2e/downloads_list_test.py asserts each stored path is present *verbatim* (json.dumps-escaped) in downloads.json's raw text, so nothing normalised or re-derived either one"
        status: pass
    human_judgment: false
  - id: D7
    description: "No MCP tool, control-socket command or protocol variant can reach the downloads opener (T-03-04-01)"
    requirement: "BROWSE-04"
    verification:
      - kind: other
        ref: "`grep -c 'downloads_open\\|downloads_list' crates/talaria-mcp/src/tools.rs` returns 0; `git diff --stat 587765e -- crates/talaria-mcp crates/talaria-protocol` is empty; UiAction::OpenDownload has exactly one construction site and one consumer, both in chrome code"
        status: pass
    human_judgment: false
  - id: D8
    description: "A cancelled or failed download never produces a list row (T-03-04-04)"
    verification:
      - kind: e2e
        ref: "tests/e2e/downloads_list_test.py REFUSED — a download over the byte cap is refused, and after a 2s window (the same one a success would have had) the store still holds exactly the two earlier rows and none named too-big.bin"
        status: pass
      - kind: other
        ref: "the send is inside `if let Outcome::Ok { result: ResultPayload::Download { .. } } = &outcome`, so every Outcome::Error path sends nothing"
        status: pass
    human_judgment: false
  - id: D9
    description: "Removing an entry deletes only the list row, never the file on disk"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/downloads.rs#removing_an_entry_leaves_the_downloaded_file_on_disk — a real file on disk survives the remove, and the row does not"
        status: pass
      - kind: other
        ref: "`grep -c 'remove_file' crates/talaria-shell/src/downloads.rs` is 0 — the module calls nothing that could delete a file, tests included"
        status: pass
    human_judgment: false
  - id: D10
    description: "A crash mid-save never leaves downloads.json partially written (T-03-04-03)"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/downloads.rs#a_save_stages_through_a_temp_file_and_leaves_none_behind, #a_stale_staging_file_does_not_block_the_next_save"
        status: pass
    human_judgment: false
  - id: D11
    description: "The completed download reaches Shared only from the main thread — nothing on the Rc-based Shared is touched off it"
    verification:
      - kind: other
        ref: "the closure captures only `proxy`, `url`, `filename` and `reply`; `state` is never moved in (it could not be — Rc is not Send, so this is compiler-enforced rather than reviewed). Shared::downloads is touched at exactly three sites, all main-thread"
        status: pass
      - kind: e2e
        ref: "tests/e2e/downloads_list_test.py drives real downloads through the socket and the shell neither panics nor deadlocks; e2e 18/18"
        status: pass
    human_judgment: false
  - id: D12
    description: "The list survives a restart"
    requirement: "BROWSE-04"
    verification:
      - kind: e2e
        ref: "tests/e2e/downloads_list_test.py RESTART — the shell is stopped and restarted against the same config directory, and both rows come back in the same order with the same paths"
        status: pass
    human_judgment: false
  - id: D13
    description: "A failed Open shows an inline error naming the filename and the OS error, with a Dismiss control, and never panics"
    verification:
      - kind: other
        ref: "the Err arm builds the filename through `file_name().map(..).unwrap_or_else(|| path.clone())` — no unwrap, no indexing; `grep -c 'unwrap()' crates/talaria-shell/src/app.rs` is 0, unchanged from baseline"
        status: pass
    human_judgment: true
    rationale: "Making xdg-open fail on demand needs a PATH without it, which the harness does not currently arrange. The error path is total by construction (every branch returns a String) and the copy matches 03-UI-SPEC.md's Copywriting Contract verbatim. Same egui-render gap as D2."
  - id: D14
    description: "Byte counts and ages render sensibly at every boundary"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/gui.rs#a_byte_count_reads_in_the_largest_unit_that_fits, #an_absurd_byte_count_still_formats (u64::MAX included)"
        status: pass
    human_judgment: false

duration: 32 min
completed: 2026-08-20
status: complete
---

# Phase 03 Plan 04: Downloads List Summary

**Every completed download now records the path the shell *actually wrote* — not the name that was asked for — reaching the main thread over the codebase's first `EventLoopProxy` on `Shared`, and surfacing in a human-only Downloads panel whose `Open` button is the browser's only process-spawning surface and is reachable from exactly one place in the codebase.**

## Performance

- **Duration:** 32 min
- **Started:** 2026-08-20T08:46:19Z
- **Completed:** 2026-08-20T09:18:13Z
- **Tasks:** 3
- **Files modified:** 8 (2 created, 6 modified)

## Accomplishments

- **BROWSE-04 delivered, and Phase 3 closes at 18/18 e2e.** The suite went 17 → 18 with `downloads_list_test`, `cargo test` 67 → 83.
- **The phase's one high-severity threat pair is mitigated structurally, not by promise.** T-03-04-01 (an agent-reachable opener) is enforced by there being exactly one construction site for `UiAction::OpenDownload`; T-03-04-02 (a re-derived path) by an unbroken chain from `create_unique`'s return value to the launch call, with `create_unique` and `download` both diffed byte-for-byte against the pre-plan revision and reported IDENTICAL.
- **The store cannot delete a file, and that is a property of what it does not import.** `downloads.rs` calls nothing that unlinks a path — it deliberately drops the one line `bookmarks.rs` has that does. `remove` cannot quietly grow into `delete`.
- **The first `EventLoopProxy` on `Shared`.** `download()` had no route back to the main thread at all; it has one now, and the compiler enforces the discipline (`Rc` is not `Send`, so the closure *cannot* capture `state` even by mistake).
- **The toolbar-coordinate prediction was confirmed to the point.** 03-03 derived 399 from the ~29pt-per-button rate; the measured rect was `[[388.3 2.0] - [409.3 20.0]]`, centre 398.8. Four moves, rate holding.
- **`app.rs` and `gui.rs` still contain zero `unwrap()`** after adding a process spawn and a fallible path-to-filename conversion.

## Task Commits

1. **Task 1 (RED): failing tests for the downloads store** — `68ffc3a` (test)
2. **Task 1 (GREEN): the downloads store** — `696f82d` (feat)
3. **Task 2: cross-thread download-completion notice** — `a779e78` (feat)
4. **Task 3: the Downloads panel, Open/Remove and the open-error notice** — `23a9217` (feat)

_Task 1 is `tdd="true"`, so it carries a test → feat pair. RED: 8 failing / 6 passing. The 6 pass honestly — the load-degrade cases are satisfied by a stub that always returns empty, and "remove never deletes the file" is satisfied by a store that cannot write at all yet. No refactor commit was needed._

## Files Created/Modified

- `crates/talaria-shell/src/downloads.rs` (new, 551 lines) — `DownloadEntry` (+ hand-mapped `to_json`/`from_json`), `Downloads` (`load`/`load_from`/`entries`/`append`/`remove`/`save`), a private `config_dir()`, and 14 unit tests.
- `crates/talaria-shell/src/app.rs` — `Shared::event_proxy`, `Shared::downloads`, `AppEvent::DownloadCompleted`, the gated send inside `Command::Download`'s spawned closure, the `user_event` arm, three `apply_ui_actions` arms (`OpenDownload`/`RemoveDownload`/`DismissDownloadError`), and the Ctrl+J shortcut.
- `crates/talaria-shell/src/gui.rs` — `ChromePanel::Downloads`, three `UiAction` variants, `Gui::download_open_error` + `set_download_open_error`/`clear_download_open_error`, the `DOWNLOAD` toolbar button, the Downloads panel body, `human_bytes`, and 2 unit tests.
- `crates/talaria-shell/src/main.rs` — `mod downloads;`.
- `tests/e2e/downloads_list_test.py` (new, 265 lines) — five checks, including the verbatim-path assertion and the refused-download negative.
- `tests/e2e/run_all.py` — registers `downloads_list_test`.
- `tests/e2e/vault_ui_test.py` — `CREDENTIALS_BUTTON` 370 → 399 (see deviations).
- `CHANGELOG.md` — a BROWSE-04 entry (see deviations).

## Decisions Made

- **`AppEvent::DownloadCompleted` carries no owning-session field.**
  `03-RESEARCH.md`'s `Pattern 3` code sample drafted a `session_owner:
  TabOwner` field, speculating the store might need to know who triggered a
  download. It does not, and the reason is a decision the phase had already
  made elsewhere: Pitfall 5 says the store records **every** completed
  download regardless of owner — the deliberate opposite of history's Me-only
  filter, because a visit an agent made is the agent's business while a *file*
  an agent wrote is on the human's filesystem. Given that rule, the field has
  no reader, ever. Carrying it would have been dead data that invites a future
  filter nobody wants. Dropped, with the reasoning recorded in the variant's
  own doc comment and in `downloads.rs`'s module header, so the difference
  from the research draft reads as a decision rather than an omission. The
  plan verified this with a negative grep, which returns 0.

- **`downloads.rs` deliberately does not clean up its stale staging file.**
  This is the one line where it diverges from `bookmarks.rs`'s `save()`, which
  it otherwise copies verbatim. `bookmarks.rs` ends a failed rename with `let _
  = fs::remove_file(&staged);`. This module does not, and the point is what
  that buys: **nothing in the file calls anything that deletes a file.** The
  module that owns paths to the user's downloaded files never imports the
  capability to unlink one, so `remove` — which is one careless commit away
  from being "helpfully" upgraded to also delete the file — has no lever within
  reach. The cost is a stale `.tmp` after a failed rename, which is harmless:
  `load` only ever opens `downloads.json`, and the next save overwrites it.
  Both halves are pinned by `a_stale_staging_file_does_not_block_the_next_save`.
  The plan asked for this as an acceptance grep (`remove_file` count 0); making
  it literally true is what forced the choice, and the choice turns out to be
  the better code.

- **The `Outcome::Ok` gate is a match on a reference, not a consuming match.**
  `outcome` still has to be sent to the agent immediately afterwards, so
  `if let Outcome::Ok { .. } = &outcome` is not a style preference — a
  consuming match would have required rebuilding the outcome to reply with.
  The gate itself is the mitigation for T-03-04-04: a download that was capped,
  cancelled, timed out or failed to connect took an `Outcome::Error` path and
  left nothing at the destination, so a row claiming otherwise would be the
  list lying about the filesystem.

- **The open-error notice lives on `Gui`, and `set_panel` clears it.**
  The vault's one-shot notices live on `Vault` because they describe something
  that happened to persisted state and must survive until acknowledged. This
  one describes a button press, so it lives on `Gui` with the other view state.
  The plan did not say to clear it in `set_panel`; doing so follows that
  function's own stated invariant ("every panel-local view state resets here"),
  and without it, closing the panel and reopening it would re-accuse the user
  of something they had already walked away from. `DismissDownloadError` still
  exists for clearing it without leaving the panel.

- **The row's tooltip shows the written path, not the requested filename.**
  `03-UI-SPEC.md` asks for this explicitly and the reason is the same one the
  whole plan turns on: those two strings differ exactly when it matters most —
  when a collision was resolved — and the written one is what `Open` will hand
  to the OS. A tooltip showing the requested name would be describing a
  different file than the button acts on.

- **`human_bytes` is hand-rolled, at powers of 1024.** The same call
  `relative_time` made in 03-01: no formatting crate is in this workspace and
  the phase adds none. Powers of 1024 with KB/MB/GB labels is what every
  browser's own downloads list shows; one decimal above a kilobyte, because
  "1.4 MB" is informative where "1 MB" is vague and "1468006 bytes" is
  unreadable. Both unit boundaries are tested from both sides, and `u64::MAX`
  is tested for not overflowing into nonsense.

- **The e2e asserts each path appears *verbatim* in the raw file**, not merely
  that the parsed value compares equal. Comparing parsed values would pass even
  if something had normalised, re-encoded or reconstructed the string on the
  way through; the security property is about the exact bytes that reach
  `xdg-open`, so the check is on the exact bytes on disk.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing critical functionality] The plan's `save()` would have failed its own acceptance grep**

- **Found during:** Task 1
- **Issue:** The plan says to copy `bookmarks.rs`'s `save()` shape, and
  separately requires `grep -c 'remove_file' crates/talaria-shell/src/downloads.rs`
  to be `0`. `bookmarks.rs`'s `save()` contains `fs::remove_file`, and so did my
  test fixture's `Drop`. The two instructions cannot both be satisfied by a
  verbatim copy.
- **Fix:** Resolved in favour of the grep, because the grep expresses the
  stronger property. `save()` drops the stale-staging cleanup (with a comment
  explaining the trade), and the test fixture was restructured from a bare temp
  path into a per-test temp *directory* removed whole with `fs::remove_dir_all`.
  The module now calls nothing that unlinks a single file by path, tests
  included.
- **Files modified:** `crates/talaria-shell/src/downloads.rs`
- **Verification:** `grep -c 'remove_file'` is 0; 14 unit tests pass, including
  the stale-staging one that proves the dropped cleanup costs nothing.
- **Committed in:** `696f82d`

---

**2. [Rule 1 - Bug] The DOWNLOAD button broke `vault_ui_test` again**

- **Found during:** Task 3
- **Issue:** `tests/e2e/vault_ui_test.py` opens the credentials panel by
  clicking a hardcoded logical point. Inserting the DOWNLOAD button ahead of
  the credentials button moves it, so the old point (`370`) would have landed
  on GEAR and opened Settings instead. A **known, documented cost** — recorded
  in `deferred-items.md` by 03-01 and re-flagged by 03-02 and 03-03, which
  predicted both that it would recur and what the new value would be.
- **Fix:** Measured, not guessed, exactly as `deferred-items.md` prescribes:
  temporarily bound the credentials button's `Response` in `gui.rs`,
  `log::warn!`ed its `.rect` under Xvfb, which reported
  `[[388.3 2.0] - [409.3 20.0]]`; took the centre of the x span (398.8 → 399);
  removed the instrumentation and confirmed with `grep -c MEASURE` that none
  survived. 03-03's predicted 399 was correct to the point. Updated the
  constant and its comment with the fourth move.
- **Files modified:** `tests/e2e/vault_ui_test.py`, `crates/talaria-shell/src/gui.rs` (instrumentation added and removed within the task)
- **Verification:** `vault_ui_test` PASS in the full run (42s); `grep -c 'MEASURE' crates/talaria-shell/src/gui.rs` is 0.
- **Committed in:** `23a9217`

---

**3. [Rule 2 - Missing critical functionality] `harness.stop(tal, None)` would have crashed the new suite's restart check**

- **Found during:** Task 3
- **Issue:** The plan's e2e sequence needs the shell stopped and restarted
  while the Xvfb stays up. `harness.stop(shell, xvfb)` unconditionally calls
  `xvfb.kill()`, so passing `None` raises `AttributeError` — and a restart that
  merely `terminate()`s races the next `start_shell` against the dying
  instance's socket, whose single-instance path would forward the URL and exit
  instead of starting.
- **Fix:** Adopted `bookmarks_test.py`'s existing `stop_shell` helper verbatim
  (terminate, poll for exit, then poll for the socket to disappear) rather than
  inventing a third teardown shape, and unrolled the `finally` block to match.
- **Files modified:** `tests/e2e/downloads_list_test.py`
- **Verification:** `downloads_list_test` PASS standalone (31s) and in the full
  run; the RESTART check reports 2 rows in order.
- **Committed in:** `23a9217`

---

**4. [Rule 2 - Missing critical functionality] CHANGELOG.md had no Phase 3 entry**

- **Found during:** Task 3
- **Issue:** `CLAUDE.md`'s global rules require every change to be logged in
  `CHANGELOG.md`. Plans 03-01 through 03-03 did not add entries, so the file's
  last entry is from Phase 2.
- **Fix:** Added a BROWSE-04 entry under `### Added`, covering the downloads
  list, the two properties a reader most needs stated (the written-vs-requested
  path, and the opener's human-only reachability), and the new e2e suite. The
  three earlier plans' entries are **not** backfilled here — that is the phase
  close's business, and inventing changelog prose for someone else's commits
  from the outside is worse than flagging the gap. Noted below.
- **Files modified:** `CHANGELOG.md`
- **Verification:** entry present under `## [Unreleased] / ### Added`.
- **Committed in:** `23a9217`

---

**Total deviations:** 4 auto-fixed (1 bug, 3 missing critical functionality)
**Impact on plan:** None on scope. One repairs a regression this plan caused in
an unrelated suite and was predicted in writing three plans ago; one resolves a
genuine contradiction inside the plan's own instructions, in favour of the
stronger property; two fill in things the plan's prose assumed. No
architectural decision was needed and no Rule 4 checkpoint was reached.

## Issues Encountered

- **Task 2's clippy gate could not pass at its own boundary, exactly as 03-01
  and 03-03 both recorded.** Task 2's `<verify>` includes `cargo clippy
  --all-targets -- -D warnings`, but after Task 2 `Downloads::entries` and
  `Downloads::remove` had no caller — the Downloads panel that calls them is
  Task 3. Two `dead_code` errors. Resolved by finishing Task 3 rather than
  papering over it: **no `#[allow(dead_code)]` was added at any point**, and
  clippy is clean from `23a9217` onward. `cargo build --release` and `cargo
  test` both passed at the Task 2 boundary; only the clippy leg did not.
  This is now the third consecutive plan to hit it, which makes it a property
  of how the phase's tasks are sliced (store first, consumer last) rather than
  an accident — worth noting for whoever writes the next store-plus-panel plan.

- **`pkill -f 'Xvfb :97'` killed the measuring shell, again.** 03-03 hit this
  and wrote it into its own summary; it was not written into
  `deferred-items.md` or into `vault_ui_test.py`'s comment, so it was
  rediscovered here (exit 144, mid-command, before the instrumentation could be
  removed). The cause is that `pkill -f` matches against full command lines and
  the killing shell's own command line contains the pattern. It is now written
  into `vault_ui_test.py`'s comment beside the measurement recipe, which is
  where the next person will actually be reading.

- **The plan's `event_proxy` acceptance grep counts 3, not the ≥4 it
  specifies.** Not a gap — a miscount in the criterion. `grep -c` counts
  *lines*, and the plan's own `<action>` prescribes `let proxy =
  state.event_proxy.clone();`, so the fourth site (the use inside the closure)
  reads `proxy.send_event(...)` and contains no `event_proxy` token. All four
  sites exist: the field declaration (`app.rs:108`), the `Shared { .. }`
  literal (`app.rs:862`), the clone before the spawn (`app.rs:1697`), and
  `proxy.send_event(AppEvent::DownloadCompleted { .. })` inside the closure.
  The local was left named `proxy` because that is what the plan's own action
  text says; renaming it purely to satisfy a line count would be gaming the
  measurement rather than meeting it.

## Verification

| Check | Result |
|---|---|
| `cargo build --release` | exit 0 |
| `cargo clippy --all-targets -- -D warnings` | exit 0 |
| `cargo test` | 83 passed, 0 failed (14 downloads, 14 settings, 14 bookmarks, 17 history, 9 gui, 12 vault, 5 app) + 2 protocol |
| `cargo test -p talaria-shell downloads::tests` | 14 passed, 0 failed |
| `python3 tests/e2e/downloads_list_test.py` | exit 0, `DOWNLOADS LIST CHECKS PASSED` |
| `python3 tests/e2e/run_all.py` | **18/18 PASS**, `failed: none` |
| `grep -c 'downloads_open\|downloads_list' crates/talaria-mcp/src/tools.rs` | **`0`** |
| `git diff --stat 587765e -- crates/talaria-mcp crates/talaria-protocol` | no change |
| `git diff --stat 587765e -- Cargo.toml Cargo.lock` | no change |
| `grep -c 'pub struct Downloads' crates/talaria-shell/src/downloads.rs` | 1 |
| `grep -c 'fs::rename' crates/talaria-shell/src/downloads.rs` | 3 |
| `grep -c 'remove_file' crates/talaria-shell/src/downloads.rs` | 0 |
| `grep -c 'unwrap()' crates/talaria-shell/src/downloads.rs` | 0 |
| `grep -c 'mod downloads;' crates/talaria-shell/src/main.rs` | 1 |
| `grep -c 'AppEvent::DownloadCompleted' crates/talaria-shell/src/app.rs` | 3 |
| `grep -c 'session_owner' app.rs + downloads.rs` | 0 |
| `grep -c 'ChromePanel::Downloads' crates/talaria-shell/src/gui.rs` | 3 |
| `grep -c 'OpenDownload\|RemoveDownload' gui.rs + app.rs` | 8 |
| `grep -c 'xdg-open' crates/talaria-shell/src/app.rs` | 1 |
| `grep -c 'entry.path.clone()' crates/talaria-shell/src/gui.rs` | 2 |
| `grep -c 'unwrap()' crates/talaria-shell/src/app.rs` | 0 (unchanged from baseline) |
| `grep -c 'unwrap()' crates/talaria-shell/src/gui.rs` | 0 (unchanged from baseline) |
| `grep -c 'MEASURE' crates/talaria-shell/src/gui.rs` | 0 (instrumentation removed) |
| `parse_agent_url` / `agent_scheme_allowed` bodies vs. pre-plan | IDENTICAL (both) |
| `create_unique` / `download` / `abandon_download` bodies vs. pre-plan | IDENTICAL (all three) |
| `resolve_location` body vs. pre-plan | IDENTICAL |

The full e2e run used a private runtime dir, as CI does:
`XDG_RUNTIME_DIR=/tmp/tal-e2e-rt TALARIA_E2E_DISPLAY=:98 TALARIA_E2E_OUT=/tmp/tal-e2e-out`.
No `dbus-run-session` wrapper was needed.

### The security argument, stated as evidence rather than intent

The plan's `<security_critical>` block asks for verification, not assurance.
Both high-severity threats reduce to reachability questions, and both are
answered by tracing rather than by review:

**T-03-04-01 — can an agent reach the opener?** `UiAction::OpenDownload` has
exactly **one** construction site in the entire workspace
(`gui.rs:1215`, inside the Downloads panel's `Open` button) and exactly **one**
consumer (`app.rs:1376`). `crates/talaria-mcp` and `crates/talaria-protocol`
are byte-identical to the pre-plan revision, so no tool, no `Command` variant
and no `ResultPayload` variant was added that could reach it. The negative grep
returns `0`. `xdg-open` appears once in the whole shell crate.

**T-03-04-02 — can the launched path differ from the file that was written?**
The chain is unbroken and every link was checked rather than assumed:

| Step | What carries the path | Evidence it does not re-derive |
|---|---|---|
| `create_unique` returns `(File, PathBuf)` | the candidate it exclusively created | body diffed against pre-plan: IDENTICAL |
| `download()` returns `ResultPayload::Download { path }` | `path.display().to_string()` | body diffed against pre-plan: IDENTICAL |
| the spawned closure | `path.clone()` out of the matched `&outcome` | the `filename` in scope is passed as a *separate* field |
| `Downloads::append` | stores it as `DownloadEntry::path` | round-trip unit test asserts every field exactly |
| the panel row | `entry.path.clone()`, twice (Open, Remove) | grep confirms 2 sites; the tooltip also shows `entry.path`, not `entry.filename` |
| `UiAction::OpenDownload` → `xdg-open` | `.arg(&path)` | the error branch's filename is derived *from the path tried*, not from the entry |

The e2e closes it from the outside: both stored paths appear **verbatim** in
`downloads.json`'s raw text, and the second is `report (1).pdf` while its
`filename` field still reads `report.pdf` — which is precisely the case where a
re-derived path would have opened the wrong file.

## Threat Model Outcomes

| Threat ID | Disposition | Outcome |
|---|---|---|
| T-03-04-01 (opener becoming an agent-reachable arbitrary-exec primitive) | mitigated | One construction site, one consumer, both in chrome code. `crates/talaria-mcp` and `crates/talaria-protocol` untouched (empty diff). `grep -c 'downloads_open\|downloads_list' crates/talaria-mcp/src/tools.rs` returns `0`. No control-socket command and no keyboard path other than opening the panel |
| T-03-04-02 (a re-derived path letting Open target an unintended file) | mitigated | The six-step chain above, every link traced; `create_unique` and `download` both byte-identical to the pre-plan revision; a unit test and the e2e's step 3 both prove two same-named downloads get two distinct, independently-correct paths |
| T-03-04-03 (a corrupted `downloads.json`) | mitigated | Unparseable content, wrong-shaped content and individually-broken entries all degrade to an empty or partial list with one `log::warn!` and no panic — 5 unit tests. Writes are atomic (`.tmp` sibling + `fs::rename`), 2 more tests |
| T-03-04-04 (a failed or cancelled download producing a misleading row) | mitigated | The send is gated on `Outcome::Ok` specifically; the e2e refuses a download over the byte cap, waits the same window a success would have had, and asserts no row appeared |
| Denial of service — unbounded `downloads.json` growth | accepted | No cap this phase, per `03-UI-SPEC.md`'s Open Question 6. Flagged in `downloads.rs`'s module header rather than left silent; a cap is additive |
| T-03-04-SC (package installs) | accepted | No dependency added; `Cargo.toml`/`Cargo.lock` diff is empty |

## Threat Flags

None. This plan adds no network endpoint, no auth path and no schema at a
trust boundary. It does add the browser's **first and only process-spawning
surface** (`xdg-open`), which is not a new *flag* because T-03-04-01 is exactly
that threat and it is mitigated above — but it is worth naming for a future
reader: `grep -rn 'process::Command' crates/` should return one site, and if it
ever returns two, the second one needs the same reachability argument this one
has.

## Known Stubs

None. The panel reads the live store on open, the store is appended from a real
completion event, and the e2e drives a real HTTP fetch end to end.

## User Setup Required

None. An existing install picks up an empty list with no migration and no
`downloads.json` on disk until the first completed download. `Open` requires
`xdg-open` on `PATH` (present on any freedesktop system); where it is absent,
the button reports the OS error inline rather than failing silently.

## Documentation Debts for the Phase Close

Three, carried forward deliberately rather than fixed here:

1. **`.claude/CLAUDE.md` says the project has "No `.env` files, no config file
   format. All configuration is environment variables plus CLI argv."** 03-03
   made that false by introducing `config.json`, and this plan does not change
   the situation but does add a third store file. The Configuration section
   should name `config.json` and the `talaria/` store directory
   (`history.jsonl`, `bookmarks.json`, `downloads.json`, `config.json`,
   `vault.json`).
2. **`CLAUDE.md`'s Component Responsibilities table has no rows for the four
   Phase 3 stores** (`history.rs`, `bookmarks.rs`, `settings.rs`,
   `downloads.rs`) and no row for `Shared::event_proxy`, which the
   Architectural Constraints section's "Threading" row should now mention as
   the sanctioned route back onto the main thread.
3. **`CHANGELOG.md` has entries for Phase 2 and for BROWSE-04, but not for
   BROWSE-01/02/03.** Backfilling those three is the phase close's business.

Also still open, unchanged: `deferred-items.md`'s recurring `vault_ui_test`
coordinate churn (now four moves) and its two candidate structural fixes,
neither of which got cheaper. Phase 3 is over so nothing further should move it
this milestone, but the fifth plan that adds a toolbar button will pay the cost
a fifth time.

## Next Phase Readiness

Phase 3 is complete: BROWSE-01 through BROWSE-04 all delivered, e2e at 18/18,
`cargo test` at 83. v1 (Phases 1–3) is feature-complete.

What a later phase inherits:

- **`Shared::event_proxy` is now the sanctioned route from any background
  thread back onto the main loop.** Anything else that wants to do off-thread
  work and report a result should add an `AppEvent` variant and a `user_event`
  arm rather than inventing a second mechanism.
- **`ChromePanel` has six variants** and the mechanism is unchanged across four
  consecutive plans. Adding one is still one variant, one toolbar button and
  one `else if` arm.
- **Four store modules now exist with three distinct shapes**: append-log
  (`history.rs`), whole-array (`bookmarks.rs`, `downloads.rs`), single-object
  (`settings.rs`). All four degrade-never-abort; the last three write
  atomically.
- **The one v1 blocker candidate is unchanged**: `Vault::load()`'s D-Bus
  autolaunch hang, already mitigated in `vault.rs` per the CHANGELOG's Fixed
  section and covered by `vault_nobus_test`.

No blockers.

---
*Phase: 03-table-stakes-browsing*
*Completed: 2026-08-20*

## Self-Check: PASSED

All nine claimed key files exist on disk, and all five claimed commits
(`68ffc3a`, `696f82d`, `a779e78`, `23a9217`, `7300bba`) are reachable in git
history.

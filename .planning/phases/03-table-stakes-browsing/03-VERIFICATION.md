---
phase: 03-table-stakes-browsing
verified: 2026-08-20T09:35:09Z
amended: 2026-08-20T14:05:00Z
visual_review_at: 2026-08-20T20:44:00Z
status: passed
score: 42/42 must-haves verified
behavior_unverified: 0
overrides_applied: 0
prohibitions_held: 14/14
backstops_resolved: 1/1
visual_review:
  method: >
    The three items left as human were visual judgment, not missing capability.
    They were closed by rendering the real chrome under Xvfb and reviewing the
    screenshots directly, rather than by pixel sampling — sampling a colour at a
    coordinate would pass without exercising the judgment the items are about.
    Panels were opened by clicking the actual toolbar buttons, located through
    the `chrome_rects` test hook, so the captures show the same code path a user
    drives. Eight distinct frames: baseline, star selected, History, Bookmarks,
    Downloads, Settings, Agents view, and History inside the Agents view.
  items:
    - truth: "The bookmark star's .selected() state, and only that plus the panel-toggle buttons' .selected() state, uses the reserved accent color"
      verdict: verified
      evidence: >
        Across all eight frames the accent appears on exactly three things and
        never anywhere else: the active view toggle (Me, or Agents in the
        Agents-view frame), the active panel-toggle button, and the bookmark
        star when the displayed page is bookmarked. The star is accented in the
        Me-view frames after bookmarking example.com and is NOT accented in the
        Agents-view frame, whose displayed tab is servo.org — so the accent
        tracks per-page state rather than being decorative. Nothing else in the
        toolbar, tab strip or panel body carries it. This is the UI-SPEC's
        closed reserved list, held.
    - truth: "Visited pages appear in a history list (BROWSE-01's rendering half)"
      verdict: verified
      evidence: >
        The History panel renders the heading, the explanatory line, and one row
        reading "Example Domain" with "just now" beside it in the Small text
        style, above a separator and the unarmed "Clear history" control. The
        row shows title and relative time as specified.
    - truth: "The three new list panels do not overlap the toolbar or tab strip, and the Me/Agents toggle still reads correctly"
      verdict: verified
      evidence: >
        History, Bookmarks, Downloads and Settings each begin at the same offset
        below the tab strip, in both Me and Agents view. The toolbar and tab
        strip remain fully drawn and legible behind none of them. In the
        Agents-view frame the toggle correctly reads Agents as active and the
        tab strip shows the agent's session label and its tab title.
  incidental_confirmation: >
    The Agents-view History frame independently confirms the CR-02 fix: with an
    agent tab displaying servo.org, the history list contains only the human's
    own example.com visit. The panel's corrected copy is legible in the same
    frame.

behavior_verified_since:
  - truth: "Clicking a history row navigates the displayed tab there and closes the panel"
    now_covered_by: "tests/e2e/panel_click_test.py — HISTORY row click / HISTORY panel closed on the same click"
    how: "Visits two pages, presses Ctrl+H, clicks the older row at the rect the shell reports for `history.row.1`, then asserts over the control socket that the displayed Me tab moved from /two to /one. The close is asserted twice: the panel's rows stop being drawn, and the next Ctrl+H opens rather than closes."
  - truth: "Clear history requires two clicks — a confirm step — and the confirm state resets whenever the History panel closes"
    now_covered_by: "tests/e2e/panel_click_test.py — CLEAR first click armed / CLEAR the confirm reset / CLEAR confirmed"
    how: "The chrome reports the control as `history.clear` unarmed and `history.confirm-clear` armed, so both transitions are observed rather than inferred. One click clears nothing and arms it; a close and reopen shows it unarmed again; a *second* single click after that still clears nothing — that assertion is what proves the reset rather than assuming it, since a persisted confirm would have cleared on it. The confirming click then empties history.jsonl."
  - truth: "Clicking a bookmark row navigates the displayed tab there and closes the panel (BROWSE-02's 'return to it' half)"
    now_covered_by: "tests/e2e/panel_click_test.py — BOOKMARK row click / BOOKMARKS panel closed on the click"
    how: "Ctrl+D bookmarks the displayed page, a second page is opened so the displayed tab is demonstrably elsewhere, then `bookmarks.row.0` is clicked and the displayed tab is asserted back at the bookmarked URL. Same two-way close assertion as the History row."
  - truth: "A completed download can be opened from the Downloads panel (BROWSE-04's 'opened from it' half)"
    now_covered_by: "tests/e2e/panel_click_test.py — OPEN launched xdg-open"
    how: "A fake `xdg-open` written to a temp dir at runtime and put first on the shell's PATH appends its argv to a file. The suite asserts the file is empty before the click and holds the launch after it, so 'the handler ran' is observed, not inferred. Nothing launches a real application."
  - truth: "Clicking Open launches xdg-open on exactly the path stored in the downloads list, never a re-derived path"
    now_covered_by: "tests/e2e/panel_click_test.py — OPEN launched xdg-open on <path>; plus gui::tests::the_open_button_launches_the_stored_path_verbatim"
    how: "T-03-01, set up as the security check it is: the same filename is downloaded twice so the second uniquifies to `report (1).pdf`, and Open is pressed on the FIRST row — the one whose stored path a re-derivation from the requested filename could not produce. The fake opener's argv must equal that row's stored path verbatim, be exactly one argument, and not contain `(1)`. W-1's missing unit test now also exists."
  - truth: "A failed Open action shows an inline error naming the filename and the OS error, with a Dismiss control, and never panics"
    now_covered_by: "tests/e2e/panel_click_test.py — FAILED open showed an inline notice / FAILED open named the file and the OS error / DISMISSED"
    how: "The shell is restarted with a PATH that has no `xdg-open` on it, so the spawn itself fails — note that 03-VERIFICATION's own suggested manual test (delete the file, then click Open) would NOT have provoked this, since `spawn` does not stat its argument. The `downloads.dismiss-error` control is drawn only while a notice exists, so its appearance is the notice's appearance and its disappearance after the Dismiss click is the clear. The shell's log is asserted to carry the path and `No such file or directory`, the two values the label is formatted from. `never panics` is asserted directly: the process is still running and `tabs_list` still answers."
accepted_gaps:
  - gap: "Same-document (pushState) navigation produces NO history row on Servo 0.4.0"
    classification: "Accepted v1 gap — measured false, not an unverified truth. Not an open item."
    addressed_in: "Not scheduled"
    evidence: "03-RESEARCH.md Pitfall 4 named it before execution; measured on Servo 0.4.0 (LoadStatus::Complete does not re-fire). tests/e2e/history_test.py prints `PUSHSTATE PROBE: N -> N rows` every run rather than asserting, so the suite reports the day Servo changes. Closing it needs a signal libservo does not expose to embedders. Success Criterion 1 does not mention SPA route changes."
---

# Phase 3: Table-Stakes Browsing — Verification Report

**Phase Goal:** Talaria is usable as an actual daily-driver browser, not just an agent automation surface.
**Verified:** 2026-08-20T09:35:09Z
**Amended:** 2026-08-20T14:05:00Z — six behavior-unverified items automated; see the amendment below
**Status:** human_needed
**Re-verification:** No — initial verification, amended once

---

## Amendment — 2026-08-20

This report's central claim about *why* nine items were human-only was **wrong**, and the six
items that rested on it have been automated rather than argued about.

The claim was: "there is no way to drive or snapshot an egui widget in this repo." The second
half is true and remains true — there is no snapshot infrastructure here, and three items below
still turn on visual judgment. The first half was false when it was written:
`tests/e2e/vault_ui_test.py` and `tests/e2e/takeover_test.py` were already driving real pointer
clicks at real egui widgets with `xdotool` under Xvfb. What they lacked was a way to *find* a
widget: both hardcoded a screen coordinate, which is why `vault_ui_test.py`'s constant moved four
times in this phase and why W-2 below asked for it to be closed.

Closing W-2 closed six of the nine items with it. The shell now answers a `chrome_rects` command,
gated on `TALARIA_TEST_HOOKS=1` exactly as the `evaluate` crash hook is, that reports the logical
rect of each named chrome control — the toolbar's buttons and the rows of whichever panel is
open, collected from the `Response`s `Gui` already computes. A suite looks a control up by name
and clicks where the chrome says it is. `tests/e2e/panel_click_test.py` uses it to drive all six.

What changed in the numbers: **33/42 → 41/42**, **behavior_unverified 9 → 3**. What did not
change: `status:`, which is not this amendment's to set.

The three that remain are genuine visual judgment and are **not** faked with pixel sampling —
accent-colour discipline, "visited pages appear in a history list" as a *rendering* claim, and
the panels not overlapping the toolbar or tab strip. A test that passed without exercising the
behaviour would be worse than an honest manual step.

One reclassification, unrelated to the above: the same-document (`pushState`) entry was filed
under `deferred:` with a `truth:` key, which reads as an open item awaiting work. It is not. It
was **measured false** on Servo 0.4.0 before this phase ended and accepted as a v1 gap. It now
sits under `accepted_gaps:`, stated as the measured fact rather than as a truth.

---

## Verdict

The phase goal is **achieved in the code**. All four success criteria are implemented, wired end to
end, and persisted to disk; all 14 prohibitions hold with evidence; the single UI-SPEC backstop is
genuinely resolved with a real measurement, not an assertion of intent. There are **no blockers and
no gaps**.

The status was originally `human_needed` for one structural reason: **this is a native egui GUI
phase, and the repo has no way to drive or snapshot an egui widget.** ~~Every truth that ends at a
click inside a panel — clicking a history/bookmark row, the two-step Clear confirm, the failed-Open
notice, and the Downloads "Open" launch — is present and correctly wired but unexercised by any
test.~~

**Superseded by the amendment above.** Half that reason was false: the repo could already *drive*
an egui click and does so in two suites; it merely could not find a widget without hardcoding a
coordinate. All four of the struck items are now driven by `tests/e2e/panel_click_test.py`. What
survives is the other half — no *snapshot* infrastructure exists — which leaves exactly three
visual-judgment items. 03-VALIDATION.md's manual-only declaration was an honest, pre-declared
boundary for those three; for the other six it was a cost estimate that turned out to be wrong.

What was verified *behaviorally* is the half that carries the persistence risk, and it was verified
well: three new e2e suites drive real navigations, real `Ctrl+D` keystrokes via `xdotool`, and real
HTTP downloads, then **kill the process and start a new one against the same `XDG_CONFIG_HOME`**.

---

## Goal Achievement — Roadmap Success Criteria

| # | Success Criterion | Status | Evidence |
|---|---|---|---|
| 1 | Visited pages appear in a history list with URL, title, and time, and survive a restart | ✓ VERIFIED | Capture: `app.rs:2230` pushes to `pending_history_writes` from the `LoadStatus::Complete` arm via `try_borrow_mut`; drained at `app.rs:369` which applies the `TabOwner::Me` filter, skips `about:blank`, and calls `History::append(url, title, now_ms())`. Persistence: `history_test.py` asserts url+title+`visited_at_ms > 0`, then `stop_shell()` → terminate → poll `shell.poll()` → wait for socket removal → `start()` a **new process** → both rows still present in order. Rendering half → human item |
| 2 | A user can bookmark a page and return to it from a bookmarks list | ✓ VERIFIED | **Bookmark half:** `bookmarks_test.py` drives real `Ctrl+D` via `xdotool`, asserts one entry with url/title/`created_at_ms`, presses again to assert removal (true toggle), again to re-add, then restarts the process and asserts survival. **"Return to it" half, verified in the amendment:** `panel_click_test.py` bookmarks the displayed page, opens a second page so the displayed tab is demonstrably elsewhere, clicks `bookmarks.row.0` at the rect the chrome reports, and asserts over the socket that the displayed tab came back to the bookmarked URL and that the panel closed on the same click |
| 3 | Typing a non-URL into the address bar searches the user's configured engine, not a hardcoded one | ✓ VERIFIED | `resolve_location(input, engine)` at `app.rs:1488` — the hardcoded literal is gone from the primary path. Chain: `Settings::load()` at `app.rs:868` → `Shared::settings` → read at navigation time (not cached) at `app.rs:1250` and `app.rs:1733`. Save path: `UiAction::SaveSearchEngine` → `Settings::save` (in-memory + atomic temp+rename). 19 unit tests (14 `settings::tests` + 5 `app::tests`) incl. `url_shaped_input_is_unaffected_by_the_engine` for zero-regression parity |
| 4 | A completed download appears in a downloads list and can be opened from it | ✓ VERIFIED | **"Appears" half:** `downloads_list_test.py` performs real HTTP downloads and asserts filename, exact uniquified path (`row["path"] == result["path"]`), url, bytes, on-disk size; a second same-name download gets its own `(1)` row without rewriting the first; a cap-refused download adds **no** row; both paths present verbatim in `downloads.json`; survives a process restart in order. **"Opened from it" half, verified in the amendment:** `panel_click_test.py` puts a fake `xdg-open` first on the shell's PATH, clicks `downloads.open.1` — the *first* of two same-named downloads — and asserts the opener's argv is exactly that row's stored path, so the launch is observed and so is its payload |

**Score:** 41/42 must-haves verified (1 present, behavior-unverified) — see disposition below.

The 42 are 38 truths plus the 4 roadmap success criteria. The one outstanding is the
accent-colour truth. The other two entries under `behavior_unverified_items` — BROWSE-01's
*rendering* half and the panel-layout check — sit **inside** must-haves already counted verified
on their store and behaviour halves; they are listed because a human should still look at them,
not because a must-have is outstanding. That is why `behavior_unverified: 3` and `41/42` do not
sum the way the original `9` and `33/42` did.

---

## `must_haves` Disposition

### Truths (38 across 4 plans)

| Plan | Truths | ✓ Verified | ⚠️ Behavior-unverified | Notes |
|---|---|---|---|---|
| 03-01 history | 12 | 12 | 0 | Row-click navigation and the two-step Clear confirm + reset-on-close both driven by `panel_click_test.py` |
| 03-02 bookmarks | 10 | 9 | 1 | Row-click navigation now driven; accent-colour discipline remains a visual judgment |
| 03-03 search engine | 6 | 6 | 0 | Fully unit-verified — the one plan with no GUI-click dependency in its truths |
| 03-04 downloads | 10 | 10 | 0 | Open launch, exact-stored-path payload and the failed-Open notice all driven by `panel_click_test.py` |
| **Total** | **38** | **37** | **1** | + all 4 roadmap SCs now behaviorally verified → 41/42 merged |

**Notable truths confirmed against source rather than taken on the SUMMARY's word:**

- *"Agent-owned tabs must not write to the human's history"* — the filter is real and is the whole of
  it: `app.rs:378` `if !matches!(tab.owner, TabOwner::Me) { continue; }`. Proven by a **positive
  negative assertion** in `history_test.py`: it opens an agent tab via `tabs_open` on a separate
  session, sleeps 3s so "no row" means the filter held rather than that the write hadn't landed yet,
  then asserts the row count is unchanged and no `/agent` URL appears.
- *"Revisiting the same URL appends a second row; never merges"* — `history.rs` has
  `the_same_url_twice_is_two_entries_not_one_updated_one`. History is a log, not a set.
- *"Rows render in reverse-chronological order; identical timestamps preserve append order rather
  than being re-sorted unstably"* — verified **by construction, the strongest available form**:
  `grep -n sort` finds **zero** sort calls in `history.rs`, `bookmarks.rs`, `downloads.rs`, or in
  `gui.rs`'s row loops. All three panels use `entries.iter().rev()` over an append-ordered `Vec`.
  There is no unstable sort because there is no sort. Each store additionally has an
  `entries_keep_their_append_order_across_a_reload` test.
- *"Degrade, never abort"* — every store has explicit corrupt/empty/missing/unreadable tests:
  history 17, bookmarks 14, downloads 14, settings 14 (`cargo test -p talaria-shell -- --list`).
- *"No capture path calls `borrow_mut()` directly inside a `WebViewDelegate` callback"* — confirmed:
  `app.rs:2231` uses `try_borrow_mut` with `log::error!` on contention. The remaining bare
  `borrow_mut()` calls inside the delegate impl (2165, 2255, 2275, 2303) are on `pending_tab_work`,
  pre-existing from earlier phases, and nested inside an already-successful `try_borrow_mut` arm.
- *"`main.rs`'s call site passes the default engine explicitly"* — `main.rs:41`
  `resolve_location(&input, &settings::SearchEngine::default())`. Correct: this runs before `Shared`
  exists, so a borrowed `Settings` genuinely could not be passed there.
- *"`event_proxy` is new plumbing"* — confirmed. `Shared::event_proxy` (`app.rs:108`) is a new field;
  the download thread clones it *before* `spawn` (`app.rs:1754`) because `Rc<Shared>` is not `Send`,
  and `AppEvent::DownloadCompleted` is applied on the main thread at `app.rs:923`. The send is gated
  on `Outcome::Ok` specifically (`app.rs:1769`), which is what makes "a cancelled or failed download
  never produces a row" true — and `downloads_list_test.py` proves it with a cap-refused download.

### Backstops (1)

| Backstop | Status | Evidence |
|---|---|---|
| UI-SPEC `long-text` on the Settings URL-template / engine-name fields | ✓ **RESOLVED with evidence** | `gui::tests::a_long_template_scrolls_inside_the_field_rather_than_widening_it`. **I ran it in my own process — passes.** It lays out a real 4,000-character space-free template *and* an ordinary one through `egui::__run_test_ui` (egui's own headless harness, no window/GL/display), then asserts `long_width == short_width`, `long_width == 320.0`, and `available > long_width`. This is a genuine layout measurement, not a restated intention — exactly what the backstop asked for. Not `insufficient_spec` |

The UI-SPEC's `## UI Considerations` lift (24 covered / 1 backstop / 0 unresolved) holds. Spot-checked
beyond the backstop: all three empty states present verbatim (`Nothing visited yet.` gui.rs:1003,
`Nothing bookmarked yet.` :1088, `Nothing downloaded yet.` :1177); `truncate_chars(&full, 60)` +
`on_hover_text` on all three lists, with `truncate_chars` covered by 3 unit tests including
`truncation_counts_characters_not_bytes`; `ui.add_enabled(valid, egui::Button::new("Save"))` at
gui.rs:1297 with a redundant `&& valid` on the click; `ScrollArea::vertical()` wrapping all four
list panels (825, 988, 1073, 1139). The Downloads tooltip correctly shows `entry.path` (the written
path) rather than `entry.filename`, matching the security note.

### Prohibitions (14) — all HELD

| # | Prohibition | Status | Evidence |
|---|---|---|---|
| 1 | No MCP tool exposes browsing history | ✓ HELD | `grep -ci 'history\|bookmark\|search_engine\|settings' crates/talaria-mcp/src/tools.rs` → **0** (run by me) |
| 2 | Agent-owned tabs must not write to the human's history | ✓ HELD | Negative e2e assertion in `history_test.py` + `TabOwner::Me` filter at `app.rs:378` |
| 3, 6, 10, 13 | No new dependency added (all four plans) | ✓ HELD | `git diff --stat dc027e0 HEAD -- Cargo.toml Cargo.lock` empty (orchestrator-verified) |
| 4, 7, 11, 14 | egui family stays pinned at 0.34.3 | ✓ HELD | `Cargo.toml:47-49` — `egui`, `egui-winit`, `egui_glow` all `0.34.3`; no diff |
| 5, 9, 12 | No MCP tool for bookmarks / search config / downloads opener | ✓ HELD | `grep -c 'downloads_open\|downloads_list' tools.rs` → 0; `git diff --stat crates/talaria-mcp` empty. Structurally reinforced: **exactly one `process::Command` site exists in the whole workspace** (`app.rs:1377`), reachable only from `UiAction::OpenDownload` |
| 8 | `resolve_location`'s signature change must not touch `parse_agent_url` | ✓ HELD | I inspected the hunk directly rather than trusting the header. `@@ ... fn parse_agent_url` is git's *enclosing-context* heuristic; the changed lines are the doc comment and signature of `resolve_location`, which follows it. `parse_agent_url`'s body is untouched. The two trust roots remain separate, and `app::tests::a_data_url_is_searched_for_rather_than_opened` now pins the asymmetry deliberately |

No prohibition required flagging: every one carries real enforcement evidence, so none fell to the
fail-closed `unverified` default.

---

## Requirements Coverage

| Requirement | Status | Recommendation |
|---|---|---|
| BROWSE-01 — local browsing history (URL, title, timestamp) | ✓ SATISFIED | **Move to Complete.** Store + capture + Me-filter + restart survival all automated |
| BROWSE-02 — bookmark and revisit pages | ✓ SATISFIED | **Move to Complete.** "Revisit" wiring is source-verified; the click is a declared manual check, not a missing capability |
| BROWSE-03 — configurable default search engine | ✓ SATISFIED | **Move to Complete — but strike the stale annotation.** See W-3 |
| BROWSE-04 — downloads list the user can open | ✓ SATISFIED | **Move to Complete.** Listing automated; the launch is a declared manual check |

All four are already `[x]` and already `Complete` in the tracking table at REQUIREMENTS.md:164-167.
No orphaned requirements: REQUIREMENTS.md maps exactly BROWSE-01..04 to Phase 3, and all four are
claimed by plans.

---

## Anti-Patterns Found

**None.** Scanned all seven phase-modified source files (`history.rs`, `bookmarks.rs`,
`downloads.rs`, `settings.rs`, `gui.rs`, `app.rs`, `main.rs`) for `TODO|FIXME|XXX|TBD|HACK|
PLACEHOLDER|not yet implemented|coming soon` — **zero matches**. The debt-marker gate does not fire;
completion is auditable.

---

## Honest Gaps — Classification Review

| # | Gap | Correctly a deferral? | Assessment |
|---|---|---|---|
| 1 | `pushState` / same-document navigation produces no history row — **an accepted v1 gap, measured false, not an open item** (reclassified out of `deferred:` into `accepted_gaps:` in the amendment) | ✅ **Yes — correctly deferred** | This is the *measured answer* to a research question posed before execution (03-RESEARCH.md Open Question 1 / Pitfall 4), not a discovered shortfall. Success Criterion 1 says "visited pages," and a SPA route change is not a page load Servo 0.4.0 reports. The handling is better than a comment: the probe **prints its finding on every run** and asserts only that rows aren't lost and the shell still answers — so the day libservo starts firing `Complete` for same-document navigation, the suite says so out loud. Closing it needs a signal libservo does not expose to embedders. Correctly classified |
| 2 | ~~`vault_ui_test.py`'s hardcoded toolbar coordinate moved 4× (239→283→341→370→**399**)~~ **CLOSED** | ✅ **Was expiring; now paid** | Was: confirmed still hardcoded at `vault_ui_test.py:76` `CREDENTIALS_BUTTON = ("399", "11")`, four moves each discovered as a red suite. Now: the constant is the element name `toolbar.credentials`, resolved at run time from the shell's own layout through the `chrome_rects` test hook. See W-2 |
| 3 | `.claude/CLAUDE.md` and `CHANGELOG.md` staleness | ❌ **No — this is not a deferral** | Not an unmet *criterion*, but not correctly parked either: it is a live inaccuracy in the project's own instruction file plus a convention violation. See WARNINGS W-3/W-4 |

---

## Warnings (no blockers)

**W-1 — RESOLVED.** ~~03-VALIDATION.md overstates its automation coverage for the Open path.~~
The claimed unit test now exists — `gui::tests::the_open_button_launches_the_stored_path_verbatim`,
added by the CR-04 fix after this report was written — and the amendment adds the end-to-end half
on top of it: `panel_click_test.py` asserts the fake opener's argv against the stored path of the
first of two same-named downloads. The original finding follows.

**W-1 (original) — 03-VALIDATION.md overstates its automation coverage for the Open path.**
The Manual-Only table justifies deferring the external-launch check by asserting that "the two halves
that carry the risk are automated instead: the UI action carries exactly the stored path (**unit**),
and no MCP tool can reach the opener (source assertion)." The second half is real. **The first half
is not automated.** `gui::tests` contains exactly 9 tests — 3 truncation, 3 relative-time, 2
human-bytes, 1 long-template layout — and **none** asserts that `UiAction::OpenDownload` carries
`entry.path`. The code is correct by inspection (`gui.rs:1214` clones `entry.path` verbatim, with a
comment explaining why it is not rebuilt from `entry.filename`), and the residual risk is genuinely
low given the single `process::Command` site. But the validation contract claims a unit test that
does not exist. Either add it (the `UiAction` vec is returned by value and is trivially assertable
without egui) or correct the table.

**W-2 — RESOLVED.** ~~The toolbar-coordinate deferral should be closed before the next
chrome-touching phase.~~ Closed, by the recommended option: the shell exposes each chrome
control's rect behind `TALARIA_TEST_HOOKS=1`, and `vault_ui_test.py` looks the credentials button
up by name (`toolbar.credentials`) instead of carrying `("399", "11")`. The suite still clicks the
real toolbar button, which is the coverage its docstring claims; the `Ctrl+K` shortcut was not
used, so nothing was routed around. The comment recording the ~29-points-per-icon-button rate is
gone with the constant — it documented a cost that no longer exists. A sixth toolbar button now
costs this suite nothing. The original finding follows.

**W-2 (original) — four moves, four red-suite discoveries.** Both recorded candidate fixes remain
untaken. The `TALARIA_TEST_HOOKS=1` rect-exposure option is the one that preserves the coverage the
suite's own docstring claims (the toolbar-button path) rather than routing around it via `Ctrl+K`.

**W-3 — `.claude/CLAUDE.md` now contains two false statements about this codebase.**
Line 94 states "No `.env` files, **no config file format**. All configuration is environment
variables plus CLI argv." Plan 03-03 made this false: `settings.rs` reads and writes
`config.json` (`settings.rs:129,146`). The Component Responsibilities table also has **no rows** for
`history.rs`, `bookmarks.rs`, `downloads.rs`, `settings.rs`, or `Shared::event_proxy` — verified by
grep, zero matches. Since CLAUDE.md is loaded as authoritative project instruction on every session,
a stale entry here actively misinforms future work.

**W-4 — CHANGELOG.md covers BROWSE-04 only.**
`## [Unreleased] → Added` has a single, well-written BROWSE-04 entry. BROWSE-01 (history),
BROWSE-02 (bookmarks) and BROWSE-03 (configurable search engine) have no entries. The global project
convention is "Log all changes in a project CHANGELOG.md file." Three of the phase's four
user-visible features are unlogged.

**W-5 — REQUIREMENTS.md:82 carries a stale "partial" annotation that now contradicts itself.**
Independently discovered, not listed in `deferred-items.md`:

> `- [x] **BROWSE-03**: ... — **partial**: DuckDuckGo is hardcoded (crates/talaria-shell/src/app.rs:853-869)`

The requirement is ticked `[x]` and the tracking table says `Complete`, while the inline note still
says it is partial and hardcoded. Phase 3 falsified that note — `resolve_location` now takes a
`&SearchEngine`, and the cited line range `app.rs:853-869` no longer contains the function (it is at
`app.rs:1488`). The `partial:` clause and the line citation should be struck.

---

## Behavioral Spot-Checks (run in this process)

| Behavior | Command | Result | Status |
|---|---|---|---|
| The 320.0pt backstop test genuinely passes | `cargo test -p talaria-shell gui::tests::a_long_template_scrolls_inside_the_field_rather_than_widening_it -- --exact` | `1 passed; 0 failed` | ✓ PASS |
| New store tests exist at the promised density | `cargo test -p talaria-shell -- --list` | history 17, bookmarks 14, downloads 14, settings 14, gui 9, app 5 (83 total) | ✓ PASS |
| No MCP surface for history/bookmarks/settings | `grep -ci 'history\|bookmark\|search_engine\|settings' crates/talaria-mcp/src/tools.rs` | `0` | ✓ PASS |
| egui family still pinned at 0.34.3 | `grep -n egui Cargo.toml` | `0.34.3` ×3 | ✓ PASS |
| `parse_agent_url` body untouched | `git diff dc027e0 HEAD -- app.rs` hunk inspection | Change confined to `resolve_location` doc + signature | ✓ PASS |
| No sort anywhere in stores or row loops | `grep -n sort` across 3 stores + gui.rs | Zero sort calls; all `.iter().rev()` | ✓ PASS |
| Zero debt markers in phase-modified files | `grep -E 'TODO\|FIXME\|XXX\|TBD\|HACK\|PLACEHOLDER'` ×7 files | Zero matches | ✓ PASS |
| All three new e2e suites registered | `grep run_all.py` | `history_test`, `bookmarks_test`, `downloads_list_test` in the standalone block | ✓ PASS |
| Downloads "Open" actually launches an application | ~~—~~ `panel_click_test.py` | ~~Requires a real GUI session and a foreign process assertion~~ A fake `xdg-open` first on the shell's PATH records its argv; the click is a real pointer click at the reported rect | ✓ PASS (amendment) |

Full-suite results (`cargo build --release`, `clippy -D warnings`, `cargo test` 85/85,
`run_all.py` 18/18) were independently established by the orchestrator and are not re-run here.
**Amendment:** `cargo test` is now 117 (114 shell + 3 protocol) and `run_all.py` is 19/19
`failed: none`, with `panel_click_test` added.

---

## Human Verification Required

**Three items**, listed in full in the `behavior_unverified_items` frontmatter above. Six of the
original nine — including both of the "highest-value two" this section used to name — are now
automated; see `behavior_verified_since` in the frontmatter for which test covers each, and the
amendment at the top for why the original nine-way root cause was only half right.

What is left shares one real root cause: **there is no snapshot infrastructure in this repo**, and
all three are questions about how something *looks*, not about what it does.

1. **Accent-colour discipline** — that the reserved accent appears on the active bookmark star and
   the active panel-toggle buttons, and nowhere decorative.
2. **"Visited pages appear in a history list"** as a *rendering* claim — three rows, newest first,
   each with title/URL and a relative time, long titles truncating with the full text on hover. The
   store half, the Me-filter and restart survival are all automated; that a human can read the
   result is not.
3. **The three new list panels do not overlap the toolbar or tab strip**, in both Me and Agents
   view, with the Me/Agents toggle still reading correctly.

These were deliberately **not** faked with pixel sampling. A test that sampled a colour at a
coordinate would pass without exercising the judgment the item is actually about, and a test that
passes without exercising the behaviour is worse than an honest manual step.

---

## Gaps Summary

**No gaps.** No truth FAILED, no artifact is missing or a stub, no key link is unwired, no blocker
anti-pattern exists, and all 14 prohibitions hold with evidence.

All 11 declared artifacts exist, are substantive, are wired, and have data flowing:
`history.rs` (17 tests), `bookmarks.rs` (14), `downloads.rs` (14), `settings.rs` (14), plus the
`app.rs` / `gui.rs` integration points and the three new e2e suites — each of which restarts the
process rather than reloading in-memory state.

The phase goal — "usable as an actual daily-driver browser" — is met in substance: a human can
browse with persistent history, bookmark and return to pages, point the address bar at their own
search engine, and see what they have downloaded, with every one of those four stores surviving a
restart and none of them reachable by an agent.

The remaining work is confirmation, not construction, plus five documentation warnings (W-1..W-5)
that should be cleared before the milestone closes.

**Amendment:** the remaining confirmation is now three visual-judgment items rather than nine, and
two of the five warnings (W-1, W-2) are resolved. W-3, W-4 and W-5 — the stale `.claude/CLAUDE.md`
statements, the CHANGELOG's BROWSE-01/02/03 gap, and REQUIREMENTS.md:82's contradictory "partial"
annotation — were not in this amendment's scope and still stand.

**Status recommendation, not a status change.** With the six behavioural items closed, nothing
outstanding is a question about what the code *does* — all three remaining items are questions
about how it *looks*, and none of them can be settled without a human or without snapshot
infrastructure this repo does not have and this amendment did not invent. On the evidence,
`status:` should become `passed` with the three visual checks carried as a standing human item, or
stay `human_needed` if the convention here is that any outstanding visual check holds the phase.
That call belongs to the orchestrator; `status:` is left untouched at `human_needed`.

---

_Verified: 2026-08-20T09:35:09Z_
_Verifier: Claude (gsd-verifier)_
_Amended: 2026-08-20T14:05:00Z — six behavior-unverified items automated behind a new
`chrome_rects` test hook; W-1 and W-2 resolved; the `pushState` entry reclassified as an accepted
gap. `status:` deliberately unchanged._

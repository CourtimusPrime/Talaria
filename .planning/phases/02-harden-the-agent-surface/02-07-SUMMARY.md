---
phase: 02-harden-the-agent-surface
plan: 07
subsystem: shell
tags: [agent-04, events, owner-filtering, reentrancy, deferred-queue, e2e]

# Dependency graph
requires:
  - phase: 02-01
    provides: green e2e baseline and the enforcing pre-commit hook, so a suite failure here is attributable to this plan
  - phase: 02-05
    provides: the background reader on the MCP side that makes an unsolicited message structurally possible (D-14); untouched here
  - phase: 02-06
    provides: the callback-safety precedent — a terminus reachable from inside a servo callback must use a primitive that cannot fail or block
provides:
  - "Shared::queue_event(owner, event) — the only way to raise a lifecycle event; replaces broadcast_event"
  - "Shared::pending_events + PendingEvent { session_id, event } — one addressee per entry"
  - "Shared::pending_tab_work + TabWork { AdoptPopup, MarkCrashed, Close, UrlChanged } — deferred tab-table mutations keyed by WebView"
  - "Shared::process_pending_events / process_pending_tab_work, chained off process_pending_captures"
  - "Shared::adopt_popup and apply_url_change — shared by the inline callback and the deferred drain"
  - "tests/e2e/crash_event_test.py — own-tab delivery, cross-session silence, human-owned silence"
affects: [02-08 MCP notifications, 02-11 CI]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Owner-addressed queueing: the addressee is a plain session id, so a human-owned tab produces no entry at all — the filter is the type, not a check at the send"
    - "Deferred tab-table work keyed by WebView rather than tab id, because resolving a webview to a tab is itself the borrow the callback could not take"
    - "Every residual try_borrow in a delegate callback is a match with a non-empty Err arm: either a queue push or an error log"
    - "One helper shared by the inline and deferred paths, so a deferred mutation is byte-for-byte the one the callback would have made"
    - "Negative-control verification: revert only the implementation file, confirm the new assertion fails, restore"

key-files:
  created: []
  modified:
    - crates/talaria-shell/src/app.rs
    - tests/e2e/crash_event_test.py

key-decisions:
  - "PendingEvent carries a plain u64 session id, not an Option: a human-owned tab has no agent addressee, so D-12's rule is expressed in the type rather than re-checked at every send"
  - "The filter keys on the tab's owner, never on the requesting session — an agent may close a tab it does not own, and the notification still belongs to whoever owned it"
  - "TabWork is keyed by WebView, not by tab id, because the webview→tab lookup is the tab-table borrow that failed in the first place"
  - "Both drains are chained off process_pending_captures rather than added to each winit handler: all three handlers already call it, so no handler needed touching"
  - "Tab work drains before events, because applying the deferred work is what raises the events the second drain delivers"
  - "The two marking callbacks (notify_new_frame_ready, notify_load_status_changed) log at error rather than deferring: their queues are never borrowed across a call into servo, so contention there is an invariant violation, not routine churn"
  - "AGENT-04 stays In Progress: the shell half is correct, but nothing reaches an MCP client until plan 02-08 fills the proxy's event arm"

patterns-established:
  - "A notification path must have no silent-drop branch: every failed borrow either defers the work or logs loudly"
  - "Read the addressee before the mutation that destroys it — owner before close, on all four close paths"

requirements-completed: []

coverage:
  - id: D1
    description: "A tab crash or close produces an event for the session that owns that tab"
    requirement: AGENT-04
    verification:
      - kind: e2e
        ref: "tests/e2e/crash_event_test.py — the actor opens a tab, crashes it through the test hook and reads tab_crashed for that tab id on its own connection; opens a second tab, closes it and reads tab_closed"
        status: pass
    human_judgment: false
  - id: D2
    description: "A session is told nothing about another session's tabs"
    requirement: AGENT-04
    verification:
      - kind: e2e
        ref: "tests/e2e/crash_event_test.py — the observer must receive no unsolicited message of any shape while three of the actor's events are delivered; asserted as a read timeout with empty buffers, not as the absence of a particular tab id"
        status: pass
      - kind: e2e
        ref: "negative control: reverting only crates/talaria-shell/src/app.rs fails it with `event-observer: unsolicited message (actor's tabs crashed and closed): {'type': 'event', 'event': 'tab_crashed', 'tab_id': 8}` — the exact leak"
        status: pass
    human_judgment: false
  - id: D3
    description: "A human-owned tab's close produces no agent-facing event at all"
    requirement: AGENT-04
    verification:
      - kind: e2e
        ref: "tests/e2e/crash_event_test.py — the actor issues open_for_user (which creates a Me-owned tab), closes it, and both connections are asserted silent. This is the assertion that fails an implementation keyed on the requesting session instead of the tab's owner"
        status: pass
      - kind: other
        ref: "code inspection: queue_event returns early on TabOwner::Me via a let-else, so a Me-owned tab cannot produce a PendingEvent"
        status: pass
    human_judgment: false
  - id: D4
    description: "The owning session is captured before the tab is removed, so a close still has an addressee"
    verification:
      - kind: other
        ref: "code inspection: all four close paths (UiAction::CloseTab, Command::TabsClose, notify_closed, TabWork::Close) read tabs.get(id).owner into a local before calling tabs.close(id)"
        status: pass
      - kind: e2e
        ref: "tests/e2e/crash_event_test.py asserts tab_closed arrives for two different tabs; reading the owner after the close would produce no event at all and the suite would time out"
        status: pass
    human_judgment: false
  - id: D5
    description: "An event raised from inside a servo delegate callback is queued and delivered from the loop rather than dropped when the session map is borrowed"
    verification:
      - kind: other
        ref: "code inspection: queue_event touches only pending_events (a Vec push) and request_redraw; the session map is read solely by process_pending_events, which runs on the loop. The old broadcast_event's try_borrow on self.sessions is gone — grep -c 'fn broadcast_event' is 0"
        status: pass
    human_judgment: true
    rationale: "The bug class is a silent drop under reentrancy, which by construction leaves no observable trace to assert on; it is designed out by moving the session-map read off the callback path entirely"
  - id: D6
    description: "A tab-table change raised from a delegate callback is deferred and applied from the loop rather than skipped"
    verification:
      - kind: other
        ref: "code inspection: notify_crashed, notify_closed, notify_url_changed and request_create_new are each a match on try_borrow_mut whose Err arm pushes a TabWork variant; process_pending_tab_work applies all four against the table from the loop"
        status: pass
      - kind: e2e
        ref: "popup_test still passes, proving the inline adoption path is unbroken after request_create_new was restructured to build the webview before taking the borrow"
        status: pass
    human_judgment: true
    rationale: "Forcing the tab table to be borrowed at the instant servo invokes one of these callbacks needs a fault-injection hook the tree does not have; the deferred branch is verified by inspection and by the shared helper being the same code the inline path runs"
  - id: D7
    description: "No remaining path silently discards work on a failed borrow"
    verification:
      - kind: other
        ref: "source: all 7 residual try_borrow sites in app.rs are match expressions with a non-empty Err arm — 4 queue pushes and 3 error logs. grep -c 'dropping popup request' is 0; grep -c 'log::error!' is 4 (the pre-existing crash log plus three invariant-violation logs)"
        status: pass
    human_judgment: false
  - id: D8
    description: "The queues are drained with the split-scope idiom and a queued entry wakes the loop"
    verification:
      - kind: other
        ref: "code inspection: both drains std::mem::take the queue under a scoped borrow and act afterwards; neither holds that borrow across a call into servo or into the session map. queue_event and every TabWork producer call window.request_redraw()"
        status: pass
      - kind: e2e
        ref: "the suite's events arrive within its 10s read bound with no other traffic to piggyback on, so the redraw is what delivers them"
        status: pass
    human_judgment: false

# Metrics
duration: 47min
completed: 2026-08-16
status: complete
---

# Phase 2 Plan 07: Owner-Addressed Tab Lifecycle Events Summary

**`broadcast_event` — which told every connected agent about every other agent's tab ids, and dropped the event entirely whenever the session map happened to be borrowed — is gone, replaced by a queue addressed to the one session that owns the tab, drained from the event loop alongside a second queue that keeps a crash mark, a close, a URL update or a whole popup instead of skipping it when the tab table is busy.**

## Performance

- **Duration:** 47 min
- **Started:** 2026-08-16T08:12Z
- **Completed:** 2026-08-16T08:59Z
- **Tasks:** 3
- **Files modified:** 2

## Accomplishments

- **The unfiltered fan-out is deleted, not bypassed.** `grep -c 'fn broadcast_event'` is 0. The only way to raise a lifecycle event now is `queue_event(&owner, event)`, which returns without doing anything for a `TabOwner::Me` tab and otherwise pushes a `PendingEvent { session_id, event }`. The addressee is a plain `u64`, not an `Option<u64>` — a human-owned tab has no agent addressee, so D-12's rule is a property of the type rather than a check someone can later forget.
- **The filter keys on the tab's owner, never on the requester.** An agent may close a tab it does not own; the notification still belongs to whoever owned it. The e2e suite's human-owned-tab assertion is precisely the case that fails a requester-keyed implementation, and it is the reason that assertion exists.
- **Every close path reads the owner before the close.** All four — the UI action, `Command::TabsClose`, `notify_closed`, and the deferred `TabWork::Close` — resolve `tabs.get(id).owner` into a local first. Closing removes the tab, so an event raised afterwards has nothing left to key an addressee on; the naive ordering produces no event at all.
- **Seven silent-drop sites became four deferrals and three loud logs.** `request_create_new`, `notify_crashed`, `notify_closed` and `notify_url_changed` now push a `TabWork` entry when the tab table is busy, so a busy table costs a loop turn rather than a whole popup, a crash mark, a close, or a URL update. `notify_new_frame_ready` and `notify_load_status_changed` keep their existing deferred-queue marking but log at **error** level on a failed borrow: those queues are never borrowed across a call into servo, so contention there is an invariant violation, and it should never be invisible again.
- **`request_create_new` builds the framebuffer and webview before touching the table.** Neither needs the borrow, so the callback always gets far enough to defer with a complete webview in hand — which is what makes the deferral possible at all. The `warn!("dropping popup request")` line is gone; a deferred popup is routine churn, so it is a `debug!`.
- **The inline and deferred paths run the same code.** `adopt_popup` and `apply_url_change` are shared helpers, so a deferred adoption produces the same tab with the same activation rule, and a deferred URL update makes byte-for-byte the same change — including the exact `about:blank` comparison that plan 02-02's allowlist and the popup grace window both depend on.
- **`tests/e2e/crash_event_test.py` was inverted and proved to bind.** It previously asserted that a *non-owner* observes another session's events — the exact leak being removed. It now asserts own-tab delivery, cross-session silence, and human-owned silence. Negative control: reverting **only** `crates/talaria-shell/src/app.rs` fails it with `event-observer: unsolicited message (actor's tabs crashed and closed): {'type': 'event', 'event': 'tab_crashed', 'tab_id': 8}`.
- Full regression is **12/12 PASS, exit 0**, run twice at the tip. Clippy is back to exactly the 7 pre-existing lints owned by plan 02-11; no new lint.

## Task Commits

Each task was committed atomically:

1. **Task 1: Add the owner-addressed event queue and the deferred tab-work queue (D-13)** - `3b4ed62` (feat)
2. **Task 2: Convert the seven drop sites and capture the owner before every close (D-12, D-13)** - `334ee6a` (fix)
3. **Task 3: Prove owner addressing end to end** - `ee44dfb` (test)

**Plan metadata:** see the `docs(02-07)` commit following this summary.

## Files Created/Modified

- `crates/talaria-shell/src/app.rs` — new `PendingEvent` struct and `TabWork` enum; `Shared::pending_events` and `Shared::pending_tab_work` fields; `queue_event`, `process_pending_events`, `process_pending_tab_work` and the `adopt_popup` helper on `Shared`; the free `apply_url_change`; both drains chained off `process_pending_captures`; `broadcast_event` deleted and all five call sites converted; all five delegate callbacks restructured. `next_capture_deadline` is deliberately unchanged — these queues are ready the moment they are enqueued.
- `tests/e2e/crash_event_test.py` — rewritten around a `Wire` class reading the raw socket, buffering events that replies overtake. Three blocks: own-tab crash and close delivery, cross-session silence, human-owned silence.

## Decisions Made

- **The addressee is a `u64`, not an `Option<u64>`.** Filtering at the point of queueing rather than at the point of sending means the "no addressee" case has no representation in the queue at all, so no future drain can accidentally fan out.
- **`TabWork` is keyed by `WebView`, not by tab id.** Resolving a webview to its tab *is* a tab-table read — the borrow that just failed. Keying by tab id would require the callback to do the thing it could not do.
- **Both drains chained off `process_pending_captures`.** All three winit handlers already call it, so both queues are reachable from every one of them without touching a single handler. Tab work drains first because applying it is what raises the events the second drain then delivers.
- **Nothing added to `next_capture_deadline`.** These queues are not deadline-driven — an entry is ready the instant it is enqueued — so the wake mechanism is `request_redraw()`, which `queue_event` and every `TabWork` producer call. (Contrast 02-06, where the in-flight registry *did* need a deadline source, because nothing else would ever expire it.)
- **Defer where the work is recoverable, log where it is not.** The four tab-table callbacks can defer, because the loop can apply the same change later. The two marking callbacks cannot usefully defer — the mark's whole purpose is to be seen by the next drain — so they log at error. Both queues are only ever borrowed inside a scope that makes no servo call, which is exactly why a failure there is an invariant violation rather than routine contention.
- **A single `try_borrow_mut` in `notify_closed`, not the two the plan describes.** The plan suggested reading the owner under one scoped borrow and closing under a second. One borrow covering both is strictly safer (no window in which another path can close the tab between the read and the close) and matches what the code already did.
- **AGENT-04 left In Progress, not marked Complete.** See below.

## Requirements Status

**AGENT-04 is deliberately NOT marked complete.** This plan is the first half of a pair.

`REQUIREMENTS.md` states AGENT-04 as "Tab open/close/crash events reach MCP clients as MCP notifications, not only via polling". What landed here is the shell half: events are correctly addressed and can no longer be dropped. **Nothing reaches an MCP client yet** — `crates/talaria-mcp/src/socket.rs` still treats a `ServerMessage::Event` as noise to skip while hunting for a reply id. Plan 02-08 fills that arm, advertises the capability and emits the notification; only then is the requirement's literal claim true.

AGENT-04 moved `Pending` → `In Progress` in the traceability table, its bullet now records precisely what landed and what did not, and its checkbox stays unchecked. `requirements mark-complete` was deliberately not run. This follows the MCP-09 and MCP-10 precedent.

## Deviations from Plan

### Task-boundary adjustment

**The five `broadcast_event` call sites were converted in Task 1, not Task 2.**

Task 1's acceptance criteria require `grep -c 'fn broadcast_event'` to be 0 *and* `cargo build --release` to exit 0. Those two cannot both hold while five call sites still reference the deleted method, so the conversion had to land in Task 1. Rather than convert them wrongly-then-rightly, Task 1 converts them **correctly** — owner captured before the close on every path — and Task 2 does what it is actually about: the reentrancy work (the four deferred-queue branches, the `request_create_new` restructure, the two invariant-violation logs). Every commit is therefore correct on its own; no commit knowingly ships a broken ordering.

### Transitional state between Task 1 and Task 2

Task 1's commit (`3b4ed62`) carries one `dead_code` warning — the four `TabWork` variants are constructed only by Task 2's producers. Task 2 (`334ee6a`) removes it, and clippy at the plan tip is back to exactly the 7 pre-existing lints owned by 02-11. Recorded rather than worked around, exactly as 02-06 did for the same plan-shape reason: inventing a use in Task 1 to silence a warning Task 2 was about to resolve would have been worse.

### Auto-fixed Issues

**1. [Rule 1 - Bug] The silence assertion could only run once per connection**

- **Found during:** Task 3
- **Issue:** The first version of the suite used `socket.makefile()` and asserted silence by letting `readline()` time out. Python poisons a buffered reader once it has timed out: every subsequent read on that object raises `OSError: cannot read from timed out object` rather than timing out again. The suite needs to assert silence **twice** on the same connection (once for the cross-session check, once for the human-owned check), so the second assertion died with an `OSError` instead of passing or failing on its merits.
- **Fix:** The suite reads the raw socket through a small `Wire` class holding its own byte buffer. `readline(timeout)` returns `None` on timeout and leaves the connection usable, and `expect_silence` additionally asserts the buffers are empty first, so "silent" means genuinely nothing pending rather than nothing *new*.
- **Files modified:** `tests/e2e/crash_event_test.py`
- **Verification:** Both silence assertions now run on both connections; full suite green; the negative control still fails on the first of them.
- **Committed in:** `ee44dfb` (Task 3 commit)

**2. [Rule 2 - Missing Critical] `notify_load_status_changed`'s tab-table borrow was also a silent drop**

- **Found during:** Task 2
- **Issue:** The plan's drop-site table names `pending_loads` at `:1338-1339` for this callback, but the line above it — `self.tabs.try_borrow().ok().and_then(...)` — discards just as silently, and losing it costs the same thing: a `tabs_open` / `navigate` reply that waits out `LOAD_WAIT` instead of answering when the page finished. Leaving it would violate the plan's own truth that *no* remaining path discards work without a trace.
- **Fix:** That borrow became a `match` with an `Err` arm logging `load-status mark lost: tab table busy`. The `None` case (webview genuinely not in the table yet) stays silent, because that is routine rather than a loss.
- **Files modified:** `crates/talaria-shell/src/app.rs`
- **Verification:** `grep -c 'log::error!'` is 4; the whole delegate region has no `try_borrow` with an empty failure path.
- **Committed in:** `334ee6a` (Task 2 commit)

**3. [Rule 1 - Bug] `expect("just found")` removed from the popup path**

- **Found during:** Task 1
- **Issue:** The inline adoption did `tabs.get(parent_id).map(|t| t.owner.clone()).expect("just found")`. Hoisting the body into a shared helper made the invariant weaker — the deferred path resolves the parent on a *later* loop turn, by which time the parent may legitimately have gone away — so the `expect` would have become a genuine panic path rather than an unreachable one.
- **Fix:** `adopt_popup` uses a `let ... else { return }` for both the parent lookup and the owner read. A popup whose opener vanished before adoption is simply not adopted.
- **Files modified:** `crates/talaria-shell/src/app.rs`
- **Verification:** `popup_test` still passes; `grep -c 'unwrap()'` remains 0.
- **Committed in:** `3b4ed62` (Task 1 commit)

---

**Total deviations:** 3 auto-fixed (2 bugs, 1 missing-critical) + 1 task-boundary adjustment + 1 recorded transitional state
**Impact on plan:** None. Every truth in `must_haves` holds, no dependency was added, no protocol change, no wire-format change.

## Issues Encountered

- **The old `crash_event_test` fails at the Task 2 commit, by design.** It asserted the removed behaviour — a non-owner observing another session's events — so between `334ee6a` and `ee44dfb` the suite is 11/12. That failure *is* the behaviour change, and it is the same evidence the negative control later reproduced deliberately.
- **Events and replies share the stream, so any suite driving this path must buffer.** A crash event raised by command *N* routinely lands after the reply to command *N+1*, because the event is delivered on the next loop turn while the reply is sent from the dispatch. The suite's `rpc` buffers events it overtakes; a naive read-until-matching-id loop silently eats them.
- **`resolve_location` is the human path, so `open_for_user` cannot take a `data:` URL** — it would be sent to the search engine. The human-owned-tab block uses `https://example.com/` like the rest of the suite.

## Verification Evidence

- `cargo build --release` — exit 0 after every task.
- `cargo test` — 3 passed, 0 failed (2 protocol round-trip, 1 vault).
- `cargo clippy --all-targets -- -D warnings` — exactly the **7 pre-existing lints** owned by plan 02-11 (`talaria-mcp/src/tools.rs:97,132`; `talaria-shell/src/app.rs:778 ×2, 803 ×2`; `talaria-shell/src/tabs.rs:93`). **No new lint.** As in 02-05 and 02-06, the plan's "exits 0" cannot hold at this phase baseline; the honest reading is "no new lints", and that holds.
- `TALARIA_E2E_DISPLAY=:98 XDG_RUNTIME_DIR=/tmp/talaria-e2e-rt python3 tests/e2e/run_all.py` — **exit 0, 12/12 PASS**, run twice at the tip (after Task 3, and again after restoring from the negative control). No suite regressed against the 02-01 baseline verdict.
- **Negative control (the assertions bind):** `git checkout 3b4ed62~1 -- crates/talaria-shell/src/app.rs`, rebuild, run `run_all.py` → `crash_event_test` **exit 1**, `AssertionError: event-observer: unsolicited message (actor's tabs crashed and closed): {'type': 'event', 'event': 'tab_crashed', 'tab_id': 8}`. That is the leak this plan removes, reproduced on demand. Restored, rebuilt, full suite green again.
- Suite output at the tip: `event-actor got: {'type': 'event', 'event': 'tab_crashed', 'tab_id': 8}`; `tab_crashed delivered to the owning session`; `tab_closed delivered to the owning session`; `event-observer stayed silent for 1.5s (actor's tabs crashed and closed)`; `event-actor stayed silent for 1.5s (closed a human-owned tab)`; `event-observer stayed silent for 1.5s (another session closed a human-owned tab)`; `closing a human-owned tab produced no event on either connection`; `CRASH EVENT CHECKS PASSED`.
- Source criteria: `fn broadcast_event` 0; `fn process_pending_events` 1; `fn process_pending_tab_work` 1; `pub pending_events: RefCell` 1; `pub pending_tab_work: RefCell` 1; `sessions.borrow().get(` 1; `unwrap()` 0; `dropping popup request` 0; `log::error!` 4 (≥3 required); `about:blank` 5 (≥2 required); `CRASH EVENT CHECKS PASSED` 1.
- All 7 residual `try_borrow` sites in `app.rs` are `match` expressions with a non-empty `Err` arm — 4 queue pushes, 3 error logs. Verified by reading each.
- `python3 -c "import ast; ast.parse(open('tests/e2e/crash_event_test.py').read())"` — exit 0.
- `git diff --stat crates/talaria-protocol/src/lib.rs crates/talaria-shell/src/tabs.rs` across the whole plan — **empty**. No wire change and no tab-table API change.

## Known Stubs

None.

## Threat Flags

None. No new network endpoint, auth path, file-access pattern, or schema change at a trust boundary beyond the plan's own register. T-02-07-01, -02, -03, -06 and -07 are all mitigated and evidenced above. T-02-07-04 (recycled session id) and -05 (unbounded event queue) were accepted by the plan and are unchanged: session ids still come from a process-lifetime monotonic counter, and the queue still drains on every loop turn.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- **AGENT-04 is In Progress, not Complete — plan 02-08 closes it.** The contract 02-08 consumes is now in place: by the time an event reaches `ServerMessage::Event` on the socket, the shell has already decided it belongs to *this* session. 02-08's reader must therefore **forward, not filter** — re-broadening by fanning out to every session it knows about would undo this plan, and its own second-proxy silence assertion is what would catch that.
- **The MCP proxy's event arm is still the documented no-op plan 02-05 left.** `crates/talaria-mcp/src/socket.rs` and `crates/talaria-mcp/src/main.rs` are untouched by this plan.
- **The probe row 02-08 carries is unaffected.** This plan invented no new `Event` variant, so a wire-level tab-open event still does not exist and remains 02-08's flagged assumption to resolve or decline.
- **Plan 02-11 (CI) inherits an unchanged clippy baseline of 7 lints** — this plan added none — and the same 12 e2e suites.
- The deferred-queue shape is now used for tab-table work as well as captures, loads and evals. A future callback that needs the table should follow it rather than adding a fifth mechanism.

## Self-Check: PASSED

- `crates/talaria-shell/src/app.rs` — FOUND
- `tests/e2e/crash_event_test.py` — FOUND
- `.planning/phases/02-harden-the-agent-surface/02-07-SUMMARY.md` — FOUND
- Commits `3b4ed62`, `334ee6a`, `ee44dfb` — all FOUND in git history.

---
*Phase: 02-harden-the-agent-surface*
*Completed: 2026-08-16*

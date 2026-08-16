---
phase: 02-harden-the-agent-surface
plan: 06
subsystem: shell
tags: [mcp, evaluate, wedged-tab, fail-fast, reentrancy, takeover, e2e]

# Dependency graph
requires:
  - phase: 02-01
    provides: green e2e baseline and the enforcing pre-commit hook, so a suite failure here is attributable to this plan
  - phase: 02-04
    provides: the rewritten download path in app.rs this plan's dispatch edits sit beside, untouched
  - phase: 02-05
    provides: per-request tasks in control.rs, which is what lets a second request reach the dispatch while the first is wedged — without it the busy check would be unreachable
provides:
  - "Shared::evaluating — one EvalGuard per outstanding evaluate, keyed by tab"
  - "EvalGuard { tab_id, done: Rc<Cell<bool>>, deadline } — a completion signal a servo callback can set without any borrow"
  - "Shared::sweep_evaluating / tab_evaluating / begin_evaluating"
  - "The busy error shape: tab {id} busy — a previous evaluate is still running"
  - "A fourth deadline source in next_capture_deadline, so a lost callback expires on time"
  - "tests/e2e/wedge_fastfail_test.py — same-tab fail-fast plus the surviving takeover route"
affects: [02-07 owner-filtered events, 02-08 MCP notifications, 02-11 CI]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Rc<Cell<bool>> as a reentrancy-safe completion signal: settable from inside a servo callback with no borrow and no failure mode"
    - "Self-healing in-flight registry: a deadline bounds the entry independently of the callback that would clear it, and that deadline is registered with the loop's WaitUntil scheduling"
    - "Guard closure applied to exactly one dispatch arm, with the non-generalisation stated in a comment and asserted by e2e"
    - "Negative-control verification: revert only the implementation file, confirm the new assertion fails, restore"

key-files:
  created:
    - tests/e2e/wedge_fastfail_test.py
  modified:
    - crates/talaria-shell/src/app.rs
    - tests/e2e/run_all.py

key-decisions:
  - "The completion flag is an Rc<Cell<bool>>, not a RefCell and not a field on Tab: every terminus of an evaluate is reachable from inside a servo callback, and a Cell set neither borrows nor can fail, so the try_borrow silent-drop class cannot leave a tab stuck busy"
  - "The registry entry expires at promise_wait() independently of its flag, so an engine callback that never arrives self-heals rather than refusing a healthy tab forever"
  - "The registry's deadlines are a source in next_capture_deadline — a deadline nothing wakes for is not a deadline"
  - "begin_evaluating runs only after the webview lookup succeeds, so a request naming a nonexistent tab leaves nothing behind"
  - "The busy check is applied to Command::Evaluate only, behind the crash check and behind the crash-simulation test hook, so a crashed tab still reports crashed and a simulated crash is never refused as busy"
  - "MCP-10 stays In Progress: the second evaluate now fails fast, but the first still runs to the timeout — making the evaluate itself complete needs an upstream libservo slow-script interrupt"

patterns-established:
  - "A completion signal reachable from a reentrant callback must be a Cell, not a RefCell"
  - "Pair every callback-cleared flag with a deadline, or a lost callback becomes a permanent refusal"

requirements-completed: []

coverage:
  - id: D1
    description: "A second evaluate against a tab that already has one in flight fails immediately with a distinct busy error naming the tab"
    requirement: MCP-10
    verification:
      - kind: e2e
        ref: "tests/e2e/wedge_fastfail_test.py — two evaluates written back to back on the same tab; the second returns outcome error carrying 'tab {id} busy — a previous evaluate is still running' in 0.00s, asserted bounded under 1.0s against a 3s command timeout"
        status: pass
      - kind: e2e
        ref: "negative control: reverting only crates/talaria-shell/src/app.rs makes that assertion fail with 'timed out after 3s (script still running?)'"
        status: pass
    human_judgment: false
  - id: D2
    description: "tabs_list, screenshot and tabs_focus against a wedged tab all keep working — the human's route back in"
    requirement: MCP-10
    verification:
      - kind: e2e
        ref: "tests/e2e/wedge_fastfail_test.py — all three return outcome ok while the wedged evaluate is still outstanding; the screenshot is decoded, PNG-magic checked and asserted over 1000 bytes; the block asserts its own elapsed time is under the command timeout so 'still outstanding' is not assumed"
        status: pass
      - kind: other
        ref: "source: grep -c 'tab_evaluating' app.rs is 2 — the method definition and the single guard closure; no other dispatch arm consults it"
        status: pass
    human_judgment: false
  - id: D3
    description: "The blanket command timeout remains the outer bound; fail-fast is added in front of it, not instead of it"
    requirement: MCP-10
    verification:
      - kind: e2e
        ref: "tests/e2e/wedge_fastfail_test.py reads the wedged evaluate's reply back and asserts it is still 'timed out after 3s (script still running?)' at 3.0s"
        status: pass
      - kind: other
        ref: "source: the TALARIA_COMMAND_TIMEOUT_SECS references in app.rs are byte-identical to HEAD~3 (3 occurrences before and after)"
        status: pass
    human_judgment: false
  - id: D4
    description: "Every way an evaluate can end clears its tab's in-flight entry, so a tab cannot be left permanently busy"
    requirement: MCP-10
    verification:
      - kind: e2e
        ref: "tests/e2e/wedge_fastfail_test.py — after the timeout fires, a further evaluate on that tab is asserted not to carry the busy wording"
        status: pass
      - kind: other
        ref: "code inspection: done.set(true) on all six terminal paths (start_evaluate's reply branch; process_pending_evals' expired-deadline, settled, unexpected-shape, evaluate-error and raw-run branches) and on none of the three parking/re-queue branches, which move the flag into the follow-up entry instead"
        status: pass
    human_judgment: true
    rationale: "The five process_pending_evals termini need a promise-returning, CSP-blocked or rejecting script to reach; the suite exercises the direct-reply and expiry paths, and the rest are enforced by the move checker — the flag is moved into the PendingEval on every non-terminal branch, so a branch that neither sets it nor carries it forward does not compile"
  - id: D5
    description: "A lost engine callback cannot wedge a tab forever"
    requirement: MCP-10
    verification:
      - kind: e2e
        ref: "tests/e2e/wedge_fastfail_test.py's recovery assertion is exactly this case: the wedged script thread never fires a callback at all, so only the deadline can clear the entry, and the later evaluate is not refused"
        status: pass
      - kind: other
        ref: "code inspection: sweep_evaluating retains on !done.get() && deadline > now, is called first in process_pending_evals and from tab_evaluating, and the deadlines are a source in next_capture_deadline"
        status: pass
    human_judgment: false
  - id: D6
    description: "In-flight state is marked and cleared without holding a RefCell borrow across a call into servo"
    verification:
      - kind: other
        ref: "code inspection: the only type reachable from inside an evaluate callback is Rc<Cell<bool>>; Cell::set takes &self, cannot panic and cannot fail. All three Shared methods that borrow self.evaluating run on the event loop (dispatch, process_pending_evals, next_capture_deadline)"
        status: pass
    human_judgment: true
    rationale: "The bug class is a silent try_borrow drop under reentrancy, which by construction leaves no observable trace to assert on; it is designed out by type choice rather than tested for"
  - id: D7
    description: "The JavaScript trampoline is untouched — MCP-10 is solved outside it"
    verification:
      - kind: other
        ref: "git diff -U0 crates/talaria-shell/src/app.rs | grep -c 'talaria_async' is 0 across the plan; wrap_script and poll_script are unmodified"
        status: pass
      - kind: other
        ref: "git diff --stat crates/talaria-shell/src/tabs.rs across the plan is empty — the in-flight state is on Shared, not on Tab"
        status: pass
    human_judgment: false

# Metrics
duration: 41min
completed: 2026-08-16
status: complete
---

# Phase 2 Plan 06: Same-Tab Evaluate Fail-Fast Summary

**A tab that wedges its own `evaluate` now refuses the next one in 0.00s with `tab {id} busy — a previous evaluate is still running` instead of stalling for the full command timeout, and `tabs_list`, `screenshot` and `tabs_focus` keep answering on that tab so the human can still take it over.**

## Performance

- **Duration:** 41 min
- **Started:** 2026-08-16T07:26Z
- **Completed:** 2026-08-16T08:07Z
- **Tasks:** 3
- **Files modified:** 2 (1 created)

## Accomplishments

- `Shared` gained an in-flight registry — `evaluating: RefCell<Vec<EvalGuard>>`, one entry per outstanding `evaluate`, carrying the tab id, a completion flag and a deadline. 02-05 closed MCP-11's cross-tab half; this closes as much of the same-tab half as is reachable without upstream libservo work.
- **The completion flag is an `Rc<Cell<bool>>`, and that is the load-bearing choice.** Every way an evaluate can end is reachable from inside a servo evaluate callback, which runs while the engine holds its own borrows. `Cell::set` takes `&self`, cannot fail and cannot block, so a terminus records completion without the `try_borrow` that silently drops work elsewhere in this file. A dropped completion here would leave a healthy tab permanently refused — the exact failure the busy check exists to prevent.
- **The entry expires on its own deadline as well as on its flag.** The wedged case in the e2e suite is precisely a callback that never arrives: servo is handed `while(true){}`, never fires the closure, and nothing but the deadline can clear the entry. `promise_wait()` is that deadline, and it was added as a fourth source in `next_capture_deadline` so the loop actually wakes to expire it.
- The flag threads through all nine branches of the evaluate machinery: `done.set(true)` on the six terminal paths, and *moved into the follow-up* `PendingEval` on the three parking / re-queue paths. That split is enforced by the move checker rather than by review — a branch that neither sets the flag nor carries it forward does not compile.
- The busy check is applied to **exactly one** dispatch arm. `grep -c 'tab_evaluating'` in `app.rs` is 2: the method definition and the single guard closure. Screenshot, tabs_close, tabs_focus, tabs_list, navigate, cookies_read, open_for_user and download are untouched, and the comment above the closure says why so a later reader does not "improve" it by generalising.
- Order in the arm is deliberate: the crash-simulation test hook keeps its position and early return (a simulated crash is never refused as busy), the crashed check stays ahead of the busy check (a crashed tab reports crashed — the actionable message), and `begin_evaluating` runs only after the webview lookup succeeds (a request naming a nonexistent tab leaves nothing behind).
- `tests/e2e/wedge_fastfail_test.py` pins all of it, and was **proved to bind by negative control**: reverting only `crates/talaria-shell/src/app.rs` makes the refusal assertion fail with `timed out after 3s (script still running?)` — the exact 30-second-shaped stall this plan removes.
- Full regression is **12/12 PASS, exit 0**. Clippy is back to exactly the 7 pre-existing lints owned by plan 02-11; no new lint was introduced.

## Task Commits

Each task was committed atomically:

1. **Task 1: Track one in-flight evaluate per tab with a callback-safe completion flag (D-04, D-06)** - `ce85027` (feat)
2. **Task 2: Fail the second evaluate immediately and leave every other command alone (D-05, D-07)** - `70e6d16` (feat)
3. **Task 3: Prove fail-fast and the surviving takeover route end to end** - `8ebe992` (test)

**Plan metadata:** see the `docs(02-06)` commit following this summary.

## Files Created/Modified

- `crates/talaria-shell/src/app.rs` — new `EvalGuard` struct and `Shared::evaluating` field; `sweep_evaluating` / `tab_evaluating` / `begin_evaluating` on `Shared`; sweep called first in `process_pending_evals`; a fourth deadline source in `next_capture_deadline`; a `done: Rc<Cell<bool>>` field on `PendingEval` and a matching parameter on `start_evaluate`; the `busy` guard closure and its single use in `Command::Evaluate`. `wrap_script`, `poll_script` and every other dispatch arm are unchanged.
- `tests/e2e/wedge_fastfail_test.py` — new shared-shell suite. Buffers replies by id (the wedged reply is overtaken by four others and must survive to be read back), asserts the refusal at both ends, exercises the three takeover commands while the wedge is outstanding, reads the wedged reply back as a timeout, and asserts a later evaluate is not refused.
- `tests/e2e/run_all.py` — suite registered in the phase-1 block after `timeout_session_test`; docstring updated.

## Decisions Made

- **`Rc<Cell<bool>>`, not a `RefCell` and not a field on `Tab`.** A `RefCell` would reintroduce the very `try_borrow` failure mode this file already suffers from on its event paths (`.planning/codebase/ARCHITECTURE.md`'s reentrancy rule), and a `Tab` field would need `tabs.borrow_mut()` from inside a callback, which is the same problem plus a second borrow. A `Cell` set is infallible and non-blocking, so the "stuck busy" case is designed out rather than guarded against.
- **A deadline as well as a flag.** The flag alone assumes the callback arrives. On a wedged script thread it never does, which is the whole scenario. `promise_wait()` — the command timeout minus two seconds — bounds the entry, so the registry cannot outlive the evaluate it describes. Consequence worth naming: with the suite's compressed 3s timeout the entry expires after 1s, so a *third* evaluate arriving after that is not refused. Self-healing beats permanent refusal, and the timeout ordering the project requires (`command timeout > promise_wait > LOAD_WAIT > capture deadline`) is preserved unchanged.
- **The registry's deadlines feed `next_capture_deadline`.** Without it the entry expires only when the loop happens to turn for some other reason. A deadline nothing wakes for is not a deadline.
- **`begin_evaluating` after the lookup, not before.** Beginning first would register an entry for a tab that never existed, and nothing would ever clear it except the deadline.
- **MCP-10 left In Progress, not marked Complete.** See below.

## Requirements Status

**MCP-10 is deliberately NOT marked complete.** The plan's own `<flagged_assumptions>` carried an unresolved probe edge on exactly this point, and it resolves against completion.

`REQUIREMENTS.md` states MCP-10 as "A heavy-JS page does not wedge `evaluate`". This plan delivers the bounded-blast-radius reading: the *second* and later evaluates on a wedged tab cost a round trip instead of a timeout, and the tab stays reachable. It does not deliver the literal reading: the *first* evaluate on a heavy-JS page still runs to the command timeout and never completes. Making the evaluate itself complete needs a SpiderMonkey slow-script interrupt exposed through libservo — upstream work, named out of scope by `.planning/codebase/CONCERNS.md`, by D-07's rationale and by the plan's objective.

MCP-10 moved `Pending` → `In Progress` in the traceability table, its bullet now records precisely what landed and what did not, and its checkbox stays unchecked. `requirements mark-complete` was deliberately not run for it. This mirrors the 02-01 lesson and the MCP-09 precedent.

## Deviations from Plan

### Accepted Acceptance-Criterion Mismatch

**Task 2, criterion "`grep -c 'TALARIA_COMMAND_TIMEOUT_SECS' crates/talaria-shell/src/app.rs` is 1" — actual is 3, before *and* after this plan.**

The criterion is stale, not violated. `app.rs` has read that variable in two places since 02-04 gave `download` its own bound (`promise_wait` at one site, the download timeout at another, plus one doc-comment mention). The count is byte-identical at `HEAD~3` and at this plan's tip, which is what the criterion is actually checking: the timeout knob was not touched. Verified directly: `git show HEAD~3:crates/talaria-shell/src/app.rs | grep -c` → 3.

### Transitional State Between Task 1 and Task 2

Task 1's commit (`ce85027`) carries two `dead_code` warnings — `EvalGuard::tab_id` never read and `tab_evaluating` never used — because the plan splits "add the predicate" and "use the predicate" across two tasks. Task 2 (`70e6d16`) removes both, and clippy at the plan tip is back to exactly the 7 pre-existing lints owned by 02-11. Recorded rather than worked around: inventing a use in Task 1 purely to silence a warning that Task 2 was about to resolve would have been worse.

### Auto-fixed Issues

None. No bug, missing critical functionality, or blocking issue was found that required a fix outside the plan's stated actions.

---

**Total deviations:** 0 auto-fixed + 1 accepted criterion mismatch + 1 recorded transitional state
**Impact on plan:** None. Every truth in `must_haves` holds, no dependency was added, no protocol change, no other dispatch arm touched.

## Issues Encountered

- **The suite's wedged tab is wedged for the life of the shell.** `while(true){}` never yields, so that tab's script thread is unusable afterwards and the suite closes it. This is fine next to `timeout_session_test`, which already wedges two `https://example.com/` tabs in the same shared shell — empirically confirmed again here: separate top-level tabs of the same origin do not share a wedged script thread, and every downstream suite (`mcp_client`, `single_instance`, `popup`) stayed green.
- **The `screenshot` assertion on a wedged tab takes ~1.5s**, because the tab is a background tab that will never paint a fresh frame, so the capture falls through to `pending_captures`' 1.5s deadline and returns the last framebuffer contents. Correct behaviour, worth knowing: it is why the takeover block is checked against the 3s command timeout rather than assumed instant.
- **The reply reader had to be rewritten to buffer by id.** `timeout_session_test`'s `reply(f, want)` discards non-matching replies; here the wedged call's reply is overtaken by four others and must survive to be read back at the end, so non-matching replies are held in a dict instead.

## Verification Evidence

- `cargo build --release` — exit 0 after every task.
- `cargo test` — 3 passed, 0 failed (2 protocol round-trip, 1 vault).
- `cargo clippy --all-targets -- -D warnings` — exactly the **7 pre-existing lints** owned by plan 02-11 (`talaria-mcp/src/tools.rs:97,132`; `talaria-shell/src/app.rs:778 ×2, 803 ×2` — the baseline's 709/734, shifted by this plan's added lines; `talaria-shell/src/tabs.rs:93`). **No new lint.** As in 02-05, the plan's "exits 0" cannot hold at this phase baseline; the honest reading is "no new lints", and that holds.
- `TALARIA_E2E_DISPLAY=:98 XDG_RUNTIME_DIR=/tmp/talaria-e2e-rt python3 tests/e2e/run_all.py` — **exit 0, 12/12 PASS**, run twice (after Task 3, and again after restoring from the negative control). No suite regressed against the 02-01 baseline verdict; the 12th is the new suite.
- **Negative control (the assertion binds):** `git checkout HEAD~2 -- crates/talaria-shell/src/app.rs`, rebuild, run `wedge_fastfail_test` → **exit 1**, `AssertionError: {'id': 3, 'outcome': 'error', 'message': 'timed out after 3s (script still running?)'}`. Restored, rebuilt, full suite green again. A timing-based assertion that passed before the change would have been worthless.
- Suite output at the tip: `second evaluate on the busy tab refused in 0.00s`; `tabs_list still answers and lists the wedged tab`; `screenshot still answers on the wedged tab: 24545 bytes`; `tabs_focus still answers on the wedged tab`; `all three answered while the wedged evaluate was still outstanding (1.7s < 3s)`; `wedged evaluate still ends in the timeout error at 3.0s`; `tab no longer refused as busy after the timeout`; `wedged tab closed`; `WEDGE FASTFAIL CHECKS PASSED`.
- Source criteria: `pub evaluating: RefCell` 1; `Rc<Cell<bool>>` 4 (≥3 required); `fn tab_evaluating` 1; `fn begin_evaluating` 1; `fn sweep_evaluating` 1; `tab_evaluating` total 2; busy message 1; `crashed — navigate it to recover` 2; `unwrap()` 0; `talaria_async` in the plan's diff 0; `wedge_fastfail_test` in `run_all.py` 1; `WEDGE FASTFAIL CHECKS PASSED` 1.
- `git diff --stat crates/talaria-shell/src/tabs.rs` across the whole plan — **empty**. The in-flight state is on `Shared`, not on `Tab`, as required.
- `python3 -c "import ast; ast.parse(open('tests/e2e/wedge_fastfail_test.py').read())"` — exit 0.

## Known Stubs

None.

## Threat Flags

None. No new network endpoint, auth path, file-access pattern, or schema change at a trust boundary beyond the plan's own register. T-02-06-01 through -04 and -06 are all mitigated and evidenced above; T-02-06-05 (busy error naming a tab id) was accepted by the plan and the message names only the tab the caller itself supplied.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- **MCP-10 is In Progress, not Complete.** The same-tab fail-fast landed; the wedged evaluate itself still burns the timeout. Closing MCP-10 literally requires a libservo slow-script interrupt and belongs in a later milestone, not in Phase 2. Whoever writes the phase-2 verification should not expect a checked box here.
- **Plans 02-07 and 02-08 are unaffected.** No event path, no `broadcast_event`, no `try_borrow` on the event paths was touched — D-13's rewrite is still entirely 02-07's to do.
- **Plan 02-11 (CI) inherits an unchanged clippy baseline of 7 lints** — this plan added none — and one more e2e suite to run.
- The `Rc<Cell<bool>>`-plus-deadline shape is reusable: 02-07 faces the same reentrancy constraint on the event paths, and a flag a callback can always set is the cheaper half of the answer there too.

## Self-Check: PASSED

- `crates/talaria-shell/src/app.rs` — FOUND
- `tests/e2e/wedge_fastfail_test.py` — FOUND
- `tests/e2e/run_all.py` — FOUND
- Commits `ce85027`, `70e6d16`, `8ebe992` — all FOUND in git history.

---
*Phase: 02-harden-the-agent-surface*
*Completed: 2026-08-16*

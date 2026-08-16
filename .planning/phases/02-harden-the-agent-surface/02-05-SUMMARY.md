---
phase: 02-harden-the-agent-surface
plan: 05
subsystem: infra
tags: [mcp, control-socket, tokio, pipelining, concurrency, idempotency, e2e]

# Dependency graph
requires:
  - phase: 02-01
    provides: green e2e baseline and the enforcing pre-commit hook, so any suite failure here is attributable to this plan
  - phase: 02-03
    provides: the SO_PEERCRED peer-UID gate in control.rs that this plan's handle_connection changes sit behind, untouched
provides:
  - "ShellConnection: background reader with an id-to-waiter map, dedicated writer task, no lock held across a round trip"
  - "A retry rule that cannot re-execute a command a live connection accepted"
  - "control.rs::handle_connection: per-request tasks in a JoinSet, drained before teardown; writer awaited, not aborted"
  - "control.rs::command_timeout_secs() — the single reader of the command-timeout knob"
  - "Bounded socket writes, so a peer that stops reading cannot strand a connection's writer"
  - "Raw-socket pipelining assertions in tests/e2e/timeout_session_test.py"
  - "Concurrent tool-call assertions through the real proxy binary in tests/e2e/mcp_client_test.py"
affects: [02-06 per-tab in-flight tracking, 02-07 owner-filtered events, 02-08 MCP notifications, 02-11 CI]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Client-side mirror of the shell's writer task: one task owns the write half and drains an unbounded channel"
    - "id-to-oneshot waiter map behind a tokio Mutex, locked for bookkeeping only and never across an await on the wire"
    - "Retry gated on a closed outbound channel — the only state that proves a message never reached a live wire"
    - "tokio JoinSet for per-request work, reaped with try_join_next each loop turn and drained before teardown"
    - "Negative-control verification: revert one file, confirm the matching e2e assertion fails, restore"

key-files:
  created: []
  modified:
    - crates/talaria-mcp/src/socket.rs
    - crates/talaria-shell/src/control.rs
    - tests/e2e/timeout_session_test.py
    - tests/e2e/mcp_client_test.py

key-decisions:
  - "The waiter map uses tokio's Mutex rather than std's, so there is no lock-poisoning Result to unwrap and the crate stays unwrap-free"
  - "Retry is expressed as an explicit two-state Attempt enum rather than a loop, so there is no unreachable!() and the never-retry boundary is a type, not a comment"
  - "Connected implements Drop to abort its reader and writer, so clearing the connection state cannot leak tasks"
  - "A dead connection is only cleared when same_channel proves it is still the wire this call used, so a concurrent caller's fresh connection is never clobbered"
  - "Each socket write is bounded by the command timeout, which is what makes awaiting the writer at teardown safe"
  - "The MCP-side ordering assertion issues its second call a beat after the first: both are dispatched concurrently by the server runtime, so without the beat a serialising connection fails only half the time"

patterns-established:
  - "Bookkeeping-only locking: acquire, register a waiter, clone a sender, release — never hold across the wire"
  - "Prove a regression test binds by reverting exactly the file it targets and watching it fail"

requirements-completed: [MCP-11]

coverage:
  - id: D1
    description: "Two calls issued back to back on one session against different tabs both complete, and the second is not gated on the first"
    requirement: MCP-11
    verification:
      - kind: e2e
        ref: "tests/e2e/timeout_session_test.py — a never-terminating evaluate on tab A, an evaluate on tab B and a tabs_list written before any reply is read; B and tabs_list both reply inside 1.5s, the wedged call times out at 3.0s"
        status: pass
      - kind: e2e
        ref: "scratchpad probe: evaluate 1+1 on tab B returned {\"value\":2} in 0.01s while tab A spun in while(true){}"
        status: pass
    human_judgment: false
  - id: D2
    description: "A wedged evaluate no longer blocks tabs_list, screenshot or tabs_close from the same agent session"
    requirement: MCP-11
    verification:
      - kind: e2e
        ref: "tests/e2e/timeout_session_test.py — tabs_list overtakes the wedged evaluate by more than 1.5s; 'connection usable after timeout' still passes"
        status: pass
      - kind: e2e
        ref: "negative control: reverting only crates/talaria-shell/src/control.rs makes this assertion fail with the wedged call's timeout error arriving first"
        status: pass
    human_judgment: false
  - id: D3
    description: "The MCP proxy holds no lock across a round trip — proved through the real binary, not just the socket"
    requirement: MCP-11
    verification:
      - kind: e2e
        ref: "tests/e2e/mcp_client_test.py — a tabs_list issued while a 1.5s evaluate is outstanding returns first, in 0.01s"
        status: pass
      - kind: e2e
        ref: "negative control: reverting only crates/talaria-mcp/src/socket.rs fails it with 'tabs_list was gated on the slow evaluate'"
        status: pass
    human_judgment: false
  - id: D4
    description: "A command is retried only when it never reached a live wire; a request a live connection accepted is never re-sent"
    verification:
      - kind: other
        ref: "code inspection: ShellConnection::attempt returns Attempt::NotSent only on outbound.send() failure, which can only mean the writer task is gone; every path after a successful send returns Attempt::Done"
        status: pass
    human_judgment: true
    rationale: "Reproducing a shell restart in the exact window between write and read needs a fault-injection hook the tree does not have; the property is enforced by the type (Attempt has no retryable state reachable after a successful send) rather than by a test"
  - id: D5
    description: "Replies are matched to their request by id, so out-of-order arrival is correct rather than tolerated"
    requirement: MCP-11
    verification:
      - kind: e2e
        ref: "both suites read replies in arrival order and match by id; the raw-socket block asserts the set {sibling, tabs_list} arrives before the wedged id, which is out-of-order delivery by construction"
        status: pass
    human_judgment: false
  - id: D6
    description: "A connection teardown does not truncate replies for requests still outstanding"
    verification:
      - kind: other
        ref: "code inspection: handle_connection drains the JoinSet, then drops out_tx, then awaits the writer; grep -c 'writer.abort()' is 0"
        status: pass
      - kind: e2e
        ref: "tests/e2e/timeout_session_test.py closes the connection with a wedged tab still spinning and the following session still sees the agent tab; full suite green"
        status: pass
    human_judgment: true
    rationale: "Forcing a reply to be in the writer's queue at the exact instant the read loop ends is a race no deterministic suite in this tree can stage; the ordering is verified by inspection and the absence of the abort"
  - id: D7
    description: "The wire format, the unreachable-shell error string and the handshake are unchanged"
    verification:
      - kind: other
        ref: "git diff --stat crates/talaria-protocol/src/lib.rs Cargo.toml crates/talaria-mcp/Cargo.toml crates/talaria-shell/Cargo.toml across the whole plan — empty"
        status: pass
      - kind: other
        ref: "grep -c 'is the Talaria browser running' socket.rs == 1; grep -c 'ServerMessage::HelloAck' socket.rs == 2"
        status: pass
    human_judgment: false

# Metrics
duration: 49min
completed: 2026-08-16
status: complete
---

# Phase 2 Plan 05: Connection Pipelining and Retry Correctness Summary

**Both sides of the control socket stopped serializing — a background reader with an id-to-waiter map on the MCP side, per-request tasks on the shell side — and the retry that could run `tabs_open` or `download` twice was narrowed to the one case that provably never reached a live wire.**

## Performance

- **Duration:** 49 min
- **Started:** 2026-08-16T06:28:13Z
- **Completed:** 2026-08-16T07:17:36Z
- **Tasks:** 3
- **Files modified:** 4

## Accomplishments

- `ShellConnection` no longer holds a mutex across a round trip. A connected state owns an outbound channel drained by a writer task, a shared id-to-oneshot waiter map, and both join handles; `request` locks only to establish the connection, allocate an id, register a waiter and clone a sender, then awaits the one-shot with the lock released. Measured effect through the real proxy binary: a `tabs_list` issued while a 1.5s `evaluate` is outstanding returns in **0.01s** instead of waiting behind it.
- The retry rule is now a type rather than a comment. `Attempt::NotSent` is reachable only from a failed send on the outbound channel — which can only mean the writer task is already gone — so a command a live connection accepted is structurally un-retryable. The old `for attempt in 0..2` loop retried on *any* round-trip error including a post-write read failure, which is what made a double `tabs_open` or `download` possible.
- `handle_connection` reads the next request line without awaiting the previous outcome. The timeout-and-map block moved into a per-request task holding the one-shot receiver, the id and a clone of the outbound sender; duplicate-hello and bad-request still answer inline because they have nothing to wait for.
- Teardown can no longer truncate an outstanding reply: the per-request `JoinSet` is drained to completion, then `out_tx` is dropped, then the writer is **awaited** rather than aborted. `grep -c 'writer.abort()'` is 0.
- MCP-11 is pinned in its own words at both layers. The raw-socket suite writes three lines before reading any reply — a never-terminating `evaluate` on tab A, an `evaluate` on tab B, and a `tabs_list` — and asserts B and the list both land inside 1.5s while A times out at 3.0s.
- Both new assertions were proved to bind by negative control: reverting **only** `control.rs` fails `timeout_session_test`; reverting **only** `socket.rs` fails `mcp_client_test`. Neither is a test that passes for free.
- The `Event` arm in the reader is the seam plan 02-08 needs. It stays a no-op here, exactly as the threat register requires — nothing unfiltered can reach a client before 02-07 lands owner filtering.

## Task Commits

Each task was committed atomically:

1. **Task 1: Pipeline the MCP-side connection and fix the retry rule (D-08, D-09, D-10)** - `9819692` (refactor)
2. **Task 2: Stop the shell serializing a session's requests (D-08, shell side)** - `3332905` (feat)
3. **Task 3: Prove concurrency on the raw socket and through the real MCP binary** - `f8bba59` (test), amended by `30ad4ba` (test) after the two-tab probe showed the stronger assertion was available

**Plan metadata:** see the `docs(02-05)` commit following this summary.

## Files Created/Modified

- `crates/talaria-mcp/src/socket.rs` — rewritten around a `Connected` state (outbound sender, `Waiters` map, reader and writer handles) with `Drop` aborting both tasks; `round_trip` deleted; `Attempt` enum encodes the never-retry boundary; `connect`'s error string and handshake preserved byte-for-byte; module docstring now states the pipelining and retry contracts.
- `crates/talaria-shell/src/control.rs` — per-request `JoinSet` tasks, reaped each loop turn with `try_join_next` and drained before `SessionEnded`; writer awaited instead of aborted; per-write timeout; the command-timeout env read extracted into `command_timeout_secs()`. The 02-03 peer-UID gate and its before-the-spawn position are untouched.
- `tests/e2e/timeout_session_test.py` — `rpc` split into `send` / `next_reply` / `reply`; three-line pipelining block; docstring extended.
- `tests/e2e/mcp_client_test.py` — `send_call` / `next_response` helpers and a concurrent tool-call block with an ordering assertion and a 0.5s bound.

## Decisions Made

- **tokio's `Mutex` for the waiter map, not std's.** A std mutex returns a `Result` from `lock()`, which would force an `unwrap`/`expect` on a poison case that carries no useful meaning here. Critical sections are a map insert or remove and are never held across an await, so the async mutex costs nothing and keeps the crate unwrap-free.
- **`Attempt` enum instead of a retry loop.** The old shape needed `unreachable!()` after the loop. Two named states — `Done` and `NotSent` — make "a sent request is never re-sent" checkable by reading the type, and remove the panic.
- **`same_channel` before clearing a dead connection.** Two concurrent callers can both observe a dead wire; without the check the second would drop the connection the first just re-established.
- **Bounded writes are what make the awaited writer safe.** Awaiting an unbounded write at teardown moves the failure from "truncated reply" to "task stranded forever" against a peer that half-closes and stops reading. The write bound is the same clock a command already runs under.
- **The MCP-side assertion issues its second call a beat later.** The server runtime dispatches tool calls concurrently, so with a serialising connection it is a coin flip which task reaches the lock first. Verified empirically: back-to-back, the assertion passed against the *old* `socket.rs`. The beat makes the slow call the definite lock holder and the assertion fails every time on a serialising connection.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical] Bound each socket write, so the newly-awaited writer cannot be stranded**

- **Found during:** Task 2
- **Issue:** The plan replaces `writer.abort()` with an await so queued replies are flushed. But `write_line` is unbounded: a peer that half-closes its write side (ending the read loop) while keeping its read side open and never reading fills the socket buffer, and the awaited writer never returns. That converts a truncation bug into a permanently stranded task holding a socket fd — reachable by a merely misbehaving client, not only a malicious one.
- **Fix:** Each write is wrapped in `tokio::time::timeout(write_bound, ...)`, `write_bound` being the same command timeout, breaking the writer loop on expiry. No `abort` is introduced anywhere, so the plan's acceptance criterion still reads 0.
- **Files modified:** `crates/talaria-shell/src/control.rs`
- **Verification:** Build + clippy clean with no new lints; full e2e green including `mcp_client_test`'s ~21KB screenshot reply, which is the largest write the suite makes.
- **Committed in:** `3332905` (Task 2 commit)

**2. [Rule 2 - Missing Critical] Reap finished per-request tasks instead of accumulating them**

- **Found during:** Task 2
- **Issue:** A `JoinSet` holds every spawned task until it is joined. As specified, the set is drained only at teardown, so a long-lived MCP session accumulates one entry per command it has ever issued — unbounded growth over a session's lifetime.
- **Fix:** `while inflight.try_join_next().is_some() {}` at the top of each read-loop turn. Never blocks; the teardown drain is unchanged.
- **Files modified:** `crates/talaria-shell/src/control.rs`
- **Verification:** Full e2e green; `mcp_client_test` and `download_bounds_test` both drive many sequential commands on one connection.
- **Committed in:** `3332905` (Task 2 commit)

**3. [Rule 1 - Bug] The MCP-side ordering assertion, as specified, passed against the unfixed code**

- **Found during:** Task 3
- **Issue:** The plan says to "send two tool calls back to back without reading between them" and assert the `tabs_list` returns first. Run against the **old** serialising `socket.rs`, that assertion **passed**. Both calls are dispatched concurrently by the MCP server runtime, so which one reaches the old mutex first is a scheduling race; the `tabs_list` won it. A regression test that passes on the code it is meant to catch is worse than no test.
- **Fix:** The `evaluate` is issued, then a 0.4s beat, then the `tabs_list`, so the slow call is the definite lock holder; and the `tabs_list` response is additionally bounded at 0.5s from its own send. Confirmed by reverting only `socket.rs`: the assertion now fails with `tabs_list was gated on the slow evaluate`.
- **Files modified:** `tests/e2e/mcp_client_test.py`
- **Verification:** Isolated revert of `crates/talaria-mcp/src/socket.rs` alone → `mcp_client_test` exit 1, `timeout_session_test` exit 0 (proving the two assertions bind to different files).
- **Committed in:** `f8bba59` (Task 3 commit)

**4. [Rule 2 - Missing Critical] The raw-socket assertion now uses a second tab, not only `tabs_list`**

- **Found during:** Task 3
- **Issue:** The plan specifies `evaluate` + `tabs_list`. MCP-11's requirement text and the plan's own first truth both say "against **other tabs**" — a `tabs_list` is not tab-scoped, so passing it proves the weaker claim that a non-script command survives.
- **Fix:** A scratchpad probe first established the stronger claim holds (`evaluate 1+1` on tab B returned in 0.01s while tab A spun forever), then the suite was extended to write three lines before reading any: the wedged `evaluate` on A, an `evaluate` on B, and the `tabs_list`.
- **Files modified:** `tests/e2e/timeout_session_test.py`
- **Verification:** Full e2e green; the strengthened suite still fails when `control.rs` alone is reverted.
- **Committed in:** `30ad4ba`

### Accepted Acceptance-Criterion Mismatch

**Task 2, criterion "`grep -c 'tokio::spawn' crates/talaria-shell/src/control.rs` is at least 3" — actual is 2.**

The plan's action text requires the per-request tasks to be collected in a task set ("Collect them in a task set. After the read loop ends, drain that set to completion"), and its own threat register and teardown ordering depend on that. `tokio::task::JoinSet::spawn` is how a task set is spawned into, so the third spawn is `inflight.spawn(...)`, not `tokio::spawn(...)`. The two criteria cannot both hold. The substance the criterion is checking — per-request handling spawned off the read loop — is present and is what the e2e assertions prove. Using `Vec<JoinHandle>` + `tokio::spawn` purely to satisfy the grep would have cost the cheap `try_join_next` reaping in deviation 2.

---

**Total deviations:** 4 auto-fixed (3 missing-critical, 1 test-correctness bug) + 1 accepted criterion mismatch
**Impact on plan:** All four are inside the plan's stated intent (no truncation, no unbounded growth, assertions that actually bind, MCP-11 in its own words). No scope creep, no new dependency, no protocol change.

## Issues Encountered

- **A full `run_all.py` appeared to hang for 18+ minutes in `download_bounds_test`, then passed cleanly on every subsequent run.** The stall was traced to a leftover control socket at the isolated `XDG_RUNTIME_DIR` from an earlier interrupted invocation, not to any code change: `download_bounds_test` passed standalone in 17s at the same commit, and three consecutive clean `run_all.py` runs were green. Worth knowing for anyone running the suite twice in a row against a private runtime dir — remove `$XDG_RUNTIME_DIR/talaria.sock` between runs if a previous run was killed.
- **`download_bounds_test`'s "CONCURRENT same name" block genuinely races now.** It writes two `download` requests for the same filename before reading either reply; under the old serialising shell those ran one after the other, and under this change they are concurrent for real. It passed on every run here, so 02-04's uniquify-then-create is holding under actual concurrency — but that suite is now exercising a stronger property than it was written against, which is worth remembering if it ever flakes.
- The plan's contingency for "the MCP server runtime dispatches tool calls sequentially" was not needed. `rust-mcp-sdk` dispatches concurrently; no skip line was added, and the MCP-side ordering assertion is a real assertion.

## Verification Evidence

- `cargo build --release` — exit 0.
- `cargo test` — 3 passed, 0 failed (2 protocol round-trip, 1 vault).
- `cargo clippy --all-targets -- -D warnings` — still exactly the **7 pre-existing lints** owned by plan 02-11 (`talaria-mcp/src/tools.rs:97,132`; `talaria-shell/src/app.rs:709 ×2, 734 ×2`; `talaria-shell/src/tabs.rs:93`). Locations captured after each task are byte-identical to the baseline: **no new lint introduced**. The plan lists this command as "exits 0", which it cannot at this phase baseline; the honest reading is "no new lints", and that holds.
- `TALARIA_E2E_DISPLAY=:98 XDG_RUNTIME_DIR=/tmp/talaria-e2e-rt python3 tests/e2e/run_all.py` — **exit 0, 11/11 PASS**, run three times across the plan (after Task 2, after Task 3, and after the Task 3 amendment). No suite regressed against the 02-01 baseline verdict.
- Negative control, both halves isolated:
  - revert `crates/talaria-shell/src/control.rs` only → `timeout_session_test` exit 1 (`AssertionError: {'id': 2, 'outcome': 'error', 'message': 'timed out after 3s...'}` arriving first), `mcp_client_test` exit 0.
  - revert `crates/talaria-mcp/src/socket.rs` only → `mcp_client_test` exit 1 (`tabs_list was gated on the slow evaluate: [14, 15]`), `timeout_session_test` exit 0.
- Source criteria: `fn round_trip` 0; `request` signature 1; unreachable-shell string 1; `ServerMessage::HelloAck` 2; `tokio::spawn` in socket.rs 2; `unwrap()` 0 in both files; `writer.abort()` 0; `TALARIA_COMMAND_TIMEOUT_SECS` 1; `script still running` 1.
- `git diff --stat crates/talaria-protocol/src/lib.rs Cargo.toml crates/talaria-mcp/Cargo.toml crates/talaria-shell/Cargo.toml` across the whole plan — **empty**. D-09's no-wire-change constraint held and no dependency was added.
- `python3 -c "import ast; ast.parse(...)"` on both suites — exit 0.

## Known Stubs

None. The `Event` arm in the reader task is a deliberate, documented no-op, not a stub: the threat register requires it to stay inert until plan 02-07 lands owner filtering, and plan 02-08 is named in the comment as the change that fills it.

## Threat Flags

None. No new network endpoint, auth path, file-access pattern, or schema change at a trust boundary beyond what the plan's own register covers. T-02-05-03 (unbounded per-request task growth) was accepted by the plan; deviation 2 partially reduces it by reaping completed tasks, though the concurrent-task count itself is still bounded only by the command timeout and the peer-UID gate, as the plan intended.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- **MCP-11 is fully closed** and moves to Complete: both layers were serialising, both were fixed, and the requirement's literal claim — a wedged tool call does not block calls against *other tabs* — is asserted at the raw socket and through the real proxy binary, each with a negative control.
- **MCP-10 is untouched and still partial.** A second script command against the *same* wedged tab still burns the full command timeout; that is plan 02-06's fail-fast, and this plan deliberately did not pre-empt it.
- **Plan 02-08's precondition (D-14) is now satisfied.** An unsolicited server-to-client message is structurally possible: the reader task exists, runs independently of any request, and already has the `Event` arm. 02-07 must land owner filtering before that arm emits anything.
- Plan 02-11 (CI) inherits an unchanged clippy baseline of 7 lints — this plan added none.

## Self-Check: PASSED

All four modified files exist on disk; all four commits (`9819692`, `3332905`, `f8bba59`, `30ad4ba`) are present in git history.

---
*Phase: 02-harden-the-agent-surface*
*Completed: 2026-08-16*

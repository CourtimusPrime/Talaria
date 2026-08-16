---
phase: 02-harden-the-agent-surface
plan: 08
subsystem: mcp
tags: [agent-04, notifications, mcp, logging-capability, owner-addressing, e2e]

# Dependency graph
requires:
  - phase: 02-01
    provides: green e2e baseline and the enforcing pre-commit hook, so a suite failure here is attributable to this plan
  - phase: 02-05
    provides: the background reader with the documented no-op event arm this plan fills (D-14)
  - phase: 02-07
    provides: the contract this plan consumes — by the time an event reaches ServerMessage::Event, the shell has already decided it belongs to this session (D-12, D-13)
provides:
  - "ShellConnection::new(events: mpsc::UnboundedSender<talaria_protocol::Event>) — the event sink lives on the connection, so a reconnect re-attaches to the same drain"
  - "socket.rs read_loop forwards every ServerMessage::Event onto that sink instead of discarding it"
  - "The `logging` server capability declared alongside `tools` in the initialize result"
  - "main.rs::notify_tab_events — the drain task turning each event into McpServer::notify_log_message"
  - "tests/e2e/mcp_client_test.py::Client — a raw-fd, notification-buffering MCP client with bounded reads"
affects: [02-11 CI]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Forward, never filter: the proxy's event arm carries a load-bearing comment stating the addressing already happened upstream, so no later reader re-broadens it"
    - "The sink lives on the connection, not on a reader, so reconnect after a shell restart keeps delivering to the same drain"
    - "wait_for_initialization() gates the drain; the unbounded channel buffers events raised before a client exists, so an early event is delayed rather than lost"
    - "Diagnostics to stderr only — stdio is the MCP transport and a stray stdout line corrupts the JSON-RPC stream"
    - "A silence assertion is only meaningful against a session that is actually connected and demonstrably able to receive: the observer owns a tab, and its own close is asserted after the silence"
    - "Negative-control verification by surgical revert of the single behavioural line, not of the whole file, when a file-level revert would short-circuit on an earlier assertion"

key-files:
  created: []
  modified:
    - crates/talaria-mcp/src/socket.rs
    - crates/talaria-mcp/src/main.rs
    - tests/e2e/mcp_client_test.py
    - .planning/REQUIREMENTS.md
    - .planning/phases/02-harden-the-agent-surface/deferred-items.md

key-decisions:
  - "The carrier is notifications/message via McpServer::notify_log_message; the capability is ServerCapabilities.logging"
  - "The runtime handle is the Arc<ServerRuntime> that create_server already returns, cloned before start() — no shared cell and no handle stashed from the tool-call handler was needed"
  - "The notification payload is the serialized protocol Event verbatim, so the client gets the event name and the tab id in one object with no follow-up tabs_list"
  - "Crash notifies at warning, close at info — a severity a client can filter on, rather than flattening both to one level"
  - "A serialization failure and a send failure are each reported to stderr and the drain continues; neither panics the task, because a dead drain task is precisely the silent-discard failure mode this requirement exists to remove"
  - "AGENT-04 stays In Progress: close and crash are delivered, `open` has no wire event and this plan declined to invent one"

patterns-established:
  - "Prove a silence assertion binds by making the layer below leak, not by trusting that it does not"
  - "An observer session in an e2e suite must be shown able to receive before its silence proves anything"

requirements-completed: []

coverage:
  - id: D1
    description: "A tab crash arrives at a connected MCP client as an MCP notification, without polling"
    requirement: AGENT-04
    verification:
      - kind: e2e
        ref: "tests/e2e/mcp_client_test.py — the actor opens a tab, evaluates the simulated-crash token and asserts a no-id notifications/message whose params.data is {\"event\":\"tab_crashed\",\"tab_id\":N}. Observed: `mcp-e2e notified: {\"data\": {\"event\": \"tab_crashed\", \"tab_id\": 5}, \"level\": \"warning\", \"logger\": \"talaria.tabs\"}`"
        status: pass
      - kind: e2e
        ref: "negative control: replacing only the `events.send(event)` line in socket.rs with a discard fails the suite with `mcp-e2e: no tab_closed notification for tab 14 within 15s`"
        status: pass
    human_judgment: false
  - id: D2
    description: "A tab close arrives at a connected MCP client as an MCP notification, without polling"
    requirement: AGENT-04
    verification:
      - kind: e2e
        ref: "tests/e2e/mcp_client_test.py asserts tab_closed for four different tabs across the suite (the tool-surface tab, the concurrency tab, the crashed tab, and the observer's own)"
        status: pass
    human_judgment: false
  - id: D3
    description: "The notification carries the event name and the tab id, so a client can act without a follow-up call"
    requirement: AGENT-04
    verification:
      - kind: e2e
        ref: "expect_note matches params.data against the exact dict {\"event\": …, \"tab_id\": …} and additionally asserts method == notifications/message; a payload missing either field fails"
        status: pass
    human_judgment: false
  - id: D4
    description: "The server advertises the capability its notification method requires"
    requirement: AGENT-04
    verification:
      - kind: e2e
        ref: "tests/e2e/mcp_client_test.py asserts `logging` is present in the initialize result's capabilities. Observed: `CAPABILITIES: {\"logging\": {}, \"tools\": {}}`"
        status: pass
      - kind: e2e
        ref: "negative control: reverting main.rs to the pre-plan capability literal fails at that assertion with `AssertionError: {'tools': {}}`"
        status: pass
    human_judgment: false
  - id: D5
    description: "An MCP session receives notifications only for tabs its own session owns; another session's tab lifecycle produces nothing on this connection"
    requirement: AGENT-04
    verification:
      - kind: e2e
        ref: "tests/e2e/mcp_client_test.py — a second talaria-mcp process, initialized and made live by owning a tab of its own, must produce no unsolicited message of ANY shape while the first session's tab crashes and closes; the first session is then asserted silent while the observer closes its own tab"
        status: pass
      - kind: e2e
        ref: "negative control at the layer below: reverting only crates/talaria-shell/src/app.rs to the pre-02-07 broadcast_event fails this assertion with `mcp-e2e-observer: unsolicited message (another MCP session's tab crashed and closed): {'jsonrpc': '2.0', 'method': 'notifications/message', 'params': {'data': {'event': 'tab_crashed', 'tab_id': 16}, …}}` — a leak surfaced through the whole stack, not just the socket"
        status: pass
      - kind: e2e
        ref: "the observer's own tab_closed is asserted immediately after its silence, so the silence cannot be a dead notification path on that process"
        status: pass
    human_judgment: false
  - id: D6
    description: "The proxy no longer discards server events: the reader's event arm forwards instead of ignoring"
    requirement: AGENT-04
    verification:
      - kind: other
        ref: "source: the ServerMessage::Event arm binds `event` and calls `events.send(event)`; the 02-05 no-op comment is replaced with the forward-never-filter contract"
        status: pass
    human_judgment: false
  - id: D7
    description: "A notification that cannot be delivered is logged rather than silently dropped, and never panics the proxy"
    verification:
      - kind: other
        ref: "code inspection: notify_tab_events has two failure branches, a serde_json::to_value error and a notify_log_message error, each writing to stderr and continuing the loop. No unwrap and no expect in the crate: `grep -c 'unwrap()'` is 0 in both main.rs and socket.rs"
        status: pass
    human_judgment: true
    rationale: "Forcing the SDK's transport to reject a send at the exact instant an event is drained needs a fault-injection hook the tree does not have; the property is that neither branch can terminate the drain, which is verified by reading the two match/if-let arms"
  - id: D8
    description: "No diagnostic reaches standard output, so the JSON-RPC stream cannot be corrupted"
    verification:
      - kind: other
        ref: "source: `grep -cE '(^|[^e[:alnum:]_])println!' crates/talaria-mcp/src/main.rs` is 0; the two writes are eprintln!"
        status: pass
      - kind: e2e
        ref: "the suite's Client skips non-JSON stdout lines and would surface a corrupt stream as a missing response; all 12 suites pass"
        status: pass
    human_judgment: false
  - id: D9
    description: "The wire format between shell and proxy is unchanged; only the proxy's handling of an existing variant changes"
    verification:
      - kind: other
        ref: "git diff --stat crates/talaria-protocol/src/lib.rs Cargo.toml crates/talaria-mcp/Cargo.toml crates/talaria-shell/Cargo.toml across the whole plan — empty"
        status: pass
    human_judgment: false
  - id: D10
    description: "The tool list and the initialize handshake are otherwise unchanged"
    verification:
      - kind: e2e
        ref: "the exact sorted tool-list equality assertion is carried unchanged and passes; the concurrency (MCP-11) block is carried unchanged and still returns tabs_list in 0.01s"
        status: pass
    human_judgment: false

# Metrics
duration: 29min
completed: 2026-08-16
status: complete
---

# Phase 2 Plan 08: MCP Tab Lifecycle Notifications Summary

**The proxy's event arm stopped being a no-op: a tab crash or close now leaves the control socket, crosses an event sink on the connection, and reaches the owning MCP session as a `notifications/message` carrying `{"event":…,"tab_id":…}` — while a second, live MCP session that owns its own tab receives nothing at all.**

## Performance

- **Duration:** 29 min
- **Started:** 2026-08-16T08:16Z
- **Completed:** 2026-08-16T08:45Z
- **Tasks:** 3
- **Files modified:** 3 source + 2 planning

## Accomplishments

- **The discard is gone.** `ServerMessage::Event` was the one arm in the 02-05 reader that threw its payload away. It now binds `event` and sends it to a sink that lives on `ShellConnection`, so a reconnect after a shell restart re-attaches to the same drain without `main` noticing.
- **Forward, never filter — and the comment says so.** The arm carries the contract 02-07 handed over: the shell has already decided this event belongs to this session, so the proxy performs no second filter and, critically, no fan-out. That comment is load-bearing; the whole information-disclosure threat (T-02-08-01) is a future reader "helpfully" broadcasting.
- **The capability is declared, not assumed.** `ServerCapabilities.logging` sits next to `tools` in the initialize result. Observed at the client: `CAPABILITIES: {"logging": {}, "tools": {}}`. A `notifications/message` sent without it is protocol noise a conformant client may drop — which would have made this requirement look delivered while it was not, and is exactly what the negative control reproduced.
- **The payload needs no follow-up call.** The serialized protocol `Event` is the notification's `data` verbatim, so an agent reading `{"event":"tab_crashed","tab_id":5}` knows both what happened and to which of its tabs, without a `tabs_list`.
- **Nothing is dropped in either direction.** The drain awaits `wait_for_initialization()` first and the channel is unbounded, so an event raised before a client exists is delayed rather than lost; a serialization failure and a send failure each go to **stderr** and the loop continues. Neither can kill the task — a dead drain task would be the same silent discard this requirement exists to remove.
- **Standard output is untouched.** stdio is the MCP transport; both diagnostics are `eprintln!`.
- **The silence assertion was proved to bind at the MCP layer, not merely inherited from 02-07.** Reverting only `crates/talaria-shell/src/app.rs` to the pre-02-07 `broadcast_event` makes the observer proxy emit `notifications/message` naming *another session's* tab id, and the suite catches it. A leak two layers down surfaces here.
- **And the observer was proved able to receive.** After the silence window, the observer closes its own tab and asserts its own `tab_closed` arrives. Without that, "silent" would be indistinguishable from "this process's notification path is broken" — the failure mode that makes a silence assertion worthless.
- Full regression: **12/12 PASS, exit 0**, run twice at the tip. Clippy is exactly the 7 pre-existing lints owned by plan 02-11; **no new lint**.

## Task Commits

Each task was committed atomically:

1. **Task 1: Forward server events out of the reader instead of discarding them (D-11)** - `5a7f571` (feat)
2. **Task 2: Advertise the capability and emit the MCP notification** - `501ea13` (feat)
3. **Task 3: Prove notification delivery and cross-session silence through the real binary** - `35448a1` (test)

**Plan metadata:** see the `docs(02-08)` commit following this summary.

## Files Created/Modified

- `crates/talaria-mcp/src/socket.rs` — `ShellConnection` gains an `events: mpsc::UnboundedSender<Event>` field; `new` takes it; `connect` and `read_loop` take a clone; the `ServerMessage::Event` arm sends. Module docstring gains a paragraph on unsolicited events. The reply path, waiter map, `Attempt` retry rule, handshake and the unreachable-shell error string are byte-for-byte as 02-05 left them.
- `crates/talaria-mcp/src/main.rs` — the event channel is created before the connection; `logging: Some(serde_json::Map::new())` joins `tools` in the capability literal; `notify_tab_events(Arc<ServerRuntime>, UnboundedReceiver<Event>)` is spawned before `server.start().await`.
- `tests/e2e/mcp_client_test.py` — restructured around a `Client` class (raw-fd buffered reads, notification buffering, bounded reads, `expect_note` / `expect_silence` / `shutdown`); every prior assertion carried unchanged; four `tab_closed` assertions, one `tab_crashed`, one capability assertion, one cross-session silence, one reverse silence.
- `.planning/REQUIREMENTS.md` — AGENT-04's bullet now records exactly what ships and what does not. Status stays In Progress.
- `.planning/phases/02-harden-the-agent-surface/deferred-items.md` — new entry for the missing wire-level tab-open event.

## Resolved SDK API (recorded as the plan requires)

| What | Resolved value |
|---|---|
| Notification method | `rust_mcp_sdk::McpServer::notify_log_message(LoggingMessageNotificationParams) -> SdkResult<()>` (`rust-mcp-sdk-1.0.1/src/mcp_traits/mcp_server.rs:349`), which wraps `NotificationFromServer::LoggingMessageNotification` — on the wire, `notifications/message` |
| Params type | `rust_mcp_sdk::schema::LoggingMessageNotificationParams { data: serde_json::Value, level: LoggingLevel, logger: Option<String>, meta: Option<Map> }` (`rust-mcp-schema-0.10.3` `generated_schema/2025_11_25/mcp_schema.rs:5353`) |
| Capability declared | `ServerCapabilities.logging: Option<serde_json::Map<String, Value>>` — set to `Some(Map::new())`, serializing as `"logging": {}` |
| Runtime-handle route | **The `Arc<ServerRuntime>` that `server_runtime::create_server` already returns.** `start` takes `self: Arc<Self>`, so the Arc is cloned into the drain task before `start()` is awaited. No shared cell and no handle stashed from `handle_call_tool_request` was needed — the plan's fallback route was not taken. |
| Early-event handling | `McpServer::wait_for_initialization()` at the top of the drain, plus the unbounded channel. An event raised before a client has initialized is buffered, not dropped. |
| Deprecated alternative avoided | `send_logging_message` is `#[deprecated(since = "0.8.0")]` in favour of `notify_log_message`; the non-deprecated name is used. |

**Why the logging notification and not something else.** It is base-protocol, it carries an arbitrary JSON payload, and it needs only the `logging` capability. The alternatives in this SDK are `notify_resource_updated` / `notify_resource_list_changed` (this server exposes no resources and would have to invent a resource URI scheme for tabs), `notify_prompt_list_changed` and `notify_tool_list_changed` (wrong subject entirely), and `notify_task_status` (bound to the SDK's task store, which this server does not use — `task_store: None`). No better-fitting server-initiated notification exists in 1.0.1 for tab lifecycle.

## Decisions Made

- **The sink lives on `ShellConnection`, not on each reader task.** A reconnect after a shell restart clones the same sender, so the drain task never has to be told a reconnect happened.
- **Severity carries meaning.** `TabCrashed` notifies at `warning`, `TabClosed` at `info`. A client that raises its level with `logging/setLevel` still sees crashes. Flattening both to one level would have thrown away free signal.
- **The drain reports, it does not panic.** `serde_json::to_value` on an `Event` is a genuine invariant and the codebase precedent (`.expect("serializable")` in the shell) would have been defensible — but a panic inside the spawned drain kills only that task, silently, and the notification path dies with it. That is precisely the failure this requirement removes, so both branches are handled and reported.
- **The negative control was surgical, not file-level.** Reverting `socket.rs` + `main.rs` wholesale *does* fail the suite, but it fails at the capability assertion on line ~160, short-circuiting before any delivery assertion runs — proving only that the capability is declared. Replacing the single `events.send(event)` line with a discard, leaving everything else intact, is what proves the *delivery* assertions bind to the event arm. Both were run; both are recorded below.
- **The observer must own a tab.** `ShellConnection` connects to the control socket lazily, on first use. An observer that only completes the MCP handshake never opens a socket session at all, so its silence would be trivially guaranteed and would prove nothing. Making it open a tab forces the connection and makes it a session the shell knows about.

## Requirements Status

**AGENT-04 is deliberately NOT marked complete.** `requirements mark-complete` was not run; the checkbox stays unchecked and the traceability status stays In Progress. This follows the MCP-09 / MCP-10 / 02-07 precedent.

AGENT-04 reads "Tab **open**/close/crash events reach MCP clients as MCP notifications, not only via polling."

- **close — delivered.** Asserted for four tabs, on the owning session, with a negative control.
- **crash — delivered.** Asserted on the owning session, with a negative control.
- **open — not delivered.** `talaria_protocol::Event` has exactly two variants and no producer raises anything on tab creation. This plan's `<flagged_assumptions>` carried that as an open question and **resolves it as an explicit decline**, not a silent drop: inventing a variant would have broken this plan's own "the wire format is unchanged" truth and its threat register.

For an agent's own `tabs_open` the gap costs nothing — the reply carries the tab. The real gap is a **popup adopted under an agent's tab** (`Shared::adopt_popup`), which the agent can still only discover by polling `tabs_list`. Closing it needs a third `Event` variant, a `queue_event` call at the adoption site, and assertions in two suites; **the proxy and notification plumbing built here need no change** — a new variant flows through the event arm and the drain as-is. Logged in `deferred-items.md` with that cost estimate.

## Deviations from Plan

### Task-boundary adjustment

**`main.rs` was touched in Task 1, by one statement.**

Task 1 changes `ShellConnection::new`'s signature and lists `cargo build --release` exiting 0 as acceptance. Those cannot both hold while `main` still calls `ShellConnection::new()` with no argument. Task 1 therefore creates the channel and passes the sender, binding the receiver so the sink stays open; Task 2 does what it is actually about — the capability, the drain task and the notification. Same shape as 02-07's Task 1/Task 2 boundary, and for the same reason. No commit knowingly ships a broken build.

### Accepted Acceptance-Criterion Mismatch

**Task 2, criterion "`grep -c 'println!' crates/talaria-mcp/src/main.rs` is 0" — the naive grep returns 2.**

`eprintln!` contains `println!` as a substring, so a substring grep cannot distinguish "writes to standard output" from "writes to standard error" — and the plan's own action text *requires* stderr diagnostics ("Do not write diagnostics to standard output… Use the `log` facade or standard error"). The two criteria cannot both hold. The substance being checked — nothing added writes to standard output — holds and is verified with a boundary-aware grep: `grep -cE '(^|[^e[:alnum:]_])println!' crates/talaria-mcp/src/main.rs` is **0**, and `grep -c 'eprintln!'` is 2. Same class of mismatch as 02-05's `tokio::spawn` criterion.

**The `log` facade was not used** because `talaria-mcp` does not depend on `log` or `tracing`, and the plan's threat register (T-02-08-07) states no dependency is added. `eprintln!` is the no-dependency half of the plan's own "log facade **or** standard error".

### Auto-fixed Issues

**1. [Rule 1 - Bug] The suite's response readers ate notifications**

- **Found during:** Task 3
- **Issue:** The existing `rpc` and `next_response` loops read `proc.stdout.readline()` and `continue` past anything whose id does not match. A notification has *no* id, so every one that arrived while a response was outstanding was silently discarded — inside the very suite written to prove notifications are not discarded. The first crash notification lands while the `evaluate` response is in flight, so this was not a theoretical race.
- **Fix:** `Client.response()` appends every id-less message to `self.notes` instead of dropping it, and `expect_note` searches that buffer before reading further. Same buffering discipline `crash_event_test.py` uses one layer down.
- **Files modified:** `tests/e2e/mcp_client_test.py`
- **Verification:** The suite's own output shows `mcp-e2e notified: {"data": {"event": "tab_closed", "tab_id": 2}…}` recovered from the buffer immediately after the `tabs_close` response.
- **Committed in:** `35448a1`

**2. [Rule 1 - Bug] A timed-out buffered reader cannot be read again**

- **Found during:** Task 3
- **Issue:** `expect_silence` must time a read out, and this suite does it twice on two different processes. Python's buffered reader raises `OSError: cannot read from timed out object` on every read after its first timeout — the exact bug 02-07 hit and fixed one layer down.
- **Fix:** `Client` reads the raw fd with `select` + `os.read` into its own byte buffer and never calls `proc.stdout.readline()`. `readline(timeout)` returns `None` on timeout and leaves the process usable; `expect_silence` additionally asserts both the notification buffer and the byte buffer are empty first, so "silent" means genuinely nothing pending rather than nothing *new*.
- **Files modified:** `tests/e2e/mcp_client_test.py`
- **Verification:** Both silence assertions run, on two different processes, in the same suite run.
- **Committed in:** `35448a1`

**3. [Rule 2 - Missing Critical] The silence assertion would have passed against a disconnected observer**

- **Found during:** Task 3
- **Issue:** The plan says to spawn a second proxy, complete its initialize handshake, "and leave it idle". `ShellConnection` connects to the control socket **lazily, on first use** — so an idle observer never opens a socket session, the shell never knows it exists, and its silence is guaranteed by construction rather than by owner addressing. That is a test that passes on the code it is meant to catch.
- **Fix:** The observer opens a tab of its own before the silence window, which forces the socket connection and registers it as a live session; and after the silence window it closes that tab and asserts its own `tab_closed` arrives, proving its notification path was live throughout.
- **Files modified:** `tests/e2e/mcp_client_test.py`
- **Verification:** Reverting only `crates/talaria-shell/src/app.rs` to the pre-02-07 fan-out makes the observer receive `{'event': 'tab_crashed', 'tab_id': 16}` and the assertion fails. Against a connection-less observer that revert would have produced nothing.
- **Committed in:** `35448a1`

---

**Total deviations:** 3 auto-fixed (2 test-correctness bugs, 1 missing-critical) + 1 task-boundary adjustment + 1 accepted criterion mismatch
**Impact on plan:** None. Every truth in `must_haves` holds. No dependency added, no protocol change, no wire-format change.

## Issues Encountered

- **A file-level revert of the two Rust files is a weak negative control here.** It fails, but at the capability assertion near the top of the suite, before any delivery assertion executes. Anyone re-running this control should use the surgical line revert instead, or read only the capability half into the result.
- **The strongest available negative control for cross-session silence lives in a different crate.** The proxy has no fan-out mechanism to break, so the only way to produce a leak is to make the shell leak — `git checkout 3b4ed62~1 -- crates/talaria-shell/src/app.rs`. That is a ~2 minute release relink each way; budget for it.
- **`grep -c 'println!'` is not a stdout check** in any file that legitimately writes to stderr. Noted here because two plans in this phase now carry that criterion.

## Verification Evidence

- `cargo build --release` — exit 0 after every task.
- `cargo test` — 3 passed, 0 failed (2 protocol round-trip, 1 vault).
- `cargo clippy --all-targets -- -D warnings` — exactly the **7 pre-existing lints** owned by plan 02-11 (`talaria-mcp/src/tools.rs:97,132`; `talaria-shell/src/app.rs:968 ×2, 993 ×2`; `talaria-shell/src/tabs.rs:93`). **No new lint.** As in 02-05, 02-06 and 02-07, the plan's "exits 0" cannot hold at this phase baseline; the honest reading is "no new lints", and that holds.
- `TALARIA_E2E_DISPLAY=:98 XDG_RUNTIME_DIR=/tmp/talaria-e2e-rt python3 tests/e2e/run_all.py` — **exit 0, 12/12 PASS**, run twice at the tip (after Task 3, and again after restoring from the second negative control). No suite regressed against the 02-01 baseline verdict.
- **Negative control A (delivery binds to the event arm):** replace only `let _ = events.send(event);` in `socket.rs` with a discard, rebuild → `mcp_client_test` **exit 1**, `AssertionError: mcp-e2e: no tab_closed notification for tab 14 within 15s (closed the tool-surface tab); saw []`. All 11 other suites still pass. Restored.
- **Negative control B (capability binds):** `git checkout 5a7f571~1 -- crates/talaria-mcp/src/{socket,main}.rs`, rebuild → `mcp_client_test` **exit 1**, `AssertionError: {'tools': {}}`. Restored.
- **Negative control C (cross-session silence binds through the whole stack):** `git checkout 3b4ed62~1 -- crates/talaria-shell/src/app.rs`, rebuild → `mcp_client_test` **exit 1**, `AssertionError: mcp-e2e-observer: unsolicited message (another MCP session's tab crashed and closed): {'jsonrpc': '2.0', 'method': 'notifications/message', 'params': {'data': {'event': 'tab_crashed', 'tab_id': 16}, 'level': 'warning', 'logger': 'talaria.tabs'}}` (and `crash_event_test` fails as 02-07 documented). Restored, rebuilt, full suite green.
- **Suite output at the tip** (standalone run against a hook-enabled shell, exit 0):
  - `CAPABILITIES: {"logging": {}, "tools": {}}`
  - `mcp-e2e notified: {"data": {"event": "tab_crashed", "tab_id": 5}, "level": "warning", "logger": "talaria.tabs"}`
  - `tab_crashed reached the owning MCP session as a notification`
  - `mcp-e2e notified: {"data": {"event": "tab_closed", "tab_id": 5}, "level": "info", "logger": "talaria.tabs"}`
  - `mcp-e2e-observer stayed silent for 2.0s (another MCP session's tab crashed and closed)`
  - `mcp-e2e-observer notified: {"data": {"event": "tab_closed", "tab_id": 4}, "level": "info", "logger": "talaria.tabs"}`
  - `the observer's notification path is live — its silence was addressing`
  - `mcp-e2e stayed silent for 2.0s (the observer closed its own tab)`
  - `CONCURRENT tool calls: tabs_list returned in 0.01s while the evaluate was still outstanding` (MCP-11, carried unchanged)
  - `ALL MCP CLIENT CHECKS PASSED`
- Source criteria: `ServerMessage::Event` in socket.rs **1**; protocol-event-typed sink **4** matches; `is the Talaria browser running` **1**; `unwrap()` **0** in socket.rs and **0** in main.rs; capability literal `Some(` fields **2**; `tokio::spawn` in main.rs **1**; boundary-aware bare `println!` in main.rs **0**; `MCP CLIENT CHECKS PASSED` **1**; `tab_crashed` **2**; `tab_closed` **5**.
- `python3 -c "import ast; ast.parse(open('tests/e2e/mcp_client_test.py').read())"` — exit 0.
- `git diff --stat crates/talaria-protocol/src/lib.rs Cargo.toml crates/talaria-mcp/Cargo.toml crates/talaria-shell/Cargo.toml` across the whole plan — **empty**. No wire change, no dependency added.

## Known Stubs

None.

## Threat Flags

None. No new network endpoint, auth path, file-access pattern, or schema change at a trust boundary beyond the plan's own register.

- **T-02-08-01 (information disclosure via re-broadening)** — mitigated. The proxy forwards its own connection's events and performs no fan-out; negative control C proves the assertion catches a leak.
- **T-02-08-02 (notification discarded before a runtime exists / on a closed channel)** — mitigated. `wait_for_initialization()` plus the unbounded buffer; every send failure is reported to stderr.
- **T-02-08-03 (diagnostics corrupting the stdio stream)** — mitigated. No stdout write added.
- **T-02-08-04 (notification without its capability)** — mitigated. `logging` declared and asserted; negative control B binds it.
- **T-02-08-06 (unexpected event shape derailing reply matching)** — mitigated. The event arm forwards and continues; reply matching is unchanged and keyed on the id map.
- **T-02-08-05 (event flood)** and **T-02-08-07 (package installs)** were accepted by the plan and are unchanged: the channel is still unbounded, and no dependency was added.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- **AGENT-04 is In Progress, not Complete.** Close and crash reach MCP clients; `open` has no wire event. The remaining slice is scoped and costed in `deferred-items.md` and needs no change to anything built here.
- **Plan 02-11 (CI) inherits an unchanged clippy baseline of 7 lints** — this plan added none — and 12 e2e suites, one of which now spawns two proxy processes and needs the same Xvfb shell.
- **The proxy is now a full-duplex MCP server.** Anything else that wants to push to a client — a download-progress signal, a takeover notice — should add an `Event` variant and reuse `notify_tab_events`, not add a second mechanism.
- **Two suites now assert cross-session silence**, `crash_event_test.py` at the socket and `mcp_client_test.py` through the real binaries. Both fail on the same pre-02-07 revert, so a future change that re-broadens addressing is caught at both layers.

## Self-Check: PASSED

- `crates/talaria-mcp/src/socket.rs` — FOUND
- `crates/talaria-mcp/src/main.rs` — FOUND
- `tests/e2e/mcp_client_test.py` — FOUND
- `.planning/phases/02-harden-the-agent-surface/02-08-SUMMARY.md` — FOUND
- Commits `5a7f571`, `501ea13`, `35448a1` — all FOUND in git history.

---
*Phase: 02-harden-the-agent-surface*
*Completed: 2026-08-16*

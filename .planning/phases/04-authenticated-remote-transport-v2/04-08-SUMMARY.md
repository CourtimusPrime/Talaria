---
phase: 04-authenticated-remote-transport-v2
plan: 08
subsystem: auth
tags: [oauth, rfc7009, revocation, streams, access-panel, egui, sockets]

# Dependency graph
requires:
  - phase: 04-authenticated-remote-transport-v2
    provides: "04-07's `/token` and its two grants, plus its reconnaissance on the open-stream problem; 04-06's endpoint map, `handle_request` shape, `sanitize_claim`, `register_client`/`handler`-`grant` split and the consent panel's arm-then-confirm; 04-05's `TalariaAuth`, `SharedAgents`, `digest_prefix`, `canonical_resource` and the live per-request `verify`; 04-04's `Agents` store — `clients`, `tokens`, `revoke_client`, `revoke_family`, `digest_of`, `digests_match` and the atomic whole-document save; 04-03's `http.rs` listener, its BYO `mcp_routes` mount, `RemoteAccess`, `ShutdownHandle` and the Access panel's status half; 04-02-SPIKE's A6 finding on live-handle addressability"
  - phase: 03-table-stakes-browsing
    provides: "`chrome_rects`/`record_rect` behind `TALARIA_TEST_HOOKS`, `click_rect`'s real-pointer technique, `wait_for_rect`'s present/absent forms, `relative_time`, `truncate_chars`, the striped scrolling list, and the arm-then-confirm control with two rect names for one button"
provides:
  - "`/revoke` (RFC 7009), declared, advertised and answered — always success, and revoking either half of a pair takes the family"
  - "`StreamRegistry` and `Connections`: a revoked client's already-open response streams are closed, at the connection, immediately"
  - "The measured answer to `04-02-SPIKE.md`'s open half of A6 — ending a session does **not** terminate a stream already open on it, and why"
  - "The Access panel's client list: one row per approved client, newest first without sorting, the claimed name sanitised and framed"
  - "`Gui::confirm_revoke: Option<String>` — arm-then-confirm keyed on identity rather than on row position"
  - "`UiAction::RevokeClient(String)`, applied in `apply_ui_actions` and reachable from no tool, command or page"
  - "`tests/e2e/revocation_test.py` — Success Criterion 3 proven including the open-stream closure, asserted from the client end"
  - "`CHANGELOG.md`, `SECURITY.md` and `deferred-items.md` closing the phase"
affects: [phase-05, 04-verify-work]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "A trait between the policy and the transport (`SessionDirectory`) so the *selection* half of a revoke is unit-testable without a bound port, and the transport half is proven end to end"
    - "Arm-then-confirm keyed on an identifier rather than a `bool` or an index, for any control that acts on one row of a list that can reorder"
    - "Rects for a state that draws no control (`access.empty`, `access.next-step`), so an empty panel is distinguishable from a panel that drew nothing"
    - "An in-process `McpHttpHandler` with an empty middleware chain, mounted on no route, to reach a `pub(crate)` SDK path"
    - "A duplicated descriptor rather than a remembered number, whenever a socket has to be closed later than it was seen"

key-files:
  created:
    - tests/e2e/revocation_test.py
  modified:
    - crates/talaria-shell/src/oauth.rs
    - crates/talaria-shell/src/http.rs
    - crates/talaria-shell/src/gui.rs
    - crates/talaria-shell/src/app.rs
    - crates/talaria-shell/src/agents.rs
    - tests/e2e/run_all.py
    - SECURITY.md
    - CHANGELOG.md
    - .planning/phases/04-authenticated-remote-transport-v2/deferred-items.md

key-decisions:
  - "Ending a session does not close a stream already open on it — measured, not assumed. The SDK's `ServerRuntime::shutdown` cancels the reader; the response body is fed from a duplex half the transport's dispatcher still owns on a task that outlives the session-store entry. So the shell closes the *connection* instead, and the bound is immediate rather than one keep-alive interval."
  - "The keep-alive fallback `04-RESEARCH.md` named was unavailable, not declined: it lives inside the SDK's own keep-alive task, which this crate cannot reach any more than it can reach `shutdown`."
  - "Revoking either half of a token pair through `/revoke` takes the whole family. RFC 7009 makes it a SHOULD one way and leaves the other open; a client that disowned its access token while keeping a live refresh token could exchange straight back into a working pair."
  - "`/revoke` removes tokens and leaves the registration. The human's revoke removes both. They are different acts: a client tidying up after itself is not a human withdrawing access."
  - "A revoked client's tabs stay open, visible in the Agents view and available for takeover. Closing them destroys state the human may want and buys nothing, since the agent can no longer drive them."
  - "`Agents::mint` deleted rather than retargeted a fourth time. What the tests needed became `mint_for_test`, compiled only under `cfg(test)`, so no production path can reach for it."
  - "Two rect names added beyond `04-UI-SPEC.md`'s list, for the empty state's two lines, because otherwise the plan's own step 14 was unassertable."

patterns-established:
  - "Assert the thing that would fail if the feature were absent: the suite asserts the stream *closes*, from the client end, and it failed on first run — which is what proved the SDK's session delete insufficient"
  - "Frame an untrusted claim with quotes plus an explicit claimed-suffix wherever a fence has no room to exist"
  - "Prune a side table at the point that already holds the authoritative list (connection entries are retained against the session listing a revoke performs anyway)"

requirements-completed: [AUTH-02]

coverage:
  - id: D1
    description: "The Access panel lists every authorized client and revokes one on a deliberate two-click gesture, and a revoked client's next request is refused while every other client keeps working (SC 3)"
    requirement: "AUTH-02"
    verification:
      - kind: e2e
        ref: "tests/e2e/revocation_test.py (PANEL, ARMED, REVOKED, REFUSED, UNTOUCHED)"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/run_all.py (22/22)"
        status: pass
    human_judgment: false
  - id: D2
    description: "A revoked client's already-open response stream closes, asserted from the client end rather than from server bookkeeping"
    requirement: "AUTH-02"
    verification:
      - kind: e2e
        ref: "tests/e2e/revocation_test.py (STREAM CLOSED — the read side sees the response body end within 8s of the revoke)"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/http.rs#a_revoke_ends_every_stream_of_that_client_and_no_other"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/http.rs#an_installed_registry_hands_the_work_to_the_listeners_runtime"
        status: pass
    human_judgment: false
  - id: D3
    description: "Revoking one client does not close another client's stream, and terminating everything is what a listener shutdown does"
    requirement: "AUTH-02"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/http.rs#shutting_the_listener_down_ends_every_stream"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/http.rs#a_session_with_no_verified_client_is_nobodys"
        status: pass
      - kind: e2e
        ref: "tests/e2e/revocation_test.py (UNTOUCHED — the second client's stream is still open after the first is revoked)"
        status: pass
    human_judgment: false
  - id: D4
    description: "Revocation survives a restart: the client is gone from the store on disk, not only from memory"
    requirement: "AUTH-02"
    verification:
      - kind: e2e
        ref: "tests/e2e/revocation_test.py (RESTART)"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/agents.rs#revoking_a_client_removes_its_registration_and_its_tokens"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/agents.rs#edge_probe_adjacency_revoking_twice_reports_changed_then_unchanged_and_writes_nothing"
        status: pass
    human_judgment: false
  - id: D5
    description: "The revoke control is armed by a first click and completed by a second, keyed on the client id, and resets when the panel closes"
    requirement: "AUTH-02"
    verification:
      - kind: e2e
        ref: "tests/e2e/revocation_test.py (ARMED — access.confirm-revoke.N appears for exactly one row and access.revoke.M is unchanged on the other)"
        status: pass
      - kind: other
        ref: "grep -c 'confirm_revoke' crates/talaria-shell/src/gui.rs == 7 (field, init, reset, frame local, armed test, two writes back)"
        status: pass
    human_judgment: false
  - id: D6
    description: "The revocation endpoint answers success for a live token, an already-revoked one and one that was never issued, and revoking either half of a pair takes the family"
    requirement: "AUTH-02"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#revoking_a_live_an_already_revoked_and_a_never_valid_token_are_one_answer"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#revoking_a_refresh_token_removes_the_access_token_issued_beside_it"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#revoking_an_access_token_takes_its_family_with_it"
        status: pass
      - kind: e2e
        ref: "tests/e2e/revocation_test.py (RFC 7009)"
        status: pass
    human_judgment: false
  - id: D7
    description: "The revoke control is unreachable from any MCP tool, any control-socket command and any page"
    requirement: "AUTH-02"
    verification:
      - kind: other
        ref: "grep -c 'RevokeClient' crates/talaria-mcp/src/tools.rs crates/talaria-protocol/src/lib.rs == 0"
        status: pass
      - kind: e2e
        ref: "tests/e2e/panel_click_test.py (GATED — chrome_rects is refused as an unknown command without TALARIA_TEST_HOOKS, unmodified by this plan)"
        status: pass
    human_judgment: false
  - id: D8
    description: "The claimed client name in an Access row is sanitised, truncated and marked as claimed, and cannot displace a control"
    requirement: "AUTH-02"
    verification:
      - kind: e2e
        ref: "tests/e2e/revocation_test.py (PANEL — a name carrying a newline, a bidi override and an ASCII double quote leaves both rows and both buttons laid out in order)"
        status: pass
      - kind: other
        ref: "grep -c 'sanitize_claim' crates/talaria-shell/src/gui.rs == 18"
        status: pass
    human_judgment: false
  - id: D9
    description: "Zero authorized clients renders an empty state, plus a next step when the listener is on"
    requirement: "AUTH-02"
    verification:
      - kind: e2e
        ref: "tests/e2e/revocation_test.py (EMPTY — access.empty and access.next-step both drawn, in that order, with no access.row.0)"
        status: pass
    human_judgment: false
  - id: D10
    description: "The Access panel's layout inside the existing chrome, with clients present and with none"
    requirement: "AUTH-02"
    verification:
      - kind: e2e
        ref: "tests/e2e/revocation_test.py (PANEL — rect ordering only: status above row 0, row 0 above row 1, both buttons non-zero width)"
        status: pass
    human_judgment: true
    rationale: "Rect ordering proves nothing overlaps or vanishes; it does not prove the panel reads well. `04-VALIDATION.md` already carries this as a held-out visual item and it stays one — open the panel populated and empty, in both Me and Agents views, and look."
  - id: D11
    description: "CHANGELOG.md, SECURITY.md and deferred-items.md record what this phase shipped, what revocation guarantees, and what a later phase may want to revisit"
    verification:
      - kind: other
        ref: "grep -c 'AUTH-0[123]' CHANGELOG.md == 5; grep -ci 'off by default' CHANGELOG.md == 1; grep -c '^## ' deferred-items.md == 8"
        status: pass
    human_judgment: true
    rationale: "A grep proves the strings are present, not that the prose is accurate or complete. Two stale claims in CHANGELOG.md were corrected here after being found by reading; a third reader may find more."

# Metrics
duration: 85min
completed: 2026-08-21
status: complete
---

# Phase 04 Plan 08: Revocation Summary

**Individual revocation that actually revokes: the Access panel's client list with an identity-keyed two-click Revoke, RFC 7009's `/revoke`, and — the part `04-RESEARCH.md` predicted would be marked done while being false — termination of a revoked client's already-open response streams, at the connection, proven from the client end after the obvious mechanism was measured and found insufficient.**

## Performance

- **Duration:** 85 min
- **Started:** 2026-08-21T00:44:11Z
- **Completed:** 2026-08-21T02:09:14Z
- **Tasks:** 3
- **Files created:** 1
- **Files modified:** 9

## Accomplishments

- **`/revoke` answers** (RFC 7009). Declared in the endpoint map, named in the RFC 8414 document with `revocation_endpoint_auth_methods_supported: ["none"]`, and handled — with `validate_allowed_methods` called in its arm like every other, because the SDK ships that table and never calls it. The response is a constant built after the store call's answer has been discarded: a live token, an already-revoked one and one that was never issued get the same status, the same headers and the same empty body. Revoking either half of a pair takes the family.
- **The open-stream gap is closed, and the closing mechanism is not the one the plan expected.** `04-02-SPIKE.md` left A6 half-open. This plan answered the open half by asserting the closure from the client end and watching it fail: the SDK's own `DELETE /mcp` — which calls `ServerRuntime::shutdown` — cancels the *reader*, while the response body is fed from the other half of a duplex the transport's message dispatcher still owns on a task that outlives the session-store entry. Nothing in the published API drops it. So `http.rs` closes the connection instead: `tap_io` notes each accepted socket, one router layer binds it to the session that opens a standalone stream over it, and a revoke shuts it down. The bound is immediate.
- **Descriptors are duplicated, never remembered.** A descriptor number is recycled the instant its connection closes, and shutting down a recycled one in *this* process could tear apart the control socket or one of the engine's own connections. `OwnedSocket` holds a `dup`, so the number it carries names that connection and nothing else for as long as it exists.
- **The Access panel's list half.** One row per client a human actually approved — a registration alone is inert and is not a row — newest first by display-side reversal rather than a sort, striped, scrolling, with the claimed name through `truncate_chars` then `sanitize_claim`, rendered in quotes with an `(as claimed)` suffix and the full values in hover text.
- **The confirm is keyed on identity.** `Gui::confirm_revoke` is an `Option<String>` holding a `client_id`, not a `bool` and not an index. A list that reorders between the arming click and the confirming one cannot move the confirmation onto a different agent, and the two states carry different rect names so the arming is read rather than inferred.
- **`UiAction::RevokeClient` does both halves in one arm** — the store mutation that refuses the next request, and the stream termination that closes the ones already open — with the concurrency boundary and the decision to leave a revoked client's tabs alone stated at the arm rather than left to discovery.
- **`tests/e2e/revocation_test.py`** — two clients authorized through real consent clicks, two open streams read with a raw socket and `select`, one revoked by real clicks on the real control, and thirteen assertions including the one that failed first.
- **The phase's closing documentation**, including two corrections: `CHANGELOG.md` no longer says the transport "refuses everything" or that the authorization server is "deliberately absent", both of which stopped being true in 04-05 through 04-07 and neither of which any plan had gone back to fix.

## Task Commits

Each task was committed atomically:

1. **Task 1: The revocation endpoint and the live-stream registry** — `72d3a63` (feat)
2. **Task 2: The Access panel's client list and the per-client Revoke** — `42d6a04` (feat)
3. **Task 3: The revocation e2e suite, and the phase's closing documentation** — `8d4e6e5` (test)

**Plan metadata:** see the final `docs(04-08)` commit.

## Files Created/Modified

- `tests/e2e/revocation_test.py` (new, 724 lines) — the suite, its `Stream` class (raw socket, `select`, chunked framing) and its honest-limits docstring.
- `crates/talaria-shell/src/http.rs` — `StreamRegistry`, `SessionDirectory`, `SessionOwner`, `terminate_matching`, `McpSessions`, `Connections`, `OwnedSocket`, the `dup`/`shutdown`/`close` extern block, `note_streaming_connection`, the `tap_io` hook, `into_make_service_with_connect_info`, `spawn`'s second return value, and 7 unit tests.
- `crates/talaria-shell/src/oauth.rs` — `REVOCATION_PATH`, the endpoint registration, `revocation_endpoint` in the metadata, `handle_revocation` / `revoke_presented_token`, and 6 unit tests; two existing tests updated for the endpoint that now exists.
- `crates/talaria-shell/src/gui.rs` — `UiAction::RevokeClient`, `Gui::confirm_revoke`, the Access panel's list body, the empty state and its two rects, the listener-off notice.
- `crates/talaria-shell/src/app.rs` — `Shared::remote_streams`, the `UiAction::RevokeClient` arm, `start_remote_listener`'s two-value spawn.
- `crates/talaria-shell/src/agents.rs` — `Agents::mint` deleted, `mint_for_test` in its place under `cfg(test)`; the last five `expect(dead_code)` attributes consumed and the module header's paragraph about them rewritten in the past tense.
- `tests/e2e/run_all.py` — `revocation_test` registered, docstring updated, and a note on which two suites are the slow ones.
- `SECURITY.md`, `CHANGELOG.md`, `deferred-items.md` — the phase close.

## Decisions Made

**Ending a session is two actions, and only the first is one the client can see.** This is the plan's central finding and it was measured, not reasoned about. The `SessionDirectory::end` implementation closes the connection *first* — that is what the client's read side observes as the response body ending — and then issues the SDK's own session delete, which retires the session so its id is dead and its runtime is released. Writing it the other way round would have looked identical in the code and been wrong in the only way that matters.

**The keep-alive fallback was unavailable, not declined.** `04-RESEARCH.md` named per-keepalive re-verification as the fallback if handles turned out not to be addressable. That fallback lives inside `rust-mcp-transport`'s own keep-alive task, which writes a bare `"\n"` on a timer and has no hook. It is as far out of reach as `ServerRuntime::shutdown` is. The connection-level close is a *third* mechanism, not the fallback, and `deferred-items.md` records what would retire it.

**Revoking either half of a pair takes the family.** RFC 7009 §2.1 makes it a SHOULD in one direction and leaves the other to the server. A client revoking its own access token has finished with this browser; leaving its refresh token live would mean the credential it just disowned can be exchanged straight back into a working pair, which is revocation that revokes nothing.

**`/revoke` leaves the registration; the panel's Revoke removes it.** Two different acts. A client tidying up after itself has not been un-approved by anybody, and a `/revoke` that dropped the registration would let any unauthenticated caller holding one stale token erase a row the human approved.

**A revoked client's tabs stay open.** Recorded in `SECURITY.md`, in `deferred-items.md`, and in the code comment at the arm. They are visible in the Agents view and the human can take any of them over; closing a person's tabs because a credential was withdrawn destroys state they may want and buys nothing, because the agent can no longer drive them. Nothing else of the human's is touched — no history row, no bookmark, no credential, no downloaded file.

**Two rect names beyond the UI-SPEC's list.** `access.empty` and `access.next-step`. The spec fixes the row and button names; it has none for a state that draws no control, which made the plan's own step 14 ("assert the empty state renders and, since the listener is on, that the next-step line renders with it") unassertable — an empty panel and a panel that drew nothing look the same through `chrome_rects`. The plan's prohibition explicitly anticipates new rect names being covered by construction, since the hook is gated at the command level.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 — Bug] The SDK's session delete does not close an already-open stream**

- **Found during:** Task 3, on the first run of the suite's step 9
- **Issue:** Task 1 implemented termination as the plan's `<action>` described — reach the SDK's own session-delete path, which calls `ServerRuntime::shutdown` before removing the entry — on the reading that shutting the transport down closes the stream's write half. The e2e asserted the closure from the client end and failed: `STREAM CLOSED` never happened, while every other assertion around it passed. Reading `rust-mcp-transport`'s `SseTransport::shut_down` explains it: the cancellation token it fires reaches only `spawn_reader`, the client-to-server direction. The response body is fed from the other half of a `duplex` owned by the `MessageDispatcher` inside the transport, which the spawned `start_stream` task still holds. The body therefore never reaches end-of-stream.
- **Fix:** Added `Connections` to `http.rs`: `tap_io` records each accepted connection's descriptor against its peer address; one `axum` router layer binds that connection to the session id whenever a `GET /mcp` is answered `200`; `SessionDirectory::end` closes it with `shutdown(2)` before issuing the delete. Descriptors are `dup`ed at bind time so a recycled number can never be the one torn down, and the duplicate is released when the entry is dropped. The peer address comes from `ConnectInfo`, which axum fills from the accepted socket — nothing a caller sends can reach it. `MAX_REMEMBERED_CONNECTIONS` bounds the waiting-room table; the streaming table is pruned against the session listing every revoke already performs.
- **Files modified:** `crates/talaria-shell/src/http.rs`
- **Verification:** `STREAM CLOSED: the revoked client's open stream ended within 8s`, and `UNTOUCHED` immediately after it proving the other client's stream is still open. The `StreamRegistry` doc comment was rewritten to state the measured finding rather than the assumed one.
- **Committed in:** `8d4e6e5`

**2. [Rule 1 — Bug] A `Stream` that waits for EOF reports a cleanly-ended stream as still delivering**

- **Found during:** Task 3
- **Issue:** The suite's first `Stream` implementation treated "the stream ended" as a closed socket. The response is HTTP/1.1 `Transfer-Encoding: chunked` on a keep-alive connection, so a stream that ends cleanly does so with a zero-length chunk while hyper holds the TCP connection open for a request that will never come. That is the false *negative* twin of the false positive this suite exists to prevent, and it would have made the suite unable to see a working implementation.
- **Fix:** `Stream` now decodes chunked framing and accepts either ending — the zero-length chunk or a closed socket. The class docstring says why, at length, because getting it wrong makes the test lie in either direction.
- **Files modified:** `tests/e2e/revocation_test.py`
- **Verification:** `STREAMS: two open event streams, both delivering` distinguishes a stream carrying keep-alive frames from one that ended, and `STREAM CLOSED` fires on the chunk terminator.
- **Committed in:** `8d4e6e5`

**3. [Rule 3 — Blocking] The empty state was unassertable**

- **Found during:** Task 3, writing step 14
- **Issue:** `04-UI-SPEC.md` fixes rect names for the rows and both button states and none for the empty state, which draws two labels and no control. Through `chrome_rects`, "the panel shows the empty state" and "the panel drew nothing at all" are the same observation, so the plan's own step 14 could not be written honestly.
- **Fix:** `access.empty` and `access.next-step`, recorded only in the state that draws them — the same trick `downloads.dismiss-error` already uses, where the rect's presence *is* the evidence. Two rect names added to the plan's stated artifact list.
- **Files modified:** `crates/talaria-shell/src/gui.rs`
- **Verification:** `EMPTY: nothing authorized renders the empty state and its next step`, including the ordering assertion between them.
- **Committed in:** `8d4e6e5`

**4. [Rule 3 — Blocking] `Agents::mint` had six test call sites**

- **Found during:** Task 1
- **Issue:** 04-07's handover said to delete `Agents::mint` rather than retarget its `expect(dead_code)` a third time. It has no production caller, but three unit tests in `agents.rs` and three in `oauth.rs` use it to place a credential of a chosen kind, family, expiry and audience — which the two production issuing paths (`authorize`, `rotate_refresh`) deliberately choose themselves.
- **Fix:** The public `mint` is gone. `mint_for_test` took its place, compiled only under `cfg(test)`, so no production path can call it and no later edit can start. `stage_token`'s doc comment records the whole history — written in 04-04, deferred three times, deleted in the plan that was meant to wire it.
- **Files modified:** `crates/talaria-shell/src/agents.rs`, `crates/talaria-shell/src/oauth.rs`
- **Verification:** `cargo clippy --all-targets --locked -- -D warnings` exits 0; the module header's `dead_code` paragraph is rewritten in the past tense because there are none left.
- **Committed in:** `72d3a63`

**5. [Rule 2 — Missing critical] `CHANGELOG.md` carried two claims that had stopped being true**

- **Found during:** Task 3
- **Issue:** The plan says no earlier plan in this phase touches `CHANGELOG.md`, so the whole phase would be written once here. Five earlier plans did touch it, and two of their claims were left stale by later work: the AUTH-03 entry is headed "off by default and **refusing everything**", and the AUTH-01 entry says the authorization server is "still deliberately absent … which is the next plan's". 04-05 through 04-07 shipped it. A changelog records what shipped, not how it was sequenced, and a reader of the Unreleased section would have been told the opposite of what the code does.
- **Fix:** The AUTH-03 heading became "off by default and loopback-only" and its second paragraph now states the token-is-the-boundary reasoning in the present tense; the AUTH-01 entry's absence list now names only TLS, the non-loopback bind, and CIMD. A new AUTH-01 entry covers the authorization server itself, and the AUTH-02 entry covers this plan.
- **Files modified:** `CHANGELOG.md`
- **Verification:** `grep -c 'AUTH-0[123]' CHANGELOG.md` is 5; `grep -ci 'off by default'` is 1.
- **Committed in:** `8d4e6e5`

**6. [Rule 1 — Bug] `deferred-items.md` claimed `sse_support` was set to `false`**

- **Found during:** Task 3
- **Issue:** 04-02's entry says `AxumServerOptions.sse_support … is set to `false` explicitly`. 04-03 took the BYO-server mount path, which has no `AxumServerOptions` at all, and 04-03 also measured that the flag only affects a log line — `mcp_routes` mounts `/sse` and `/messages` unconditionally either way.
- **Fix:** The 04-02 entry now points at the new "`sse_support: false` is cosmetic" entry, which records the measurement, why it is safe today (both routes sit behind the same middleware chain, and the `sse` feature is absent from the crate), and the two-line change that would close it.
- **Files modified:** `.planning/phases/04-authenticated-remote-transport-v2/deferred-items.md`
- **Verification:** read against `build_router`'s own comment and `Cargo.toml`'s `rust-mcp-sdk` feature list.
- **Committed in:** `8d4e6e5`

---

**Total deviations:** 6 auto-fixed (3 bugs, 1 missing critical, 2 blocking)
**Impact on plan:** Deviation 1 changed the *mechanism* the plan specified — the plan named the session-delete path and it does not work — but not its shape, its bound or its acceptance criteria. Everything else made an assertion assertable or corrected a document that had gone stale. No new dependency, no `Cargo.toml` or `Cargo.lock` change, no new config key, no new environment variable, no new on-disk file.

## What the plan's verification section asked the SUMMARY to record

**Which mechanism the stream termination used, and why (the A6 finding).** Neither of the two the plan anticipated. A6's settled half held: a live session is addressable by client identity, through `session_store.keys()` and `auth_info_cloned()`, and that is what `terminate_matching` uses to decide *which* sessions to end. A6's open half — whether ending a session terminates a stream already open on it — resolved **negative**, and this plan is where it was finally observed rather than inferred. The recorded fallback was unavailable for the same reason the delete was insufficient: both live inside the SDK. What works is closing the TCP connection the stream is riding on, which the listener owns because the listener accepted it. The bound is immediate, so no keep-alive interval goes into `SECURITY.md`; what goes in instead is the guarantee itself and the mid-flight boundary beside it.

**Whether revoking an access token takes its family.** Yes, and it is this module's decision rather than the specification's. RFC 7009 §2.1 requires a refresh token to take its access tokens and leaves the reverse open. Talaria takes it both ways: a client revoking its own access token has finished, and a live refresh token beside a disowned access token is a credential that can be exchanged straight back into a working pair. Unit-tested from both ends.

**The decision to leave a revoked client's tabs open.** Recorded above, in `SECURITY.md`, in `deferred-items.md` with the condition for revisiting, and in the code comment at the arm that performs the revoke.

**Closing test counts against the 117-unit / 19-e2e baseline.** **290 unit tests** (285 shell + 3 protocol + 2 mcp lib), up from 276 entering this plan and from 117 when the phase's baseline was written. **e2e 22/22**, up from the 19 the baseline names and the 21 this plan inherited. Wall clock for `run_all.py`: 11 min 5 s; `revocation_test.py` alone is 68 s, of which roughly 12 s is the deliberate wait for a keep-alive frame that proves a stream is delivering before the suite asserts that revoking closes it.

## Verification Results

| Check | Result |
|---|---|
| `cargo build --release --locked` | exit 0 |
| `cargo clippy --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --locked` | **290 passed** (285 shell + 3 protocol + 2 mcp lib), up from 276 |
| `cargo test -p talaria-shell oauth::` | 86 passed, up from 79 |
| `cargo test -p talaria-shell http::` | 7 passed, up from 0 |
| `python3 tests/e2e/revocation_test.py` | exit 0, `REVOCATION CHECKS PASSED`, 68 s |
| `python3 tests/e2e/run_all.py` | **22/22 PASS**, `failed: none`, 11 m 5 s |
| `grep -ci 'revoke' oauth.rs` | 37 (≥ 3) |
| `grep -c 'revoke_family\|revoke_client' oauth.rs` | 2 (≥ 1) |
| `grep -ci 'terminate' http.rs` | 20 (≥ 2) |
| `grep -c 'unwrap()' oauth.rs http.rs gui.rs app.rs` | 0 in each |
| `grep -c 'access.revoke\|access.confirm-revoke\|access.row' gui.rs` | 3 (≥ 3) |
| `grep -c 'confirm_revoke' gui.rs` | 7 (≥ 3) |
| `grep -c 'sanitize_claim' gui.rs` | 18 (≥ 2) |
| `grep -c 'sort_by\|sort_unstable\|\.sort()' gui.rs` | 0 |
| `grep -c 'RevokeClient' crates/talaria-mcp/src/tools.rs crates/talaria-protocol/src/lib.rs` | 0 and 0 |
| `grep -c 'RevokeClient' app.rs` | 3 (≥ 1) |
| `grep -c 'revocation_test' tests/e2e/run_all.py` | 3 (≥ 2) |
| `grep -c 'access.confirm-revoke' revocation_test.py` | 7 (≥ 1) |
| `grep -ci 'select' revocation_test.py` | 4 (≥ 1) |
| `grep -ci 'open stream\|stream closed' revocation_test.py` | 5 (≥ 1) |
| `grep -ci 'already passed verification\|in-flight\|mid-flight' SECURITY.md` | 2 (≥ 1) |
| `grep -c '^## ' deferred-items.md` | 8 (≥ 5) |
| `grep -ci 'SSRF\|request forgery' deferred-items.md` | 2 (≥ 1) |
| `grep -c '2026-07-28' deferred-items.md` | 7 |
| `grep -c 'AUTH-0[123]' CHANGELOG.md` | 5 (≥ 3) |
| `grep -ci 'off by default' CHANGELOG.md` | 1 (≥ 1) |
| `git diff --stat Cargo.toml Cargo.lock crates/talaria-shell/Cargo.toml` | no change |
| `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'` | **1** |

The 9 *ignored* doc-tests on `crates/talaria-mcp/src/tools.rs` are pre-existing and untouched. `mcp_client_test.py`, `panel_click_test.py`, `http_transport_test.py`, `oauth_flow_test.py` and `scheme_refusal_test.py` all pass unmodified. No `expect(dead_code)` or `allow(dead_code)` attribute remains anywhere in `crates/`.

## Threat Model Disposition

| Threat | Disposition | Where it landed |
|---|---|---|
| T-7 (a revoked agent's open event stream continuing to deliver) | **mitigated** | Closing the connection the stream rides on, in the same action as the store mutation. Asserted from the client end in `revocation_test.py` at `STREAM CLOSED`, after the first mechanism was tried and measured insufficient. Unit-tested for selection (`a_revoke_ends_every_stream_of_that_client_and_no_other`) and for delivery to the listener's runtime. |
| T-04-08-01 (a revoke reachable from an MCP tool or a control-socket command) | mitigated | `RevokeClient` follows `OpenDownload`'s rule and carries its doc comment. `grep -c 'RevokeClient'` across `tools.rs` and the protocol crate is 0; `panel_click_test.py`'s hook-gating assertion is unmodified and still passes. |
| T-04-08-02 (a confirming click landing on a different client than the one it was armed on) | mitigated | `confirm_revoke` is an `Option<String>` holding a `client_id`. The e2e asserts one row armed and the other unchanged, by rect name, and asserts the surviving row is *not* armed after the revoke. |
| T-04-08-03 (the revocation endpoint as an oracle) | mitigated | One response expression, built after the store's answer is discarded. Status and headers compared as values across a live, an already-revoked and a never-valid token, in a unit test and again over the wire in the e2e. |
| T-4 (an attacker-chosen display name in a list row) | mitigated | `truncate_chars` then `sanitize_claim`, quoted with a claimed-suffix. The e2e's second client registers a name carrying a newline, a bidi override and an ASCII double quote, and both rows stay laid out in order with both buttons clickable. |
| T-04-08-04 (revocation not surviving a restart) | mitigated | `revoke_client` removes the registration with the tokens, through the store's atomic whole-document rewrite. The e2e stops the shell, waits for the socket to go, restarts, and re-asserts the refusal, the row count and the surviving client. |
| T-04-08-05 (the stream registry growing without bound) | mitigated | The streaming table is pruned against the session listing every revoke already performs; the accepted-connection table is capped at `MAX_REMEMBERED_CONNECTIONS` and its entries are consumed on use. Unit-tested that a session which ended normally is not there to be ended. |
| T-04-08-SC (package installs) | accept | No dependency change. `git diff --stat Cargo.toml Cargo.lock` is empty; the three POSIX calls this plan needed are an `extern "C"` block, the same call `talaria_protocol` makes for `getuid`. |

**New surface introduced by this plan, and why it is not reachable from the network:** one `axum` router layer that reads the request and changes nothing, and one `McpHttpHandler` with an empty middleware chain that is mounted on no route. The second is the one worth naming. It exists to reach a `pub(crate)` SDK path from inside this process, and it is not a hole in authentication because it is not reachable *at all* — in the same way `apply_ui_actions` is not reachable. Its one caller is the chrome's own revoke.

**Concurrency, stated rather than mitigated:** a request that passed verification a moment before a revoke completes runs to completion. Verification is per request, and a command already executing against the engine is not interruptible. Documented at the arm in `app.rs`, in `SECURITY.md`'s "What revocation guarantees", and in the e2e suite's docstring, which says plainly why the suite does not try to catch one.

## Issues Encountered

The genuine one was deviation 1, and it is worth restating because it is the whole reason this plan was written the way it was. Task 1 shipped a termination mechanism that read correctly, compiled, logged `closed 1 open stream(s) belonging to client …`, and did not close the stream. Every piece of server-side bookkeeping agreed that it had worked. The only thing that disagreed was a client holding the connection, which is exactly the assertion `04-RESEARCH.md`'s Pitfall 4 says revocation tests skip and exactly the assertion this suite was required to make. A test that had checked "does a fresh request now fail" would have passed at every stage of this plan, including the stage where the feature did not work.

The second, smaller one is deviation 2, and it has the same shape in reverse: the first `Stream` implementation could not have seen a working fix either, because it waited for a socket close that a keep-alive connection never gives. Both directions of that framing mistake are now documented in the class's own docstring.

## Notes for the phase

- **`Connections` is a workaround with a retirement condition**, recorded in `deferred-items.md`. If `rust-mcp-sdk` gains a public way to end a stream's write half — or if the sessionless model of revision `2026-07-28` lands and changes the shape of the problem — `Connections`, its `extern "C"` block and the router layer should go. `revocation_test.py` should keep asserting exactly what it asserts now, so the replacement has to prove the same thing.
- **The `extern "C"` block is the second one in this workspace.** The first is `talaria_protocol`'s `getuid`. Anyone adding a third should ask whether `libc` is now worth the dependency.
- **`Agents` has no `expect(dead_code)` attributes left**, and neither does anything else in `crates/`. The convention 04-05 established — one attribute per unreachable item, each naming the plan that will reach it, deleted by that plan when the expectation becomes unfulfilled — worked exactly as designed across five plans and is worth reusing.
- **The Access panel's visual quality is the one held-out item.** `04-VALIDATION.md` already carries it, D10 above repeats it, and rect ordering is not a substitute for looking at it.

## User Setup Required

None — no external service configuration required.

## Next Phase Readiness

AUTH-02 is complete and Success Criterion 3 is closed, including the part of it that `04-RESEARCH.md` predicted would be marked done while being false. With AUTH-01 (04-05 through 04-07) and AUTH-03 (04-03) already closed, all three of the phase's requirements are met: a remote MCP transport that is off by default and loopback-only, an OAuth 2.1 authorization server this browser hosts itself with approval in native chrome, and individual revocation that takes both a client's credentials and its live connections.

Phase 5 inherits two named prerequisites, both already in `deferred-items.md`: a certificate story before any non-loopback bind, and the `2026-07-28` migration with its sessionless model. It also inherits one thing this plan built that Phase 5 should expect to revisit — the connection-level stream termination, which is correct today and is a workaround for an SDK limitation rather than a design.

---
*Phase: 04-authenticated-remote-transport-v2*
*Completed: 2026-08-21*

## Self-Check: PASSED

Every file named above exists on disk. All four commits (`72d3a63`, `42d6a04`,
`8d4e6e5`, `a73e24f`) are in the log. Every unit test named in the coverage
block resolves to a `fn` in the file it names — one initially did not
(`agents.rs#revoking_a_client_removes_its_registration_and_survives_a_reload`,
which does not exist) and was corrected to the two tests that do cover the
claim.

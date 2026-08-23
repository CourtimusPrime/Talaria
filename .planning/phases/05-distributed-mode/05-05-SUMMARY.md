---
phase: 05-distributed-mode
plan: 05
subsystem: infra
tags: [websocket, axum, oauth-bearer, cswsh, revocation, tab-ownership, e2e]

# Dependency graph
requires:
  - phase: 05-distributed-mode
    provides: "05-01's A2 spike — Serve proxies a WebSocket upgrade cleanly and synthesises no `Origin`, which is why the outermost origin refusal closes CSWSH without costing a legitimate viewer anything"
  - phase: 05-distributed-mode
    provides: "05-03's wire vocabulary — `Channel`, `ClientView`, `ServerView`, `TabList`, and the reason `Refused` carries no field"
  - phase: 05-distributed-mode
    provides: "05-04's `admitted_hosts` two-name allowlist and `AdvertisedIdentity`, which the view route inherits through the layer it is merged inside"
  - phase: 04-authenticated-remote-transport-v2
    provides: "`TalariaAuth` as the shared `Arc<dyn AuthProvider>`, `canonical_resource` as the sole audience spelling, `refuse_page_originated` as the router's outermost layer, `StreamRegistry`/`Connections` as the revoke's stream half, and `revocation_test.py`'s client-end closure assertion"
provides:
  - "`GET /view` — a WebSocket upgrade merged inside `build_router`'s two existing layers, authenticated by its **own** direct call to the shared authorization provider"
  - "`http::ViewSockets` — the stream registry's second half: close handles keyed by verified client id, so `terminate_client` and the shutdown path reach a socket the SDK's session directory cannot name"
  - "`crates/talaria-shell/src/view.rs` — the view session table, the agent-only snapshot, attach/detach, the concurrent-attachment cap and the per-connection input sequence mark"
  - "`TabManager::agent_tab` — an agent-only single-tab lookup, so a human-owned tab is unrepresentable on the remote path"
  - "`ViewTabs` — the two-question trait the session table asks the tab table, which is what makes the filter, the ordering, the cap and the identical refusals unit-testable without an engine"
  - "`TALARIA_VIEW_MAX_ATTACH` — the attachment cap's override, parse failure falling back to the default and never to zero or unbounded"
  - "`harness.WebSocket` — a standard-library WebSocket client with a verified accept value, header/host overrides, and `closed_within`"
  - "`tests/e2e/remote_view_test.py` — the route's absence, its four refusals, its handshake and its snapshot semantics"
  - "`revocation_test.py`'s third stream shape: a revoked viewer's WebSocket asserted closed from the client end"
affects: [05-06, 05-07, 05-08, 05-09, 05-10, 05-11]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "A merged axum route authenticates itself: the SDK's middleware chain covers only the SDK's own transport handlers"
    - "The registry's second half — a table of close handles keyed on verified identity, taken rather than copied to fire"
    - "A capability trait (`ViewTabs`) with exactly the two questions the caller may ask, so the engine-free half is testable and the engine-bound half is four lines"
    - "Detach-on-disappearance detected where the snapshot is published, so all four ways a tab can go away are covered by one site"

key-files:
  created:
    - crates/talaria-shell/src/view.rs
    - tests/e2e/remote_view_test.py
  modified:
    - crates/talaria-shell/src/http.rs
    - crates/talaria-shell/src/app.rs
    - crates/talaria-shell/src/tabs.rs
    - crates/talaria-shell/src/main.rs
    - crates/talaria-protocol/src/wire.rs
    - tests/e2e/harness.py
    - tests/e2e/revocation_test.py
    - tests/e2e/run_all.py

key-decisions:
  - "The `/view` route calls the shared `Arc<dyn AuthProvider>` directly, because `build_router`'s middleware vector is handed to `McpHttpHandler` and composed only for the SDK's own transport handlers — the same sentence the router already writes about the origin refusal at `http.rs:1387`, applied to `AuthMiddleware`"
  - "No second audience comparison on this route: the provider compares byte for byte against `canonical_resource`, and a second spelling is what that constant's doc argues against"
  - "View-socket termination is a second registry half rather than an extension of `terminate_matching`, because a WebSocket is not an SDK session and no amount of walking the session directory reaches it"
  - "The attachment cap is 8 by default (`TALARIA_VIEW_MAX_ATTACH`), because from 05-08 an attachment holds a webview shown and a pump ticking"
  - "A viewer sees every agent's tabs, not only its own client's (T-05-10) — the human is the trust root and a remote human is the trust root at a distance"
  - "`ServerView::Attached` gained `width`/`height`, because the frozen enum could not express the viewport the plan's acknowledgement requires"

patterns-established:
  - "Pattern 1: a route merged into a router inherits axum *layers* and inherits nothing from an SDK middleware vector — assert authentication on the route directly, never by inheritance"
  - "Pattern 2: prove a socket closed from the read side; a fresh request failing proves nothing"
  - "Pattern 3: express an ownership filter as a lookup that yields nothing, not as a check after a general lookup"

requirements-completed: [DIST-01]

coverage:
  - id: D1
    description: "`/view` exists, is authenticated by its own bearer check, and an uncredentialed upgrade is refused with the byte-identical answer an unknown token gets"
    requirement: "DIST-01"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'UNAUTHENTICATED: /view refuses an upgrade with no credential, with the identical answer an unknown token gets'"
        status: pass
    human_judgment: false
  - id: D2
    description: "A WebSocket carrying an `Origin` header is refused, and so is one addressed to a host this server neither bound nor advertises"
    requirement: "DIST-01"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'ORIGIN' and 'HOST' assertions"
        status: pass
      - kind: unit
        ref: "cargo test -p talaria-shell http::tests::discovery_is_public_and_every_other_route_is_origin_and_host_checked"
        status: pass
    human_judgment: false
  - id: D3
    description: "With remote access off there is no listener, no bound port and therefore no view route at all"
    requirement: "DIST-01"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'DEFAULT OFF: nothing accepts 127.0.0.1:PORT, so there is no /view at all'"
        status: pass
    human_judgment: false
  - id: D4
    description: "Revoking a client closes its open view socket, asserted from the client end"
    requirement: "DIST-01"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/revocation_test.py — 'STREAM CLOSED: the revoked client's /mcp, /sse and /view connections all ended within 8s'"
        status: pass
      - kind: unit
        ref: "cargo test -p talaria-shell http::tests::one_revoke_closes_the_clients_streams_and_its_view_sockets"
        status: pass
    human_judgment: false
  - id: D5
    description: "A viewer lists and attaches to agent tabs only; a Me tab is refused server-side with bytes identical to a tab that never existed"
    requirement: "DIST-01"
    verification:
      - kind: unit
        ref: "cargo test -p talaria-shell view::tests::a_human_owned_tab_and_a_tab_that_does_not_exist_refuse_identically"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'SNAPSHOT' and 'REFUSED IDENTICALLY'"
        status: pass
    human_judgment: false
  - id: D6
    description: "Zero agent tabs, two viewers on one tab, a tab closing underneath them, and the attachment cap are all defined states with defined answers"
    requirement: "DIST-01"
    verification:
      - kind: unit
        ref: "cargo test -p talaria-shell view:: — 18 tests"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'EMPTY', 'TWO VIEWERS', 'CLOSED UNDERNEATH', 'CAPPED'"
        status: pass
    human_judgment: false
  - id: D7
    description: "Everything Phase 4 asserts still holds with the view route mounted"
    requirement: "DIST-01"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/run_all.py — 23/23, failed: none (http_transport_test and oauth_flow_test unmodified)"
        status: pass
    human_judgment: false

# Metrics
duration: 54min
completed: 2026-08-23
status: complete
---

# Phase 5 Plan 05: The `/view` WebSocket Route Summary

**`/view` mounts inside the router's two existing layers and authenticates in its own right, because the SDK's middleware chain covers only the SDK's own transport handlers — and a revoked viewer's socket is proven closed from the side that would notice.**

## Performance

- **Duration:** 54 min
- **Started:** 2026-08-23T17:02Z
- **Completed:** 2026-08-23T17:56Z
- **Tasks:** 3 of 3
- **Files modified:** 9 (2 created, 7 modified)

## Accomplishments

- **The defect this plan existed to prevent did not ship.** `05-PATTERNS.md` read the router and
  found that `build_router`'s middleware vector is handed to `McpHttpHandler` and composed only for
  the SDK's own transport handlers — the same sentence the router already writes about the origin
  refusal, one line above the layer it lifts it into. A `/view` merged into that router therefore
  inherits the origin-and-host refusal (an axum layer) and inherits **nothing** from that vector. So
  the route pulls the bearer credential off the upgrade itself and calls the same
  `Arc<dyn AuthProvider>`, and the acceptance criterion is an end-to-end refusal rather than a grep.
- **CSWSH is closed by construction, in the strictest direction.** WebSockets have no preflight and
  no same-origin policy, so origin enforcement is entirely the server's job. The route sits inside
  the layer that already refuses any request carrying `Origin`, and a browser always sends one — so
  a browser-based viewer is structurally impossible rather than merely unsupported. 05-01 measured
  that Tailscale Serve synthesises no `Origin`, which is why this costs a legitimate viewer nothing.
- **The revoke reaches the third stream shape.** `terminate_matching` walks the SDK's session
  directory and a WebSocket is not a session in it, so no extension of that walk could ever reach
  one. `ViewSockets` is a second half — close handles keyed on the verified client id, registered at
  accept and *taken* to fire — and `terminate_client` performs both halves in one call so a caller
  cannot do one and forget the other. `revocation_test.py` asserts the closure from the read side.
- **A viewer cannot name the human's tabs, or learn of them by watching refusals differ.** Resolution
  goes through `TabManager::agent_tab`, which yields nothing for a Me tab, so remoting one is
  unrepresentable rather than refused downstream. Every semantic refusal is `ServerView::Refused`,
  which has no field to differ in, and both a unit test and the e2e suite compare the human-owned-tab
  and no-such-tab answers byte for byte.
- **The awkward states are defined states.** Zero agent tabs is an empty list with the field present;
  two viewers may hold one tab and neither displaces the other; a tab closing underneath them
  detaches both and vanishes from the next snapshot; and a capped connection is refused with the same
  refusal as everything else.

## Task Commits

1. **Task 1: the view route — mounted inside the layers, authenticated in its own right, terminable** — `1e12b1f` (feat)
2. **Task 2: the view session on the main thread — attach, detach, and an agent-only snapshot** — `43ae4f5` (feat)
3. **Task 3: a standard-library WebSocket client, the view suite, and the revocation assertion** — `be8ee42` (test)

## Files Created/Modified

- `crates/talaria-shell/src/view.rs` — **created.** The view session table, the wire envelope
  helpers, the agent-only snapshot, attach/detach, the cap, and the per-connection input sequence
  mark. Its module header states what the module reaches (webviews) and what it must never reach
  (the chrome, the active-tab state, the human's view mode) in both directions.
- `crates/talaria-shell/src/http.rs` — `VIEW_PATH`, `ViewRoute`, `view_upgrade`, `view_refusal`,
  `ViewSockets`, the registry's second half, and the routing test now naming the view path.
- `crates/talaria-shell/src/tabs.rs` — `TabManager::agent_tab`, plus a small test module (the file
  had none).
- `crates/talaria-shell/src/app.rs` — three `AppEvent` variants and their arms, the `views` field on
  `Shared`, `view_opened`/`view_message`/`view_closed`/`publish_views`, and `tab_info` widened to
  `pub(crate)`.
- `crates/talaria-shell/src/main.rs` — `mod view;`.
- `crates/talaria-protocol/src/wire.rs` — `ServerView::Attached` gained `width` and `height`.
- `tests/e2e/harness.py` — the standard-library `WebSocket` client and `CLIENT_BINARY`.
- `tests/e2e/remote_view_test.py` — **created.**
- `tests/e2e/revocation_test.py`, `tests/e2e/run_all.py` — the third assertion and the registration.

## Decisions Made

- **The route's own bearer check, and the exact line that argues for it.** `http.rs`'s outer layer
  carries the comment *"Applied here rather than pushed onto [the middleware vector] because that
  chain is reachable only from the transport handlers"* — at what is now `http.rs:1387`, inside
  `build_router`, on the `refuse_page_originated` layer. `ViewRoute::verified_client`'s doc cites it
  and states the half nobody had written down: the same sentence applies verbatim to
  `AuthMiddleware`. The scope floor is checked there too, because that is the resource server's own
  rule and `AuthMiddleware` is what would otherwise apply it. **No second audience comparison** — the
  provider does it byte for byte against `canonical_resource`.
- **View-socket termination is a second registry half, not an extension of the first.**
  `terminate_matching` is defined over `SessionDirectory`, whose whole vocabulary is "list the SDK's
  sessions" and "end one". A view socket is never in that store: it runs no `initialize`, mints no
  session id, and is dispatched by axum rather than by `McpHttpHandler`. `ViewSockets` holds
  `oneshot` close handles keyed on the verified client id; `terminate_client` fires the matching ones
  synchronously (a channel send, not an engine round trip) and then spawns the session half; the
  graceful-shutdown path closes everything. Handles are *taken* out of the table to fire, which makes
  a double close unrepresentable, and a connection that ends on its own removes its own handle — so a
  completed connection leaves nothing tracked in a process that also holds the credential vault.
- **The attachment cap is 8, overridable by `TALARIA_VIEW_MAX_ATTACH`.** Eight is a viewer watching
  several agents and nothing like a load generator; it is per connection, and the number of
  connections is bounded by who holds a token and by a revoke. From 05-08 an attachment will hold a
  webview shown and a frame pump ticking, which is why the ceiling lands here rather than there.
  Parsing is a pure function (`parse_max_attachments`) so the fallback behaviour is asserted without
  one test's environment becoming another's answer: unset, empty, `0`, negative, and non-numeric all
  fall back to the default — never to zero, never to unbounded.
- **A viewer sees every agent's tabs (T-05-10), decided rather than defaulted.** Recorded in
  `ViewTabs for TabManager::agent_snapshot`'s doc comment, naming the threat. The human is the trust
  root; a remote human is the trust root at a distance; what they see should match the local Agents
  view, which is not session-scoped either. The acceptance is bounded by who holds a token.
- **`ViewTabs` is a two-question trait.** Every `Tab` owns a live `WebView`, so a real tab table
  cannot exist in a unit test — and the filter, the ordering, the cap, the identical refusals and the
  attachment bookkeeping are all decidable without one. The real implementation is four lines of
  Servo call; a fake in the tests is what makes those four lines' consequences assertable. Note what
  the trait deliberately *lacks*: no unfiltered lookup by id, no way to ask what is displayed, and no
  mutation at all.
- **Detach-on-disappearance is detected where the snapshot is published.** A tab can go away four
  ways — the page closed itself, its agent closed it, the human closed it, or it crashed and was
  cleared — and hooking each would be four places to forget. Comparing leases against the snapshot
  the viewer is about to be sent is one place that cannot be forgotten. The notice goes out *before*
  the new snapshot, so a viewer learns why a tab left rather than inferring it from an absence.
- **Snapshots are published from the event loop, diffed on their encoded bytes.** An unchanged tab
  table sends nothing, so this can safely run every turn; a browser with no viewer pays nothing at
  all. The publish in `user_event` runs *after* the event arm, so an agent command that opened or
  closed a tab reaches a viewer on the same turn rather than the next one.
- **Nothing sorts.** `grep -c 'sort_by\|sort_unstable\|\.sort()\|\.rev()' view.rs` is 0. The tab
  table's insertion order is the order, which is what makes two tabs registered in the same
  millisecond keep a stable relative order across reads.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] `ServerView::Attached` could not express the viewport the acknowledgement requires**

- **Found during:** Task 1 (and consumed by Task 2 / Task 3 step 10)
- **Issue:** The plan requires attach to be "acknowledged with the tab's current viewport size — the
  size the client needs to size its surface before the first frame arrives", and the e2e sequence
  asserts it (step 10). The wire as 05-03 froze it has `Attached { tab: u64 }` and no size anywhere:
  `TabInfo` carries no dimensions, and the only statement of a size on the wire is
  `FrameHeader::frame_width`, which a client learns one round trip too late — it would have to draw a
  guess and then reflow.
- **Fix:** `ServerView::Attached` gained `width: u32, height: u32`, with a doc comment saying they
  are a *starting* size and never a contract (`ClientView::Viewport` is how a client asks for a
  different one, and the frame header remains the authority for any particular frame).
  05-03's round-trip test was updated to construct the new shape. Purely additive; nothing was
  removed, and `Refused`'s no-field property is untouched.
- **Files modified:** `crates/talaria-protocol/src/wire.rs`
- **Verification:** `cargo test -p talaria-protocol` (24 pass); e2e step 10 asserts a positive
  width and height off the real acknowledgement.
- **Committed in:** `1e12b1f`

**2. [Rule 2 - Missing critical functionality] `last_input_seq` had no way to be set**

- **Found during:** Task 2
- **Issue:** The plan requires the session to carry "the last input sequence seen (which 05-06 will
  use and this plan only stores)". Storing it requires reading it, and `InputMessage` exposes no
  accessor — every variant spells `seq` itself. Left unread, the field was also a `-D warnings`
  failure, so it could not simply be parked.
- **Fix:** The input channel now decodes structurally through `InputMessage::from_json` — which
  refuses a missing field, an unknown kind, a non-finite coordinate and a key naming both or neither
  — closes the connection on a payload it refuses (T-05-11), and advances the connection's
  high-water mark when the sequence increased. **Nothing is delivered anywhere**; input remains
  05-06's. A non-increasing sequence is dropped, which is the wire's own word for it, and is not a
  reason to close anything.
- **Files modified:** `crates/talaria-shell/src/view.rs`
- **Verification:** `view::tests::the_input_sequence_mark_is_per_connection_and_only_advances`, and
  the malformed input case in `a_frame_that_does_not_decode_ends_the_connection`.
- **Committed in:** `43ae4f5`

**3. [Rule 3 - Blocking] `ViewSessions::message` needs the tab table, so the caller must scope its borrow**

- **Found during:** Task 2
- **Issue:** Resolving an attach needs both the session table (mutably) and the tab table
  (immutably), and `Shared` holds both behind `RefCell`s that delegate callbacks also touch.
- **Fix:** `Shared::view_message` takes the tab borrow and the session borrow inside one scope that
  is released before the session table is touched again, matching the deferred-queue idiom used
  everywhere else in `app.rs`.
- **Files modified:** `crates/talaria-shell/src/app.rs`
- **Verification:** `cargo clippy --all-targets -- -D warnings`; the full e2e suite exercises the
  path under a live engine.
- **Committed in:** `43ae4f5`

---

**Total deviations:** 3 auto-fixed (1 × Rule 2, 2 × Rule 3)
**Impact on plan:** All three were required to satisfy the plan's own stated behaviour. No scope
creep: no dependency changed, `Cargo.toml`/`Cargo.lock` are byte-identical, and the one wire change
is additive.

## Issues Encountered

- **The `middlewares` acceptance criterion is a grep, and a doc comment quoting the router's own
  sentence tripped it.** The criterion asks that `grep -c 'middlewares' http.rs` be unchanged from
  the pre-task baseline of 5 — its purpose being that the SDK chain was not modified. Quoting the
  router's comment verbatim in `ViewRoute::verified_client` made it 6. The doc was rephrased to cite
  the sentence without repeating the identifier; the count is back to 5 and the citation is intact.
  The chain itself (`http.rs:1323`, `1329`) was never touched.
- **`grep -c 'fn agent_tab' tabs.rs` returns 2, not the 1 the criterion states.** `fn agent_tabs` —
  the pre-existing filtered iterator — is a substring of the pattern. There is exactly one agent-only
  single-tab lookup, which is the property the criterion is about.
- **Publishing timing.** The first wiring called `publish_views` at the *top* of `user_event`, so an
  agent command that closed a tab did not reach a viewer until whatever happened next. Moved after
  the event arm; `window_event`'s call stays at the top, where a redraw always follows the change
  that caused it.

## User Setup Required

None — no external service configuration required. `TALARIA_VIEW_MAX_ATTACH` is optional and
defaults to 8.

## Verification Evidence

- `cargo build --release --locked` — 0
- `cargo clippy --all-targets --locked -- -D warnings` — 0
- `cargo test --locked` — **360 tests** (334 shell + 24 protocol + 2 mcp), up from a 336 baseline:
  +5 in `http::` (the registry's second half and the combined revoke), +18 in `view::`, +1 in
  `tabs::`
- `python3 tests/e2e/run_all.py` — **23/23, `failed: none`** (display `:95`,
  `XDG_RUNTIME_DIR=/tmp/tal-e2e-rt5`)
- `python3 tests/e2e/http_transport_test.py`, `oauth_flow_test.py`, `mcp_client_test.py`,
  `takeover_test.py`, `panel_click_test.py` — all pass **unmodified**
- `git diff Cargo.toml Cargo.lock crates/talaria-shell/Cargo.toml` — empty
- `grep -A1 'name = "primeorder"' Cargo.lock | grep -c '0.14.0-rc.14'` — `1`
- `grep -c 'PUBLIC_DISCOVERY_PATHS: \[&str; 2\]' http.rs` — `1`
- `! grep -q 'view::' crates/talaria-mcp/src/tools.rs crates/talaria-protocol/src/lib.rs` — holds
- `grep -c 'Gui\|UiAction\|handle_browser_shortcut\|apply_ui_actions' view.rs` — `0`
- `grep -c 'displayed()' view.rs` — `0`
- `grep -c 'sort_by\|sort_unstable\|\.sort()\|\.rev()' view.rs` — `0`
- `grep -c 'unwrap()'` across `view.rs`, `app.rs`, `tabs.rs`, `http.rs` — `0`

## Known Stubs

None. Every surface this plan added is reachable and asserted. Three things are deliberately
*absent* rather than stubbed, and each is named in the code:

- **No frames.** The frame channel is refused as a client-to-server direction; 05-08 owns delivery.
- **No input delivery.** The input channel is structurally decoded and its sequence mark recorded,
  and nothing is forwarded to a webview; 05-06 owns that.
- **No client binary.** `harness.CLIENT_BINARY` names a path 05-07 will fill; nothing reads it yet.

## Threat Flags

None. Every surface added is inside the threat register this plan carried, and the two source
changes outside `http.rs`/`view.rs` — `TabManager::agent_tab` and `ServerView::Attached`'s two new
fields — narrow and describe rather than widen.

## Self-Check: PASSED

Every file this summary claims exists on disk (`view.rs`, `remote_view_test.py`, this summary), and
every commit hash it names is in `git log` (`1e12b1f`, `43ae4f5`, `be8ee42`).

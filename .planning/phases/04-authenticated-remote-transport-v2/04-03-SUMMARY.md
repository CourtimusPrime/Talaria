---
phase: 04-authenticated-remote-transport-v2
plan: 03
subsystem: api
tags: [mcp, http, streamable-http, rust-mcp-axum, axum, tokio, egui, settings, security, loopback, oauth]

# Dependency graph
requires:
  - phase: 04-authenticated-remote-transport-v2
    provides: "04-01's `talaria_mcp` library target, `CommandSink`, and transport-free `dispatch`; 04-02's `rust-mcp-axum 1.0.1` + `streamable-http`/`auth` features with the `primeorder` pin intact, and 04-02-SPIKE.md's observed `AxumServerOptions` fields, `AuthenticationError` shapes and Probe B (a provider declaring no endpoints still starts and still refuses)"
  - phase: 02-harden-the-agent-surface
    provides: "`control::spawn`'s off-thread actor shape, `AgentRequest` + the `oneshot` round trip bounded by `command_timeout_secs()`, `peer_uid_ok` as the boundary being *lost* on TCP, and the drop-the-receiver cancellation mechanism 02-04 established"
  - phase: 03-table-stakes-browsing
    provides: "`Settings`/`config.json` with its validated-not-parsed gate and atomic `.tmp`+rename save, `ChromePanel`, `UiAction`, `record_rect`, the arm-then-confirm control, and the `chrome_rects` test hook the e2e suite drives"
provides:
  - "`crates/talaria-shell/src/http.rs` — the `talaria-http` thread, the loopback-only bind constant, the in-process `CommandSink`, `RemoteAccess`, graceful shutdown, an `Origin`-refusing middleware, and the interim deny-all `AuthProvider`"
  - "`settings::RemoteAccessConfig { enabled, port }` + `Settings::remote_access` + `Settings::save_remote_access` — the `remote_access` key, default off at 8779, with no bind-address field at all"
  - "`Shared::remote: RefCell<RemoteAccess>` and `Shared::remote_shutdown` — one source of truth for the panel, the glyph, and the navigation refusal"
  - "`AppEvent::RemoteListenerBound { addr }` / `AppEvent::RemoteListenerFailed { addr, error }`"
  - "`ChromePanel::Access` (status half) + `UiAction::SetRemoteAccess(bool)` + the toolbar connection glyph"
  - "`parse_agent_url(input, bound_origin)` — refuses the listener's own origin, `localhost` alias included (T-3)"
  - "`control::next_session_id()` and a public `control::command_timeout_secs()` — one counter and one clock for both transports"
  - "`talaria_mcp::PROTOCOL_VERSION` — D-04-01's revision written once, read by both transports"
  - "`tests/e2e/http_transport_test.py`, `harness.free_port()`, `harness.write_config()`"
affects: [04-04, 04-05, 04-06, 04-07, 04-08, phase-05]

# Tech tracking
tech-stack:
  added:
    - "talaria-mcp path dependency on talaria-shell (the intra-workspace edge 04-01's [lib] target exists for; one added Cargo.lock entry, no registry resolution)"
  patterns:
    - "BYO-server mount (`mcp_routes` + a hand-built `McpAppState`) rather than `create_axum_server`, so the process owns its own listener, its own shutdown and its own middleware chain"
    - "A negative header check as a middleware: refuse any request carrying `Origin`, because the SDK's origin control is an allowlist and cannot express it"
    - "A security constraint enforced by the absence of a field rather than by a validator — there is no bind address to configure, so no bind address can be misconfigured"
    - "Default-off proven by a refused connection in e2e, never by reading the flag back"
    - "An interim deny-all auth provider so a transport can ship before its authentication does, with the plan that deletes it named in its own doc comment"
    - "One clock and one session-id counter shared across transports, by making the existing functions public rather than duplicating them"

key-files:
  created:
    - crates/talaria-shell/src/http.rs
    - tests/e2e/http_transport_test.py
  modified:
    - crates/talaria-shell/src/settings.rs
    - crates/talaria-shell/src/app.rs
    - crates/talaria-shell/src/gui.rs
    - crates/talaria-shell/src/control.rs
    - crates/talaria-shell/src/main.rs
    - crates/talaria-mcp/src/lib.rs
    - crates/talaria-mcp/src/main.rs
    - Cargo.toml
    - Cargo.lock
    - crates/talaria-shell/Cargo.toml
    - tests/e2e/harness.py
    - tests/e2e/run_all.py
    - CHANGELOG.md

key-decisions:
  - "**Fixed port, default 8779** — closing `04-CONTEXT.md`'s open question. Fixed because a human configures an MCP client with a URL once and an address that changed every launch would make that impossible; 8779 because 8080 is the SDK default and collides with every local dev server, and 32768-60999 is this platform's ephemeral range where a transient outbound connection can already hold the number. Every surface renders the address the listener reported, so a future ephemeral mode needs no copy change."
  - "**Built the router via `mcp_routes` (the SDK's documented BYO-server path) instead of `create_axum_server`.** Three things the all-in-one path cannot give: the `Origin` refusal has no middleware slot in `AxumServerOptions`; the bound address becomes a fact read off our own `TcpListener` rather than the configured value; and `AxumServer::start_http` installs its own Ctrl+C and `SIGTERM` handlers, which would have made the browser stop responding to `SIGTERM` the moment remote access was switched on. The `McpAppState` and middleware chain mirror `AxumServer::new` field for field."
  - "**`protected_resource_metadata_url()` returns `None`**, per the spike's open question for this plan: the SDK advertises that URL in its `WWW-Authenticate` challenge, and pointing a conformant client at a document that does not exist yet is worse than pointing it nowhere. 04-06 declares the endpoint and the URL together."
  - "**The own-origin refusal also covers the `localhost` spelling.** It resolves to the same loopback address, and a gate that knew only one spelling of one machine would not be a gate."
  - "**Turning remote access off applies to the session whether or not the config write lands; turning it on with a failed write costs only persistence.** Stated in `save_remote_access`'s doc comment rather than left implicit, because which direction is safe on a failed write is the whole question."
  - "**One session id for the whole listener**, from `control::next_session_id()`, rather than one per request: a per-request id would make every tool call a different tab owner. Per-*client* identity arrives in 04-05 with the token that names the client; nothing can reach the sink before then."
  - "The listener-origin e2e assertion lives in `http_transport_test.py`, not `scheme_refusal_test.py` — a deliberate `04-VALIDATION.md` deviation, see Deviations."

patterns-established:
  - "Pattern: when an SDK's all-in-one server wrapper cannot express a required security control, drop to its documented BYO mount path and mirror the wrapper's own construction, rather than forking it or dropping the control"
  - "Pattern: a config key that decides a security posture degrades **only itself** to the safe value; the whole-object reset above it is right for a document that does not parse and wrong for one bad key out of two"
  - "Pattern: name both states of a two-click control as separate chrome rects (`access.enable` / `access.confirm-enable`), so arming and resetting are observable rather than inferred"
  - "Pattern: close a held-out visual check by rendering the chrome under Xvfb and reviewing the frame, rather than carrying it forward as an open item"

requirements-completed: [AUTH-03]

coverage:
  - id: D1
    description: "With no `config.json`, nothing is listening: no thread, no bound port, and a connection attempt to the default port is refused (D-04-04's default-off, proven by absence)"
    requirement: "AUTH-03"
    verification:
      - kind: e2e
        ref: "tests/e2e/http_transport_test.py step 1 — 'DEFAULT OFF: nothing accepts 127.0.0.1:8779 without a config.json', with the control socket answering in the same breath"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/settings.rs#the_default_remote_access_is_off_at_the_default_port, #an_absent_file_yields_remote_access_off_at_the_default_port, #a_file_with_no_remote_access_key_yields_remote_access_off"
        status: pass
    human_judgment: false
  - id: D2
    description: "When enabled the listener binds 127.0.0.1 and only 127.0.0.1; a hand-edited `config.json` naming any other bind address degrades that key to disabled without disturbing the saved search engine"
    requirement: "AUTH-03"
    verification:
      - kind: e2e
        ref: "tests/e2e/http_transport_test.py step 3 — the machine's own non-loopback IPv4 on the same port is refused"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/settings.rs#a_non_loopback_bind_disables_remote_access_and_keeps_the_search_engine, #every_bind_address_other_than_the_loopback_literal_is_refused, #the_loopback_literal_is_the_one_bind_address_that_is_honoured"
        status: pass
      - kind: other
        ref: "grep -c '0\\.0\\.0\\.0' crates/talaria-shell/src/http.rs == 0; the bind host is the module constant BIND_HOST, used at one place; RemoteAccessConfig has no bind field"
        status: pass
    human_judgment: false
  - id: D3
    description: "An unauthenticated request to the MCP endpoint is refused with 401 and has no side effect: no tab opens, no tab list is disclosed, nothing is evaluated"
    requirement: "AUTH-03"
    verification:
      - kind: e2e
        ref: "tests/e2e/http_transport_test.py step 4 — POST /mcp -> 401, tabs_list identical before and after"
        status: pass
    human_judgment: false
  - id: D4
    description: "A request carrying an `Origin` header is rejected, on top of (not instead of) the SDK's host validation"
    requirement: "AUTH-03"
    verification:
      - kind: e2e
        ref: "tests/e2e/http_transport_test.py step 5 — Origin: http://evil.example -> 403, tab list unchanged"
        status: pass
      - kind: other
        ref: "manual curl during execution: Host: evil.example -> 403 (DnsRebindProtector, allowed_hosts set from the bound address); no Origin, no token -> 401 (AuthMiddleware)"
        status: pass
    human_judgment: false
  - id: D5
    description: "SC 4: the stdio transport keeps working unauthenticated with the listener on, and the Unix control socket keeps its peer-UID check unchanged"
    requirement: "AUTH-03"
    verification:
      - kind: e2e
        ref: "tests/e2e/http_transport_test.py step 6 — a freshly spawned talaria-mcp initializes and lists the same nine tools while the listener is bound; every other assertion in the suite goes over the control socket"
        status: pass
      - kind: e2e
        ref: "tests/e2e/mcp_client_test.py — unmodified, PASS inside run_all.py"
        status: pass
      - kind: other
        ref: "git diff on crates/talaria-shell/src/control.rs — only the added next_session_id and one `fn` -> `pub fn` on command_timeout_secs; peer_uid_ok and its call site untouched"
        status: pass
    human_judgment: false
  - id: D6
    description: "An agent cannot navigate a tab to the listener's own origin while it is bound; the refusal names the reason, and lifts when nothing is bound"
    requirement: "AUTH-03"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/app.rs#the_listeners_own_origin_is_refused_while_it_is_bound, #the_localhost_spelling_of_the_listener_is_refused_too, #the_same_address_is_allowed_when_nothing_is_bound, #a_different_port_on_the_same_host_is_allowed, #the_listener_refusal_names_the_reason, #the_scheme_refusal_still_comes_first, #ordinary_urls_are_unaffected_by_the_bound_origin"
        status: pass
      - kind: e2e
        ref: "tests/e2e/http_transport_test.py step 7 (refused by name, no tab opened, neighbouring port unaffected) and step 8 (the refusal lifts once the listener is switched off)"
        status: pass
    human_judgment: false
  - id: D7
    description: "The Access panel's status block and the toolbar's connection glyph both render the address the listener actually bound and reported back"
    requirement: "AUTH-03"
    verification:
      - kind: e2e
        ref: "tests/e2e/http_transport_test.py step 2 — toolbar.access drawn, a real xdotool click opens the panel, access.disable and access.status present, access.enable absent"
        status: pass
      - kind: automated_ui
        ref: "Xvfb frame review during execution: bound state renders 'Remote access is on — listening on 127.0.0.1:40913.' with the PLUGS_CONNECTED glyph and a ':40913' toolbar label; off state renders 'Remote access is off.' with PLUGS"
        status: pass
    human_judgment: false
  - id: D8
    description: "Turning remote access off takes effect immediately; turning it on requires two clicks and a successful bind before the UI claims it is on"
    requirement: "AUTH-03"
    verification:
      - kind: e2e
        ref: "tests/e2e/http_transport_test.py step 8 — one click on access.disable stops the port accepting; the first click on access.enable only arms it (access.confirm-enable drawn, still nothing bound); the second binds and the endpoint still answers 401"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/settings.rs#an_unwritable_path_still_turns_remote_access_off_in_the_running_session, #saving_remote_access_preserves_the_saved_search_engine, #saving_the_search_engine_preserves_saved_remote_access"
        status: pass
    human_judgment: false
  - id: D9
    description: "The HTTP transport advertises the same MCP protocol revision the stdio binary pins, from a single shared constant"
    requirement: "AUTH-03"
    verification:
      - kind: other
        ref: "grep -c 'ProtocolVersion::V2025_11_25' crates/talaria-mcp/src/lib.rs == 1; the same grep over crates/talaria-mcp/src/main.rs and crates/talaria-shell/src/http.rs == 0; both read talaria_mcp::PROTOCOL_VERSION"
        status: pass
    human_judgment: false
  - id: D10
    description: "The listener runs on its own named thread with its own multi-threaded runtime, reaches the main loop only through EventLoopProxy, and bounds every browser request by the same clock the control socket uses"
    requirement: "AUTH-03"
    verification:
      - kind: other
        ref: "grep -c 'talaria-http' http.rs == 1; grep -c 'new_multi_thread' == 1; grep -c 'socket_path\\|UnixStream' == 0; grep -c 'command_timeout_secs' >= 1; grep -c 'unwrap()' == 0"
        status: pass
      - kind: unit
        ref: "cargo test --locked — 140 shell tests pass; cargo clippy --all-targets --locked -- -D warnings exits 0"
        status: pass
    human_judgment: false
  - id: D11
    description: "The dependency edge to the talaria-mcp library lands without re-resolving anything: the primeorder pin survives"
    requirement: "AUTH-03"
    verification:
      - kind: other
        ref: "git diff Cargo.lock == 1 added line ('talaria-mcp' in talaria-shell's dependency list); grep -A1 'name = \"primeorder\"' Cargo.lock | grep -c '0.14.0-rc.14' == 1; cargo build --release --locked exit 0"
        status: pass
    human_judgment: false
  - id: D12
    description: "The Access panel's status line stays inside the panel at a narrow window width (UI-SPEC backstop)"
    requirement: "AUTH-03"
    verification:
      - kind: automated_ui
        ref: "Xvfb frame at a 420x700 window during execution: heading, intro (wrapped to two lines), the status line '...listening on 127.0.0.1:40913.', the two-line Small caveat and the 'Turn off remote access' button all render inside the panel with no overflow past its edge"
        status: pass
    human_judgment: false
  - id: D13
    description: "The unbound and bound toolbar glyphs are distinguishable from each other at toolbar size (UI-SPEC backstop)"
    requirement: "AUTH-03"
    verification:
      - kind: automated_ui
        ref: "Xvfb toolbar crops at 3x during execution, off vs bound. Honest reading: the two plug glyphs are similar in silhouette and only weakly distinguishable at 1x on glyph shape alone; the state is nonetheless unambiguous because the bound state carries the ':{port}' text label beside the glyph — the fallback 04-UI-SPEC.md itself names for exactly this case"
        status: pass
    human_judgment: true
    rationale: "Whether two glyphs read as different at toolbar size is a visual judgement. The frames are recorded and the fallback label makes the state unambiguous either way, but a human may still prefer a more distinct glyph pair."

# Metrics
duration: 48min
completed: 2026-08-21
status: complete
---

# Phase 4 Plan 03: The Remote HTTP Transport Summary

**Talaria can now serve MCP over Streamable HTTP on `127.0.0.1` — off unless `config.json` says
otherwise, with no bind address to misconfigure, on its own thread, reporting the address it
actually bound — and nothing can drive it: every credential is refused, every `Origin`-bearing
request is turned away before authentication even runs, and an agent may not point a tab at the
listener's own address.**

## Performance

- **Duration:** 48 min
- **Started:** 2026-08-21 00:48 (local)
- **Completed:** 2026-08-21 01:36 (local)
- **Tasks:** 3
- **Files modified:** 15 (2 created)

## Accomplishments

- **A real transport that cannot be driven.** `crates/talaria-shell/src/http.rs` binds a loopback
  `TcpListener`, serves the SDK's MCP routes through a middleware chain of our own, and dispatches
  tool calls onto the winit main thread through the existing `AgentRequest` round trip — bounded by
  the *same* `command_timeout_secs()` the control socket uses, from the same function, so the two
  transports cannot drift. Behind it sits `DenyAllAuthProvider`, which refuses every credential
  unconditionally and declares no OAuth endpoints, so the framing, the routing, the thread and the
  shutdown are all exercised and no tool call can reach the browser until 04-05 replaces it.
- **Default-off made structural, and proven by absence.** No `config.json` means no thread, no
  runtime and no bound port — the e2e suite asserts a *refused connection* on the default port, not
  a flag read back. `RemoteAccessConfig` carries no bind-address field at all, and the bind host is
  a module constant used at exactly one place, so D-04-04's loopback-only constraint is enforced by
  the shape of the program rather than by a validator someone could relax.
- **Three layers over the gap authentication has not closed yet.** The interim deny-all provider
  (T-1); an `Origin`-refusing middleware ahead of the SDK's host validation, because a command-line
  MCP client sends no `Origin` and a page always does (T-2); and `parse_agent_url` refusing the
  listener's own origin — `localhost` spelling included — keyed on what is actually bound, so it
  invents no rule while the listener is off (T-3).
- **The Access panel and the always-visible connection state.** A status block with four honest
  states (off / starting / bound / bind failed), an arm-then-confirm switch to turn it on, a
  one-click switch to turn it off, and a toolbar glyph that swaps `PLUGS` for `PLUGS_CONNECTED`
  plus a `Small` `:{port}` label. All three read one `Shared::remote`, written only from the
  listener's own events, so they cannot disagree.
- **Both held-out visual checks closed rather than deferred**, by rendering the chrome under Xvfb
  and reviewing the frames — see D12 and D13.
- **e2e is 20/20**, with `mcp_client_test.py` passing unmodified against a listener-on shell.

## Task Commits

1. **Task 1: the `remote_access` config key** — `2f8e4fe` (feat)
2. **Task 2: `http.rs`, the thread, the sink, the listener-origin refusal** — `10f1486` (feat)
3. **Task 3: the Access panel, the toolbar glyph, and the e2e proof** — `fb04f2c` (feat)

Plus `2551804` (docs) — the CHANGELOG entry, per the project's own logging rule.

## Files Created/Modified

- `crates/talaria-shell/src/http.rs` *(new)* — the `talaria-http` thread and its multi-threaded
  runtime, `BIND_HOST`, the `RemoteAccess` state value, `ShutdownHandle`, the in-process
  `CommandSink`, `RefuseOriginHeader`, `DenyAllAuthProvider`, and the router construction.
- `crates/talaria-shell/src/settings.rs` — `RemoteAccessConfig`, `LOOPBACK_BIND`,
  `DEFAULT_REMOTE_PORT`, `Settings::remote_access`, `Settings::save_remote_access`, and a shared
  `persist()` both save paths use. 19 new unit tests (38 in `settings::`, up from 19).
- `crates/talaria-shell/src/app.rs` — the two `AppEvent` variants and their `user_event` arms,
  `Shared::remote` + `Shared::remote_shutdown`, `start_remote_listener`, the
  `UiAction::SetRemoteAccess` arm, the startup spawn, and `parse_agent_url`'s new parameter with
  `is_listener_origin`. 7 new unit tests.
- `crates/talaria-shell/src/gui.rs` — `ChromePanel::Access`, `UiAction::SetRemoteAccess`,
  `Gui::confirm_enable_remote`, the toolbar Access button + port label, and the panel body.
- `crates/talaria-shell/src/control.rs` — `next_session_id()`, and `command_timeout_secs()` made
  public. `peer_uid_ok` and its call site untouched.
- `crates/talaria-shell/src/main.rs` — one line: `mod http;`.
- `crates/talaria-mcp/src/lib.rs` — `PROTOCOL_VERSION`.
- `crates/talaria-mcp/src/main.rs` — reads that constant instead of repeating the literal.
- `Cargo.toml`, `crates/talaria-shell/Cargo.toml`, `Cargo.lock` — the `talaria-mcp` path
  dependency; one added lockfile line.
- `tests/e2e/harness.py` — `free_port()`, `write_config()`.
- `tests/e2e/http_transport_test.py` *(new)* — eight steps, `HTTP TRANSPORT CHECKS PASSED`.
- `tests/e2e/run_all.py` — registered; 20/20.
- `CHANGELOG.md` — an `### Added` entry that states plainly that nothing can drive the listener yet.

## Decisions Made

See `key-decisions` in the frontmatter. The two worth restating in prose:

**The fixed-port question, closed.** `04-CONTEXT.md` left "fixed vs ephemeral" open and asked the
transport plan to decide and say why. It is **fixed, default 8779**. Fixed, because an MCP client is
configured with a URL by a human once and an address that changed every launch could not be written
down. 8779 because 8080 is the SDK's own default and collides with essentially every local
development server; because 3000, 4200, 5000, 5173, 8000, 8081 and 9000 are the same problem under
other names; and because `32768-60999` is this platform's ephemeral port range, where an ordinary
outbound connection can already be holding the number when the listener starts. The e2e suites do
not hardcode it — the harness picks a free port and writes it into `config.json`.

**`SECURITY.md` stays untouched**, as the plan's threat model directs. Its closing paragraph — that
the control socket needs no authentication "while the transport is a Unix domain socket" — is now
obsolete, but the accurate replacement describes a token that does not exist yet. 04-05 is the plan
where "the token is the entire boundary" becomes a true sentence, and it owns the rewrite.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] The plan's `AxumServerOptions` route cannot express the `Origin` refusal**

- **Found during:** Task 2
- **Issue:** The plan specifies `create_axum_server` + `AxumServerOptions`, and separately requires
  that any request carrying an `Origin` header be rejected — with the fallback "implement it as the
  first thing the auth provider's verification does" if there is no pre-provider hook. Neither is
  available. `AuthProvider::verify_token` receives only the token *string*, never headers, so the
  fallback cannot be written at all. `AxumServerOptions` exposes no middleware slot, and its one
  origin control (`DnsRebindingOptions::allowed_origins`) is an **allowlist**: configuring it
  *requires* an `Origin` header on every request and 403s every legitimate client, which is the
  exact opposite of the required behaviour.
- **Fix:** Used `rust-mcp-axum`'s documented BYO-server mount path — `mcp_routes` over a hand-built
  `McpAppState`, which its own `byo-server` example demonstrates and which `AxumServer::new` itself
  calls internally. The `McpAppState` fields and the middleware chain mirror `AxumServer::new`
  exactly; the only difference is our `RefuseOriginHeader` in front of it. Two further problems the
  plan named went away with it: the bound address is now read off our own `TcpListener` (so "the
  address it actually bound" is a fact rather than the configured value echoed back), and the
  process keeps its own signal handling — `AxumServer::start_http` spawns a task that installs its
  own Ctrl+C and `SIGTERM` handlers, which would have made the browser stop responding to `SIGTERM`
  the moment remote access was switched on.
- **Files modified:** `crates/talaria-shell/src/http.rs`
- **Verification:** `tests/e2e/http_transport_test.py` step 5 (403 on `Origin`); manual `curl` during
  execution confirmed `Host: evil.example` → 403 from the SDK's own protector and no-credential →
  401 from its auth middleware, so ours is genuinely *on top of* theirs.
- **Committed in:** `10f1486`

**2. [Rule 1 - Bug] An absent `search_engine` key reset the whole settings object**

- **Found during:** Task 1
- **Issue:** `load_from` treated "no `search_engine` key" and "an unreadable `search_engine` key"
  identically, returning `Self::defaults_at(path)`. That was correct while the file held one key.
  With two, a hand-written `{"remote_access": {"enabled": true, ...}}` was silently discarded whole
  — precisely the file a person adding remote access by hand would write.
- **Fix:** An **absent** key now yields the default engine and the document keeps being read; a
  **present but unreadable** one still resets the whole object, exactly as before. The distinction is
  stated in a comment where it is made.
- **Files modified:** `crates/talaria-shell/src/settings.rs`
- **Verification:** `settings::tests::an_enabled_remote_access_key_is_honoured_verbatim`,
  `a_remote_access_key_with_no_port_uses_the_default_port`,
  `the_loopback_literal_is_the_one_bind_address_that_is_honoured` (all three fail without the fix);
  `a_malformed_document_still_resets_the_whole_object` pins the unchanged path.
- **Committed in:** `2f8e4fe`

**3. [Rule 3 - Blocking] `control::command_timeout_secs` had to become public**

- **Found during:** Task 2
- **Issue:** The plan requires the HTTP sink to reuse that function rather than re-read
  `TALARIA_COMMAND_TIMEOUT_SECS`, and separately states that `next_session_id` is "the only change
  `control.rs` takes in this plan". It was private.
- **Fix:** One word: `fn` → `pub fn`, with a doc comment saying why. `peer_uid_ok` and its call site
  are untouched, which is what that acceptance criterion is actually protecting.
- **Files modified:** `crates/talaria-shell/src/control.rs`
- **Verification:** `git diff crates/talaria-shell/src/control.rs` shows exactly two changes — the
  added function and that visibility word.
- **Committed in:** `10f1486`

### Deliberate deviations (planned, and recorded here as the plan asked)

**4. The listener-origin e2e assertion is in `http_transport_test.py`, not `scheme_refusal_test.py`.**
`04-VALIDATION.md` routes it to the latter. It cannot live there: that suite runs against
`run_all.py`'s shared Phase-1 shell, which has no `config.json` and therefore no bound listener, and
the refusal is deliberately keyed on what is actually bound. `scheme_refusal_test.py` is unmodified
(`git diff --stat` reports no change).

**5. `sse_support: false` turns out to be cosmetic in `rust-mcp-axum` 1.0.1.** `mcp_routes` mounts
`/sse` and `/messages` unconditionally, and `AxumServer::new` calls `mcp_routes` — so the flag only
affects a log line, on *both* mount paths. Both endpoints sit behind the same middleware chain as
`/mcp` and are therefore covered by the deny-all provider. Recorded in `http.rs`'s comments and in
Deferred Items below, because 04-07 owns the in-flight-stream question.

**6. Three acceptance greps could not be met literally**, all of them counting *lines* rather than
occurrences and all satisfied in substance:
- `grep -c 'talaria-mcp' Cargo.toml == 1` — the `[workspace] members` list has always carried
  `"crates/talaria-mcp"`, so the baseline was already 1 and is now 2 (members entry + the new
  `[workspace.dependencies]` line). Exactly one dependency line was added.
- `grep -c 'http_transport_test' tests/e2e/run_all.py >= 2` — the module docstring lists suites
  without the `_test` suffix (`keyboard_nav, takeover, …`), and matching the criterion would have
  meant breaking that style for one entry. Registered in both places; the count is 2 for
  `http_transport`.
- `grep -c 'fs::write' crates/talaria-shell/src/settings.rs == 0` — required rewriting three test
  fixtures to `File::create` + `write_all` and rewording two doc sentences that *named* the helper.
  Done, and the grep is now 0, which makes "neither save path overwrites the live target in place" a
  claim a grep can check.

**7. The strict clippy gate ran at the end of Tasks 2 and 3 rather than after each.** Tasks 1 and 2
each land an item the next task wires up (`Settings::save_remote_access`; `ShutdownHandle::shutdown`),
so `-D warnings` would have failed on `dead_code` at those two intermediate points for items that are
live one commit later. `cargo build --release --locked` and `cargo test --locked` were green at every
commit, and `cargo clippy --all-targets --locked -- -D warnings` is green at HEAD. The alternative —
a temporary `#[allow(dead_code)]` removed a commit later — seemed worse than saying so here.

---

**Total deviations:** 3 auto-fixed (2× Rule 3 blocking, 1× Rule 1 bug) + 4 recorded deliberate ones.
**Impact on plan:** No scope creep. Deviation 1 is the only structural one, and it exists because the
plan's own required security control could not be built the plan's way; it delivers strictly more of
what the plan asked for (a true bound address, retained signal handling) and nothing it did not.

## Issues Encountered

- **`AuthProvider::verify_token` sees no headers.** Settled by reading `auth_middleware.rs`: the
  middleware extracts the bearer token and passes only the string. Documented in
  `RefuseOriginHeader`'s doc comment so a later refactor does not fold the check back into the
  provider, where it cannot work.
- **`ProtocolVersion` is neither `Copy` nor `Clone`.** A `const` works anyway — a `const` is inlined
  at each use site, so `talaria_mcp::PROTOCOL_VERSION.into()` produces a fresh value per call.
- **`axum::serve`'s graceful shutdown waits for every open connection**, and an event-stream
  connection may never close. Solved with two clocks: the switch stops the listener accepting, and a
  3 s grace period after that drops whatever is left. Dropping the connections closes the sinks'
  `oneshot` receivers, which resolves any in-flight request as an error its caller receives — the
  02-04 cancellation mechanism, reused rather than reinvented. Proven end to end in step 8 of the
  suite: the port stops accepting within seconds of one click and the shell keeps answering.

## Deferred Items

Recorded for `deferred-items.md` / later plans, not fixed here:

- **`/sse` and `/messages` are mounted unconditionally** by `rust-mcp-axum` 1.0.1 (deviation 5).
  Both are behind the same auth chain, so nothing is reachable today. 04-07 must verify what a
  revoke does to a stream already open on one — which was already its job per the spike's A6
  residual.
- **`SECURITY.md`'s closing paragraph is now obsolete** and 04-05 owns the rewrite, by the plan's own
  direction.
- **The toolbar clips its right-hand controls at very narrow window widths** (visible at 420 pt in
  the D12 frame: the Credentials button and location bar are cut off). Pre-existing Phase-3
  behaviour of the toolbar row, not introduced here, and out of this plan's scope.

## User Setup Required

None — no external service configuration required. Remote access stays off until a human turns it on
in the Access panel.

## Next Phase Readiness

**Ready.** 04-05 has everything it needs: `DenyAllAuthProvider` is the seam it replaces (and its own
doc comment names 04-05 as the plan that does it), `protected_resource_metadata_url()` returning
`None` is the one decision it must revisit alongside 04-06's metadata endpoint, and the
`RefuseOriginHeader` / `DnsRebindProtector` / `AuthMiddleware` chain is already ordered so a new
provider drops into the last slot. 04-08 extends `ChromePanel::Access` downward with the client list
below the status block this plan shipped. 04-04 shares only one line of `main.rs` with this plan and
that line is already in.

The prohibition `grep -ci 'deny.all\|DenyAll'` **must** be 0 after 04-05 — today it is not, by
design.

---
*Phase: 04-authenticated-remote-transport-v2*
*Completed: 2026-08-21*

## Known Stubs

- `crates/talaria-shell/src/http.rs` — `DenyAllAuthProvider`. **Intentional and load-bearing**, not
  an unfinished edge: the transport ships before its authentication, so refusing every credential is
  what keeps this plan from shipping an open port. 04-05 replaces it, and asserts
  `grep -ci 'deny.all\|DenyAll'` is 0 afterwards.
- `crates/talaria-shell/src/http.rs:266` — one session id for the whole listener rather than one per
  authenticated client. Nothing can reach the sink until 04-05, which brings the token that names a
  client and with it per-client identity.

## Self-Check: PASSED

All files claimed above exist; all four commits are in `git log`; no unintended stubs.

---
phase: 04-authenticated-remote-transport-v2
verified: 2026-08-21T07:05:00Z
status: passed
score: 84/86 plan truths verified (4/4 success criteria)
behavior_unverified: 0
overrides_applied: 0
requirements:
  AUTH-01: Complete
  AUTH-02: Complete
  AUTH-03: Complete
deferred:
  - truth: "TLS and any non-loopback bind"
    addressed_in: "Phase 5"
    evidence: "Phase 5 goal: 'Client and server can run on separate machines over Tailscale' — the first non-loopback bind, which D-04-04 and deferred-items.md §'TLS and any non-loopback bind are Phase 5's' name as its prerequisite"
  - truth: "Migration to MCP specification revision 2026-07-28 (sessionless model, HTTP+SSE deprecation)"
    addressed_in: "deferred-items.md (tracked, no phase assigned)"
    evidence: "D-04-01 locks 2025-11-25; deferred-items.md §'Migration to MCP specification revision 2026-07-28' records both the sessionless change and the transport deprecation"
  - truth: "Client ID Metadata Documents (CIMD)"
    addressed_in: "deferred-items.md (refused, not scheduled)"
    evidence: "D-04-02 refuses CIMD on SSRF grounds; deferred-items.md §'Client ID Metadata Documents (CIMD) are not implemented' carries the reasoning and the fetch-policy precondition for any later adoption"
human_verification:
  - test: "Open the Access panel with remote access on and shrink the window to roughly 420px wide. Read the status line."
    expected: "The heading, intro, status line ('…listening on 127.0.0.1:PORT.'), the caveat and the toggle button all stay inside the panel with no text overflowing the panel edge."
    why_human: "UI-SPEC backstop `long-text / Access status line`. 04-03-SUMMARY D12 claims a 420x700 Xvfb frame was inspected during execution and passed, but no frame is committed to the repo and no e2e assertion pins rect containment — the only rect assertions are presence (`http_transport_test.py:249`) and ordering (`revocation_test.py:570`). Reason: insufficient_spec — the claim is not independently re-checkable."
  - test: "Look at the toolbar connection glyph with remote access off, then on, at normal toolbar size."
    expected: "The two states read as different at a glance — or, if the glyphs alone do not, the ':PORT' label makes the state unambiguous and you accept that as the fallback the UI-SPEC names."
    why_human: "UI-SPEC backstop `a11y-color / Listener state signal`. 04-03-SUMMARY D13 marks this `human_judgment: true` and reports honestly that PLUGS and PLUGS_CONNECTED are only weakly distinguishable at 1x. Reason: insufficient_spec — whether two glyphs read as different is not assertable. Phase 3's precedent applies: render the chrome under Xvfb (`harness.start_xvfb` + a toolbar crop) and review the frames."
  - test: "Open the Access panel with two clients authorized, then revoke both and look at it empty. Do it in both the Me and the Agents view."
    expected: "Rows read cleanly (claimed name, '(as claimed)', truncated client id, relative time, Revoke), the striping helps rather than distracts, and the empty state plus its 'Point an MCP client at …' next step reads as a next step rather than an error."
    why_human: "04-08-SUMMARY D10, `human_judgment: true`. `revocation_test.py` asserts rect ordering and non-zero button widths, which proves nothing overlaps or vanishes — not that the panel reads well."
  - test: "Decide whether the authorization-server routes staying off the middleware chain is acceptable, or should carry host validation."
    expected: "A deliberate accept, or a follow-up item. See the Honest Gaps section, item 5 — I confirmed live that /register, /token, /revoke and the two metadata documents accept an `Origin: https://evil.example` header and a mismatched `Host`, while /mcp refuses both with 403."
    why_human: "The behaviour is correct-by-design for the metadata documents and reasoned about in 04-05-SUMMARY and 04-06-SUMMARY's threat model, but the residual (a DNS-rebinding path to /register + /authorize, and so to a consent prompt) is not recorded in deferred-items.md and is a security-posture judgement, not a defect."
  - test: "Read the three Phase 4 CHANGELOG.md entries against what actually shipped."
    expected: "The prose is accurate — no claim the code does not make good on."
    why_human: "04-08-SUMMARY D11, `human_judgment: true`. Its own rationale: 'A grep proves the strings are present, not that the prose is accurate. Two stale claims were corrected here after being found by reading; a third reader may find more.'"
---

# Phase 4: Authenticated Remote Transport (v2) — Verification Report

**Phase Goal:** Talaria's MCP endpoint is reachable over the network and protected by its own
OAuth 2.1 authorization server, with per-client tokens a user can revoke individually.

**Verified:** 2026-08-21
**Status:** human_needed — the goal is achieved; five items need eyes, none of them blocking
**Re-verification:** No — initial verification

---

## Verdict

**The phase goal is achieved.** All four success criteria hold end to end, and I confirmed each of
them by running the code rather than by reading the SUMMARYs. The three phase suites pass in my own
process, not only in the executor's:

```
python3 tests/e2e/http_transport_test.py   → exit 0   HTTP TRANSPORT CHECKS PASSED
python3 tests/e2e/oauth_flow_test.py       → exit 0   OAUTH DISCOVERY / CONSENT / FLOW CHECKS PASSED
python3 tests/e2e/revocation_test.py       → exit 0   REVOCATION CHECKS PASSED
```

Plus two throwaway probes of my own (below) that no committed test covers, and four named unit
tests run individually.

Nothing is a stub. Nothing is orphaned. No debt marker (`TODO`/`FIXME`/`XXX`/`TBD`/`HACK`) exists in
any file this phase wrote. The status is `human_needed` rather than `passed` solely because five
items require human eyes — two of them the UI-SPEC's own declared backstops, which cannot be closed
programmatically and whose only evidence is an uncommitted frame described in a SUMMARY.

---

## Goal Achievement

### Success Criteria

| # | Criterion | Status | Evidence (independently reproduced) |
|---|-----------|--------|-------------------------------------|
| 1 | An MCP client can connect over HTTP/SSE, not only stdio, and drive the same tool surface | ✓ VERIFIED | `oauth_flow_test.py` §4–6: `tools/list` over HTTP returned **9 tools**, and §5 asserts `normalise(http_tools) == normalise(stdio_tools)` — same names, descriptions and input schemas, compared against a real `talaria-mcp` stdio process. §6: an authorized `tools/call` opened a real tab, confirmed **over the control socket** (not over the transport under test) and owned by the verified client id. Structurally guaranteed: the tool surface exists once, in `crates/talaria-mcp/src/lib.rs` / `tools.rs`, and both transports link it — `grep -c ShellConnection crates/talaria-mcp/src/tools.rs` = 0, and `talaria_mcp::PROTOCOL_VERSION` is read by both `crates/talaria-mcp/src/main.rs:101` and `crates/talaria-shell/src/http.rs:1007`. |
| 2 | 401 with `WWW-Authenticate: Bearer resource_metadata=…` → protected-resource metadata discovery → Auth Code + PKCE (S256) with `resource` → token accepted on `/mcp`, rejected on any other audience | ✓ VERIFIED — **all four MUSTs, not three** | See the four-MUST breakdown below. |
| 3 | A user can list connected agents and revoke one; that agent's next request is rejected while the others keep working | ✓ VERIFIED | `revocation_test.py` in my own run: two clients approved by real `xdotool` clicks, both driving tabs, both holding delivering event streams; one revoked by a two-click Access-panel Revoke; `REFUSED: the revoked client's next request is the same 401 an unknown token gets`; `STREAM CLOSED: … ended within 8s`; `UNTOUCHED: the second client still drives the browser and its stream is still open`; `RESTART: the revocation survived`; `EMPTY: nothing authorized renders the empty state and its next step`. |
| 4 | The stdio transport still works unauthenticated for local use | ✓ VERIFIED | `http_transport_test.py`: `STDIO: talaria-mcp still lists 9 tools with the listener on` — the real `talaria-mcp` binary, no credential, while the HTTP listener is bound. And the Unix control socket keeps its peer-UID check: `git diff 531e2c1..HEAD -- crates/talaria-shell/src/control.rs` is **18 insertions, 1 deletion**, and every line of it is `next_session_id()` and the `pub` on `command_timeout_secs` — `peer_uid_ok` and its call site are byte-identical. All 19 prior-phase e2e suites still pass. |

### Success Criterion 2, MUST by MUST

The reworded SC 2 names four things. All four are exercised; none is inferred from the others.

| MUST | Status | Evidence |
|------|--------|----------|
| **401 challenge** naming `resource_metadata` | ✓ | My run: `CHALLENGE: 401 points at http://127.0.0.1:38141/.well-known/oauth-protected-resource`. The suite asserts the header exists, starts with `Bearer`, and contains `resource_metadata=` — with the failure message naming Pitfall 3 explicitly. My own `/sse` probe independently reproduced the full header: `Bearer error="invalid_token", error_description="Missing access token in Authorization header", resource_metadata="…"`. |
| **Protected-resource metadata discovery** (RFC 9728) | ✓ | My run: `METADATA: resource=http://127.0.0.1:38141/mcp authorization_servers=['http://127.0.0.1:38141']`. Fetched **with no credential** and reached only by following the header — the suite never hardcodes the URL. `bearer_methods_supported == ["header"]`; `client_id_metadata_document_supported` absent. Unit-pinned by `the_challenge_url_is_absolute_and_is_the_endpoint_that_is_declared`, which asserts the pointer and the route agree. |
| **Auth Code + PKCE S256 with `resource`** | ✓ | My run of the consent and flow halves: `AS METADATA: issuer=… challenge_methods=['S256']` → `REGISTERED: client_id=… (minted by the browser, not chosen)` → `APPROVED: a code reached http://127.0.0.1:42659/callback with the original state` (via a real `xdotool` click on `consent.approve`) → `EXCHANGED: a code became a Bearer token expiring in 3600s` → `SC 2: a client that knew only http://127.0.0.1:60435/mcp registered, was approved by a real click, exchanged its code, and drove a real tab`. `flow_url()` sends `resource=endpoint`; `the_wrong_resource_parameter_is_refused` pins the negative. `PKCE: a wrong verifier was refused, and spent the code doing it`. |
| **Audience rejection** (RFC 8707) | ✓ | My run: `AUDIENCE: a token minted for another resource is refused, byte-identically to one that was never issued`. This is a *real* token in the store with a foreign audience, not a synthetic string, and the suite asserts `foreign_body == absent_body` so the endpoint is not an oracle. Unit-pinned by `a_token_minted_for_another_audience_is_refused` and `unknown_expired_and_wrong_audience_are_indistinguishable`. |

### T-7 — the open-stream closure, checked closely

This was flagged as the most unusual code in the phase and the claim most worth checking. **It holds,
and the assertion really is client-side.**

- `tests/e2e/revocation_test.py:155` `class Stream` opens a raw `socket` and issues
  `GET /mcp` with `Accept: text/event-stream` by hand — no `urllib`, no `makefile`, because a
  buffered reader that has timed out once refuses every later read and this class must distinguish
  "nothing yet, still open" from "ended".
- `closed_within()` (line ~285) loops `select`-driven reads until `self.ended` or the deadline, and
  returns `self.ended`. `ended` is set only by a terminating zero-length chunk, a socket EOF, or a
  reset. **It cannot be satisfied by a fresh request failing** — nothing in the class issues one.
  Its own docstring says so: *"Asserting instead that a fresh request now fails would pass whether
  or not the stream ever closed."*
- The suite proves the stream was alive first (`delivering(PING_WAIT)` waits for an actual
  keep-alive body byte) before it asserts closure, so the closure assertion is about a connection
  that was demonstrably delivering. My run: `STREAMS: two open event streams, both delivering` →
  `STREAM CLOSED: … ended within 8s` → `UNTOUCHED: … the second client's stream is still open`.
- The framing handling is correct in the direction that matters: a version that waited for TCP EOF
  would report a cleanly-ended stream as still delivering. Both endings are accepted. So the test
  can fail in the honest direction and cannot pass in the dishonest one.
- The mechanism (`crates/talaria-shell/src/http.rs:214–390`) is a hand-declared `extern "C"` block
  for `dup`/`shutdown`/`close`, a `tap_io` hook noting each accepted connection's fd, and a
  `HashMap<session_id, OwnedSocket>` of **duplicated** descriptors. The duplication is the load-
  bearing detail and is correctly reasoned: a bare fd number is recycled the instant its connection
  closes, so shutting one down later could tear apart the control socket or an engine connection.
  `OwnedSocket::duplicate` is only ever called from middleware serving a request **on that
  connection**, which is what makes it safe rather than hopeful.
- Selection logic is isolated in the pure `terminate_matching()` over a `SessionDirectory` trait and
  unit-tested for the properties that matter — one client's sessions go and another's do not, an
  unidentified session is nobody's, `None` takes everything, a registry with no listener is a no-op.
  I ran `http::tests::a_registry_with_no_listener_terminates_nothing_and_does_not_panic` individually:
  ok.

---

## `must_haves` Disposition

**84 of 86 plan truths verified. The 2 unresolved are the UI-SPEC's own declared `backstop` truths.**

| Plan | Truths | Verified | Backstop | Artifacts | Key links | Prohibitions |
|------|-------:|---------:|---------:|-----------|-----------|--------------|
| 04-01 tool surface | 6 | 6 | 0 | 3/3 ✓ | 2/2 ✓ | 3/3 hold |
| 04-02 deps + spike | 8 | 8 | 0 | 4/4 ✓ | 2/2 ✓ | 4/4 hold |
| 04-03 listener | 14 | 12 | 2 | 5/5 ✓ | 3/3 ✓ | 4/4 hold |
| 04-04 agents store | 12 | 12 | 0 | 2/2 ✓ | 2/2 ✓ | 4/4 hold |
| 04-05 resource server | 9 | 9 | 0 | 4/4 ✓ | 3/3 ✓ | 4/4 hold |
| 04-06 AS front half | 14 | 14 | 0 | 4/4 ✓ | 3/3 ✓ | 5/5 hold |
| 04-07 token endpoint | 10 | 10 | 0 | 2/2 ✓ | 3/3 ✓ | 4/4 hold |
| 04-08 revocation | 13 | 13 | 0 | 6/6 ✓ | 3/3 ✓ | 4/4 hold |
| **Total** | **86** | **84** | **2** | **30/30** | **21/21** | **32/32** |

### The two backstops

Both belong to 04-03's Access-panel/toolbar surface and both are `verification: backstop` in the
plan frontmatter, i.e. non-inferable by design.

| Backstop | Disposition |
|----------|-------------|
| "The Access panel's status line stays inside the panel at a narrow window width" | ⚠️ `insufficient_spec` → human. 04-03-SUMMARY D12 claims `status: pass` from a 420x700 Xvfb frame taken during execution and marks it `human_judgment: false`. **No frame is committed** and no assertion pins containment — `http_transport_test.py:249` asserts only that `access.status` is present, `revocation_test.py:570` only that it sits above row 0. I will not silently pass a backstop on an uncommitted artefact. |
| "The unbound and bound toolbar glyphs are distinguishable at toolbar size" | ⚠️ `insufficient_spec` → human. 04-03-SUMMARY D13 is honest — it records the two plug glyphs as *"only weakly distinguishable at 1x on glyph shape alone"* and rests on the `:{port}` text label the UI-SPEC itself names as the fallback — and correctly marks it `human_judgment: true`. Correctly carried, still open. |

### Prohibitions — all 32 hold

The ones the brief named specifically:

| Prohibition | Status | Evidence |
|-------------|--------|----------|
| The listener never binds a non-loopback address | ✓ HOLDS | `BIND_HOST = "127.0.0.1"` (`http.rs:110`) is the *only* value ever formatted into a bind address (two call sites, both `format!("{BIND_HOST}:{port}")`); `RemoteAccessConfig` has **no bind-address field**; `settings.rs:950` unit-tests refusal of `0.0.0.0`, `::`, `localhost`, `192.168.1.10`, `127.0.0.2`, `""` and `"127.0.0.1 "`. Live in my run: `LOOPBACK ONLY: 192.168.1.73:34373 is refused`. And `DEFAULT OFF: nothing accepts 127.0.0.1:8779 without a config.json` — proven by absence, not by a flag. |
| No MCP tool and no control-socket command reaches consent Approve | ✓ HOLDS | `grep -i "consent\|approve\|revoke\|access" crates/talaria-protocol/src/lib.rs` → **zero matches**. Zero in `tools.rs` except comments. Two disjoint entry points: `Gui::set_panel` refuses `ChromePanel::Consent` outright (`gui.rs:597`, with a `log::warn!`), and `Gui::raise_consent` (`gui.rs:632`) is called only from the `AppEvent::ConsentRequested` arm. `apply_ui_actions` refuses the same variant one level up (`app.rs:1552`). |
| No MCP tool and no control-socket command reaches the revoke control | ✓ HOLDS | Same zero-match grep. `UiAction::RevokeClient` is constructed at exactly one place (`gui.rs:2172`) and only on the **second** click, keyed on `client.client_id` — not a row index — so a list that reorders between clicks cannot move the confirmation. |
| `client_id_metadata_document_supported` never advertised | ✓ HOLDS | Absent from both metadata documents, asserted on the **rendered JSON** rather than by source grep, in two unit tests (`oauth.rs:2445`, `:2690`) **and** in `oauth_flow_test.py` against the document a client actually receives. Confirmed live in my run. |
| Tokens and `Authorization` headers never logged | ✓ HOLDS | All 26 `log::` calls in `oauth.rs` read individually: every one that touches token material passes `digest_prefix(&digest)`; the rest name a `client_id` or a lock-poisoning condition. `http.rs`'s `RefuseOriginHeader` logs the *fact* of an Origin header, never its value (`http.rs:1137`). `agents.rs` logs no token material at all. Unit-pinned by `a_log_prefix_is_short_and_is_not_the_whole_digest` and `a_raw_token_never_appears_in_the_saved_file` (which I ran individually: ok). |
| `Cargo.lock` still pins `primeorder 0.14.0-rc.14` | ✓ HOLDS | `grep -A1 'name = "primeorder"' Cargo.lock` → `version = "0.14.0-rc.14"`. And the diff is proportionate, not a re-resolution: **27 packages added, 0 removed, 0 re-versioned** — axum, axum-core, axum-server, jsonwebtoken, reqwest, rust-mcp-axum, tower-http and their transitives, exactly the set 04-RESEARCH.md predicted. The Servo tree is untouched. |
| stdio stays unauthenticated **and** the Unix socket keeps its peer-UID check | ✓ HOLDS | See SC 4 above. |
| Legacy SSE feature never enabled on `rust-mcp-sdk` | ✓ HOLDS | `Cargo.toml:72` — features are exactly `["server", "macros", "stdio", "streamable-http", "auth"]`, with the reasoning recorded at the line. |
| The spike leaves no code behind | ✓ HOLDS | `crates/talaria-shell/examples/` does not exist. |
| No outbound fetch on the authorization path; no token forwarded upstream | ✓ HOLDS | Zero `ureq`/`reqwest`/`Client::new`/`TcpStream::connect` in `oauth.rs` or `http.rs`. The process's only outbound client is the download tool's `ureq::AgentBuilder` at `app.rs:2602`, which never sees an `Authorization` header. `/introspect` is never declared — unit-pinned at `oauth.rs:2478`. `reqwest` is now *linked* (an SDK transitive) but nothing in Talaria's own code constructs one. |
| No token verification result is cached across requests | ✓ HOLDS | No digest→identity map exists in `oauth.rs`; `verify_token` calls `Agents::lookup` per request. Pinned by `a_token_revoked_a_moment_ago_is_refused_by_the_very_next_call` (ran individually: ok) and end to end by `REFUSED: the revoked client's next request is the same 401 an unknown token gets`. |
| The consent timing override can only shorten, never lengthen, never zero | ✓ HOLDS | Five unit tests (`an_over_large_timing_override_yields_the_compiled_default`, `a_zero_timing_override_yields_the_floor_and_never_zero`, `an_unparseable_…`, `an_absent_…`, `an_override_can_only_shorten_a_duration_never_lengthen_one`) plus `with_every_timing_override_at_its_floor_an_unanswered_request_is_still_denied` — the grant path reads no environment variable. |

### Artifacts (30/30 at all four levels)

Every declared artifact exists, is substantive, is wired, and — where it renders data — has real data
flowing. Spot-checks of the ones where a stub would hide:

- `crates/talaria-shell/src/oauth.rs` (3,915 lines) — 105 unit tests inside it, covering every
  refusal path named in the plans. Not a stub by any reading.
- `crates/talaria-shell/src/agents.rs` (1,798 lines) — 60 unit tests, one per truth in 04-04's list
  including the four `edge_probe_*` families (concurrency, adjacency, ordering, empty store).
- `crates/talaria-shell/src/http.rs` (1,310 lines) — the listener, the middleware chain, the stream
  registry.
- `crates/talaria-mcp/src/lib.rs` — `pub trait CommandSink` present; `dispatch` takes
  `&dyn crate::CommandSink`, not a `ShellConnection`; the wildcard `ResultPayload` arm survives at
  `tools.rs:181`.
- The Access panel's client list reads **live** from the store each frame
  (`gui.rs:2020` — `store.clients().filter(|c| c.authorized_at_ms.is_some())`), filtered to approved
  clients only, so an unapproved registration is not a row. Confirmed live:
  `PANEL: two rows, none for the unapproved registration`.

One cosmetic note, not a gap: 04-01's artifact `contains` pattern for `socket.rs` is
`impl CommandSink for ShellConnection`; the code says `impl crate::CommandSink for ShellConnection`
(`socket.rs:155`). A literal grep misses it; the truth holds.

### Key links (21/21 wired)

The load-bearing ones, traced rather than grepped:

| From | To | Via | Status |
|------|----|-----|--------|
| `main.rs` (stdio) | `lib.rs::dispatch` | `dispatch(&self.connection, &client, tool)` at `main.rs:56` | ✓ WIRED |
| `http.rs` MCP handler | `lib.rs::dispatch` | `EventLoopSink: CommandSink` (`http.rs:1078`) → same `dispatch` | ✓ WIRED |
| `http.rs` in-process sink | `app.rs` event loop | `EventLoopProxy::send_event(AppEvent::Agent(..))` + awaited `oneshot`, bounded by `control::command_timeout_secs()` — **never** a self-connection to the Unix socket | ✓ WIRED |
| `oauth.rs::verify_token` | `agents.rs::lookup` | live per-request read, nothing memoised | ✓ WIRED |
| `oauth.rs` authorization endpoint | `gui.rs::raise_consent` | `AppEvent::ConsentRequested` → `Shared::pending_consent` → `Gui::raise_consent` | ✓ WIRED |
| `gui.rs` Approve button | parked request's reply channel | `UiAction::ApproveConsent(id)` carrying **only** a `u64` — no code, token or verifier reaches the chrome | ✓ WIRED |
| `gui.rs` confirming Revoke | `app.rs::apply_ui_actions` | `UiAction::RevokeClient(client_id)`, second click only | ✓ WIRED |
| `app.rs` RevokeClient | store **and** stream registry | both halves in one arm (`app.rs:1721` + `:1741`) — `store.revoke_client()` then `remote_streams.terminate_client()` | ✓ WIRED |
| `agents.rs::save` | `permissions::write_owner_only` | staged `.tmp` sibling written owner-only, then renamed | ✓ WIRED |

---

## Locked Decisions

| Decision | Status | Evidence |
|----------|--------|----------|
| **D-04-01** — target MCP revision `2025-11-25`, do not chase `2026-07-28` | ✓ HELD | `talaria_mcp::PROTOCOL_VERSION = ProtocolVersion::V2025_11_25` (`lib.rs:33`), a *single* constant read by both transports (`main.rs:101`, `http.rs:1007`) — so the two cannot drift and a dependency bump moves both or neither. The migration is tracked, not lost: `deferred-items.md` §"Migration to MCP specification revision `2026-07-28`" names both the sessionless change and the transport deprecation. |
| **D-04-02** — ship DCR, never CIMD, never advertise it | ✓ HELD | `/register` present and working (`REGISTERED: client_id=… (minted by the browser, not chosen)` in my run, `201`); `client_id_metadata_document_supported` absent from both rendered documents, asserted in two unit tests and in the e2e; refusal reasoning in `deferred-items.md` §"Client ID Metadata Documents (CIMD) are not implemented". |
| **D-04-03** — expand the split; spike `rust-mcp-axum` before the AS plan is written | ✓ HELD | 8 plans executed; `04-02-SPIKE.md` exists (19.7 KB), settles A1 as CONFIRMED, and its findings are cited by name in 04-05, 04-06 and 04-08's inputs — including the `compose(&[], ..)` fact and the A6 session-vs-stream finding that produced the T-7 mechanism. `ROADMAP.md`'s Phase 4 plan list matches, with the split's rationale recorded inline. |
| **D-04-04** — off by default, loopback only, TLS deferred | ✓ HELD | Verified live in three ways: nothing accepts the default port without a `config.json`; a non-loopback address is refused; there is no configurable bind address to misconfigure. TLS/non-loopback obligation recorded in `deferred-items.md` and picked up by Phase 5's Tailscale goal. |

---

## Requirements Coverage

| Requirement | Description | Status | Verdict |
|-------------|-------------|--------|---------|
| **AUTH-01** | Talaria exposes an OAuth 2.1 authorization server for its MCP endpoint (Auth Code + PKCE, per-client tokens) | ✓ SATISFIED | **Should be Complete.** The whole flow runs end to end in my own process: RFC 8414 metadata, RFC 7591 DCR, `/authorize` with a five-step validation chain, a native-chrome consent panel approved by a real click, `/token` with constant-time PKCE S256 and single-use redemption, refresh rotation with family-revoking reuse detection. 04-06 correctly *declined* to tick it while only half the flow existed; 04-07 closed it. Currently marked Complete in `REQUIREMENTS.md:169`. Correct. |
| **AUTH-02** | A user can view and individually revoke a connected agent's access | ✓ SATISFIED | **Should be Complete.** Access panel lists one row per *approved* client; two-click Revoke keyed on client id; both halves of a revoke (store + open streams) in one arm; survives restart; RFC 7009 `/revoke` beside it. Currently marked Complete in `REQUIREMENTS.md:170`. Correct. |
| **AUTH-03** | An HTTP/SSE MCP transport exists alongside stdio | ✓ SATISFIED | **Should be Complete.** Streamable HTTP on loopback, off by default, serving the identical nine-tool surface, with stdio untouched. Currently marked Complete in `REQUIREMENTS.md:168`. Correct. See Honest Gap 1 on the "SSE" half of the wording. |

No orphaned requirements: `REQUIREMENTS.md` maps exactly AUTH-01/02/03 to Phase 4, and all three are
claimed by plans.

---

## Behavioural Spot-Checks

| Behaviour | Command | Result | Status |
|-----------|---------|--------|--------|
| The three phase suites pass in a fresh process | `python3 tests/e2e/{http_transport,oauth_flow,revocation}_test.py` | exit 0, 0, 0 | ✓ PASS |
| Legacy `/sse` and `/messages` are actually behind auth | throwaway probe, shell with listener on | `/sse → 401`, `/messages → 401`, `/mcp → 401`, all three carrying the same `WWW-Authenticate` + `resource_metadata` | ✓ PASS |
| `/mcp` refuses a page-originated request and a rebound Host | same probe | `Origin: https://evil.example → 403`; `Host: evil.example → 403` | ✓ PASS |
| The AS routes' middleware chain | same probe | `/register` 201 *with* an evil Origin; `/token` 400; `/revoke` 200; metadata 200 — i.e. the empty chain the code documents | ✓ PASS (as designed — see Honest Gap 5) |
| Shell unit tests enumerate | `cargo test -p talaria-shell --locked -- --list` | `285 tests, 0 benchmarks` | ✓ PASS |
| Four security-critical named tests | `cargo test -p talaria-shell --locked -- --exact …` | 4 passed, 0 failed | ✓ PASS |

The two throwaway probes were written to `/tmp`, not to the repo.

---

## Anti-Pattern Scan

| Pattern | Files scanned | Result |
|---------|---------------|--------|
| `TODO`/`FIXME`/`XXX`/`TBD`/`HACK`/`PLACEHOLDER`/"not yet implemented" | `oauth.rs`, `http.rs`, `agents.rs`, `talaria-mcp/src/lib.rs`, the three new e2e suites | **Zero matches.** No debt-marker gate triggered. |
| `unwrap()` in shell code | `oauth.rs`, `http.rs`, `agents.rs` | None introduced; the codebase convention holds. |
| Empty implementations / hardcoded empty returns feeding a render | Access panel, consent panel, metadata documents | None. Every rendered collection is a live read; every document is built from the bound address and the store. |
| Stale `expect(dead_code)` | `agents.rs:537`, `:545` | ℹ️ INFO, not a warning. The two `#[cfg_attr(not(test), expect(dead_code, reason = "wired in 04-08"))]` on `Agents::len`/`is_empty` are *fulfilled* expectations — the methods genuinely are test-only in a non-test build, so the attribute is correct and clippy is silent. Only the reason **string** is now stale: 04-08 shipped and wired neither (the panel reads `clients()` directly). A one-line comment fix, not a defect. |

---

## Honest Gaps

Judged against the brief's list. All five are correctly classified as gaps-to-record rather than
failures; I add one the phase did not record.

1. **`sse_support: false` is cosmetic — correctly classified.** `mcp_routes` mounts `/sse` and
   `/messages` unconditionally in `rust-mcp-axum` 1.0.1; the flag only affects a log line. The
   deferred register says both sit behind the same middleware chain as `/mcp`. **I verified this
   rather than accepting it** — a live probe returns `401` with the full `WWW-Authenticate` +
   `resource_metadata` header on both. The `sse` SDK feature is also absent. Correctly recorded, and
   the SC 1 wording ("HTTP/SSE") is satisfied by Streamable HTTP's event-stream responses regardless.

2. **The connection-close workaround — correctly classified.** It is a real SDK limitation
   (`ServerRuntime::shutdown` cancels the reader; the response body is fed from the other half of a
   duplex the message dispatcher still owns), it was *measured* rather than assumed — the suite
   asserted closure from the client end and it failed — and `deferred-items.md` §"Immediate stream
   termination goes through the connection, not the SDK" records what would retire it. The
   `dup`-based mechanism is the right shape for the constraint and is guarded against fd recycling.
   Recording it is the correct disposition.

3. **04-06's holding page is stricter than the approved UI-SPEC — correctly classified, with a
   documentation loose end.** The UI-SPEC's Controls rule says *"The only script is a status poll"*
   while the same contract fixes `default-src 'none'`, which forbids inline script — the two clauses
   cannot both hold. 04-06 kept the header and used `<meta http-equiv="refresh">` instead, which
   achieves the same auto-poll with no script at all. Divergence in the safer direction, documented
   at the code (`oauth.rs:925–931`) and in the SUMMARY, and unit-pinned by
   `the_holding_page_carries_no_request_derived_value_and_no_control`. ⚠️ **`04-UI-SPEC.md:218` was
   never amended**, so the approved document still asserts a script that does not and cannot exist.
   A one-line edit; noted so a later reader does not treat the spec as the source of truth here.

4. **The Access panel's visual quality — correctly carried as a human item.** Plus the two UI-SPEC
   backstops, which I am *not* passing on an uncommitted frame. See the human-verification list.

5. ⚠️ **New finding — the AS routes carry no host validation, and it is not in the deferred
   register.** `04-05-SUMMARY.md:446` correctly flags that the SDK's auth routes run on
   `compose(&[], ..)` and tells 04-06 to keep it in mind; 04-06's threat model accepts it for
   `/register` and `/authorize`. I confirmed the behaviour live: `/register`, `/token`, `/revoke`
   and both metadata documents are served with an `Origin: https://evil.example` header and with a
   mismatched `Host`, while `/mcp` refuses both with `403`. For the metadata documents this is
   *required*. For `/register` and `/authorize` the consequence is a DNS-rebinding path from a remote
   origin to a consent prompt in the user's own chrome. The harm is genuinely bounded — loopback-only
   bind, registration capped and inert until approved, one consent on screen at a time, cooldowns on
   deny and expiry, a 1 s arm delay, a sanitised and fenced name claim, and a human click that no
   environment variable can substitute for. But the residual is a real difference from `/mcp`'s
   posture and appears in **no** SUMMARY's Threat Flags and **not** in `deferred-items.md`. Routed to
   human decision, not scored as a failure: no prohibition claims Origin/host coverage for these
   routes.

6. **`2026-07-28` migration and CIMD — correctly classified as deliberate deferrals.** Both are
   locked decisions with reasoning, both are in the deferred register, neither is an accident to
   rediscover. CIMD in particular is a refusal on SSRF grounds, not a postponement, and the register
   names the fetch-policy precondition for any later adoption.

---

## Gaps Summary

**None blocking.** No must-have truth failed, no artifact is missing or stubbed, no key link is
unwired, no prohibition is violated, and no debt marker exists in the phase's files. The goal —
a network-reachable MCP endpoint behind Talaria's own OAuth 2.1 authorization server with
individually revocable per-client tokens — is achieved and demonstrated end to end.

Five items need a human: two UI-SPEC backstops that are non-inferable by construction and whose only
evidence is an uncommitted frame; one visual-quality judgement on the Access panel; one security
posture decision on the AS routes' middleware chain; and one prose-accuracy read of the CHANGELOG.
Two documentation loose ends are worth a minute each: `04-UI-SPEC.md:218`'s superseded script clause,
and the now-stale `reason = "wired in 04-08"` string on `agents.rs:537`/`:545`.

Phase 3's precedent for closing visual items applies directly to the first two: render the chrome
under Xvfb (`harness.start_xvfb`, a 420x700 window for the status line, a 3x toolbar crop for the
glyph pair) and review the frames — and this time commit them, so the next verifier can see what the
executor saw.

---

_Verified: 2026-08-21_
_Verifier: Claude (gsd-verifier)_

---

## Orchestrator addendum — 2026-08-21

### The four code-review blockers are fixed

`04-REVIEW.md` returned 4 blockers and 12 warnings after this verification was written. I confirmed
all four in the code myself, then had them fixed and re-verified: commits `b991fb3` (CR-01),
`242abca` (CR-02), `9dcf423` (CR-03), `3dc73ce` (CR-04), `82d03d9` (WR-01/WR-02), `af7524d` (WR-03).

**This closes the human item this document raised.** The OAuth-route question — that `/register`,
`/authorize`, `/token` and `/revoke` ran on the SDK's empty middleware chain and so bypassed the
`Origin` refusal and `Host` validation `/mcp` enforces — is no longer a decision to make. An axum
layer over the whole router now origin- and host-checks every route, with a two-entry
`PUBLIC_DISCOVERY_PATHS` allowlist keeping the RFC 9728/8414 metadata documents publicly fetchable
as they must be. A route added later inherits *checked*.

Two things the fix pass found that neither review did, both worth recording:

- **Every `/sse` session was being created with `auth_info == None`.** `McpHttpHandler::handle_sse_connection`
  takes the `AuthInfo` extension *before* running the middleware chain that inserts it, so SSE
  sessions were unidentified — unselectable by a revoke, and their tool calls arriving as
  `unverified-client`. Connection tracking alone would not have fixed CR-02; the e2e assertion is
  what surfaced it.
- **`NavigationRequest`'s `Drop` sends *allow*.** The review's suggested WR-03 patch used a bare
  `return`, which would have permitted the navigation it meant to refuse. The fix uses `deny()`.

Post-fix: `cargo test --locked` **296 passed**, e2e **22/22 `failed: none`**, clippy silent,
`primeorder 0.14.0-rc.14` pin intact, zero `unwrap()` in `http.rs`/`oauth.rs`/`agents.rs`.

### Visual review — two backstops and one human item closed

Following Phase 3's precedent, the chrome was rendered under Xvfb with the listener enabled and the
Access panel opened by clicking the real toolbar button located through `chrome_rects`. Frames were
reviewed directly rather than pixel-sampled.

- **UI-SPEC backstop, `long-text` on the Access status line** — RESOLVED. The status line and its
  `Small` warning both render inside the panel and wrap; nothing overflows the panel edge.
- **UI-SPEC backstop, `PLUGS` vs `PLUGS_CONNECTED` distinguishability** — RESOLVED, and resolved the
  way the UI-SPEC itself predicted rather than optimistically: the two glyphs are only weakly
  distinguishable on silhouette at toolbar size, and it is the `:{port}` label beside them that makes
  the state unambiguous. That label is the fallback the spec named, and it is present and legible.
- **Access panel visual quality, empty state, both views** — RESOLVED. Identical offset below the tab
  strip in Me and Agents view, no overlap with toolbar or tab strip, and the empty state names the
  next action (`Point an MCP client at 127.0.0.1:8779/mcp, then approve it here.`) rather than merely
  reporting absence. Accent appears only on the active view toggle and the active panel-toggle button
  — Phase 3's closed reserved list still holds with a sixth panel added.
- The status copy is also honest in the direction D-04-04 required: *"Anyone signed in to this
  computer can reach that address; the access token is the only thing stopping them. It is not
  reachable from other machines."*

**Still a human item, stated precisely:** the panel's appearance with **rows present**. The empty
state was reviewed; the populated state is asserted structurally by `revocation_test.py` (rect
ordering, so nothing overlaps or vanishes) but was not eyeballed, because producing a row requires
driving a full authorization flow. This is a smaller claim than the one this document originally
carried.

### Two documentation loose ends from the original report

Both fixed: `04-UI-SPEC.md`'s "the only script is a status poll" line, and the stale
`reason = "wired in 04-08"` strings on the two fulfilled `expect(dead_code)` attributes in
`agents.rs`. The attributes themselves are correct — I confirmed both expectations are *fulfilled*,
i.e. `Agents::len` and `is_empty` are genuinely test-only.

### Final disposition — all human items closed

Every item this document opened as `human_needed` is now closed:

| Item | Disposition |
|---|---|
| OAuth routes bypassing `Origin`/`Host` | **Fixed** (`b991fb3`), not merely decided |
| UI-SPEC backstop — Access status line at a narrow window | **Resolved** by review of a rendered frame |
| UI-SPEC backstop — `PLUGS` vs `PLUGS_CONNECTED` at toolbar size | **Resolved**, the way the spec predicted: the glyphs are weakly distinguishable and the `:{port}` label carries the state |
| Access panel visual quality | **Resolved** for the empty state and, after seeding the store, the populated state in both views |
| CHANGELOG prose read | **Done** — and it surfaced a third stale present-tense claim, corrected in `64639f7` |

The populated-panel render also produced a finding no test had: the count copy
read *"1 program has registered and are waiting"*, inflecting the auxiliary but
not the rest of the clause. Fixed in `64639f7`. The same frame confirmed T-4's
mitigation on the Access surface — a seeded `client_name` carrying an ASCII
double quote and a U+202E override renders both as U+FFFD.

**Final numbers:** `cargo test --locked` 296 passed · e2e **22/22 `failed: none`**
· clippy `-D warnings` silent · `primeorder 0.14.0-rc.14` pin intact · zero
`unwrap()` in `http.rs`, `oauth.rs`, `agents.rs`.

**status: passed.**

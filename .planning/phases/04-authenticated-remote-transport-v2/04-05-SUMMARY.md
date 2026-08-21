---
phase: 04-authenticated-remote-transport-v2
plan: 05
subsystem: auth
tags: [oauth, oauth2.1, rfc9728, rfc8707, bearer-token, resource-server, mcp, http, discovery, audience]

# Dependency graph
requires:
  - phase: 04-authenticated-remote-transport-v2
    provides: "04-01's shared `talaria_mcp` library target, `CommandSink` and transport-free `dispatch`; 04-02's `rust-mcp-sdk`/`rust-mcp-axum` 1.0.1 landing plus 04-02-SPIKE.md's measured facts (A1 CONFIRMED, the SDK builds the `WWW-Authenticate` challenge itself, `AuthenticationError::InvalidToken` is a struct variant, `OauthEndpoint` has no `Debug`, and nothing calls `validate_allowed_methods`); 04-03's `http.rs` listener, its BYO `mcp_routes` mount, `RefuseOriginHeader`, `Shared::remote` as the single source of truth for the bound address, `control::next_session_id`, `harness.free_port`/`write_config`; 04-04's `Agents` store, `lookup`'s live-read contract, `digest_of`, and `TokenRecord`'s `audience` and `consumed_at_ms`"
  - phase: 03-table-stakes-browsing
    provides: "the `TempPath` unit-test idiom, the seed-a-store-file-before-launch e2e technique `history_test.py` and `bookmarks_test.py` established, and the `RefCell` store convention this plan deliberately departs from for one field"
provides:
  - "`crates/talaria-shell/src/oauth.rs` — `TalariaAuth`, the SDK `AuthProvider` implementation: live per-request verification, RFC 8707 audience binding, one indistinguishable refusal, and the RFC 9728 protected-resource metadata document"
  - "`oauth::canonical_resource(bound)` — the single construction site for this server's RFC 8707 resource identifier, and `oauth::metadata_url(bound)` beside it"
  - "`oauth::SharedAgents` (`Arc<Mutex<Agents>>`) and `Shared::agents` — one store shared by the winit main thread and the `talaria-http` thread"
  - "`oauth::TALARIA_SCOPE` (`talaria:drive`) — the one scope this browser issues and requires"
  - "HTTP route `/.well-known/oauth-protected-resource` (unauthenticated, `no-store`), and a `401` on `/mcp` carrying `WWW-Authenticate: Bearer … resource_metadata=…`"
  - "An HTTP client's tab-owner label is the verified `client_id`, not the self-asserted `clientInfo.name`"
  - "`tests/e2e/oauth_flow_test.py` (discovery half; 04-06 extends it) and `harness.write_agents()`"
  - "`SECURITY.md` rewritten for a network transport: a fourth party, and the bearer token named as the entire boundary"
  - "Removed: 04-03's `DenyAllAuthProvider`, and 04-04's blanket `#[expect(dead_code)]` on `mod agents;`"
affects: [04-06, 04-07, 04-08, phase-05]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "A single-construction-site identifier: a value compared byte for byte is built by exactly one function, whose doc comment says that a second site would be a way for a credential to validate against one spelling and not another"
    - "One refusal constant as an anti-oracle: every failure mode returns the same `&'static str`, so the SDK renders one byte-identical body and the endpoint cannot be used to learn which tokens exist"
    - "The log-safety rule written into the doc comment of the tool that satisfies it, so a reviewer who finds the helper has found the rule"
    - "A shared handle documented at the point of departure from convention — the one non-`RefCell` field on `Shared`, with the two-stores-disagree bug named in the field comment"
    - "`#[cfg_attr(not(test), expect(lint, reason = ...))]` per item, replacing one blanket module-level expectation: scoped to the build where the claim is true, and itemised so a plan that wires half a module errors on exactly what it forgot"
    - "An e2e suite that starts from the endpoint URL and follows the protocol's own pointers, rather than hardcoding the paths it is meant to be discovering"
    - "A test fixture whose docstring states what it is *not*, so a later reader does not mistake a seeded store for a test-only bypass"

key-files:
  created:
    - crates/talaria-shell/src/oauth.rs
    - tests/e2e/oauth_flow_test.py
  modified:
    - crates/talaria-shell/src/http.rs
    - crates/talaria-shell/src/app.rs
    - crates/talaria-shell/src/agents.rs
    - crates/talaria-shell/src/main.rs
    - tests/e2e/harness.py
    - tests/e2e/run_all.py
    - SECURITY.md
    - CHANGELOG.md

key-decisions:
  - "**The canonical resource identifier is `http://{bound}/mcp`** — the MCP endpoint's own URL, which is what the specification names as an MCP server's canonical URI, built from the address the listener *actually bound* rather than the configured port. One function, `oauth::canonical_resource`, whose doc comment states that it is compared byte for byte and that `localhost` is therefore a different audience from `127.0.0.1`."
  - "**The single scope string is `talaria:drive`.** One scope, because `SECURITY.md` states per-agent permission scoping is out of scope by design and the access decision is binary. The constant's doc comment says not to grow it into a permission matrix without changing that document first."
  - "**Every verification failure returns one `REFUSAL` constant.** Unknown, expired, consumed, wrong-kind and wrong-audience are byte-identical to the caller — asserted in a unit test and again by a byte comparison in the e2e suite. `InvalidToken`'s `&'static str` is treated as a feature: a runtime-formatted reason, the thing that would leak, is awkward to write by accident."
  - "**An HTTP client's `AgentRequest.client` label is the verified `client_id`, not a self-asserted name.** Taken from `McpServer::auth_info_cloned`. This is the first transport where the shell knows who is calling. Stdio sessions keep their `Hello` string, and both `http.rs` and this summary say the two are deliberately different — the Agents view will show an opaque id for one and a friendly name for the other."
  - "**`Shared::agents` is an `Arc<Mutex<Agents>>`, the one field on `Shared` that is not a `RefCell`.** The verifier runs on `talaria-http` and the chrome's revoke runs on the main thread; two independently-loaded stores would let a revocation the human just performed keep working over HTTP until the next restart."
  - "**A poisoned store lock refuses every credential** rather than reading through with `into_inner`. The module's degrade direction is deny, and a store no thread can vouch for is exactly the case for it."
  - "**A broken clock refuses rather than admits.** `now_ms` saturates to `u64::MAX` when `SystemTime` cannot be read, so every stored expiry reads as past. Unit-tested."
  - "**`handle_request` calls `validate_allowed_methods` itself**, in this plan rather than waiting for 04-06, because 04-02-SPIKE measured that nothing else ever calls it and the metadata endpoint is a declared endpoint from today."
  - "**The metadata document deliberately omits `client_id_metadata_document_supported`** (D-04-02) and advertises `bearer_methods_supported: [\"header\"]` only. Both asserted, the first in a unit test and again in the e2e suite."
  - "**AUTH-01 is still not marked complete**, for the same reason 04-04 gave: it reads \"Talaria exposes an OAuth 2.1 authorization server … (Authorization Code + PKCE, per-client tokens)\". The resource-server half landed here; the authorization server is 04-06's, and checking the box now would make the traceability table assert something a reader could not verify."

patterns-established:
  - "Pattern: when a value is compared byte for byte, give it exactly one construction site and say in that function's doc comment what a second one would cost — the failure mode (a credential that validates against one spelling and not another) is invisible at the call sites"
  - "Pattern: make indistinguishability structural. One error constant, not five call sites that happen to agree, and a test that compares the rendered bodies rather than the variants"
  - "Pattern: replace a blanket lint expectation with itemised ones the moment a module becomes partly reachable, and use `cfg_attr(not(test), ..)` so the expectation applies only to the build in which the claim is true"
  - "Pattern: an e2e suite for a discovery mechanism must consume the mechanism — parse the challenge, follow its URL — not restate the paths, or it proves the code exists rather than that a client can find it"

requirements-completed: []

coverage:
  - id: D1
    description: "An unauthenticated, malformed, unknown or expired credential is refused with 401 carrying a `WWW-Authenticate` challenge that names the protected-resource metadata document"
    requirement: "AUTH-01"
    verification:
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py step 2 — 401, the challenge starts `Bearer`, carries `resource_metadata=`, and the suite parses the URL out of it rather than knowing it"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py step 8 — five malformed credentials (no scheme, scheme only, empty token, wrong scheme, a 200 KB token) each answered with a status code, and the browser still answers its socket afterwards"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#an_unknown_token_is_refused, #an_expired_token_is_refused, #an_empty_token_is_refused_rather_than_panicking, #a_multi_megabyte_token_is_refused_rather_than_panicking, #a_non_ascii_token_is_refused_rather_than_panicking"
        status: pass
      - kind: e2e
        ref: "tests/e2e/http_transport_test.py step 4 — 04-03's credential-free assertion, unmodified, still 401 with no side effect on the tab list"
        status: pass
    human_judgment: false
  - id: D2
    description: "The RFC 9728 protected-resource metadata document is fetchable without a credential, is valid JSON, names Talaria's canonical resource identifier, and names Talaria's own authorization server"
    requirement: "AUTH-01"
    verification:
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py step 3 — GET with no credential, 200, `json` in Content-Type, `resource` equals the endpoint URL, `authorization_servers` non-empty, `bearer_methods_supported == [\"header\"]`, and no `client_id_metadata_document_supported` key (D-04-02)"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#the_metadata_document_names_the_resource_and_an_authorization_server, #the_metadata_document_advertises_no_capability_this_browser_lacks, #the_metadata_response_is_json_and_uncacheable, #only_the_protected_resource_metadata_endpoint_is_declared, #the_challenge_url_is_absolute_and_is_the_endpoint_that_is_declared"
        status: pass
    human_judgment: false
  - id: D3
    description: "A token whose audience is not Talaria's canonical resource identifier is rejected, and Talaria never accepts or forwards a token issued for anything else (T-6)"
    requirement: "AUTH-01"
    verification:
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py step 7 — a seeded token bound to `http://127.0.0.1:1/mcp` is refused 401, and its body is byte-identical to an entirely unknown token's; neither request changed the tab list"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_token_minted_for_another_audience_is_refused (which also verifies the same token succeeds against a provider whose identifier *is* that other one, so the test is about the audience check and not about a broken token), #the_canonical_identifier_comes_from_the_bound_address_and_nowhere_else"
        status: pass
      - kind: other
        ref: "no-forwarding: the only outbound HTTP client in the process is the `download` tool's `ureq` call in app.rs, which constructs its own request and never reads an Authorization header; `grep -c 'Authorization' crates/talaria-shell/src/app.rs` is 0"
        status: pass
    human_judgment: false
  - id: D4
    description: "SC 1 — an authorized request drives the identical tool surface the stdio proxy serves, and can actually open and close a tab"
    requirement: "AUTH-01"
    verification:
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py step 5 — `tools/list` over HTTP normalised to sorted JSON equals `tools/list` over a freshly spawned `talaria-mcp`: nine tools, same names, descriptions and input schemas"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py step 6 — an authorized `tools/call` opens a tab that the control socket reports as owned by the verified `client_id` (and asserts the self-asserted `clientInfo.name` labels nothing), then closes it"
        status: pass
      - kind: e2e
        ref: "tests/e2e/mcp_client_test.py — unmodified, PASS inside run_all.py; the stdio surface is unchanged"
        status: pass
    human_judgment: false
  - id: D5
    description: "Token verification is a live read of the store on every request, with nothing memoised past the request boundary (SC 3's mechanism)"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_token_revoked_a_moment_ago_is_refused_by_the_very_next_call — verifies, revokes, verifies again against the same provider"
        status: pass
      - kind: other
        ref: "structural: `grep -cE 'LruCache|HashMap<String, *AuthInfo>|OnceLock<.*AuthInfo' crates/talaria-shell/src/oauth.rs` is 0 — no type exists that could hold a verification result between requests; `verify` calls `Agents::lookup` and copies four owned fields out before the guard drops"
        status: pass
      - kind: other
        ref: "`AuthMiddleware::validate` calls `verify_token` on every request before the handler (04-02-SPIKE, auth_middleware.rs:23-77), so there is no connection-setup path that could cache one"
        status: pass
    human_judgment: false
  - id: D6
    description: "Unknown, expired and wrong-audience tokens are indistinguishable to the caller — one opaque 401 with one message"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#unknown_expired_and_wrong_audience_are_indistinguishable — the three rendered error bodies compared to each other, plus assertions that the message names neither 'audience' nor 'expire'"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py step 7 — byte comparison of the two 401 bodies"
        status: pass
    human_judgment: false
  - id: D7
    description: "No request path can panic: a malformed header, malformed JSON or an oversized body degrades to a status code (T-14)"
    requirement: "AUTH-01"
    verification:
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py step 8 — the five malformed credentials, followed by a control-socket round trip proving the browser survived"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#an_empty_token_is_refused_rather_than_panicking, #a_multi_megabyte_token_is_refused_rather_than_panicking, #a_non_ascii_token_is_refused_rather_than_panicking (which also exercises `digest_prefix` on a non-hex, multibyte value), #a_broken_clock_refuses_rather_than_admits"
        status: pass
      - kind: other
        ref: "`grep -c 'unwrap()' crates/talaria-shell/src/oauth.rs` is 0 and the same over `http.rs` is 0; `expiry_at` uses `checked_add`, `digest_prefix` slices on a `char_indices` boundary, and `now_ms` uses `try_from().unwrap_or(u64::MAX)`"
        status: pass
    human_judgment: false
  - id: D8
    description: "Token material never reaches a log: what is logged is the client id and a short digest prefix, never a raw token and never an Authorization header"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_log_prefix_is_short_and_is_not_the_whole_digest — the prefix is 8 characters, is a prefix of the digest, is not the digest, and a shorter input is returned whole rather than sliced past its end"
        status: pass
      - kind: other
        ref: "reviewed by reading, as the plan's prohibition specifies rather than by a negative grep: `oauth.rs` has five log calls, each interpolating only `digest_prefix(&digest)` and `client_id`; `verify_token` takes the token by value and it is never named in a `log::` macro; `http.rs`'s `RefuseOriginHeader` logs the fact of an Origin header and never its value"
        status: pass
    human_judgment: true
    rationale: "A negative grep cannot prove the absence of an interpolation — a future edit could add one, and the compiler will not object. The rule is written into `digest_prefix`'s own doc comment so the tool and the rule are found together, but this is a code-review item by construction."
  - id: D9
    description: "The interim deny-all provider is removed, not left reachable alongside the real one"
    requirement: "AUTH-01"
    verification:
      - kind: other
        ref: "`grep -ci 'deny.all\\|DenyAll' crates/talaria-shell/src/http.rs` is 0; `grep -c 'TalariaAuth' crates/talaria-shell/src/http.rs` is 3 and `build_router` constructs exactly one `Arc<dyn AuthProvider>`"
        status: pass
      - kind: e2e
        ref: "tests/e2e/http_transport_test.py — 04-03's whole suite passes unmodified against the real provider, including its Origin (403) and credential-free (401) assertions, so the swap preserved every refusal that was already in force"
        status: pass
    human_judgment: false
  - id: D10
    description: "SECURITY.md's trust model names the fourth party this phase created and states plainly that the bearer token is the entire boundary on the TCP transport"
    requirement: "AUTH-01"
    verification:
      - kind: other
        ref: "`grep -ci 'unauthenticated network peer' SECURITY.md` is 2; `grep -ci 'loopback is not' SECURITY.md` is 1; `grep -ci 'loopback exception' SECURITY.md` is 1; `grep -c 'That changes the moment a network transport exists' SECURITY.md` is 0; `grep -c 'peer-UID check\\|peer-credential check' SECURITY.md` is 4"
        status: pass
    human_judgment: true
    rationale: "Whether a published threat model reads as honest and complete is a judgement about prose, not a property a grep can settle. The greps confirm the specific claims the plan required are present and the obsolete one is gone; whether the document as a whole now describes the browser that exists is worth one reader's pass."

# Metrics
duration: 34min
completed: 2026-08-21
status: complete
---

# Phase 4 Plan 05: The Resource Server — Discovery and Audience Binding Summary

**Talaria's MCP endpoint is now protected *and findable*: an unauthenticated request is refused with
a `WWW-Authenticate` challenge that points at an RFC 9728 metadata document naming this server's
canonical identifier and its own authorization server, a token minted for any other resource is
refused byte-identically to one that was never issued, and a client holding a real token drives the
same nine tools over HTTP that the stdio proxy serves — opening a tab the browser labels with the
identity it verified rather than the name the caller typed.**

## Performance

- **Duration:** 34 min
- **Started:** 2026-08-21 02:14 (local)
- **Completed:** 2026-08-21 02:48 (local)
- **Tasks:** 3
- **Files modified:** 10 (2 created)

## Accomplishments

- **The wall became a door, and the door has a sign on it.** 04-03 shipped a listener behind a
  provider that refused every credential unconditionally. `crates/talaria-shell/src/oauth.rs`
  replaces it with a real resource server: `verify_token` hashes the presented value with the
  store's own `digest_of`, calls `Agents::lookup` **live**, and checks the record's audience against
  a canonical identifier built by exactly one function from the address the listener actually bound.
  The interim provider is deleted, not parked — a struct that refuses everything is harmless alone
  and a footgun one line from the struct that does not.

- **Discovery, which is the half that is easy to skip and the reason this plan exists.** The `401`
  now carries `WWW-Authenticate: Bearer … resource_metadata="…"`, and that URL serves the RFC 9728
  document naming the resource identifier, the authorization server, `header` as the only bearer
  method, and the one scope. `04-RESEARCH.md`'s Pitfall 3 is a server that mints and validates
  tokens while publishing none of this, so a real client cannot find the flow and gives up before it
  starts. The e2e suite proves the pointer works by *following* it: it starts from the MCP endpoint
  URL, parses the URL out of the challenge, and never names the well-known path itself.

- **SC 1 is proven, not argued.** `tools/list` over HTTP, normalised to sorted JSON, is byte-equal
  to `tools/list` over a freshly spawned `talaria-mcp` against the same running browser — nine
  tools, same names, same descriptions, same input schemas. 04-01 made that structurally true by
  putting both transports on one library; this is the assertion that says so. And it is not only a
  list: an authorized `tools/call` opens a real tab, which the control socket then reports as owned
  by the verified `client_id`, with an explicit assertion that the self-asserted `clientInfo.name`
  the suite sent labels nothing.

- **The 401 is not an oracle.** Unknown, expired, consumed, wrong-kind and wrong-audience all return
  one `REFUSAL` constant, so the SDK renders one byte-identical body. Asserted twice: a unit test
  compares the three rendered errors to each other and checks the message names neither "audience"
  nor "expire", and the e2e suite compares the two 401 bodies byte for byte.

- **One store, two threads, and the reason written where the departure happens.** `Shared::agents`
  is the only field on `Shared` that is not a `RefCell`. Its comment says why: the verifier runs on
  `talaria-http`, the chrome's revoke runs on the main thread, and two independently-loaded stores
  would let a revocation a human just performed keep working over HTTP until the next restart. Every
  critical section on either side is one synchronous operation with no `await` inside it.

- **Two degrade paths were chosen toward deny rather than left implicit.** A poisoned store lock
  refuses every credential rather than reading through with `into_inner`, and `now_ms` saturates to
  `u64::MAX` when the clock cannot be read, so a browser whose clock is broken refuses agents rather
  than admitting them. The second is unit-tested.

- **`SECURITY.md` describes the browser that now exists.** A fourth party — an unauthenticated
  network peer — and the observation that it also *changes* the third: "a connected agent" is no
  longer a synonym for "a process running as you". The bearer token is named as the entire boundary,
  with host validation and the `Origin` refusal as defence in depth and explicitly not a substitute.

- **20 unit tests in `oauth::`** against a target of 12. Shell tests are **207**, up from 187; the
  workspace is **212**. **e2e is 21/21**, with `mcp_client_test.py` and `http_transport_test.py` both
  passing unmodified.

## Task Commits

1. **Task 1: `oauth.rs` — `TalariaAuth`, live verification, the metadata document** — `44d2a79` (feat)
2. **Task 2: replace the interim provider, share one store** — `675bf06` (feat)
3. **Task 3: the discovery e2e suite and the `SECURITY.md` rewrite** — `3132b94` (feat)

Plus `e5de700` (docs) — the CHANGELOG entry, per the project's own logging rule.

## Files Created/Modified

- `crates/talaria-shell/src/oauth.rs` *(new, ~700 lines incl. tests)* — the module header stating the
  roles and the rejected alternative before any mechanics; `TALARIA_SCOPE`, `REFUSAL`,
  `SharedAgents`, `canonical_resource`, `metadata_url`, `digest_prefix`, `now_ms`, `expiry_at`,
  `TalariaAuth` and its `AuthProvider` impl (`verify_token`, `required_scopes`, `auth_endpoints`,
  `handle_request`, `protected_resource_metadata_url`), plus `metadata_document`,
  `metadata_response`, `refused`, and 20 tests behind the house `TempPath` guard.
- `crates/talaria-shell/src/http.rs` — `DenyAllAuthProvider` deleted; `build_router` constructs
  `TalariaAuth`; `spawn` and `serve` carry the `SharedAgents` handle; `Handler` takes its client
  label from `McpServer::auth_info_cloned`; `UNVERIFIED_CLIENT`; module header rewritten for the
  swap, including the note that `/sse` and `/messages` sit behind the same chain and are therefore
  covered by the real provider exactly as they were by the interim one.
- `crates/talaria-shell/src/app.rs` — `Shared::agents` and its comment, `Agents::load()` at startup,
  the `Arc` clone into `http::spawn`.
- `crates/talaria-shell/src/agents.rs` — `digest_of` and `load_from` become `pub(crate)`; the
  blanket suppression replaced by 26 itemised `#[cfg_attr(not(test), expect(dead_code, reason))]`
  attributes naming 04-06 or 04-07; a module-header section explaining the whole arrangement.
- `crates/talaria-shell/src/main.rs` — `mod oauth;`, and the `#[expect(dead_code)]` block above
  `mod agents;` removed.
- `tests/e2e/harness.py` — `write_agents()`.
- `tests/e2e/oauth_flow_test.py` *(new)* — eight steps, `OAUTH DISCOVERY CHECKS PASSED`.
- `tests/e2e/run_all.py` — registered before `vault_nobus_test`; 21/21.
- `SECURITY.md` — the fourth party, the remote-transport enforcement entry, the socket's check
  reaffirmed, the stdio path's deliberate lack of auth explained as conformance, both known
  limitations extended, and the obsolete closing sentence replaced.
- `CHANGELOG.md` — an `### Added` entry, and one clause of the AUTH-03 entry re-tensed now that
  "nothing can drive it yet" is no longer true.

## Decisions Made

See `key-decisions` in the frontmatter. The four worth restating in prose:

**The canonical resource identifier is `http://{bound}/mcp`, and there is exactly one function that
builds it.** RFC 8707 audience validation compares strings byte for byte, so the failure mode of a
second construction site is not duplication — it is a token that validates against one spelling of
this server and not another, presenting as credentials that were issued and then mysteriously never
worked. `canonical_resource`'s doc comment says that, and says explicitly that `localhost:PORT` is
therefore a *different* audience from `127.0.0.1:PORT`. Deriving it from the address the listener
actually bound (04-03's `Shared::remote` single source of truth) is what keeps the metadata
document, the token records and the URL a human was given from disagreeing.

**The single scope is `talaria:drive`.** One scope, deliberately, because `SECURITY.md` states that
per-agent permission scoping is out of scope by design: Talaria is infrastructure, not a policy
layer, and the access decision is binary. The field exists so the protocol's responses are
well-formed, not so a decision can be made from it — and the constant's doc comment says not to grow
it into a permission matrix without changing that document first, because the document is the design.

**An HTTP client's tab-owner label is the verified `client_id`.** This is the first transport where
the shell knows who is calling, and spending that knowledge is the difference the phase buys.
`clientInfo.name` is self-asserted and anything can claim to be anything; `client_id` was minted by
this browser and approved by a human. The fallback when no `AuthInfo` is attached — unreachable,
since `AuthMiddleware` runs ahead of every handler — is a label that claims nothing
(`unverified-client`) rather than the self-asserted name, because a check whose failure path
silently accepts attacker-chosen input is a check that has been made optional. **Stdio sessions keep
their self-asserted `Hello` string**, and a later reader of the Agents view should expect an opaque
identifier on one side and a friendly name on the other. That difference is intentional and is
recorded in `http.rs` at the point it is made.

**Two degrade paths, both pointed at deny.** A poisoned store mutex refuses every credential rather
than recovering with `into_inner`: the module's degrade direction is deny, and a store no thread can
vouch for is exactly the case for it. And `now_ms` saturates to `u64::MAX` rather than falling back
to `0` when `SystemTime` cannot be read — `0` would read every stored expiry as being in the future,
which is the wrong direction for a broken clock.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] `agents::digest_of` and `Agents::load_from` had to become `pub(crate)`**

- **Found during:** Task 1
- **Issue:** The plan requires `oauth.rs` to hash with "the same helper `agents.rs` defines — do not
  define a second digest function here", and makes `grep -c 'fn digest_of\|fn sha256'` returning 0 an
  acceptance criterion. Both were private to `agents.rs`. `load_from` is the same problem one level
  down: the plan requires unit tests against "a store built at a temp path with the `TempPath` idiom",
  and the only public constructor is `Agents::load()`, which resolves `dirs::config_dir()` and would
  have made every test write to the developer's real config directory.
- **Fix:** One visibility word on each. Not `pub`: `pub(crate)` is the narrowest thing that satisfies
  the requirement, and it keeps both out of any future public surface.
- **Files modified:** `crates/talaria-shell/src/agents.rs`
- **Verification:** `grep -c 'fn digest_of\|fn sha256' crates/talaria-shell/src/oauth.rs` is 0; the
  20 `oauth::` tests all run against temp paths and leave nothing behind.
- **Committed in:** `44d2a79`

**2. [Rule 3 - Blocking] Removing the blanket `#[expect(dead_code)]` cannot be done by deletion alone**

- **Found during:** Task 2
- **Issue:** The executor brief and 04-04's own deferred item both require deleting the
  `#[expect(dead_code)]` on `mod agents;` and "letting clippy report whatever is still unreachable,
  rather than leaving a suppression that a *partial* wiring would not trip". This plan **is** a
  partial wiring: it makes the store's *reading* half live (`load`, `lookup`) and touches none of the
  writing half, which belongs to 04-06 and 04-07. Deleting the attribute produced 14 clippy errors
  under `-D warnings`, all of them accurate.
- **Fix:** The blanket attribute is gone, and clippy's report was read rather than re-suppressed:
  each still-unreachable item carries **its own** `#[cfg_attr(not(test), expect(dead_code, reason =
  "wired in 04-06"))]` (or `04-07`), 26 of them. That is strictly stronger than what was replaced —
  a lint expectation is fulfilled by *any* diagnostic in its scope, so one blanket attribute stays
  quietly fulfilled while a plan wires half a module, whereas an itemised one errors on exactly the
  item that was forgotten. The `cfg_attr(not(test), ..)` wrapper is not decoration: these items are
  all exercised by the module's own tests, so an unconditional `#[expect]` would be *unfulfilled* in
  the test build and fail the same gate. This was measured, not assumed — the unconditional form was
  tried first and produced `error: this lint expectation is unfulfilled`.
- **Files modified:** `crates/talaria-shell/src/agents.rs`, `crates/talaria-shell/src/main.rs`
- **Verification:** `grep -c 'expect(dead_code' crates/talaria-shell/src/main.rs` is 0;
  `cargo clippy --all-targets --locked -- -D warnings` exits 0.
- **Committed in:** `675bf06`

### Deliberate deviations, recorded rather than suppressed

**3. The plan's "a malformed `Authorization` header is refused with 400" is not what ships, and
should not be.**

The plan's Task 1 `<behavior>` list asks for `400` on a syntactically malformed header. Two
findings make that both impossible and undesirable:

- **The provider never sees one.** `AuthMiddleware::validate` parses the header itself and returns
  `InvalidToken` — a `401` with a challenge — before `verify_token` is ever called
  (`auth_middleware.rs:23-77`, confirmed in 04-02-SPIKE). A provider receives only the token string.
- **Returning `400` for a malformed *token value* would be the oracle this plan exists to prevent.**
  A caller who could tell "that is not even the right shape for a token" from "that is a well-formed
  token I do not know" has learned something about the token space. Every presented value therefore
  takes the same route and gets the same `401`.

The plan's own `must_haves` truth #1 agrees with what shipped ("a malformed credential … refused
with 401 carrying a WWW-Authenticate challenge"), so this resolves an internal disagreement in the
plan in favour of the security property. The `400`/`405`/`404` shapes still exist and are used on
the endpoint-handling path, where they are correct: a wrong verb on the metadata endpoint is `405`
and an undeclared endpoint is `404`, both unit-tested.

**4. Task 1's commit is not clippy-green; Task 2's is.**

An unreferenced module in a binary crate is dead code, and `TalariaAuth` has no caller until
`http.rs` constructs it. Task 1's gate was therefore `cargo build --release --locked` plus
`cargo test -p talaria-shell oauth::` (20 pass), with `-D warnings` deferred one commit rather than
satisfied by a throwaway `#[allow]` that 04-06 would have inherited. The commit message says so.
This is the same structural fact 04-04 hit and recorded from the other side.

**5. `write_agents` takes an `extra_tokens` keyword the plan's signature did not name.**

The plan gives `write_agents(talaria_dir, client_id, client_name, token, **kwargs)` and separately
requires step 7 to "seed a second token whose audience is a different resource identifier". A helper
that writes one token cannot seed two. `extra_tokens` takes `(token, audience)` pairs belonging to
the same client in their own families. Signature and criterion (`grep -c 'def write_agents'` is 1)
both unaffected.

---

**Total deviations:** 2 auto-fixed (both Rule 3 — blocking), 3 deliberate and recorded.
**Impact on plan:** No scope change. Both auto-fixes were single-line or mechanical and were forced
by acceptance criteria the plan itself wrote. Deviation 3 resolves a contradiction *within* the plan
in favour of its own stated security property.

## Issues Encountered

**Confirming that `mcp_routes` mounts the declared auth endpoint at all.** 04-03 deviated from its
plan by using the BYO `mcp_routes` path rather than `create_axum_server`, and 04-02's spike measured
endpoint routing only through `create_axum_server`. Rather than assume the two agree, the source was
read: `rust-mcp-axum-1.0.1/src/routes.rs:45` merges `auth_routes::routes(http_handler)` into the BYO
router, which folds over the same `oauth_endpoints()` keys — so both mount paths route the provider's
declared endpoints identically. Then confirmed live during Task 2 with a running shell:
`GET /.well-known/oauth-protected-resource` returns `200 application/json no-store` with the expected
document, and the `401` on `/mcp` carries the matching `resource_metadata` pointer.

One consequence worth carrying forward: those auth routes run with an **empty** middleware chain
(`compose(&[], ..)`), so the metadata document is reachable without a token *and* without passing the
`Origin` refusal. Both are correct — a document a client must read before it has a token cannot demand
one, and the document contains no secrets — but 04-06 should keep it in mind when it declares
`/authorize`, `/token`, `/register` and `/revoke` on that same unprotected chain.

## Threat Flags

None. This plan adds one route, and it is the RFC 9728 metadata document — unauthenticated by
specification, serving a fixed JSON object derived entirely from the bound address, with no request
input reaching it beyond the verb (which is checked) and the path (which the router matched). No new
network surface, no new file, no new schema, and no dependency change.

## Known Stubs

None. Every function in `oauth.rs` is implemented and exercised. What is *absent* rather than stubbed
is the authorization server — `/authorize`, `/token`, `/register`, `/revoke` and the RFC 8414
document — which this plan deliberately does not declare, because an endpoint declared but
unimplemented is a `500` waiting for the first client that follows the metadata document to it.
`TalariaAuth::new`'s comment names what is not declared yet, what 04-06 adds, and what
(introspection) is never declared at all.

## Deferred Items

- **`#[cfg_attr(not(test), expect(dead_code, reason = "wired in 04-06"))]` on 19 items and
  `"wired in 04-07"` on 7.** 04-06 must delete the first set as it wires `register`, `authorize`,
  `mint`, `rotate_refresh` and the random/serialisation helpers; 04-07 the second as it wires
  `clients`, `tokens`, `len`, `is_empty`, `revoke_client` and `revoke_family`. Each will error on the
  exact item it forgets, which is the improvement over the blanket attribute it replaced.
- **The authorization server's endpoints share the metadata document's unprotected middleware
  chain.** See Issues Encountered. 04-06 owns the consequences, and in particular must call
  `validate_allowed_methods` in every arm of its own `handle_request` — this plan already does so
  for the one endpoint it declares.
- **`04-RESEARCH.md`'s "revocation must also terminate an open `text/event-stream`" is untouched
  here.** Verification is per-request, so revocation lands on the next *request*; a client with an
  already-open stream has no next request to fail. 04-07 owns it, as 04-03 and 04-04 both recorded.
- **The scope string is now a wire value.** `talaria:drive` appears in issued token records, in the
  metadata document, and in `harness.write_agents`'s default. Changing it later invalidates every
  stored record's scope, so a plan that wants to would need a migration or a re-authorization.

## User Setup Required

None — no external service configuration, no new environment variable, no new config key. Remote
access is still off unless `config.json` says otherwise, and the credential a client presents still
has to come from somewhere: today that means a record already in `agents.json`, and from 04-06 it
means the authorization flow.

## Verification

All gates run from a clean tree at the plan tip:

| Gate | Result |
|------|--------|
| `cargo build --release --locked` | exit 0 |
| `cargo clippy --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --locked` | 212 pass (207 shell + 3 protocol + 2 mcp-lib), 0 fail |
| `cargo test -p talaria-shell oauth::` | 20 pass, against a target of 12 |
| `python3 tests/e2e/oauth_flow_test.py` | exit 0, `OAUTH DISCOVERY CHECKS PASSED` |
| `python3 tests/e2e/http_transport_test.py` | exit 0, unmodified |
| `python3 tests/e2e/run_all.py` (Xvfb `:98`) | **21/21 PASS**, `failed: none` |
| `git diff --stat Cargo.toml Cargo.lock crates/talaria-shell/Cargo.toml` | no change |
| `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'` | 1 |

The 9 ignored doc-tests on `crates/talaria-mcp/src/tools.rs` are pre-existing and untouched.

Source criteria, all met — `oauth.rs`: `pub struct TalariaAuth` 1; `oauth-protected-resource` 5
(≥1); `www-authenticate` 3 (≥1); `unwrap()` 0; `fn digest_of|fn sha256` 0;
`LruCache|HashMap<String, *AuthInfo>|OnceLock<.*AuthInfo` 0; `lookup` 3 (≥1); `jsonwebtoken` 0.
`main.rs`: `mod oauth;` 1, `expect(dead_code` 0. `http.rs`: `TalariaAuth` 3 (≥1);
`deny.all|DenyAll` 0; `origin` 22 (≥1); `next_session_id` 2 (≥1); `dispatch` 3 (≥1); `unwrap()` 0.
`harness.py`: `def write_agents` 1. `run_all.py`: `oauth_flow` 2 (≥2). `SECURITY.md`:
`unauthenticated network peer` 2 (≥1); `loopback is not` 1 (≥1); `loopback exception` 1 (≥1);
`That changes the moment a network transport exists` 0; `peer-UID check|peer-credential check` 4 (≥1).

## Next Phase Readiness

**04-06 (the authorization server) has everything it needs.** `TalariaAuth` is the struct it extends
rather than replaces: adding `/authorize`, `/token`, `/register`, `/revoke` and the RFC 8414 document
means adding entries to the `endpoints` map in `TalariaAuth::new` and arms to `handle_request`. The
canonical resource identifier it must mint tokens against is `oauth::canonical_resource`, the scope
is `oauth::TALARIA_SCOPE`, and the store handle is already on `Shared`. Three things it must carry:
`validate_allowed_methods` has to be called in every arm (this plan does it for the one endpoint it
declares, so the pattern is in place); `OauthEndpoint` has no `Debug`, so naming one in a log needs a
hand-written match; and its endpoints will run on the same unprotected middleware chain as the
metadata document.

**04-07 (revocation and the Access panel)** has the shared store on `Shared::agents` and can mutate
it from `apply_ui_actions` with a short synchronous lock. The mechanism SC 3 depends on is already
proven at the unit level (`#a_token_revoked_a_moment_ago_is_refused_by_the_very_next_call`); what
remains is the end-to-end proof and the in-flight-stream question.

No blockers.

---
*Phase: 04-authenticated-remote-transport-v2*
*Completed: 2026-08-21*

## Self-Check: PASSED

Both claimed new files exist on disk (`crates/talaria-shell/src/oauth.rs`,
`tests/e2e/oauth_flow_test.py`) and so does this summary. All five commits (`44d2a79`, `675bf06`,
`3132b94`, `e5de700`, `7137b29`) are in the history. `git diff --diff-filter=D --name-only` over the
plan's whole range reports **no deleted tracked files**. The working tree is clean apart from a
pre-existing untracked handoff note that is not this plan's.

One correction found by the check and applied above: the itemised suppressions split 19/7 between
04-06 and 04-07, not 20/6. The total, 26, was right.

---
phase: 04-authenticated-remote-transport-v2
plan: 06
subsystem: auth
tags: [oauth, oauth2.1, rfc8414, rfc7591, rfc7636, rfc8252, rfc9207, pkce, consent, egui, dcr]

# Dependency graph
requires:
  - phase: 04-authenticated-remote-transport-v2
    provides: "04-02-SPIKE's measured facts (A1 CONFIRMED — every declared `OauthEndpoint` reaches `handle_request`; nothing calls `validate_allowed_methods`; auth routes run on `compose(&[], ..)`; `OauthEndpoint` has no `Debug`); 04-03's `http.rs` listener, its BYO `mcp_routes` mount and `Shared::remote`; 04-04's `Agents` store — `register`, the registration cap, `digest_of`, and the inert-until-approved contract on `authorized_at_ms`; 04-05's `TalariaAuth`, `canonical_resource`, `SharedAgents`, `TALARIA_SCOPE`, the `handle_request` shape that calls `validate_allowed_methods` itself, and the 26 itemised `expect(dead_code)` attributes that made this plan's wiring self-checking"
  - phase: 03-table-stakes-browsing
    provides: "`ChromePanel`/`UiAction`/`apply_ui_actions` round trip, `Gui::set_panel`'s reset discipline, `truncate_chars`, `is_display_unsafe`/`sanitize_for_display`, `record_rect` and the `TALARIA_TEST_HOOKS` gate, the arm-then-confirm control, and `panel_click_test.py`'s real-pointer-click technique"
provides:
  - "RFC 8414 authorization-server metadata at `/.well-known/oauth-authorization-server` — S256 only, `iss` advertised because `iss` is emitted, and no capability this phase did not build"
  - "RFC 7591 dynamic client registration at `/register` — redirect URIs validated at the boundary, refused whole rather than stored in part, and never dereferenced"
  - "`/authorize` and its five-step validation chain, every refusal landing before a human is asked; `/authorize/status`, the path the holding page refreshes to and the one that carries the `302` back to the client"
  - "`/token` declared and answered with an explicit not-yet-implemented error, so the metadata advertises no path that routes nowhere (04-07 fills it in)"
  - "`oauth::ConsentRequest` / `ConsentDecision` / `ConsentRaiser` — the parked-request round trip, with the raise expressed as a callback so the window loop stays `http.rs`'s business and the whole flow is unit-testable"
  - "The authorization-code store's write side: `AuthorizationCodes::mint` and a `take` that removes and returns in one operation (04-07 redeems)"
  - "The consent state machine: one parked slot, and a cooldown armed by every terminal resolution the human did not consent to — denial and expiry alike, both on the HTTP side"
  - "Three consent durations as compiled constants with a `TALARIA_TEST_HOOKS=1`-gated, shorten-only override: `TALARIA_CONSENT_LIFETIME_MS`, `TALARIA_CONSENT_DENY_COOLDOWN_MS`, `TALARIA_CONSENT_EXPIRY_COOLDOWN_MS`"
  - "`ChromePanel::Consent`, `Gui::raise_consent()`, and the refusal of `SetPanel(Consent)` in both `apply_ui_actions` and `Gui::set_panel`"
  - "`gui::sanitize_claim` — `sanitize_for_display` plus the ASCII double quote, for the one caller where the quote is the delimiter"
  - "`UiAction::ApproveConsent(u64)` / `UiAction::DenyConsent(u64)`, carrying a parked request id and nothing else"
  - "`Shared::pending_consent` and `AppEvent::ConsentRequested`"
  - "Chrome rects `consent.approve` and `consent.deny`; harness helpers `pkce_pair()`, `loopback_callback()`, `focus_window()` and `click_rect(..., focus=False)`"
  - "`tests/e2e/oauth_flow_test.py`'s consent half — `OAUTH CONSENT CHECKS PASSED`"
affects: [04-07, 04-08, phase-05]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "A security-relevant knob expressed as a clamp rather than a value: the compiled constant is the ceiling and a non-zero floor is the floor, so the override's *type* forbids lengthening, zeroing and switching off, and the three failure directions are unit-tested rather than promised"
    - "A refusal, not a convention: adding an enum variant that makes a dangerous action representable is paired with an explicit refusal at every entry point, so the property is confirmed in two functions rather than maintained by every future call site"
    - "The dangerous side effect expressed as an injected callback (`ConsentRaiser`) rather than a held handle, so the layer that owns *what is asked* does not depend on the layer that owns *how the message travels* — and the whole path becomes testable without a window"
    - "A page hardened by having nothing in it: no control of any kind and no request-derived value, which removes the injection and phishing surface rather than neutralising it"
    - "An e2e latency assertion that fails loudly when its own precondition was not met, rather than silently not running"

key-files:
  created: []
  modified:
    - crates/talaria-shell/src/oauth.rs
    - crates/talaria-shell/src/gui.rs
    - crates/talaria-shell/src/app.rs
    - crates/talaria-shell/src/http.rs
    - crates/talaria-shell/src/agents.rs
    - tests/e2e/harness.py
    - tests/e2e/oauth_flow_test.py
    - tests/e2e/panel_click_test.py

key-decisions:
  - "**The holding page carries no script, only a `<meta http-equiv=\"refresh\">`.** `04-UI-SPEC.md` asks for 'the only script is a status poll' and fixes `Content-Security-Policy: default-src 'none'` in the same contract; those two cannot both hold, because `default-src 'none'` forbids inline script. Resolved toward the headers: a meta refresh does the same job, is not a control, cannot be driven by an agent, and is strictly stricter than what the spec described."
  - "**The status poll is a second path on the same `OauthEndpoint::AuthorizationEndpoint`.** The SDK builds its auth router by folding over the *keys* of `auth_endpoints()`, so a key is a route; there is no `OauthEndpoint` variant for a poll and inventing one would mean forking the SDK's enum. The verb table for the authorization endpoint (`GET`/`HEAD`/`OPTIONS`) is exactly the one a poll wants."
  - "**The `iss` parameter is emitted *and* advertised.** RFC 9207 is a SHOULD moving toward a MUST, it is nearly free, and the two halves must agree — an emitted parameter the metadata does not claim is as broken as the reverse. `authorization_response_iss_parameter_supported: true`, and `iss` on every authorization response, success and error alike."
  - "**The revocation endpoint is left to 04-08 and is not advertised.** 04-08's own plan owns `/revoke`; declaring it here would put a `500` behind a path the metadata sent clients to. The spike confirmed either choice routes, so this is a scoping call, not a technical one."
  - "**`Gui::raise_consent()` takes no argument**, reconciling `04-UI-SPEC.md`'s literal signature with the same contract's 'no timing state lives on `Gui`'. The request is parked on `Shared::pending_consent` and read by the panel each frame; the reset both entry points perform is factored into one helper, which is what the contract actually fixes."
  - "**The raise is a `ConsentRaiser` callback, not an `EventLoopProxy` held in `oauth.rs`.** Two payoffs: the module owns what is asked and what the answer means rather than how a message reaches the main thread, and the whole authorization path — parking, both harassment caps, approval, denial, expiry — is exercised by unit tests with no window."
  - "**`ParkedAuthorization` and `ConsentRequest` live in `oauth.rs`, not `app.rs`.** The plan placed the type in `app.rs`; it is defined beside the code that constructs and resolves it instead, following `control::AgentRequest`'s precedent, which is what let Task 1 stand alone."
  - "**An unknown client and a mismatched redirect URI are one refusal, byte for byte.** Distinguishing them would make `/authorize` an oracle for which client ids are registered — the same anti-oracle reasoning 04-05 applied to token verification, unit-tested by comparing the two rendered refusals."
  - "**`resource` is required, not merely checked when present.** The MCP specification obliges a client to name the resource it wants a token for, and a code minted without one could not be audience-bound at redemption — which is the confused-deputy defence the resource-server half depends on."
  - "**Loopback redirect URIs must be IP literals.** `http://localhost:PORT/cb` is refused at registration: RFC 8252 §8.3 prefers the literals, and what `localhost` resolves to is the resolver's business, which would turn the one-field-wide loopback exception into a hole."

patterns-established:
  - "Pattern: when a plan's own contract contradicts itself, resolve toward the security property and record *which* half was kept — here the CSP was kept and the script was dropped, and the page ended up stricter than the document describing it"
  - "Pattern: express a test-only knob as a clamp over a compiled constant, so the unsafe directions are unrepresentable rather than merely unused, and unit-test all three degrade directions (over-large, zero, unparseable)"
  - "Pattern: pair every newly-representable dangerous action with an explicit refusal at each entry point, and say in the comment what the refusal buys — that the property is confirmed in one place rather than maintained by discipline"
  - "Pattern: inject the side effect that crosses a thread boundary as a callback; the layer boundary improves and the tests stop needing the runtime"
  - "Pattern: an e2e assertion whose validity depends on its own latency must assert that latency and fail on it, never skip silently"

requirements-completed: []

coverage:
  - id: D1
    description: "A client that knows only the MCP endpoint URL can discover this authorization server, register dynamically, and reach a human approval prompt — the discovery-through-consent half of SC 2"
    requirement: "AUTH-01"
    verification:
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py consent half, steps 2-6 — 401 challenge → protected-resource metadata → RFC 8414 construction from the issuer → registration → authorization → the real Approve control; no metadata, registration or authorization path is written down anywhere in the suite"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_valid_request_raises_exactly_one_consent_request_and_answers_a_holding_page, #registering_mints_an_identifier_the_caller_did_not_choose"
        status: pass
    human_judgment: false
  - id: D2
    description: "The authorization-server metadata advertises the S256 challenge method and a registration endpoint, and advertises no capability this phase did not implement"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#the_authorization_server_metadata_advertises_only_the_s256_challenge_method, #the_authorization_server_metadata_claims_no_client_metadata_document_capability, #the_issuer_parameter_is_advertised_because_it_is_emitted, #every_endpoint_the_metadata_names_is_one_this_provider_declares — all asserted on the rendered JSON, not by a source grep"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py consent half, step 2 — `code_challenge_methods_supported == ['S256']`, no `client_id_metadata_document_supported`, `authorization_response_iss_parameter_supported` true"
        status: pass
    human_judgment: false
  - id: D3
    description: "A code_challenge_method other than S256 is refused at the authorization endpoint, before any panel is raised (T-5, Pitfall 5)"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#an_unsafe_challenge_method_is_refused (plain, PLAIN, s256, S512, empty and absent), #a_missing_or_malformed_challenge_is_refused"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py consent half, step 4 — `code_challenge_method=plain` answered 302 `invalid_request`, with `wait_for_rect(present=False)` proving no panel appeared"
        status: pass
    human_judgment: false
  - id: D4
    description: "A redirect URI must match the registration exactly, with the loopback port as the only permitted variance (RFC 8252), and a mismatch is refused before any panel is raised"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_loopback_redirect_on_another_port_matches_the_registration, #a_non_loopback_port_difference_is_refused, #a_redirect_uri_that_differs_anywhere_but_the_loopback_port_is_refused, #a_mismatched_redirect_uri_is_refused_before_any_panel_is_raised, #an_unknown_client_and_a_mismatched_redirect_are_one_refusal"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py consent half — the callback binds port 0, so the registered port and the presented port differ on every successful flow; step 4 refuses an unregistered redirect with a direct 400 rather than redirecting to it"
        status: pass
    human_judgment: false
  - id: D5
    description: "Approving mints a single-use authorization code bound to the client, the challenge and the redirect URI, and redirects the caller to its registered redirect URI carrying that code and the original state"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#approving_mints_a_code_and_redirects_to_the_registered_uri_with_the_state, #a_code_is_removed_and_returned_in_one_operation_and_carries_its_binding, #an_expired_code_is_not_returned"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py consent half, step 7 — the loopback callback receives code, state and iss, and no error"
        status: pass
    human_judgment: false
  - id: D6
    description: "The Approve control exists only in native chrome: ChromePanel::Consent is raised solely by Gui::raise_consent, and UiAction::SetPanel(Consent) is refused by both apply_ui_actions and Gui::set_panel (T-3)"
    requirement: "AUTH-01"
    verification:
      - kind: other
        ref: "structural: `grep -c 'ChromePanel::Consent' gui.rs` is 5 (variant, the `set_panel` refusal, the panel body, the two-entry-point comment) and `app.rs` is 1 (the refused arm); the panel is set in exactly two functions, `set_panel` (which returns early for this variant) and `raise_consent`"
        status: pass
      - kind: other
        ref: "`grep -c 'ApproveConsent\\|DenyConsent' crates/talaria-mcp/src/tools.rs crates/talaria-protocol/src/lib.rs` is 0 — the decision is named in no MCP tool and no control-socket variant"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py consent half, step 5 — the page served at /authorize contains no `<form`, `<button`, `<a `, `<input`, `<script` or `onclick`, so the surface an agent can navigate to and script has nothing on it to script"
        status: pass
    human_judgment: true
    rationale: "The refusals and the absent tool/protocol variants are checkable, and the two are checked. What no test can settle is whether a *future* control could reach the panel some third way — that is precisely why the refusals exist, and confirming that the two entry points really are the only ones is a code-review reading rather than an assertion."
  - id: D7
    description: "The client's claimed name is truncated, sanitised, fenced and labelled as a claim above and below, and cannot close the quotes it is rendered inside (T-4)"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/gui.rs#a_claimed_name_cannot_close_the_quotes_it_is_rendered_inside, #a_claimed_name_cannot_take_a_second_line_or_reorder_the_text_around_it, #a_claimed_name_keeps_its_curly_quotes_and_its_multibyte_characters, #the_claim_sanitiser_is_the_display_one_plus_the_delimiter_rule, #a_kilobyte_of_claimed_name_is_cut_to_the_rendered_limit"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py consent half, step 11 — a display name carrying a newline, U+202E and an ASCII double quote leaves exactly two consent rects, both on screen, both clickable"
        status: pass
    human_judgment: true
    rationale: "The rects prove the hostile name displaced no control; whether the rendered screen *reads* as 'here is what a stranger says, and here is what Talaria can verify' is a judgement about prose and layout that geometry cannot settle. The copy is verbatim from the approved contract, but one reader's eyes-on pass is worth having."
  - id: D8
    description: "Approve is disabled for the first second the panel is on screen; Deny is live immediately (T-04-06-02)"
    requirement: "AUTH-01"
    verification:
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py consent half, step 6 — a real pointer click at ~0.31s leaves the request `pending`; the same click after 1.2s approves it. The step fails loudly if the harness took longer than 0.9s to reach the control, so the assertion can never silently not run"
        status: pass
    human_judgment: false
  - id: D9
    description: "At most one consent request is on screen at a time, and a cooldown follows every terminal resolution — Deny and expiry alike, both armed by the HTTP layer (T-04-06-01)"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_second_authorization_while_one_is_parked_is_refused_without_raising_anything, #denying_returns_access_denied_and_arms_the_shorter_cooldown, #an_unanswered_request_expires_and_arms_the_longer_cooldown, #an_approval_arms_no_cooldown"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py consent half, steps 8-10 — the denial cooldown, the expiry cooldown and the one-at-a-time cap asserted separately, each with `wait_for_rect(present=False)`; a denial-only cooldown passes step 8 and fails step 9"
        status: pass
    human_judgment: false
  - id: D10
    description: "The page served at /authorize carries no request-derived value and no control of any kind (T-04-06-04)"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#the_holding_page_carries_no_request_derived_value_and_no_control, #the_holding_page_carries_the_framing_and_caching_headers"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py consent half, step 5 — the five headers asserted verbatim, six control shapes absent, and none of the display name, callback URL, challenge, state or client id in the bytes"
        status: pass
    human_judgment: false
  - id: D11
    description: "The chrome never handles a code, a token, or a verifier: the actions it emits carry a parked request id and nothing else (T-04-06-03)"
    requirement: "AUTH-01"
    verification:
      - kind: other
        ref: "structural: `UiAction::ApproveConsent(u64)` and `DenyConsent(u64)` have no other payload, so a secret in one is unrepresentable; the code is minted inside `settle` on the HTTP side, after the decision arrives"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#approving_mints_a_code_and_redirects_to_the_registered_uri_with_the_state — the code is read out of the Location header, never out of anything the chrome touched"
        status: pass
    human_judgment: false
  - id: D12
    description: "A registration that was never approved holds no tokens and appears in no list, and a flood is refused rather than evicting (T-11)"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#registering_mints_an_identifier_the_caller_did_not_choose (asserts `authorized_at_ms` is None and the store holds no token), #a_registration_past_the_cap_is_refused_rather_than_evicting (the first client is still there), #a_registration_with_one_inadmissible_redirect_uri_is_refused_whole, #a_registration_with_a_non_loopback_http_redirect_is_refused"
        status: pass
    human_judgment: false
  - id: D13
    description: "Every consent duration is a named constant that a gated override may only shorten — never lengthen, never zero — and no override in any combination can cause a request to be approved (T-04-06-05)"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#an_over_large_timing_override_yields_the_compiled_default, #a_zero_timing_override_yields_the_floor_and_never_zero, #an_unparseable_timing_override_yields_the_compiled_default, #an_override_can_only_shorten_a_duration_never_lengthen_one, #with_every_timing_override_at_its_floor_an_unanswered_request_is_still_denied"
        status: pass
      - kind: other
        ref: "source: `grep -c 'TALARIA_TEST_HOOKS' oauth.rs` is 1 and `grep -c 'env::var(\"TALARIA_CONSENT' oauth.rs` is 3 — the gate is read at one point and every override name is confined to the single timing function; the grant path (`settle`) takes its timings as a parameter and reads no environment variable"
        status: pass
    human_judgment: false
  - id: D14
    description: "The authorization server never fetches a URL supplied by a caller (T-12)"
    requirement: "AUTH-01"
    verification:
      - kind: other
        ref: "source: `grep -c 'reqwest\\|ureq\\|Client::new' crates/talaria-shell/src/oauth.rs` is 0 — no outbound HTTP client is constructed anywhere in the module; the redirect URI is parsed, compared and displayed, never dereferenced"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#the_authorization_server_metadata_claims_no_client_metadata_document_capability — the mechanism that would require a fetch is absent from the JSON a client receives"
        status: pass
    human_judgment: false

# Metrics
duration: 64 min
completed: 2026-08-21
status: complete
---

# Phase 4 Plan 06: The Authorization Server's Front Half Summary

**A client that knows only Talaria's MCP endpoint URL now discovers the authorization server, registers itself, is refused for every invalid parameter without troubling anybody, and — on a human's real click on a native control that no page, tool or environment variable can reach — walks away with an authorization code.**

## Performance

- **Duration:** 64 min
- **Started:** 2026-08-21T02:41 (+04:00)
- **Completed:** 2026-08-21T03:45 (+04:00)
- **Tasks:** 3 of 3
- **Files modified:** 8 (5 Rust, 3 Python)

## Accomplishments

- **The authorization server's front half** in `crates/talaria-shell/src/oauth.rs`: RFC 8414 metadata, RFC 7591 registration, `/authorize` with its full validation chain, `/authorize/status`, request parking, the holding page, the authorization-code store's write side, the cooldown state machine, and the consent timing constants with their gated override. `/token` is declared and answered with an explicit not-yet-implemented error so the metadata advertises no path that routes nowhere.
- **The consent decision in native chrome**, structurally unreachable from the party asking for permission. The page an agent can navigate to has no control of any kind, and `SetPanel(Consent)` is refused in both `apply_ui_actions` and `Gui::set_panel` so "no control can open the Approve button" is confirmed in two functions rather than maintained by every future panel button.
- **`sanitize_claim`** — `sanitize_for_display` plus the ASCII double quote, because on this one screen the quote is the delimiter and a `client_name` of `Talaria" verified by Talaria "` would otherwise render as Talaria's own words.
- **A one-second arm delay on Approve**, proven behaviourally end to end rather than by finding a constant in the source: a real pointer click at ~0.31 s grants nothing, and the same click after 1.2 s approves.
- **Two anti-harassment caps**, each asserted separately because a denial-only cooldown passes one and fails the other: one request on screen at a time (structural — an `Option`, not a counter), and a cooldown after every terminal resolution the human did not consent to.
- **39 new Rust tests** (59 in `oauth::`, 251 in the shell, 256 in the workspace, from 212) and the consent half of `tests/e2e/oauth_flow_test.py`, which runs in 60 s against a 90 s budget. e2e **21/21**.

## Task Commits

| Task | Name | Commit | Key files |
| ---- | ---- | ------ | --------- |
| 1 | Metadata, registration, the authorization endpoint, parking, and the holding page | `49a2466` | `oauth.rs`, `agents.rs`, `app.rs`, `http.rs` |
| 2 | The consent panel — raised only by an event, refused as a target | `dc12eb6` | `gui.rs`, `app.rs` |
| 3 | Discovery through consent, end to end, approved by a real click | `ffc2365` | `harness.py`, `oauth_flow_test.py`, `panel_click_test.py` |
| — | The clippy hunk Task 2's commit missed (see Deviations 6) | `cc16ea3` | `oauth.rs` |

## Decisions Made

See `key-decisions` in the frontmatter. The five worth restating in prose, and the four the plan's `<verification>` step 7 asked for by name.

### The consent timings, and what the override is and is not

The three durations are exactly the ones `04-UI-SPEC.md` fixes, and all three are **invented anti-harassment caps rather than specification requirements**:

| Duration | Constant | Value | Override | Ceiling | Floor |
|---|---|---|---|---|---|
| Parked-request lifetime | `CONSENT_LIFETIME_MS` | 120 000 ms | `TALARIA_CONSENT_LIFETIME_MS` | 120 000 ms | 200 ms |
| Post-denial cooldown | `DENY_COOLDOWN_MS` | 10 000 ms | `TALARIA_CONSENT_DENY_COOLDOWN_MS` | 10 000 ms | 200 ms |
| Post-expiry cooldown | `EXPIRY_COOLDOWN_MS` | 30 000 ms | `TALARIA_CONSENT_EXPIRY_COOLDOWN_MS` | 30 000 ms | 200 ms |
| Authorization-code lifetime | `AUTHORIZATION_CODE_LIFETIME_MS` | 60 000 ms | **none, deliberately** | — | — |

The code lifetime is not overridable and no fourth variable was invented to make the count tidy: a suite redeems immediately, so there is nothing to wait out, and the whole defence of this mechanism is how few knobs it has.

Four properties keep it a timing knob rather than a bypass, and each is independently asserted:

1. **Gated, read once.** `grep -c 'TALARIA_TEST_HOOKS' oauth.rs` is 1. The gate is a named constant (`TEST_HOOKS_VAR`) so the module's own tests can refer to it without adding a second occurrence — the same problem the plan anticipated for the doc comment, one level along.
2. **Durations only.** `grep -c 'env::var("TALARIA_CONSENT' oauth.rs` is 3, one per duration, all three inside `consent_timings()`. The doc comment names them freely, which the plan's scoped criterion explicitly permits.
3. **Shortening only.** Every value passes through `shorten`, which is `value.max(FLOOR).min(compiled)` — `max`/`min` rather than `clamp`, which panics when the floor exceeds the ceiling; it cannot here, and a panic on a network-facing path is not worth leaving one constant edit away. Four tests: over-large, zero, unparseable (five spellings, including a negative), and the property itself over seven values.
4. **The grant path is unreachable from it.** `settle` — the only function that mints a code — takes its timings as a parameter and reads no environment variable. `with_every_timing_override_at_its_floor_an_unanswered_request_is_still_denied` builds the floors *through `shorten` itself*, so they are literally the shortest any variable could produce, parks a request, answers nothing, and asserts the caller is refused, no code exists, and no client is marked authorized.

### The issuer parameter: emitted, and advertised

Both. `authorization_response_iss_parameter_supported: true` is in the RFC 8414 document and `iss` is appended to every authorization response — success, error and cooldown refusal alike — through the same `redirect_with` helper, so the two cannot drift apart. `the_issuer_parameter_is_advertised_because_it_is_emitted` asserts both halves in one test for that reason.

### The revocation endpoint: left to 04-08, and not advertised

The spike measured that a declared `/revoke` routes cleanly and an undeclared path `404`s, so this is a scoping call rather than a technical one. 04-08's plan owns "the standard revocation endpoint accepts a client's own token and always answers success", and declaring the path here would put a `500` behind something the metadata sent conformant clients to. The metadata therefore carries no `revocation_endpoint` key, and a unit test asserts its absence beside the CIMD one. `TalariaAuth::new`'s comment records all three cases — declared-and-answered (`/token`), deferred-and-unadvertised (`/revoke`), and never-declared (introspection, because the authorization server and the resource server are one process reading one `Vec`).

### `Gui::raise_consent()`'s signature — a reconciliation, not a drift

`04-UI-SPEC.md` writes `Gui::raise_consent(request)` and, in the same contract, fixes that **no timing state lives on `Gui`** — the arm delay reads `raised_at_ms` off the parked request on `Shared`. Both cannot hold if the request is handed to the chrome. The request is parked on `Shared::pending_consent` and read by the panel body each frame, the way every other panel reads its store, and `raise_consent()` takes no argument. What the contract actually fixes is that the same view-state reset happens on **both** paths and that `set_panel` is not one of them; the reset is factored into `reset_panel_view_state`, which both call, so a revealed password or a half-armed confirm cannot survive a consent interruption and cannot drift apart between the two entry points.

### The holding page has no script at all

`04-UI-SPEC.md`'s Copywriting Contract says of the holding page "the only script is a status poll" and, three rows later, fixes `Content-Security-Policy: default-src 'none'; frame-ancestors 'none'`. Those two cannot both hold: `default-src 'none'` forbids inline script, and a scripted poll would have been dead on arrival behind the very header the same table specifies. Resolved toward the headers rather than toward the sentence — a `<meta http-equiv="refresh">` does the same job, is not a control, is not something page JavaScript or `evaluate` can drive, and leaves the page strictly stricter than the document describing it. Recorded in `holding_page`'s own doc comment so the next reader meets the reasoning rather than the discrepancy.

## Deviations from Plan

### Auto-fixed issues

**1. [Rule 3 — Blocking] The parked-request type lives in `oauth.rs`, not `app.rs`**

- **Found during:** Task 1
- **Issue:** The plan puts `ConsentRequest` in `app.rs` (Task 2's file) while Task 1's `<files>` is `oauth.rs` alone — but Task 1's `/authorize` cannot park anything without the type. Task 1 could not have compiled.
- **Fix:** The type is defined beside the code that constructs and resolves it, following `control::AgentRequest`'s existing precedent (type in the producing module, hand-written `Debug` because a `oneshot::Sender` is not `Debug`). Task 1 also added the minimum of `app.rs` it needed — `AppEvent::ConsentRequested`, `Shared::pending_consent`, and the `user_event` arm that parks — leaving the panel itself to Task 2.
- **Files modified:** `crates/talaria-shell/src/oauth.rs`, `crates/talaria-shell/src/app.rs`
- **Verification:** both commits build; `cargo test --locked` green at each.
- **Committed in:** `49a2466`

**2. [Rule 3 — Blocking] `http.rs` had to be touched, and the provider takes a callback rather than a proxy**

- **Found during:** Task 1
- **Issue:** `http.rs` is not in the plan's `files_modified`, but `TalariaAuth` is constructed there and `/authorize` has to reach the winit main thread. There was no way to raise a panel without changing that construction.
- **Fix:** Three lines in `http.rs` — `build_router` takes the proxy and builds a `ConsentRaiser` closure from it. Holding the closure rather than the proxy turned out strictly better than the minimal change: `oauth.rs` no longer depends on `winit` or `AppEvent` at all, the layer boundary reads correctly (this module owns *what is asked*, not *how the message travels*), and a test can supply its own raiser — which is what made the whole authorization path, both harassment caps included, unit-testable without a window.
- **Files modified:** `crates/talaria-shell/src/http.rs`, `crates/talaria-shell/src/oauth.rs`
- **Verification:** 13 of the 39 new unit tests drive `handle_authorization` end to end through a stand-in chrome; e2e 21/21 proves the real proxy path.
- **Committed in:** `49a2466`

**3. [Rule 3 — Blocking] Twelve `expect(dead_code)` attributes deleted from `agents.rs`, nine retargeted**

- **Found during:** Task 1
- **Issue:** `agents.rs` is not in the plan's `files_modified` either, but wiring `register` made twelve items live (`register`, `save`, both `to_json`s, `TokenKind::as_str`, `clients`, `random_bytes`, `random_id`, `ID_BYTES`, `MAX_REGISTERED_CLIENTS`, and the `path` and `clients` fields). An `expect` whose item has become live is *unfulfilled*, which is an error under `-D warnings` — so each announced itself rather than having to be hunted for, which is exactly what 04-05 built that arrangement for.
- **Fix:** All twelve deleted. Nine more that stay dead but named 04-06 (`mint`, `stage_token`, `authorize`, `rotate_refresh`, `RefreshOutcome`, `random_token`, `TOKEN_BYTES`, and the two default TTLs) retargeted to 04-07, so every remaining attribute names the plan that will actually wire it. The module header rewritten to describe the arrangement as it now stands.
- **Files modified:** `crates/talaria-shell/src/agents.rs`
- **Verification:** `cargo clippy --all-targets --locked -- -D warnings` exits 0; nine `expect(dead_code)` attributes remain, split 5/4 between 04-07 and 04-08.
- **Committed in:** `49a2466`

**4. [Rule 2 — Missing critical] `HEAD` and `OPTIONS` on `/authorize` park nothing**

- **Found during:** Task 1
- **Issue:** Both verbs are in the SDK's table for the authorization endpoint, and the plan's action does not mention them. A request that raises a consent panel *as a side effect of being probed* is a request an attacker sends in a loop — it would have been a second harassment vector past the caps.
- **Fix:** Non-`GET` returns an empty `200` and parks nothing, with the reasoning in the comment. `probing_the_authorization_endpoint_raises_nothing` asserts it for both verbs.
- **Files modified:** `crates/talaria-shell/src/oauth.rs`
- **Committed in:** `49a2466`

**5. [Rule 2 — Missing critical] A raise that fails gives the slot straight back**

- **Found during:** Task 1
- **Issue:** The slot is claimed before the panel is raised, so a raise that fails — the event loop gone, the browser shutting down — would leave the one-on-screen cap held forever by a request nobody ever saw.
- **Fix:** The failure path clears `parked` and refuses with `access_denied`. `a_request_raised_while_the_chrome_is_gone_is_refused_and_gives_the_slot_back` asserts both halves.
- **Files modified:** `crates/talaria-shell/src/oauth.rs`
- **Committed in:** `49a2466`

**6. [Rule 1 — Bug] Task 2's commit claimed clippy-green and left one file unstaged**

- **Found during:** the close-out sweep
- **Issue:** The two clippy fixes Task 2's gate forced (`err_expect` on five assertions, `assertions_on_constants` on the floor check) are in `oauth.rs`, and only `gui.rs` and `app.rs` were staged for `dc12eb6`. The claim in that commit message is therefore true of the working tree and not of that tree.
- **Fix:** Committed as `cc16ea3`, whose message says plainly that it is `dc12eb6`'s missing hunk rather than new work. Not amended, because `ffc2365` was already on top and rewriting two commits to fix a staging slip is a worse trade than a two-line correction that says what it is.
- **Files modified:** `crates/talaria-shell/src/oauth.rs`
- **Verification:** `cargo clippy --all-targets --locked -- -D warnings` exits 0 at `cc16ea3`.
- **Committed in:** `cc16ea3`

### Deliberate deviations, recorded rather than suppressed

**7. Task 1's commit is not clippy-green; Task 2's is.**

`ConsentDecision`, `ConsentRequest::resolve` and `ConsentRequest::is_abandoned` have no caller in the binary until the consent panel lands in Task 2. The brief offers two ways out — a scoped per-item `expect` naming its owner, or deferring the commit one task and saying which — and the second was taken, because the first would have meant adding three attributes and deleting them two commits later. `CodeRecord`'s three bound fields *did* get the scoped treatment, because their consumer is genuinely 04-07 and the attribute will outlive this plan. This is the same call 04-05 made and recorded from the same position.

**8. `grep -c 'focus_window' crates/talaria-shell/src/app.rs` is 1, not 0.**

The criterion's intent — the consent panel requests attention without stealing focus — holds: the `ConsentRequested` arm calls `request_user_attention(Informational)` and nothing else, with the reasoning in its comment. The one occurrence is pre-existing and correct: `Command::OpenForUser`, the single-instance forward, where a *human* just typed a URL on a command line and focusing the window is the right answer. Deleting it to satisfy a count would have been a regression in behaviour to satisfy a proxy for behaviour.

**9. The bypass name is not spelled out in the suite's prose.**

The plan requires `grep -rc 'TALARIA_AUTO_APPROVE\|auto_approve' crates/talaria-shell/src tests/e2e` to be 0, and also requires the suite to say plainly why no such affordance exists. Naming it in a sentence about its absence would have failed the grep — the same trap the plan itself anticipated for the `TALARIA_CONSENT` doc comment in `oauth.rs`, where it scoped the criterion to the reads. Here the prose was reworded instead ("no auto-approving environment variable exists to be tempted by"), which keeps both the criterion and the explanation.

**10. AUTH-01 is still not marked complete, and `requirements-completed` is empty.**

The plan's frontmatter carries `requirements: [AUTH-01]`, and the summary template asks for those
IDs copied into `requirements-completed`. It reads *"Talaria exposes an OAuth 2.1 authorization
server … (Authorization Code + PKCE, per-client tokens)"* — and `/token` answers `503` until 04-07.
Checking the box now would make the traceability table assert something a reader could not verify,
which is the same call 04-04 and 04-05 both made and recorded from the same position. 04-07 closes
it, with the phase's own success criterion 2 closed in the same plan.

**11. `harness.click_rect` gained a `focus=False` option.**

The arm-delay assertion has to land a real pointer on Approve inside 1000 ms of the request that raised it, and `click_rect`'s unconditional `xdotool windowfocus --sync` plus its 0.3 s settle was a third of that budget. The first run measured 0.82 s against a 0.9 s guard — passing, and one busy runner away from flaky. Focusing once up front through the new `harness.focus_window` and clicking with `focus=False` brought it to 0.31 s across three runs. The default is unchanged, and the new parameter's docstring says when *not* to use it.

---

**Total deviations:** 6 auto-fixed (4 Rule 3 — blocking, 2 Rule 2 — missing critical, 1 Rule 1 — bug; the counts overlap because deviation 6 is a bug in this plan's own bookkeeping), 5 deliberate and recorded.
**Impact on plan:** No scope change. Three of the four blocking fixes were forced by the plan's own file split against its own acceptance criteria; the fourth (deviation 2) improved the layering while it was there. Deviations 4 and 5 close two ways past the harassment caps that the plan did not name.

## Authentication Gates

None. Nothing in this plan needed a credential, an account or an external service.

## Issues Encountered

**The plan's own contract contradicted itself once, and the resolution is recorded above** — `04-UI-SPEC.md` asks the holding page for a status-poll script and, in the same table, forbids inline script through `default-src 'none'`. Kept the header, dropped the script, used a meta refresh. Worth a reviewer's glance because it is the one place where what shipped is *stricter* than the approved contract rather than equal to it.

**The status poll needed a route the SDK's enum has no variant for.** `OauthEndpoint` covers seven endpoint kinds and a consent status poll is none of them. Rather than fork the enum, `/authorize/status` is a second *path* mapped to `OauthEndpoint::AuthorizationEndpoint` — which works because the SDK builds its auth router by folding over the keys of `auth_endpoints()`, so a key is a route, and because that endpoint's verb table (`GET`/`HEAD`/`OPTIONS`) is exactly right for a poll. Recorded in `AUTHORIZATION_STATUS_PATH`'s doc comment. 04-07 and 04-08 should know the pattern exists before inventing a second one.

**`consent_timings()` is read once per listener, not per request.** The plan says "read the test-hooks variable exactly once"; the natural reading is once per call, but reading it at `TalariaAuth::new` is strictly stronger and matches `Gui`'s own precedent for the same variable. A consequence worth carrying: the timings are fixed for the life of a listener, so a test that wanted to change them mid-run would have to restart the shell. None does.

## Threat Flags

None. Every route this plan adds is inside the `<threat_model>` the plan itself carries, and each has its disposition met: `/register` and `/authorize` are the two new unauthenticated network surfaces, both on the same `compose(&[], ..)` chain as 04-05's metadata document (correct — a client that has not registered has no credential to present), both refusing before they act, and neither dereferencing anything a caller supplied. `/authorize/status` reads a `u64` and returns one of four static pages. No new file, no new schema, no dependency change.

## Known Stubs

**`/token` answers `503 temporarily_unavailable` with `"this browser cannot exchange an authorization code yet"`.** Deliberate and named in the plan's scope note: this plan *writes* authorization codes and 04-07 *redeems* them. It is declared rather than absent so the RFC 8414 metadata does not advertise a `token_endpoint` that routes nowhere, and answered rather than left to fall through so a client that follows the metadata gets an OAuth error object instead of a `404`. `AuthorizationCodes::take` is written and unit-tested but carries `expect(dead_code, reason = "redeemed in 04-07")` — 04-07 deletes that attribute as it wires redemption.

Nothing else. No hardcoded empty value, no placeholder, no component drawing from a mock source.

## Deferred Items

- **`AuthorizationCodes::take` compares digests with `==`.** Constant-time comparison at redemption is 04-07's, along with the PKCE verifier check that has the same requirement. The record is keyed by `digest_of(code)` rather than by the code, so what a comparison leaks about is already a hash of a 256-bit random value; the timing-safe compare still belongs in the plan that does the comparing, next to `agents::digests_match`.
- **CHANGELOG.md is untouched.** 04-08's plan owns the phase's entry under AUTH-01, AUTH-02 and AUTH-03 — deliberately one entry for the phase rather than eight competing ones. The project's "log all changes in CHANGELOG.md" rule is met by that plan, not skipped by this one.
- **The consent panel repaints at 5 Hz while it is on screen.** Necessary — the arm delay has to enable a button without input and the expiry has to be noticed without an event that may never come — and the same shape the Access panel's `Starting…` state already uses. It is bounded by the parked-request lifetime, so it is not a background cost. Worth a look if a later phase adds a second self-repainting surface.
- **`04-UI-SPEC.md`'s two `🧪 backstop` rows are still open** — the Access status line at a narrow window size, and whether `PLUGS` and `PLUGS_CONNECTED` are distinguishable at toolbar size. Both belong to the Access panel, which is 04-03's and 04-08's, not this plan's.

## User Setup Required

None. No new config key, no new on-disk file, no external service. The three `TALARIA_CONSENT_*` variables are inert in any build that is not started with `TALARIA_TEST_HOOKS=1`, and are for the e2e harness rather than for a user.

## Verification

All gates run from a clean tree at the plan tip (`cc16ea3`):

| Gate | Result |
|------|--------|
| `cargo build --release --locked` | exit 0 |
| `cargo clippy --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --locked` | **256 pass** (251 shell + 3 protocol + 2 mcp-lib), 0 fail — from 212 |
| `cargo test -p talaria-shell oauth::` | **59 pass**, against a target of 30 |
| `cargo test -p talaria-shell gui::` | 19 pass, including the five `sanitize_claim` tests |
| `python3 tests/e2e/oauth_flow_test.py` | exit 0, both `OAUTH DISCOVERY CHECKS PASSED` and `OAUTH CONSENT CHECKS PASSED`, **60 s** against a 90 s budget |
| `python3 tests/e2e/panel_click_test.py` | exit 0 |
| `python3 tests/e2e/http_transport_test.py` | exit 0, unmodified |
| `python3 tests/e2e/mcp_client_test.py` | exit 0, unmodified |
| `python3 tests/e2e/run_all.py` (Xvfb `:98`) | **21/21 PASS**, `failed: none` |
| `git diff --stat Cargo.toml Cargo.lock crates/talaria-shell/Cargo.toml` | no change |
| `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'` | **1** |

The 9 ignored doc-tests on `crates/talaria-mcp/src/tools.rs` are pre-existing and untouched.

Source criteria — `oauth.rs`: `oauth-authorization-server` 1 (≥1); `S256` 8 (≥2); `TALARIA_TEST_HOOKS` **1**; `env::var("TALARIA_CONSENT` **3**; `reqwest|ureq|Client::new` **0**; `unwrap()` **0**; `frame-ancestors|X_FRAME_OPTIONS` 4 (≥1).
`gui.rs`: `ChromePanel::Consent` 5 (≥4); `fn sanitize_claim` **1**; `truncate_chars` 20 (≥3); `consent.approve|consent.deny` **2**.
`app.rs`: `ChromePanel::Consent` 1 (≥1); `unwrap()` **0** (unchanged from the pre-task baseline of 0).
`gui.rs` + `app.rs`: `raise_consent` 8 (≥3).
`harness.py`: `def pkce_pair|def loopback_callback` **2**.
`oauth_flow_test.py`: `consent.approve|consent.deny` 14 (≥2).
`panel_click_test.py`: `consent` 3 (≥1).
Repo-wide: `TALARIA_AUTO_APPROVE|auto_approve` over `crates/talaria-shell/src` and `tests/e2e` **0**; `ApproveConsent|DenyConsent` over `tools.rs` and `talaria-protocol/src/lib.rs` **0**.

One criterion is not met literally and is argued rather than met: `grep -c 'focus_window' app.rs` is 1, not 0 — see Deviation 8.

## Next Phase Readiness

**04-07 (the token endpoint) has everything it needs.** `AuthorizationCodes::take` removes and returns in one operation and is unit-tested; each `CodeRecord` carries the `client_id`, `code_challenge` and `redirect_uri` it was bound to at minting, and its three fields carry `expect(dead_code, reason = "…in 04-07")` attributes that will error on whichever one redemption forgets to check. The `/token` route is declared and its arm is a single `oauth_error` call to replace. `agents::authorize` and `rotate_refresh` are still dead and still waiting, with their reasons now correctly naming 04-07. Two things to carry: the constant-time digest compare belongs there (`agents::digests_match` is private and will need one visibility word, exactly as `digest_of` did in 04-05), and `/authorize/status` is a second path on an existing endpoint kind rather than a new one — reuse that shape rather than inventing a second.

**04-08 (revocation and the Access panel)** owns `/revoke`, which is neither declared nor advertised here, so declaring it is an insert into `TalariaAuth::new`'s map plus a `revocation_endpoint` key in `authorization_server_metadata` — and the metadata test that asserts its absence today is the one to update, deliberately, rather than delete. It also owns this phase's CHANGELOG entry.

No blockers.

---
*Phase: 04-authenticated-remote-transport-v2*
*Completed: 2026-08-21*

## Self-Check: PASSED

Every file this summary names exists on disk, and all four commits (`49a2466`, `dc12eb6`,
`ffc2365`, `cc16ea3`) are in the history. `git diff --diff-filter=D --name-only dac7f4f..HEAD`
reports **no deleted tracked files** across the plan's whole range. The working tree is clean apart
from a pre-existing untracked handoff note that is not this plan's.

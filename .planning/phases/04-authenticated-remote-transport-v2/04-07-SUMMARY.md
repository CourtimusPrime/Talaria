---
phase: 04-authenticated-remote-transport-v2
plan: 07
subsystem: auth
tags: [oauth, oauth2.1, rfc6749, rfc7636, pkce, refresh-rotation, reuse-detection, tokens]

# Dependency graph
requires:
  - phase: 04-authenticated-remote-transport-v2
    provides: "04-06's `/token` stub, `AuthorizationCodes::mint`/`take` and the remove-and-return contract, `ParkedAuthorization`, `settle`, the `/authorize` validation chain and its consent gate, the shortened-only `TALARIA_CONSENT_*` timings and the fixed 60s code lifetime; 04-05's `TalariaAuth`, `oauth_error`, `digest_prefix`, `canonical_resource`, `TALARIA_SCOPE`, `SharedAgents` and the `handle_request` shape that calls `validate_allowed_methods` itself; 04-04's `Agents` store — `authorize`, `rotate_refresh`, `RefreshOutcome`, `revoke_family`, `digest_of`, `digests_match`, `consumed_at_ms` and the two default TTLs; 04-03's `http.rs` listener and BYO `mcp_routes` mount; 04-02-SPIKE's measured facts about `validate_allowed_methods` and the 4 MiB body cap"
  - phase: 03-table-stakes-browsing
    provides: "`chrome_rects`/`record_rect` behind `TALARIA_TEST_HOOKS`, `click_rect`'s real-pointer technique, and the arm-then-confirm control pattern the consent panel reuses"
provides:
  - "`/token` answered: the authorization-code grant and the refresh-token grant, and no third arm — OAuth 2.1 removed the implicit and password grants"
  - "Single-use redemption that holds under concurrency: the code is removed in the operation that reads it, and a *failed* redemption still leaves it spent"
  - "Constant-time PKCE S256 verification through `agents::digest_of` + `agents::digests_match`, with `challenge_as_digest` re-spelling RFC 7636's base64url as the store's hex — no second SHA-256 and no second comparison in this browser"
  - "`CodeRecord::code_challenge_method`, so the S256 gate at redemption is a real check rather than an assumption about what the authorization endpoint let through"
  - "Refresh rotation and reuse detection wired to the store: reuse revokes the family, and the reuse and unknown answers are byte-identical"
  - "One `400 invalid_grant` for every failure on the token path, a body-size cap, and no `unwrap()` anywhere on it"
  - "`agents::digests_match` is `pub(crate)`"
  - "`tests/e2e/oauth_flow_test.py`'s third section — Success Criterion 2 demonstrated end to end in one automated suite"
affects: [04-08, phase-05]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Consume-then-validate at redemption: `take` first, check the bindings afterwards, and let a refused check leave the code spent"
    - "`handler` / `grant` split — the response wrapper beside a function returning `Result<Value, (StatusCode, &str, &str)>`, so both the document and the refusal are values a test can compare (the shape `register_client` established in 04-06)"
    - "Two spellings of one digest reconciled by a re-speller rather than a second hash function"
    - "e2e: age a credential in the background across an earlier section instead of waiting out a non-overridable lifetime"

key-files:
  created: []
  modified:
    - crates/talaria-shell/src/oauth.rs
    - crates/talaria-shell/src/agents.rs
    - tests/e2e/oauth_flow_test.py

key-decisions:
  - "The token endpoint refuses with `400 invalid_grant`, not `401` — RFC 6749 §5.2's status for this endpoint. The resource server's 401/403 table does not apply where nothing is presenting a bearer token, and a client told `401` looks for a `WWW-Authenticate` challenge a token endpoint has no business sending. Indistinguishability, not the number, is the property that matters, and one `grant_refused()` is what gives it."
  - "A failed redemption consumes the code. `AuthorizationCodes::take` removes the record before any binding is checked, so a code that fails the client, redirect-URI, method or verifier test is gone. A code that survived a refused redemption would be retryable by whoever intercepted it — T-5's attacker gets exactly one guess, and it costs the legitimate client only a fresh authorization."
  - "`CodeRecord` gained `code_challenge_method`. Without it, 'a code whose stored method is not S256 cannot be redeemed' was an assumption about the authorization endpoint rather than a check, and the plan's own prohibition was unassertable."
  - "The challenge is re-spelled, not re-hashed: `challenge_as_digest` base64url-decodes RFC 7636's challenge and hex-encodes it, so the comparison still goes through `agents::digest_of` and `agents::digests_match` and this file defines neither."
  - "The refresh branch does not check `client_id`. The token is the credential and the store binds it to a client; a caller-supplied id would be one more attacker-controlled field to compare and would grant nothing extra."
  - "The e2e asserts code expiry against the code 04-06's consent section collected and deliberately never exchanged, rather than adding a fourth timing knob. The code lifetime is fixed at 60s by 04-06's own decision, and shortening it would have broken this plan's two confinement criteria."

patterns-established:
  - "Consume-then-validate: single use is a property of the store's remove-and-return, and every check that follows runs on a code that is already spent"
  - "One refusal constant per endpoint (`REFUSAL` for the resource server, `GRANT_REFUSAL` for the token endpoint), produced by exactly one function, so indistinguishability is structural rather than reviewed"
  - "Dead-code tripwires are deleted by the plan that fulfils them and retargeted — never blanket-allowed — by the plan that finds them still dead"

requirements-completed: [AUTH-01]

coverage:
  - id: D1
    description: "The token endpoint: the authorization-code grant and the refresh-token grant, with every other grant type refused"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_code_becomes_a_pair_carrying_the_type_the_expiry_and_the_scope"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#an_unsupported_grant_type_is_refused_rather_than_falling_through"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py (EXCHANGED)"
        status: pass
    human_judgment: false
  - id: D2
    description: "An authorization code is single-use, including under concurrent redemption, and a refused redemption still spends it"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#edge_probe_concurrency_two_exchanges_of_one_code_yield_exactly_one_pair"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_code_redeemed_twice_yields_one_pair_and_then_nothing"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_wrong_verifier_is_refused_and_leaves_the_code_spent"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py (SINGLE USE, PKCE)"
        status: pass
    human_judgment: false
  - id: D3
    description: "PKCE S256 verified in constant time, with the challenge method re-checked at redemption and `plain` refused there too"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_code_minted_under_a_method_other_than_s256_cannot_be_redeemed"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_challenge_is_the_same_digest_the_token_store_writes"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_verifier_one_byte_from_correct_is_refused_like_any_other"
        status: pass
      - kind: other
        ref: "grep -c 'fn digest_of|fn sha256|fn constant_time' crates/talaria-shell/src/oauth.rs == 0"
        status: pass
    human_judgment: false
  - id: D4
    description: "The client and redirect-URI bindings made at minting are re-checked at redemption"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_code_presented_by_another_client_is_refused"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_code_presented_with_another_redirect_uri_is_refused"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py (BINDING)"
        status: pass
    human_judgment: false
  - id: D5
    description: "Refresh rotation with reuse detection: a replay revokes the family, a client retrying its current token does not"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_consumed_refresh_token_is_refused_and_takes_its_family_with_it"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_client_rotating_its_current_token_is_not_treated_as_reuse"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#an_unknown_refresh_token_and_a_detected_reuse_are_one_answer"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py (ROTATED, REUSE)"
        status: pass
    human_judgment: false
  - id: D6
    description: "Nothing on the token path panics: a malformed, unparseable, incomplete or oversized body is answered with a status code"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_malformed_or_oversized_token_request_is_answered_rather_than_panicking"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py (MALFORMED GRANT)"
        status: pass
      - kind: other
        ref: "grep -c 'unwrap()' crates/talaria-shell/src/oauth.rs == 0"
        status: pass
    human_judgment: false
  - id: D7
    description: "Success Criterion 2 end to end: endpoint URL → 401 → protected-resource metadata → authorization-server metadata → DCR → /authorize with PKCE and a resource parameter → a human's real pointer click on the real Approve control → /token → an authorized tools/call that opens and closes a real tab"
    requirement: "AUTH-01"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/oauth_flow_test.py (OAUTH FLOW CHECKS PASSED, 104s)"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/run_all.py (21/21)"
        status: pass
    human_judgment: false
  - id: D8
    description: "The token endpoint issues nothing to a client that registered and was never approved"
    requirement: "AUTH-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#a_registration_alone_carries_no_code_and_no_token"
        status: pass
      - kind: e2e
        ref: "tests/e2e/oauth_flow_test.py (UNAPPROVED)"
        status: pass
    human_judgment: false
  - id: D9
    description: "No token, verifier or Authorization header is written to a log on this path"
    verification:
      - kind: other
        ref: "manual read of every log! call in crates/talaria-shell/src/oauth.rs — client_id and digest_prefix only"
        status: pass
    human_judgment: true
    rationale: "A negative grep cannot prove the absence of an interpolation; the plan records this as a code-review item and it stays one."

# Metrics
duration: 55min
completed: 2026-08-21
status: complete
---

# Phase 04 Plan 07: The Token Endpoint Summary

**Authorization codes become credentials: consume-then-validate redemption that is single-use under concurrency, constant-time PKCE S256 through the store's own digest and comparison helpers, and the refresh grant wired to 04-04's rotation and reuse detection — closing AUTH-01 and Success Criterion 2 in one automated suite.**

## Performance

- **Duration:** 55 min
- **Started:** 2026-08-21T03:53:00+04:00
- **Completed:** 2026-08-21T04:48:00+04:00
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments

- **`/token` answers.** 04-06 declared the path and returned `503` so the RFC 8414 metadata never advertised a route that went nowhere; that arm is now the real handler, with the authorization-code grant, the refresh-token grant, and no third arm to write because OAuth 2.1 removed the implicit and password grants outright.
- **Single use is a property of the store, not of good manners.** Redemption goes through `AuthorizationCodes::take`, which removes and returns in one operation under one lock. Two real threads exchanging one code produce exactly one token pair — asserted with `std::thread::scope`, not by inspection.
- **A failed redemption spends the code.** Every binding check runs *after* the code has been taken, so a wrong client, a wrong redirect URI, a non-S256 method or a wrong verifier all leave nothing to retry. The e2e proves it the hard way: present a wrong verifier, then present the *correct* one for the same code, and watch the second attempt fail too.
- **PKCE compared in constant time, with no second helper.** The verifier is hashed with `agents::digest_of` and compared with `agents::digests_match`. RFC 7636 spells a challenge in base64url and the store spells digests in hex, so `challenge_as_digest` re-spells rather than re-hashing — `grep -c 'fn digest_of\|fn sha256\|fn constant_time'` on `oauth.rs` is `0`.
- **The S256 gate at redemption is a real check.** `CodeRecord` gained `code_challenge_method`, recorded at minting and re-checked at redemption, so the second gate holds even if the first were ever edited away.
- **Rotation and reuse detection are called, not reimplemented.** `rotate_refresh` appears exactly once in `oauth.rs`. Reuse revokes the family and returns an answer byte-identical to the one an unknown token gets; a client retrying with its *current* token rotates normally, asserted over three consecutive honest rotations.
- **Success Criterion 2 is closed end to end.** `oauth_flow_test.py`'s third section starts from the MCP endpoint URL and nothing else, reads the token endpoint out of the authorization-server metadata, gets a human's approval by a real pointer click on the real Approve control, exchanges the code, and drives a real tab as the *verified* client id.
- **All four `expect(dead_code, ... 04-07)` tripwires in `oauth.rs` deleted, none worked around** — each one caught a binding redemption would otherwise have skipped, which is exactly what 04-06 built them for. Nine more in `agents.rs` went with them; five that stay dead were retargeted to 04-08.

## Task Commits

Each task was committed atomically:

1. **Task 1: The token endpoint — single-use redemption and constant-time PKCE** — `635f310` (feat)
2. **Task 2: The completed round trip, from the endpoint URL to a driven browser** — `6faa1ed` (test)

**Plan metadata:** see the final `docs(04-07)` commit.

## Files Created/Modified

- `crates/talaria-shell/src/oauth.rs` — the token endpoint (`handle_token`, `token_grant`, `redeem_code`, `refresh_grant`), the `grant_refused` / `challenge_as_digest` / `token_document` / `token_response` helpers, the `CHALLENGE_METHOD` / grant-type / body-cap / `GRANT_REFUSAL` constants, `CodeRecord::code_challenge_method` threaded through `ParkedAuthorization`, `validate_authorization`, `settle` and `AuthorizationCodes::mint`, and 20 unit tests. Module header rewritten: both halves live here now, and only `/revoke` is still to come.
- `crates/talaria-shell/src/agents.rs` — `digests_match` is `pub(crate)`; nine fulfilled `expect(dead_code)` attributes deleted; five that stay dead retargeted to 04-08.
- `tests/e2e/oauth_flow_test.py` — a third section (steps 12–21) completing the flow, plus the token endpoint read out of the AS metadata and 04-06's uncollected code kept for the staleness assertion.

## Decisions Made

**`400 invalid_grant`, not `401`.** The brief said "the same `401`/`invalid_grant` for every failure mode". RFC 6749 §5.2 gives `400` for `invalid_grant`; `401` belongs to `invalid_client`, and the resource server's `401`/`403` table in `04-RESEARCH.md`'s normative requirements is about a server being presented a bearer token, which this endpoint never is. A client told `401` goes looking for a `WWW-Authenticate` challenge. The property the brief was protecting is *indistinguishability*, and one `grant_refused()` function producing one `(StatusCode, &str, &str)` tuple is what delivers it — asserted directly by comparing the reuse and unknown answers as values.

**The refresh branch ignores a presented `client_id`.** The refresh token *is* the credential and the store already binds it to a client and a family. Comparing a caller-supplied id would add an attacker-controlled field and grant nothing; the store's outcome does not expose the client id anyway.

**Redirect-URI matching at redemption is exact, with no loopback-port exception.** `redirect_matches` widens the *registration* comparison by one field because RFC 8252 requires it; RFC 6749 §4.1.3 requires the redemption URI to be identical to the one in the authorization request, and the code recorded that exact string. The unit test presents a different loopback port specifically to pin that the exception does not leak into this comparison.

**A body cap of 8 KiB, not the framework's 4 MiB.** `rust-mcp-axum`'s `max_request_body_size` is sized for MCP payloads. A token request is five short form parameters, and this endpoint is unauthenticated.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 — Blocking] The code lifetime is not one of the overridable timings the plan assumed**

- **Found during:** Task 2 (the e2e round trip)
- **Issue:** The plan's `<interfaces>` states "The **code** lifetime is one of those constants too, which is what lets this plan assert code expiry in seconds rather than a minute." It is not. 04-06 fixed `AUTHORIZATION_CODE_LIFETIME_MS` at 60 s and recorded in its doc comment that it is "deliberately **not** overridable … a fourth knob to a mechanism whose whole defence is how few it has." Adding one would also have broken two of this plan's own acceptance criteria (`TALARIA_TEST_HOOKS` appearing once, `TALARIA_CONSENT` three times).
- **Fix:** Kept the constant untouched and asserted expiry against a code that was *already ageing*: 04-06's consent section mints a code at its step 7 and deliberately never exchanges it. That code is now carried forward as `stale_code`, and step 20 waits out only whatever the intervening ~30 s of suite has not already spent. The comment at both ends says so.
- **Files modified:** `tests/e2e/oauth_flow_test.py`
- **Verification:** `EXPIRED CODE: a code minted 61s ago was refused`; suite total 104 s, inside the 150 s budget. Unit-covered independently by `an_expired_code_cannot_be_redeemed` against a pinned clock.
- **Committed in:** `6faa1ed`

**2. [Rule 2 — Missing critical] `CodeRecord` recorded no challenge method**

- **Found during:** Task 1
- **Issue:** The plan requires "a code whose stored challenge method is anything other than S256 cannot be redeemed", and lists it as a prohibition with a unit test as its verification. `CodeRecord` carried the challenge but not the method, so the check could only ever have been an assumption about what `/authorize` let through — the opposite of a second gate.
- **Fix:** Added `CodeRecord::code_challenge_method`, threaded through `ParkedAuthorization`, `validate_authorization`, `settle` and `AuthorizationCodes::mint`, and re-checked at redemption. Introduced `CHALLENGE_METHOD` so the metadata, the authorization check, the minting and the redemption check are one spelling.
- **Files modified:** `crates/talaria-shell/src/oauth.rs`
- **Verification:** `a_code_minted_under_a_method_other_than_s256_cannot_be_redeemed` stages a `plain` code and asserts it is refused.
- **Committed in:** `635f310`

**3. [Rule 3 — Blocking] RFC 7636 challenges and `agents::digest_of` are two spellings of one hash**

- **Found during:** Task 1
- **Issue:** The plan says to hash the verifier with `agents.rs`'s digest helper and compare to the stored challenge with `agents.rs`'s constant-time helper. `digest_of` returns lowercase hex; a PKCE challenge is base64url-without-padding. A direct comparison could never match.
- **Fix:** `challenge_as_digest` base64url-decodes the stored challenge and hex-encodes it, so the comparison still goes through both of the store's helpers and this file defines neither a hash nor a comparison. A challenge that is not 32 base64url bytes — reachable, because `/authorize`'s well-formedness check admits a few characters base64url does not — lands on the same refusal as everything else.
- **Files modified:** `crates/talaria-shell/src/oauth.rs`
- **Verification:** `a_challenge_is_the_same_digest_the_token_store_writes` checks the re-spelling against RFC 7636 Appendix B's own verifier/challenge pair.
- **Committed in:** `635f310`

**4. [Rule 3 — Blocking] `agents::digests_match` was private**

- **Found during:** Task 1
- **Issue:** Anticipated by 04-06's handover note. The constant-time comparison lives in `agents.rs` and had no visibility.
- **Fix:** `pub(crate)`, exactly as `digest_of` got in 04-05. One word, no new definition.
- **Files modified:** `crates/talaria-shell/src/agents.rs`
- **Verification:** `cargo clippy --all-targets --locked -- -D warnings` exits 0.
- **Committed in:** `635f310`

**5. [Rule 3 — Blocking] The consent panel's arm transition moves the button row**

- **Found during:** Task 2
- **Issue:** The first e2e approval failed with the request still `pending`. Instrumenting it showed why: while Approve is disabled, `gui.rs` renders an extra "Approve turns on in a moment" line above the button row. When the delay elapses that line disappears and the row moves **up by 13 logical points** — from y=435 to y=422. `click_rect` reads the rect from the last frame the shell drew, so a click scheduled off an unarmed frame lands 13 points below the button and does nothing at all. 04-06's suite never hit this because its approval clicks twice: the first click (inside the delay, asserting it grants nothing) leaves the pointer in place, and the second reads an already-armed frame.
- **Fix:** The new `approve()` helper waits 1.25 s — past the 1000 ms delay — so the rect `click_rect` reads is from the armed layout. The comment says what the wait is for and that section 6 remains the place the delay itself is asserted.
- **Files modified:** `tests/e2e/oauth_flow_test.py`
- **Verification:** three approvals in section 3, all landing; suite passes repeatably.
- **Committed in:** `6faa1ed`

**6. [Rule 3 — Blocking] Dead-code expectations became unfulfilled**

- **Found during:** Task 1
- **Issue:** `expect` errors when its lint does not fire, so wiring `authorize`, `rotate_refresh`, `RefreshOutcome`, `revoke_family`, `stage_token`, `random_token`, `TOKEN_BYTES` and the two default TTLs turned nine attributes into clippy errors — the tripwire working as designed.
- **Fix:** Deleted all nine, plus the four in `oauth.rs` (`take` and `CodeRecord`'s three bound fields). Five in `agents.rs` are still genuinely dead (`tokens`, `len`, `is_empty`, `mint`, `revoke_client`) and were **retargeted to 04-08** rather than blanket-allowed — the same call 04-05 and 04-06 both made and recorded from the same position.
- **Files modified:** `crates/talaria-shell/src/agents.rs`, `crates/talaria-shell/src/oauth.rs`
- **Verification:** `cargo clippy --all-targets --locked -- -D warnings` exits 0.
- **Committed in:** `635f310`

---

**Total deviations:** 6 auto-fixed (1 missing critical, 5 blocking)
**Impact on plan:** All six were necessary to make the plan's own assertions assertable or to make its code compile. No scope creep: no new dependency, no new module, no new environment variable, no new config key, no UI surface.

## What the plan's verification section asked the SUMMARY to record

**Does a failed redemption consume the code?** Yes, and deliberately. `AuthorizationCodes::take` removes the record in the operation that reads it, *before* the client, redirect-URI, challenge-method and verifier checks run. So a code that fails any of them is gone. The alternative — putting a failed code back — would give an interceptor unlimited attempts at the verifier, which is the whole of what PKCE is defending. The cost to a legitimate client is one fresh authorization; the cost of the alternative is that T-5's interception stops being useless.

**The access-token lifetime returned to clients.** `expires_in: 3600` — one hour, from `agents::DEFAULT_ACCESS_TTL_MS`, which 04-04 chose as the conventional value the specification's SHOULD leaves open. Refresh tokens carry `DEFAULT_REFRESH_TTL_MS` (thirty days), which bounds an *idle* client since active ones rotate.

**Measured wall-clock runtime of `oauth_flow_test.py`.** **104 s** with all three sections against two shells, up from 60 s for two sections. Roughly 30 s of that is the deliberate wait for the stale code to pass its 60-second lifetime, which is already mostly paid for by the consent section running in front of it. Inside the plan's 150 s budget. `run_all.py` total: 9 m 53 s, 21/21.

## Verification Results

| Check | Result |
|---|---|
| `cargo build --release --locked` | exit 0 |
| `cargo clippy --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --locked` | 276 passed (271 shell + 3 protocol + 2 mcp lib), up from 256 |
| `cargo test -p talaria-shell oauth::` | **79** passed, up from 59 — criterion was ≥ 44 |
| `python3 tests/e2e/oauth_flow_test.py` | exit 0, all three markers, **104 s** (< 150 s) |
| `python3 tests/e2e/run_all.py` | **21/21 PASS** |
| `grep -c 'grant_type' oauth.rs` | 23 (≥ 1) |
| `grep -c 'rotate_refresh' oauth.rs` | 1 (≥ 1) — called, not reimplemented |
| `grep -c 'fn digest_of\|fn sha256\|fn constant_time' oauth.rs` | 0 |
| `grep -c 'reqwest\|ureq\|Client::new' oauth.rs` | 0 — still no outbound HTTP client |
| `grep -c 'unwrap()' oauth.rs` | 0 |
| `grep -c 'TALARIA_TEST_HOOKS' oauth.rs` | 1 — unchanged |
| `grep -c 'TALARIA_CONSENT' oauth.rs` | 3 — unchanged; this plan adds no environment read |
| `grep -c '…CHECKS PASSED' oauth_flow_test.py` | 3 — all three sections' markers survive |
| `grep -rc 'TALARIA_AUTO_APPROVE\|auto_approve' src tests/e2e` | 0 — still no approval bypass anywhere |
| `git diff --stat Cargo.toml Cargo.lock crates/talaria-shell/Cargo.toml` | no change |
| `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'` | **1** |

The 9 *ignored* doc-tests on `crates/talaria-mcp/src/tools.rs` are pre-existing and untouched.
`mcp_client_test.py`, `http_transport_test.py` and `panel_click_test.py` all pass unmodified.

## Threat Model Disposition

| Threat | Disposition | Where it landed |
|---|---|---|
| T-5 (intercepted authorization code) | mitigated | Constant-time S256 verification through `agents::digests_match`; single use under concurrency by remove-and-return; a failed redemption still spends the code; client and redirect-URI bindings re-checked. Unit-tested five ways and asserted end to end at `SINGLE USE`, `PKCE` and `BINDING`. |
| T-9 (replayed refresh token) | mitigated | `rotate_refresh` called, never reimplemented; reuse revokes the family; the honest-retry case asserted over three consecutive rotations. End to end at `REUSE`, including the family's access token ceasing to resolve. |
| T-04-07-01 (token endpoint as an oracle) | mitigated | One `grant_refused()`; the reuse and unknown answers compared as values in a unit test and as parsed bodies in the e2e. |
| T-04-07-02 (a panic on the token path) | mitigated | Zero `unwrap()`; seven malformed bodies unit-tested and six more end to end, each answered with a client-error status. |
| T-10 (a token in a log) | mitigated, code-review item | Every `log!` on this path names a `client_id` or a `digest_prefix`. Recorded as D9 with `human_judgment: true`, per the plan's own prohibition. |
| T-12 (server-side request forgery) | mitigated, re-asserted | `grep -c 'reqwest\|ureq\|Client::new'` on `oauth.rs` is still 0 after this plan's additions. |
| T-04-07-03 (a second environment reader) | mitigated, re-asserted | `TALARIA_TEST_HOOKS` appears once and `TALARIA_CONSENT` three times, all inside `consent_timings`. This plan reads no environment variable at all. |

No new threat surface: this plan adds one HTTP route *behaviour* on a path 04-06 already declared, and no new endpoint, file, socket or process boundary.

## Issues Encountered

The one genuine debugging episode was deviation 5 — the arm-transition relayout. Worth stating plainly because the failure mode is silent: `chrome_rects` reports the last frame the shell *drew*, so any suite that reads a rect and then clicks it after a state change that reflows the panel will click empty space and get no error, only a missing effect. The general lesson for anyone extending this suite: after anything that changes what the panel renders, read the rect from a frame drawn *after* the change, not before it.

## Notes for 04-08

- **`/revoke` is the only endpoint left undeclared**, and `authorization_server_metadata` still advertises no `revocation_endpoint`. Declare and answer them in the same commit, exactly as 04-06 did for `/token`.
- **Reuse the `/authorize/status` shape** if you need a second path on an existing `OauthEndpoint` kind; do not fork the SDK's enum.
- **Five `expect(dead_code, reason = "wired in 04-08")` attributes are waiting for you** in `agents.rs`: `tokens`, `len`, `is_empty`, `revoke_client` and `mint`. Four have an obvious consumer in the Access panel and `/revoke`. **`Agents::mint` does not** — it is a public single-token convenience wrapper over `stage_token` that nothing in the phase plans to call, and both pair-issuing paths stage twice and persist once instead. If 04-08 does not wire it, **delete it rather than retargeting it a third time**.
- **On revoking an open stream** (the one thing 04-03 left for you): nothing was learned here that changes the shape of the problem, but one concrete observation is worth carrying. Family revocation goes through `Agents::revoke_family`, which mutates the token list and nothing else — **the store has no handle on any HTTP session and no way to notify one.** Reuse detection therefore lands on the next request, exactly as `AuthMiddleware`'s per-request `verify_token` guarantees, and this plan's e2e proves that end to end at `REUSE`. It does *not* touch an already-open `text/event-stream`. So terminating a revoked client's open stream will need a second mechanism beside the store — a `token_hash → live session handles` map owned by the listener side (`http.rs` / `Shared::remote`), driven from the same place the Access panel's revoke arm is. Do not expect `agents.rs` to grow it; the store deliberately has no clock, no I/O and no knowledge of transports.
- **Every client this suite creates is a real registration.** The e2e now registers six clients across its three sections against `MAX_REGISTERED_CLIENTS = 32`. If 04-08's revocation section adds many more, watch the cap.

## User Setup Required

None — no external service configuration required.

## Next Phase Readiness

AUTH-01 is complete and Success Criterion 2 is closed: a client that knows only Talaria's MCP endpoint URL discovers the challenge, reads both metadata documents, registers itself, obtains a human's approval by a real pointer click on a native control the network cannot reach, exchanges its code under PKCE, and drives the browser — proven in one automated suite that also proves every way that exchange is meant to fail.

What remains in this phase is 04-08: the revocation endpoint, the Access panel's client list, and terminating a revoked client's open streams (AUTH-02, Success Criterion 3). Nothing in this plan blocks it.

---
*Phase: 04-authenticated-remote-transport-v2*
*Completed: 2026-08-21*

## Self-Check: PASSED

All files claimed above exist on disk; both task commits (`635f310`, `6faa1ed`) are in the log.

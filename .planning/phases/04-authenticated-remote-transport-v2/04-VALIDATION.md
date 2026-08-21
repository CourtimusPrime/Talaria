---
phase: 4
slug: authenticated-remote-transport-v2
# status lifecycle: draft (seeded by plan-phase) → validated (set by validate-phase §6)
status: validated
nyquist_compliant: true
wave_0_complete: false
created: 2026-08-20
reconciled: 2026-08-20  # against 04-01..04-08-PLAN.md; three deviations accepted, see Planner Deviations
---

# Phase 4 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.
> Derived from `04-RESEARCH.md` § Validation Architecture.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework (Rust)** | Built-in `#[cfg(test)]` + `cargo test`. No test-framework crate; `tempfile` is deliberately not a dependency — store tests build unique temp paths by hand (`permissions.rs`, `vault.rs`). |
| **Framework (e2e)** | Python 3 standard library only. No pytest, no third-party deps. |
| **Config file** | None for Rust. `tests/e2e/run_all.py` is the runner; `tests/e2e/harness.py` the shared launcher. |
| **Quick run command** | `cargo test -p talaria-shell` |
| **Full suite command** | `cargo build --release && cargo clippy --all-targets -- -D warnings && cargo test && python3 tests/e2e/run_all.py` |
| **Static gate** | `cargo clippy --all-targets -- -D warnings` (CI runs full `--all-targets`, no allow-list — decision 02-11) |
| **Baseline entering this phase** | e2e 19/19, `cargo test` 117 |
| **Target leaving this phase** | e2e 22/22 (three new suites: `http_transport_test`, `oauth_flow_test`, `revocation_test`) |

---

## Sampling Rate

- **After every task commit:** `cargo test -p talaria-shell` plus `cargo clippy --all-targets -- -D warnings`
- **After every plan wave:** `cargo build --release --locked`, then the affected e2e suite standalone
- **Before `/gsd-verify-work`:** `python3 tests/e2e/run_all.py` fully green plus `cargo test` green
- **Max feedback latency:** ~120 seconds (quick run)

---

## Per-Task Verification Map

| Requirement | Behaviour | Threat Ref | Test Type | Automated Command | File Exists | Status |
|-------------|-----------|------------|-----------|-------------------|-------------|--------|
| AUTH-03 / SC 1 | The HTTP endpoint serves the same nine tools as stdio | — | e2e | `python3 tests/e2e/oauth_flow_test.py` (04-05 T3 step 5) — needs a token, so it lands with the resource server, not the bare listener | ❌ W0 | ⬜ |
| AUTH-03 / SC 1 | `tools/list` over HTTP is byte-identical to `tools/list` over stdio | — | e2e | same suite (04-05 T3 step 5), against a seeded `agents.json` — see Deviation 3 | ❌ W0 | ⬜ |
| AUTH-03 / SC 4 | stdio still works unauthenticated while the listener is on | T-04-SC4 | e2e | `tests/e2e/mcp_client_test.py` **unmodified**, run against a listener-on shell; plus the stdio comparison inside `http_transport_test.py` (04-03 T3) | ✅ | ⬜ |
| AUTH-03 | **Listener absent by default** — nothing bound with no `config.json` | D-04-04 | e2e | `python3 tests/e2e/http_transport_test.py` | ❌ W0 | ⬜ |
| AUTH-03 | When enabled, the listener binds loopback only — never a non-loopback address | D-04-04 | unit + e2e | `cargo test` + same suite | ❌ W0 | ⬜ |
| AUTH-01 | Unauthenticated `/mcp` returns 401 with `WWW-Authenticate: Bearer resource_metadata=…` | high | e2e | `python3 tests/e2e/oauth_flow_test.py` | ❌ W0 | ⬜ |
| AUTH-01 | Protected-resource metadata (RFC 9728) is fetchable and names the AS | high | e2e | same suite | ❌ W0 | ⬜ |
| AUTH-01 / SC 2 | Full discovery → DCR → `/authorize` → chrome approve → `/token` → authorized `tools/call` | high | e2e | `oauth_flow_test.py`, three sections: discovery (04-05 T3), consent through code (04-06 T3), exchange through driven tab (04-07 T2); consent driven via `chrome_rects` + `click_rect` | ❌ W0 | ⬜ |
| AUTH-01 | A wrong PKCE verifier is rejected; the unsafe challenge method is rejected at the authorization endpoint (04-06 T1) and again at redemption (04-07 T1) | high | unit | `cargo test -p talaria-shell oauth::` | ❌ W0 | ⬜ |
| AUTH-01 | An authorization code is single-use and expires, including under concurrent redemption | high | unit | `cargo test -p talaria-shell oauth::` (04-07 T1) | ❌ W0 | ⬜ |
| AUTH-01 | Refresh rotation issues a new token; reusing the old one revokes the family; a client retrying its current token is not caught | high | unit | `cargo test -p talaria-shell agents::` (04-04 T2, the store primitive) + `cargo test -p talaria-shell oauth::` (04-07 T1, the grant) | ❌ W0 | ⬜ |
| AUTH-01 | Redirect URI matches exactly, loopback port excepted (RFC 8252) — at authorization (04-06 T1) and re-checked at redemption (04-07 T1) | high | unit | `cargo test -p talaria-shell oauth::` | ❌ W0 | ⬜ |
| AUTH-01 | A token whose audience is not Talaria is rejected (RFC 8707) | high | unit + e2e | `cargo test` + `oauth_flow_test.py` | ❌ W0 | ⬜ |
| AUTH-02 / SC 3 | Two agents authorized; revoking one 401s its next request while the other still works | high | e2e | `python3 tests/e2e/revocation_test.py` (04-08 T3) | ❌ W0 | ⬜ |
| AUTH-02 / SC 3 | A revoked agent's **open stream closes** | high | e2e | same suite (04-08 T3 step 9); mechanism chosen by `04-02-SPIKE.md`'s A6 finding | ❌ W0 | ⬜ |
| AUTH-02 | The Access panel lists authorized clients; Revoke is clickable by rect | — | e2e | same suite | ❌ W0 | ⬜ |
| AUTH-02 | Revocation survives a shell restart | — | e2e | same suite | ❌ W0 | ⬜ |
| — | `agents.json` lands at `0600` | — | unit | `cargo test -p talaria-shell agents::` | ❌ W0 | ⬜ |
| — | **A corrupt `agents.json` yields an empty client set, not an empty allow** | high | unit | `cargo test -p talaria-shell agents::` (04-04 T1) | ❌ W0 | ⬜ |
| — | `chrome_rects` still refused without `TALARIA_TEST_HOOKS=1`, including new rect names | — | e2e | `tests/e2e/panel_click_test.py` (extend) | ✅ | ⬜ |
| — | An agent cannot navigate a tab to the listener's own origin | high | unit + e2e | `cargo test -p talaria-shell parse_agent_url` (04-03 T2) + `python3 tests/e2e/http_transport_test.py` step 7 (04-03 T3) — **not** `scheme_refusal_test.py`, see Deviation 1 | ❌ W0 | ⬜ |
| — | `Cargo.lock` still pins `primeorder 0.14.0-rc.14` after the dependency additions | — | source assertion | `grep -c '0.14.0-rc.14' Cargo.lock` non-zero; `cargo build --release --locked` (04-02 T1, re-checked 04-03 T2) | ✅ | ⬜ |
| — | The HTTP transport pins the same MCP protocol revision as stdio, from one shared constant | D-04-01 | source assertion | `grep -c 'ProtocolVersion::V2025_11_25' crates/talaria-mcp/src/lib.rs` is 1 and 0 in both call sites (04-03 T2) | ❌ W0 | ⬜ |
| — | The consent timing override can only shorten, is gated on `TALARIA_TEST_HOOKS=1`, and cannot approve a request | high | unit | `cargo test -p talaria-shell oauth::` (04-06 T1): the three clamp tests (over-large, zero, unparseable), plus the floor-values-still-deny test | ❌ W0 | ⬜ |
| — | `CHANGELOG.md` records the phase under AUTH-01, AUTH-02 and AUTH-03 (constraint C-9) | — | source assertion | `grep -c 'AUTH-0[123]' CHANGELOG.md` is at least 3 (04-08 T3) | ✅ | ⬜ |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

*Task IDs are assigned by the planner and are reconciled above against `04-01-PLAN.md` …
`04-08-PLAN.md` as written.*

---

## Planner Deviations From This Document

Three rows above changed during planning. All three were verified against the real files by the
plan-checker and accepted; they are recorded here so the table and the plans agree rather than
diverging silently.

**Deviation 1 — the listener-origin refusal moved out of `scheme_refusal_test.py`.**
This document originally routed "an agent cannot navigate a tab to the listener's own origin" into
`tests/e2e/scheme_refusal_test.py`. It cannot live there. That suite runs in `run_all.py`'s Phase-1
**shared-shell** block, which starts with no `config.json` and therefore has no bound listener — and
the refusal is deliberately keyed on the address actually bound, so that it invents no rule when the
listener is off. Asserting it there would require either enabling the listener for every Phase-1
suite or inventing a refusal that fires against an address nothing is listening on. It lives instead
in `http_transport_test.py` (which owns a shell with the listener on) plus a unit test on
`parse_agent_url` covering bound, unbound, and a different port on the same host.
`scheme_refusal_test.py` is left unmodified, and a source criterion in 04-03 asserts that.

**Deviation 2 — SC 4 runs `mcp_client_test.py` unmodified rather than extending it.**
This document said "(extend)". The plans instead run that suite **as-is** against a shell with the
listener enabled. This is the stronger form: the claim is that turning on a network transport
changes nothing about stdio, and a suite edited to accommodate the new state could not prove that. A
`git diff --stat` on the file reporting no change is itself part of the assertion. The complementary
comparison — that `tools/list` over HTTP matches `tools/list` over stdio — lives in
`http_transport_test.py` and `oauth_flow_test.py`, where a second transport actually exists to
compare against.

**Deviation 3 — 04-05's SC 1 assertion seeds `agents.json` before launch.**
Proving "the HTTP endpoint serves the same tool surface" needs an authorized request, and 04-05 is
the resource-server half: it *verifies* tokens, while minting them is 04-06's and 04-07's. Rather
than deferring SC 1 two more plans, 04-05's suite writes an `agents.json` containing one client and
one token digest before starting the shell — the same seed-the-store-before-launch technique
`history_test.py` and `bookmarks_test.py` already use. This is not a bypass. The seed is a file the
test writes **as the user who owns the process**, and it is exactly what a prior approval leaves on
disk; no code path in the shell treats a seeded record differently from an earned one, and an
attacker who can write that file already owns the account running Talaria. No environment variable,
hook, or branch grants access, and 04-06's own suite starts from an empty store to prove the earned
path end to end.

---

## Plan Assignment

| Plan | Wave | Closes |
|------|------|--------|
| 04-01 | 1 | Tool-surface extraction behind `CommandSink` (makes SC 1 structural) |
| 04-02 | 1 | Dependencies, lockfile pin, the A1 spike, the deferred register |
| 04-03 | 2 | AUTH-03, SC 4 — the listener, default-off and loopback-only |
| 04-04 | 2 | The client and token store; five of the six edge probes |
| 04-05 | 3 | AUTH-01 (resource server), SC 1 — discovery and audience binding |
| 04-06 | 4 | AUTH-01 (authorization server, front half) — metadata, DCR, consent |
| 04-07 | 5 | AUTH-01 (authorization server, back half), SC 2 — token exchange |
| 04-08 | 6 | AUTH-02, SC 3 — revocation and open-stream termination |

---

## Wave 0 Requirements

- [ ] `tests/e2e/http_transport_test.py` — AUTH-03, SC 1, SC 4, and the default-off assertion
- [ ] `tests/e2e/oauth_flow_test.py` — AUTH-01 end to end, including the chrome consent step
- [ ] `tests/e2e/revocation_test.py` — AUTH-02, SC 3, including open-stream closure and restart survival
- [ ] `#[cfg(test)] mod tests` in each new module (`oauth`, `agents`, the listener config), at `vault.rs` density
- [ ] Register all three new suites in `tests/e2e/run_all.py`'s standalone-shell block
- [ ] Extend `panel_click_test.py` (04-06, for the consent rects). Run `mcp_client_test.py` and
      `scheme_refusal_test.py` **unmodified** — see Deviations 1 and 2. For `mcp_client_test.py`
      the unmodified state is load-bearing: `git diff --stat` reporting no change to it *is*
      the SC 4 assertion, so extending it would destroy the property it proves.

---

## Manual-Only Verifications

| Behaviour | Requirement | Why Manual | Test Instructions |
|-----------|-------------|------------|-------------------|
| The consent screen reads clearly enough that a user can tell *what* they are approving and *for whom* | AUTH-01 | Copy quality is human judgment. The mechanism is automated via `chrome_rects`; whether the wording actually informs consent is not machine-checkable. | Trigger an authorization from a fresh client, read the consent surface, confirm it names the client and the scope in terms a non-expert can act on. |
| The Access panel's layout inside the existing chrome | AUTH-02 | egui layout quality is human judgment; no snapshot infrastructure exists and this phase does not add one. | Open the Access panel with clients present and with none; confirm no overlap with the toolbar or tab strip in both Me and Agents views. |

*Phase 3 closed its equivalents by rendering the chrome under Xvfb and reviewing the frames
directly; the same route is available here and is preferred over leaving them open.*

---

## Validation Sign-Off

- [x] All tasks have `<automated>` verify or Wave 0 dependencies
- [x] Sampling continuity: no 3 consecutive tasks without automated verify
- [x] Wave 0 covers all MISSING references
- [x] No watch-mode flags
- [x] Feedback latency < 120s (`cargo test -p talaria-shell` is the per-task gate; the e2e suites are
      per-wave, and `oauth_flow_test.py` is bounded under 150s by 04-06's shorten-only consent
      timing override)
- [x] `nyquist_compliant: true` set in frontmatter

**Approval:** reconciled against `04-01-PLAN.md` … `04-08-PLAN.md`, 2026-08-20. Three deviations
recorded above and accepted.

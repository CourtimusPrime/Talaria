# Phase 2: Harden the Agent Surface — Discussion Log

**Date:** 2026-08-15
**Mode:** auto-selected (user declined the interactive gate; recommended option taken for every area)

For human reference only. Downstream agents read `02-CONTEXT.md`, not this file.

## How this ran

The interactive gray-area question was presented and rejected, then `/gsd-discuss-phase 2` was
re-invoked. Read as "don't gate me" — consistent with the standing preference to skip approval
gates in GSD workflows. Every area below was resolved by taking the recommended option, with
rationale recorded in CONTEXT.md so any of them can be reversed during planning at low cost.

## Areas identified

Six gray areas came out of `analyze_phase`, grounded in `.planning/codebase/CONCERNS.md`:

1. Scheme allowlist policy (MCP-09)
2. Wedged-tab semantics (MCP-10)
3. Credential capture & autofill UX (CRED-02/03)
4. Download bounds & routing (MCP-12)
5. MCP notification scope (AGENT-04)
6. CI scope (TEST-03)

Pipelining (MCP-11) and socket peer auth (SEC-01) were treated as implementation detail rather
than gray areas — the audit's recommended approach was unambiguous in both cases and neither
involves a product judgment call.

## Auto-selected decisions

| # | Area | Question | Selected | Why this over the alternative |
|---|---|---|---|---|
| 1 | Scheme allowlist | Which schemes may agents reach? | `http`/`https`/`about:blank`/`data:`, everything else refused | Refusing by default is the reversible direction; an opt-in escape hatch adds a settings surface this phase doesn't have |
| 2 | Scheme allowlist | Build the opt-in "agents may read files" toggle now? | No — deferred | New capability + new UI surface; belongs in its own slice |
| 3 | Wedged tab | Fail fast, or keep burning the 30s timeout? | Fail fast with a distinct `tab busy` error, timeout retained as outer bound | Converts a 30s stall into an instant actionable error; upstream fix (SpiderMonkey slow-script interrupt) is out of reach |
| 4 | Wedged tab | Do non-script commands survive on a wedged tab? | Yes — `screenshot`/`tabs_close`/`tabs_list`/`tabs_focus` must keep working | This is the takeover path; losing it would break the product's whole reactive backstop |
| 5 | Pipelining | Wire-format change? | No — socket layer only | Protocol already IDs requests and documents out-of-order replies |
| 6 | Pipelining | Fix the stale-connection retry bug in the same change? | Yes | Pipelining makes double-execution of `tabs_open`/`download` *more* likely, not less |
| 7 | Notifications | Owner-filtered or broadcast? | Owner-filtered | Current broadcast leaks other agents' tab IDs; filtering is free now, expensive to retrofit |
| 8 | Notifications | Ordering vs pipelining? | Notifications depend on pipelining — sequence it second | The request/reply mutex structurally cannot raise an unsolicited notification |
| 9 | Credentials | How does an entry get into the vault? | Explicit credentials panel + `Vault::upsert` | Gets `cookies_read` off `{"entries": []}` with no new Servo delegate surface |
| 10 | Credentials | Build the "save password?" prompt on form submit? | No — deferred | Needs Servo form-submission interception; materially bigger than the rest of the phase |
| 11 | Credentials | Autofill inline in the page, or toolbar suggestion? | Toolbar suggestion | Inline injection collides with the agent's own `evaluate` and needs form-field detection |
| 12 | Downloads | Size cap? | 2 GB default, env-overridable, delete partial on exceed | Closes the disk-fill hole cheaply |
| 13 | Downloads | Name collision behaviour? | Uniquify (`file (1).pdf`) | Agent-initiated downloads can't block on a UI the agent can't see; clobbering a user file is unacceptable |
| 14 | Downloads | Route through Servo's network stack now? | No — deferred, documented as a known limitation | Real re-architecture (`ureq` → Servo fetch); Phase 3 builds the storage model anyway |
| 15 | CI | Scope? | Build + clippy `-D warnings` + `cargo test` + Xvfb e2e, on push and PR | e2e carries essentially all real coverage (3 unit tests total) — a build-only CI would be theatre |
| 16 | CI | Guard the `primeorder` pin? | Yes — scheduled job regenerating `Cargo.lock` from scratch | Catches drift deliberately instead of ambushing a fresh checkout |
| 17 | Lock | Close the stale-lock finding here? | Yes — `kill -0` liveness + call `acquire` from preflight | Not a new requirement; closes the audit finding under TEST-02's existing scope |

## Flagged during analysis

- **`HEAD` has no full e2e pass on record.** The 2026-08-16 loop's closing regression was skipped
  to avoid killing a parallel session's processes. Recorded in CONTEXT.md `<specifics>` as the
  first plan's first action — without a green baseline, every failure in this phase is ambiguous.
- **The on-disk `.overnight-lock` holds a dead PID** (2796577, owner `c85a1ef3`). The next loop
  gets refused by a corpse unless it `--force`s.
- **`app.rs` is 1425 lines and most of this phase touches it.** Splitting it is out of scope, but
  parallel plans in that file will collide — noted in `<code_context>` as a sequencing constraint.

## Scope creep redirected

Nine items were captured as deferred ideas rather than folded in — see CONTEXT.md `<deferred>`.
The largest are the form-submit credential prompt, routing `download` through Servo's network
stack, and splitting `app.rs`.

---

*Phase: 2-Harden the Agent Surface*
*Logged: 2026-08-15*

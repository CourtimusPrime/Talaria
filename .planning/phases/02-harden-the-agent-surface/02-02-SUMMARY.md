---
phase: 02-harden-the-agent-surface
plan: 02
subsystem: agent-control-surface
tags: [security, mcp, url-parsing, e2e]
status: complete
requires:
  - "02-01 green e2e baseline at 16411ee"
provides:
  - "Scheme allowlist on the agent URL chokepoint (parse_agent_url)"
  - "Message-shaped refusal naming the rejected scheme"
  - "tests/e2e/scheme_refusal_test.py, registered in run_all.py"
  - "A working human path for user-typed file: URLs"
affects:
  - crates/talaria-shell/src/app.rs
  - tests/e2e/run_all.py
tech-stack:
  added: []
  patterns:
    - "Agent-facing failures stay values (Outcome::Error), never panics"
    - "Agent path and human path remain separate functions"
key-files:
  created:
    - tests/e2e/scheme_refusal_test.py
    - .planning/phases/02-harden-the-agent-surface/deferred-items.md
  modified:
    - crates/talaria-shell/src/app.rs
    - tests/e2e/run_all.py
    - .planning/REQUIREMENTS.md
decisions:
  - "Allowlist is http, https, data, and the exact about:blank string — not the about: scheme"
  - "parse_agent_url returns Result<Url, String> so a policy refusal is not forced into a url::ParseError"
  - "resolve_location gains a file: exception; D-02's unrestricted human path did not actually work before"
  - "MCP-09 is NOT marked complete — the evaluate half of the requirement is still open"
metrics:
  duration: "~25m"
  completed: 2026-08-16
  tasks: 2
  commits: 3
---

# Phase 02 Plan 02: Agent URL Scheme Allowlist Summary

Agent-supplied URLs are now allowlisted to `http`, `https`, `data:` and the literal
`about:blank` at `parse_agent_url`, with a refusal that names the rejected scheme — but the
`evaluate` half of MCP-09 is still open and the requirement stays unchecked.

## What Was Built

**Task 1 — allowlist at the chokepoint** (`a692444`)

`parse_agent_url` changed from `Result<Url, url::ParseError>` to `Result<Url, String>`, because a
scheme refusal is a policy decision, not a parse failure, and the error channel had to be able to
carry a sentence. Parse behaviour is otherwise unchanged: a URL that parses is taken as-is, a
`RelativeUrlWithoutBase` failure on space-free input retries with an `https://` prefix, and every
parse failure maps to `format!("bad url: {error}")` — the exact wire message the existing e2e
suites already assert on, now living in one place instead of being duplicated at both call sites.

A new private helper `agent_scheme_allowed(&Url) -> bool` sits directly above it. `http` and
`https` are the web; `data` is admitted because it grants an agent nothing it cannot already
produce through `evaluate` on a page it controls; the blank page is admitted by exact
`url.as_str() == "about:blank"` match rather than by scheme, because popup adoption registers and
compares that exact literal (`app.rs:1312`, `app.rs:1374`) and admitting the whole `about` scheme
would open the engine's internal pages. Everything else is refused with
`scheme {scheme} is not allowed for agents — use http, https, data:, or about:blank`.

Both call sites now forward the `String` unchanged. `Command::Navigate` keeps its tuple match and
arm ordering exactly as it was; only the third arm's body changed.

**Task 2 — end-to-end proof** (`bafb599`)

`tests/e2e/scheme_refusal_test.py`, a shared-shell suite in the house style, registered in
`run_all.py`'s phase-1 block right after `control_socket_test`. It asserts:

- `tabs_open` refuses `file:`, `javascript:` and `blob:`, each message naming the scheme, and
  `tabs_list` is byte-identical afterward — no tab is created as a side effect.
- `navigate` on a real https tab refuses `file:` the same way, and the tab is still on
  `https://example.com/` afterward. A refusal that leaves a half-navigated tab behind is not a
  refusal.
- Three rejected-scheme requests written back to back before any reply is read each come back with
  their own refusal (the probe-edge concurrency row). `parse_agent_url` holds no state, so this is
  a guard against a future implementation that adds some.
- The positives: `https`, the exact `about:blank` literal, and a `data:` document whose title
  proves it actually rendered.
- `open_for_user` — the wire command a second `talaria <url>` launch sends, which routes through
  `resolve_location` rather than `parse_agent_url` — still opens a local file as a `me` tab. This
  is the assertion that fires if someone later "simplifies" by pointing both paths at the allowlist.

The suite closes every tab it opened, so the shared shell is left as it was found.

## Key Decisions

| Decision | Why |
|----------|-----|
| `Result<Url, String>` rather than a new error enum | Two call sites, both of which immediately stringify into `Outcome::Error`. An enum would add a type for nobody to match on. |
| `about:blank` by exact string, not by scheme | Popup adoption depends on that one literal; the whole `about:` scheme is the engine's internal-page surface (T-02-02-05). |
| `data:` admitted | Same-capability with what `evaluate` can already inject into a page the agent controls. Refusing it removes capability without removing risk (T-02-02-04). |
| MCP-09 left unchecked in REQUIREMENTS.md | The requirement names `evaluate` as well as `navigate`, and the `evaluate` half is demonstrably still open. See below. |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] `resolve_location` could not open a user-typed `file:` URL** (`1c62200`)

- **Found during:** Task 2, while writing the human-path assertion.
- **Issue:** The plan's must-have truth and Task 2's acceptance criteria both assert that
  `resolve_location` "still opens a file: URL typed by the user". It did not. The guard is
  `!url.scheme().is_empty() && url.host().is_some() || url.scheme() == "about"`, and a `file:` URL
  has no host — `Url::parse("file:///tmp/x.html").host()` is `None`. So `file:///tmp/x.html` fell
  through to the https-prefix branch and resolved to `https://file///tmp/x.html`. Typing a local
  path into the URL bar, or launching `talaria file:///tmp/x.html` against a running instance,
  opened a bogus https URL. Pre-existing, unrelated to the allowlist.
- **Fix:** Named `file` alongside the existing `about` exception. Nothing is taken away; this is
  the direction D-02 requires ("resolve_location stays free to open any scheme the user types").
- **Files modified:** `crates/talaria-shell/src/app.rs`
- **Commit:** `1c62200`

The plan's action text says "Do not touch `resolve_location`", scoped to *not applying the
allowlist to it*. Making its stated behaviour true is the opposite of restricting it, and Task 2's
acceptance criteria cannot pass without it.

### Deferred Issues

**`cargo clippy --all-targets -- -D warnings` is red at the phase baseline**

Task 1's acceptance criteria include this command exiting 0. It does not — and it already did not
at `16411ee`, plan 02-01's green baseline, on unmodified code. Seven lints, none in any line this
plan touched (this plan's diff is `app.rs` 838-880, 963, 1003 only):

`enum_variant_names` and `redundant_closure` in `crates/talaria-mcp/src/tools.rs:97,132`;
`unnecessary_cast` ×4 in `crates/talaria-shell/src/app.rs:708,733`; `too_many_arguments` in
`crates/talaria-shell/src/tabs.rs:93`.

Logged in `deferred-items.md`. It belongs to plan 02-11 (CI, D-26), which wires this exact gate
into GitHub Actions and cannot go green until these are resolved. Every plan in this phase
carrying the clippy gate as acceptance inherits the same red baseline.

## Known Gaps

**MCP-09 is half closed. The requirement is NOT marked complete.**

MCP-09 reads "scheme allowlist enforced on `navigate` **and `evaluate`**". This plan closed the
`navigate`/`tabs_open` half. The `evaluate` half is still open, and it was verified open against
the built binary, not assumed:

| Vector via `evaluate` on an agent tab | Result |
|---|---|
| `location.href = 'file:///tmp/x.html'` | **Navigates.** Tab lands on the file URL. |
| `window.open('file:///tmp/x.html')` | **Opens a new tab** at the file URL via popup adoption. |
| `document.body.textContent` after either | **Returns the file's contents** — `SECRET-HTML-abc123` and `SECRET-TXT-def456` came back from `/tmp/probe-secret.html` and `.txt`. |
| `fetch('file:///etc/hostname')` | Refused by Servo — `Network error: Unsupported scheme`. |

So the exfiltration path CONCERNS.md describes is narrowed, not closed: an agent can no longer ask
for a file URL directly, but any agent that can call `evaluate` on any page can still reach and
read the user's disk in two steps.

Closing it is a different design than this plan's, which is why it was not folded in. Servo does
expose the hook — `WebViewDelegate::request_navigation(&self, webview, NavigationRequest)` with
`.allow()` / `.deny()` (`servo-0.4.0/webview_delegate.rs:984`), accepted by default and currently
not implemented by Talaria. Using it raises three questions this plan has no mandate to answer:

1. It fires for every in-page navigation and every nested `<iframe>`, not just agent commands — so
   the policy applies far more broadly than the two command arms.
2. Under takeover, a **human** driving an agent-owned tab would be filtered by the same
   owner-keyed policy. That collides directly with D-02's "the user is the trust root".
3. `request_create_new` (popup adoption, `app.rs:1290`) needs the same check, since
   `window.open('file://…')` demonstrably opens a tab.

Recorded as a blocker in STATE.md and as a partial-status note on MCP-09 in REQUIREMENTS.md.
Recommended: a follow-up plan in this phase, sequenced after the pipelining/notification chain so
it does not collide in `app.rs`.

## Verification

| Check | Result |
|-------|--------|
| `cargo build --release` | exit 0 |
| `cargo test` | exit 0 — 3 tests pass |
| `cargo clippy --all-targets -- -D warnings` | **red, pre-existing** — see Deferred Issues |
| `python3 tests/e2e/run_all.py` (isolated: `TALARIA_E2E_DISPLAY=:98`, private `XDG_RUNTIME_DIR`) | **10/10 PASS, `failed: none`** — the nine 02-01 baseline suites plus `scheme_refusal_test`. No regression. |
| Task 1 source greps (7 criteria) | all pass, incl. `unwrap()` count 0 and `duckduckgo` count 1 |
| Task 2 source greps | `scheme_refusal_test` in run_all.py ×1, sentinel ×1, `open_for_user` ×4 |

## Self-Check: PASSED

- `crates/talaria-shell/src/app.rs` — FOUND (modified)
- `tests/e2e/scheme_refusal_test.py` — FOUND
- `tests/e2e/run_all.py` — FOUND (modified)
- `.planning/phases/02-harden-the-agent-surface/deferred-items.md` — FOUND
- commit `a692444` — FOUND
- commit `1c62200` — FOUND
- commit `bafb599` — FOUND

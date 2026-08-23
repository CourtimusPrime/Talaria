# Requirements: Talaria

**Defined:** 2026-08-15
**Core Value:** An agent can drive a real, already-logged-in browsing session, and a human can take over instantly the moment it hits something only a human can clear — without either side sacrificing performance.

## Overview

Talaria succeeds if a human can use it as a genuine daily-driver browser, and an agent can drive it
directly through MCP with real credentials and real sessions, handing off to the human seamlessly
whenever it hits a wall only a human can clear.

Statuses below were verified against the working tree at commit `5b1f4db` on 2026-08-15, not carried
over from the originating brief — see `.planning/codebase/CONCERNS.md` for evidence and
`.planning/BRIEF.md` for the original claims.

## Status vocabulary

`Complete` and `Pending` mean what they say. Three others do not:

- **Documented Limitation** — the tractable work is done and the remainder is
  blocked on something outside a scheduling decision: an unresolved design
  conflict, or a capability an upstream dependency does not expose. These are
  published in `SECURITY.md` rather than carried as open rows, because "not yet
  scheduled" and "cannot currently be built" are different facts and only one of
  them is actionable.

- **Deferred (v2)** — buildable, understood, and deliberately not in v1 scope.
- **In Progress** — actually being worked on right now.

## v1 Requirements

### Core Engine & Shell (ENGINE / SHELL)

- [x] **ENGINE-01**: System renders web pages using the Servo engine (libservo), not a Chromium-based engine — *Must Have*
- [x] **ENGINE-02**: Web pages load and become interactive within the shell at competitive speed — *Must Have*
- [x] **SHELL-01**: Browser chrome is a native Rust UI (egui-on-winit), independent of any webview or Tauri — *Must Have*
- [x] **SHELL-02**: User can navigate via keyboard shortcuts (new tab, close tab, reload, address-bar focus, tab cycling) — *Should Have*
- [x] **SHELL-03**: A crashed tab shows a recovery panel with a reload action instead of hanging — *Must Have*

### Agent/Human Interaction (AGENT)

- [x] **AGENT-01**: User can toggle between "Me" tabs and "Agents" tabs from a single control — *Must Have*
- [x] **AGENT-02**: While viewing an agent's tab, user input (click/scroll/type) drives that same live session (takeover) — *Must Have*
- [x] **AGENT-03**: Multiple concurrent agent sessions are each visible and distinguishable in the Agents view — *Should Have*
- [x] **AGENT-04**: Tab open/close/crash events reach MCP clients as MCP notifications, not only via polling — *Should Have* — **complete**. Plan 02-07 addressed each event to the one session that owns the tab; plan 02-08 forwards it out of the proxy reader, declares the `logging` capability and emits it as a `notifications/message`. Quick task 260817-kbw closed the remaining `open` slice: `talaria_protocol::Event::TabOpened { tab_id, opener_tab_id }` is raised at `Shared::adopt_popup`, so a popup a driven page opens is announced to the owning session instead of being discoverable only by polling `tabs_list`. `opener_tab_id` is carried because adoption is the only case that raises it — a tab the agent opened itself is already named in the reply to its own `tabs_open`, so the opener is the whole point. Proved end to end at both levels: `tests/e2e/popup_test.py` on the raw control socket, `tests/e2e/mcp_client_test.py` as an MCP `notifications/message`, with the second session's silence still asserted so the new variant cannot fan out

### MCP Tool Surface (MCP)

- [x] **MCP-01**: An agent can open, list, close, and focus tabs via MCP tools — *Must Have*
- [x] **MCP-02**: An agent can navigate a tab to a URL and receive the final loaded URL/title once the page finishes loading — *Must Have*
- [x] **MCP-03**: An agent can run JavaScript via `evaluate`, including async/await and Promise-returning code — *Must Have*
- [x] **MCP-04**: An agent can screenshot any tab, displayed or backgrounded, well under the 60ms takeover latency target — *Must Have* — measured 4–36ms
- [x] **MCP-05**: Malformed or hostile protocol input (junk lines, oversized payloads, out-of-order requests) does not crash or wedge the connection — *Must Have*
- [x] **MCP-06**: An agent can read stored session cookies for a given domain via `cookies_read` — *Must Have* — `crates/talaria-mcp/src/tools.rs:78-87`
- [x] **MCP-07**: An agent can download a file via `download(url, filename)` — *Should Have* — `crates/talaria-mcp/src/tools.rs:89-98`
- [x] **MCP-08**: Popup/new-tab requests from a page (`window.open`, `target=_blank`) open a real tab under the parent's owner — *Must Have* — commit `5b1f4db`, e2e `tests/e2e/popup_test.py`
- [ ] **MCP-09**: Agents cannot navigate to `file://` URLs (scheme allowlist enforced on `navigate` and `evaluate`) — *Must Have* — **partial** (plan 02-02): `parse_agent_url` now allowlists `http`/`https`/`data`/`about:blank`, so `tabs_open` and `navigate` refuse `file:` naming the scheme (`tests/e2e/scheme_refusal_test.py`). The `evaluate` half is still open — `location.href='file://…'` and `window.open('file://…')` from a script reach the filesystem and the content reads straight back out. Closing it needs `WebViewDelegate::request_navigation` + `request_create_new` policy on agent-owned tabs. **Status: Documented Limitation** — the hook that would carry the policy cannot tell a script-initiated navigation on an agent tab apart from one the human triggered by clicking during takeover, so a naive policy there would break the human-is-trust-root rule. Needs provenance in the navigation decision, which is a design change, not an unscheduled task. Published in `SECURITY.md`.
- [ ] **MCP-10**: A heavy-JS page does not wedge `evaluate` — *Must Have* — **partial**: plan 02-06 added per-tab in-flight tracking (`Shared::evaluating`), so a second `evaluate` on a tab whose script thread is already busy is refused instantly with `tab {id} busy — a previous evaluate is still running` instead of burning the command timeout, and `screenshot`/`tabs_close`/`tabs_focus`/`tabs_list` keep answering on that tab. The *first* evaluate still runs to the timeout and never completes: making the evaluate itself complete needs a SpiderMonkey slow-script interrupt exposed through libservo, which is upstream work outside Phase 2. **Status: Documented Limitation** — not fixable in this repository at any effort level until libservo exposes the interrupt. Published in `SECURITY.md`
- [x] **MCP-11**: One slow or wedged tool call does not block tool calls against other tabs — *Must Have* — `ShellConnection`'s single mutex serializes every call, compounding MCP-10
- [x] **MCP-12**: `download` is bounded — enforced size cap and no silent overwrite of an existing file — *Should Have*

### Security & Local IPC (SEC)

<!-- Local IPC hygiene, deliberately NOT the per-agent policy layer ruled out in PROJECT.md Out of Scope. -->

- [x] **SEC-01**: The control socket authenticates its peer (UID check), so another local user cannot drive the browser — *Must Have* — no auth or peer check today; the `/tmp/talaria-$UID.sock` fallback path is world-connectable
- [x] **SEC-02**: Importing credentials removes the plaintext source file rather than leaving it on disk — *Must Have* — plaintext `vault.json` survives import (`crates/talaria-shell/src/vault.rs:113-124`)

### Auth & Credentials (AUTH / CRED)

- [x] **AUTH-01**: Talaria exposes an OAuth 2.1 authorization server for its MCP endpoint (Authorization Code + PKCE, per-client tokens) — *Must Have* — no auth/token/PKCE code exists
- [x] **AUTH-02**: A user can view and individually revoke a connected agent's access — *Should Have*
- [x] **AUTH-03**: An HTTP/SSE MCP transport exists alongside stdio — *Must Have* — prerequisite for AUTH-01; today `rust-mcp-sdk` features are `["server","macros","stdio"]` (`Cargo.toml:45`)
- [x] **CRED-01**: Credentials are stored in a local, domain-keyed file, encrypted at rest — *Must Have* — ChaCha20-Poly1305, keychain with 0600 key-file fallback (`crates/talaria-shell/src/vault.rs`)
- [x] **CRED-02**: Stored credentials are suggested for autofill by domain match in the shell UI; agents never use them for fresh/interactive logins (that's takeover's job) — *Must Have* — **partial**: `matching()` domain keying exists, no autofill UI in `crates/talaria-shell/src/gui.rs`
- [x] **CRED-03**: A user can save a credential from within Talaria — *Must Have* — the vault is read-only in-app and populated only by external import

### Table-Stakes Browsing (BROWSE)

- [x] **BROWSE-01**: User has local browsing history (URL, title, timestamp) — *Should Have*
- [x] **BROWSE-02**: User can bookmark and revisit pages — *Should Have*
- [x] **BROWSE-03**: The address bar searches a **configurable** default engine when input isn't a URL — *Should Have* — **complete**. Phase 3 plan 03-03 replaced the hardcoded DuckDuckGo fallback: `resolve_location` now takes a `&SearchEngine` read from `config.json`, editable from the Settings panel, and a malformed config degrades to the default rather than panicking. The earlier note on this line ("DuckDuckGo is hardcoded at `app.rs:853-869`") described the pre-Phase-3 state and no longer holds.
- [x] **BROWSE-04**: Downloaded files appear in a downloads list the user can open — *Should Have*

### Distributed Mode (DIST)

- [x] **DIST-01**: Client and server can run on separate machines connected via Tailscale — *Must Have*
- [ ] **DIST-02**: A human can watch and take over an agent's tab when the server is remote, with adaptive frame polling (~200–500ms passive, ~30–60ms during takeover) — *Must Have*
- [ ] **DIST-03**: A dropped Tailscale connection does not kill in-progress agent work; the client resyncs on reconnect — *Must Have*
- [ ] **DIST-04**: The server persists a session manifest and offers to restore tabs after a crash or restart in distributed mode — *Should Have*

### Platform Coverage (PLAT)

- [x] **PLAT-01**: Talaria builds and runs on Linux — *Must Have* — de-facto baseline; the Xvfb/`xdotool` e2e suite runs here
- [ ] **PLAT-02**: Talaria builds and runs on macOS — *Must Have* — originally the stated baseline, now unverified
- [ ] **PLAT-03**: Talaria builds and runs on Windows — *Must Have*

### Release Readiness (REL)

- [x] **REL-01**: The repository includes MIT and Apache-2.0 license files matching `Cargo.toml` — *Must Have*
- [ ] **REL-02**: The app has a working update mechanism appropriate to the egui-on-winit shell — *Must Have* — **no approach chosen**; Tauri's updater plugin no longer applies
- [ ] **REL-03**: Screen-reader accessibility (accesskit wiring) is manually verified — *Should Have*
- [ ] **REL-04**: The landing page (hero, About copy, install instructions, GitHub link) is live — *Should Have*

### Testing & Process (TEST)

- [x] **TEST-01**: An end-to-end suite runs headless (Xvfb) and passes cleanly, with a shared harness avoiding display/socket race conditions — *Must Have*
- [x] **TEST-02**: Autonomous overnight loops cannot run two sessions against the same branch concurrently — *Must Have* — `tests/e2e/overnight_lock.py`, commit `ff0000a`. Advisory: nothing auto-enforces the call.
- [x] **TEST-03**: CI runs build, clippy, Rust unit tests, and the e2e suite on every push — *Should Have* — plan 02-01 landed the branch-lock half (D-28/D-29); plan 02-11 wrote both workflows (D-26/D-27) and made the tree clippy-clean, so all four checks exit 0 locally at the phase tip (e2e 14/14, `cargo test` 9/9). **Still open because nothing has run:** the repository has no git remote, so `.github/workflows/ci.yml` has never executed — no run identifier, no green tick. The requirement's verb is "runs". **Closed 2026-08-17.** Remote added (`git@github.com:CourtimusPrime/Talaria.git`), `main` pushed, and both workflows have executed green on a self-hosted runner: `ci.yml` run `32019859735` (build + clippy + `cargo test`, 2m54s) and `e2e.yml` run `32019859744` (14/14 suites PASS, 0 fail). The gate was reshaped in the process — the e2e suite runs on merges to `main` and nightly rather than on every push, because the runner is one machine with one job slot and a timing-sensitive xdotool suite should not block a push that changed no Rust; see the deviation recorded against Phase 2 success criterion 6 in `ROADMAP.md`. The first run also found a real bug the local suite could never have found — `Vault::load()` hanging the main thread on D-Bus autolaunch where no session bus exists — logged in `deferred-items.md`.

## v2 Requirements

Deferred. Tracked but not in the current roadmap.

- **TEST-04**: Meaningful Rust unit-test coverage across `talaria-shell` (currently 3 tests total; the e2e suite carries nearly all verification weight)
- **AGENT-05**: Per-agent session naming/labelling in the Agents view beyond the current distinguishability

## Out of Scope

| Feature | Reason |
|---------|--------|
| Browser extensions | Servo's WebExtensions support is immature; revisit once it matures |
| OS-level multi-window | Depends on a libservo embedding API Servo's own tracker lists as in progress |
| Per-agent permission scoping, rate limiting, audit trails | Product philosophy: the browser is infrastructure, not a policy layer. SEC-01 is local IPC hygiene, a different thing. |
| Rendering-correctness / visual-regression testing | Servo's own WPT conformance is the signal; visual judgment is a poor fit for code-based validation, and takeover is the reactive backstop |
| Cloud sync/backup | Vault and session state are deliberately plain local files that work with existing backup tooling |
| Telemetry | None, by design — stated privacy stance |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| ENGINE-01 | Phase 1 | Complete |
| ENGINE-02 | Phase 1 | Complete |
| SHELL-01 | Phase 1 | Complete |
| SHELL-02 | Phase 1 | Complete |
| SHELL-03 | Phase 1 | Complete |
| AGENT-01 | Phase 1 | Complete |
| AGENT-02 | Phase 1 | Complete |
| AGENT-03 | Phase 1 | Complete |
| MCP-01 | Phase 1 | Complete |
| MCP-02 | Phase 1 | Complete |
| MCP-03 | Phase 1 | Complete |
| MCP-04 | Phase 1 | Complete |
| MCP-05 | Phase 1 | Complete |
| MCP-06 | Phase 1 | Complete |
| MCP-07 | Phase 1 | Complete |
| MCP-08 | Phase 1 | Complete |
| CRED-01 | Phase 1 | Complete |
| PLAT-01 | Phase 1 | Complete |
| REL-01 | Phase 1 | Complete |
| TEST-01 | Phase 1 | Complete |
| TEST-02 | Phase 1 | Complete |
| MCP-09 | Phase 2 | Documented Limitation |
| MCP-10 | Phase 2 | Documented Limitation |
| MCP-11 | Phase 2 | Complete |
| MCP-12 | Phase 2 | Complete |
| SEC-01 | Phase 2 | Complete |
| SEC-02 | Phase 2 | Complete |
| AGENT-04 | Phase 2 | Complete |
| CRED-02 | Phase 2 | Complete |
| CRED-03 | Phase 2 | Complete |
| TEST-03 | Phase 2 | Complete |
| BROWSE-01 | Phase 3 | Complete |
| BROWSE-02 | Phase 3 | Complete |
| BROWSE-03 | Phase 3 | Complete |
| BROWSE-04 | Phase 3 | Complete |
| AUTH-03 | Phase 4 | Complete |
| AUTH-01 | Phase 4 | Complete |
| AUTH-02 | Phase 4 | Complete |
| DIST-01 | Phase 5 | Complete |
| DIST-02 | Phase 5 | Pending |
| DIST-03 | Phase 5.1 | Pending |
| DIST-04 | Phase 5.1 | Pending |
| PLAT-02 | Phase 6 | Pending |
| PLAT-03 | Phase 6 | Pending |
| REL-02 | Phase 7 | Pending |
| REL-03 | Phase 7 | Pending |
| REL-04 | Phase 7 | Pending |

**Coverage:**

- v1 requirements: 47 total
- Mapped to phases: 47
- Unmapped: 0 ✓
- Already complete: 21

---
*Requirements defined: 2026-08-15*
*Last updated: 2026-08-15 after initialization*

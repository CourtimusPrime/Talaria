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
- [ ] **AGENT-04**: Tab open/close/crash events reach MCP clients as MCP notifications, not only via polling — *Should Have* — **partial**: events exist on the control socket (`crates/talaria-protocol/src/lib.rs:71-77`) but the MCP proxy discards them (`crates/talaria-mcp/src/socket.rs:75-77`)

### MCP Tool Surface (MCP)

- [x] **MCP-01**: An agent can open, list, close, and focus tabs via MCP tools — *Must Have*
- [x] **MCP-02**: An agent can navigate a tab to a URL and receive the final loaded URL/title once the page finishes loading — *Must Have*
- [x] **MCP-03**: An agent can run JavaScript via `evaluate`, including async/await and Promise-returning code — *Must Have*
- [x] **MCP-04**: An agent can screenshot any tab, displayed or backgrounded, well under the 60ms takeover latency target — *Must Have* — measured 4–36ms
- [x] **MCP-05**: Malformed or hostile protocol input (junk lines, oversized payloads, out-of-order requests) does not crash or wedge the connection — *Must Have*
- [x] **MCP-06**: An agent can read stored session cookies for a given domain via `cookies_read` — *Must Have* — `crates/talaria-mcp/src/tools.rs:78-87`
- [x] **MCP-07**: An agent can download a file via `download(url, filename)` — *Should Have* — `crates/talaria-mcp/src/tools.rs:89-98`
- [x] **MCP-08**: Popup/new-tab requests from a page (`window.open`, `target=_blank`) open a real tab under the parent's owner — *Must Have* — commit `5b1f4db`, e2e `tests/e2e/popup_test.py`
- [ ] **MCP-09**: Agents cannot navigate to `file://` URLs (scheme allowlist enforced on `navigate` and `evaluate`) — *Must Have* — **partial** (plan 02-02): `parse_agent_url` now allowlists `http`/`https`/`data`/`about:blank`, so `tabs_open` and `navigate` refuse `file:` naming the scheme (`tests/e2e/scheme_refusal_test.py`). The `evaluate` half is still open — `location.href='file://…'` and `window.open('file://…')` from a script reach the filesystem and the content reads straight back out. Closing it needs `WebViewDelegate::request_navigation` + `request_create_new` policy on agent-owned tabs.
- [ ] **MCP-10**: A heavy-JS page does not wedge `evaluate` — *Must Have* — **partial**: a blanket command timeout bounds it (`crates/talaria-shell/src/control.rs:185-201`) but there is no per-tab script isolation
- [ ] **MCP-11**: One slow or wedged tool call does not block tool calls against other tabs — *Must Have* — `ShellConnection`'s single mutex serializes every call, compounding MCP-10
- [ ] **MCP-12**: `download` is bounded — enforced size cap and no silent overwrite of an existing file — *Should Have*

### Security & Local IPC (SEC)

<!-- Local IPC hygiene, deliberately NOT the per-agent policy layer ruled out in PROJECT.md Out of Scope. -->

- [x] **SEC-01**: The control socket authenticates its peer (UID check), so another local user cannot drive the browser — *Must Have* — no auth or peer check today; the `/tmp/talaria-$UID.sock` fallback path is world-connectable
- [ ] **SEC-02**: Importing credentials removes the plaintext source file rather than leaving it on disk — *Must Have* — plaintext `vault.json` survives import (`crates/talaria-shell/src/vault.rs:113-124`)

### Auth & Credentials (AUTH / CRED)

- [ ] **AUTH-01**: Talaria exposes an OAuth 2.1 authorization server for its MCP endpoint (Authorization Code + PKCE, per-client tokens) — *Must Have* — no auth/token/PKCE code exists
- [ ] **AUTH-02**: A user can view and individually revoke a connected agent's access — *Should Have*
- [ ] **AUTH-03**: An HTTP/SSE MCP transport exists alongside stdio — *Must Have* — prerequisite for AUTH-01; today `rust-mcp-sdk` features are `["server","macros","stdio"]` (`Cargo.toml:45`)
- [x] **CRED-01**: Credentials are stored in a local, domain-keyed file, encrypted at rest — *Must Have* — ChaCha20-Poly1305, keychain with 0600 key-file fallback (`crates/talaria-shell/src/vault.rs`)
- [ ] **CRED-02**: Stored credentials are suggested for autofill by domain match in the shell UI; agents never use them for fresh/interactive logins (that's takeover's job) — *Must Have* — **partial**: `matching()` domain keying exists, no autofill UI in `crates/talaria-shell/src/gui.rs`
- [ ] **CRED-03**: A user can save a credential from within Talaria — *Must Have* — the vault is read-only in-app and populated only by external import

### Table-Stakes Browsing (BROWSE)

- [ ] **BROWSE-01**: User has local browsing history (URL, title, timestamp) — *Should Have*
- [ ] **BROWSE-02**: User can bookmark and revisit pages — *Should Have*
- [ ] **BROWSE-03**: The address bar searches a **configurable** default engine when input isn't a URL — *Should Have* — **partial**: DuckDuckGo is hardcoded (`crates/talaria-shell/src/app.rs:853-869`)
- [ ] **BROWSE-04**: Downloaded files appear in a downloads list the user can open — *Should Have*

### Distributed Mode (DIST)

- [ ] **DIST-01**: Client and server can run on separate machines connected via Tailscale — *Must Have*
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
- [ ] **TEST-03**: CI runs build, clippy, Rust unit tests, and the e2e suite on every push — *Should Have* — no CI exists; three Rust unit tests total. Plan 02-01 landed the branch-lock half (D-28/D-29); the CI job itself (D-26/D-27) ships in plan 02-11.

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
| MCP-09 | Phase 2 | Pending |
| MCP-10 | Phase 2 | Pending |
| MCP-11 | Phase 2 | Pending |
| MCP-12 | Phase 2 | Pending |
| SEC-01 | Phase 2 | Complete |
| SEC-02 | Phase 2 | Pending |
| AGENT-04 | Phase 2 | Pending |
| CRED-02 | Phase 2 | Pending |
| CRED-03 | Phase 2 | Pending |
| TEST-03 | Phase 2 | In Progress |
| BROWSE-01 | Phase 3 | Pending |
| BROWSE-02 | Phase 3 | Pending |
| BROWSE-03 | Phase 3 | Pending |
| BROWSE-04 | Phase 3 | Pending |
| AUTH-03 | Phase 4 | Pending |
| AUTH-01 | Phase 4 | Pending |
| AUTH-02 | Phase 4 | Pending |
| DIST-01 | Phase 5 | Pending |
| DIST-02 | Phase 5 | Pending |
| DIST-03 | Phase 5 | Pending |
| DIST-04 | Phase 5 | Pending |
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

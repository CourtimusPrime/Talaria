# Roadmap: Talaria

## Overview

Talaria's foundation — a working Servo-based engine, a native egui-on-winit shell, live agent
takeover, and a fast MCP tool surface — is already built and verified through two overnight
autonomous development loops, and rather more shipped than the originating brief claimed. What
remains is hardening the agent-facing surface and closing the concerns a codebase audit surfaced
(Phase 2), making Talaria a real daily driver (Phase 3), adding an authenticated remote transport
(Phase 4), splitting client and server across machines over Tailscale (Phase 5), covering macOS and
Windows (Phase 6), and shipping it (Phase 7). By the end of Phase 7, Talaria is a publicly
released, cross-platform, open-source browser that humans and AI agents use interchangeably in the
same session.

**Phase ordering note:** OAuth moved from the brief's Phase 2 to Phase 4, immediately before
distributed mode. Local stdio transport needs no auth — auth's actual driver is remote access, and
it depends on adding an HTTP transport that doesn't exist yet.

## Phases

**Phase Numbering:**

- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

- [x] **Phase 1: Foundation & Core Engine** — Servo rendering, egui shell, live agent takeover, a fast MCP tool surface, credential vault, and an Xvfb e2e harness. **Complete.**
- [ ] **Phase 2: Harden the Agent Surface** — Close the audit's open gaps: scheme allowlist, `evaluate` wedging and the serializing mutex, control-socket peer auth, bounded downloads, real MCP notifications, a usable vault, and CI.
- [ ] **Phase 3: Table-Stakes Browsing** — History, bookmarks, configurable search, and a downloads UI, so Talaria works as a real daily driver.
- [ ] **Phase 4: Authenticated Remote Transport** — An HTTP/SSE MCP transport plus the OAuth 2.1 authorization server that becomes possible once it exists.
- [ ] **Phase 5: Distributed Mode** — Client and server split across machines over Tailscale, with remote live-viewing and takeover.
- [ ] **Phase 6: Platform Coverage** — Confirm macOS, then Windows.
- [ ] **Phase 7: Release Readiness** — An update mechanism, manual accessibility verification, and a live landing page.

## Phase Details

### Phase 1: Foundation & Core Engine

**Goal**: A working single-machine browser where a human can browse normally, an agent can drive tabs end-to-end via MCP, and live takeover works.
**Depends on**: Nothing (first phase)
**Requirements**: [ENGINE-01, ENGINE-02, SHELL-01, SHELL-02, SHELL-03, AGENT-01, AGENT-02, AGENT-03, MCP-01, MCP-02, MCP-03, MCP-04, MCP-05, MCP-06, MCP-07, MCP-08, CRED-01, PLAT-01, REL-01, TEST-01, TEST-02]
**Success Criteria** (what must be TRUE):

  1. Startup reaches an MCP-socket-ready state in ~21ms and a page load completes in ~219ms *(measured)*
  2. An agent can drive a tab end-to-end (open → navigate → evaluate → screenshot) and a human can take over mid-session with a real click reflected live *(verified via `xdotool`)*
  3. A multi-hour soak test completes with zero crashes and flat memory *(verified: 719 cycles / 4 hours, RSS steady ~454–478MB)*
  4. The full e2e suite passes on a clean run

**Plans**: Complete — see `OVERNIGHT_LOG.md` and the `overnight/2026-08-15` / `overnight/2026-08-16` branches for the full record.

### Phase 2: Harden the Agent Surface

**Goal**: Every gap the 2026-08-15 codebase audit surfaced is closed — an agent cannot reach the local filesystem, one wedged call cannot stall the server, the control socket is not open to other local users, and the credential vault is actually usable from inside the browser.
**Depends on**: Phase 1
**Requirements**: [MCP-09, MCP-10, MCP-11, MCP-12, SEC-01, SEC-02, AGENT-04, CRED-02, CRED-03, TEST-03]
**Success Criteria** (what must be TRUE):

  1. An agent asked to navigate or evaluate against a `file://` URL is refused with an error, and an e2e test proves it
  2. A heavy-JS page (a real SPA) wedging its own `evaluate` does not block tool calls against any other tab
  3. A process running as a different local UID cannot drive the control socket
  4. A tab crash or close arrives at a connected MCP client as a notification without the client polling
  5. A user can save a credential in Talaria and have it offered back by domain match on a later visit, with no plaintext credential file left on disk
  6. CI runs build, clippy, unit tests, and the e2e suite on every push, and is green

**Plans**: 8/11 plans executed

Plans:

- [x] 02-01-PLAN.md — Green e2e baseline for HEAD + overnight-lock PID liveness and a pre-commit gate (wave 1)
- [x] 02-02-PLAN.md — Scheme allowlist on `parse_agent_url`, refusal naming the scheme, human omnibox untouched (wave 2)
- [x] 02-03-PLAN.md — Control-socket peer-UID check, 0600 socket inside a 0700 per-UID dir, Python path alignment (wave 2)
- [x] 02-04-PLAN.md — Bound `download`: byte cap, uniquifying create, request timeout and cancellation (wave 3)
- [x] 02-05-PLAN.md — Pipeline both sides of the control socket and fix the retry that can re-execute a command (wave 3)
- [x] 02-06-PLAN.md — Per-tab `evaluate` in-flight tracking with instant busy refusal; takeover route stays open (wave 4)
- [x] 02-07-PLAN.md — Owner-addressed event queue + deferred tab-work queue; no silent drops in delegate callbacks (wave 5)
- [x] 02-08-PLAN.md — Surface tab crash/close to MCP clients as notifications, owner-scoped (wave 6)
- [ ] 02-09-PLAN.md — Vault write API, verified plaintext-import cleanup, checked key permissions, user notices (wave 6)
- [ ] 02-10-PLAN.md — Credentials panel and domain-matched toolbar autofill suggestion in the chrome (wave 7)
- [ ] 02-11-PLAN.md — CI pipeline (build, clippy, unit tests, Xvfb e2e) plus a scheduled lockfile-drift audit (wave 8)

### Phase 3: Table-Stakes Browsing

**Goal**: Talaria is usable as an actual daily-driver browser, not just an agent automation surface.
**Depends on**: Phase 2
**Requirements**: [BROWSE-01, BROWSE-02, BROWSE-03, BROWSE-04]
**Success Criteria** (what must be TRUE):

  1. Visited pages appear in a history list with URL, title, and time, and survive a restart
  2. A user can bookmark a page and return to it from a bookmarks list
  3. Typing a non-URL into the address bar searches the user's configured engine, not a hardcoded one
  4. A completed download appears in a downloads list and can be opened from it

**Plans**: TBD

Plans:

- [ ] 03-01: Local history store + UI
- [ ] 03-02: Bookmarks (flat list) + UI
- [ ] 03-03: Configurable default search engine, replacing the hardcoded DuckDuckGo fallback
- [ ] 03-04: Downloads list UI, wired to the `download` tool's storage

### Phase 4: Authenticated Remote Transport

**Goal**: Talaria's MCP endpoint is reachable over the network and protected by its own OAuth 2.1 authorization server, with per-client tokens a user can revoke individually.
**Depends on**: Phase 2
**Requirements**: [AUTH-03, AUTH-01, AUTH-02]
**Success Criteria** (what must be TRUE):

  1. An MCP client can connect over HTTP/SSE, not only stdio, and drive the same tool surface
  2. A new agent client can complete the Authorization Code + PKCE flow against Talaria's own authorization server and receive a working token
  3. A user can list connected agents and revoke one, and that agent's next request is rejected while the others keep working
  4. The stdio transport still works unauthenticated for local use

**Plans**: TBD

Plans:

- [ ] 04-01: Add an HTTP/SSE MCP transport alongside stdio (`rust-mcp-sdk` feature + server wiring)
- [ ] 04-02: OAuth 2.1 authorization server — metadata discovery, dynamic client registration, PKCE, token issuance
- [ ] 04-03: Per-client token storage, revocation, and a connected-agents management UI in the shell

### Phase 5: Distributed Mode

**Goal**: Client and server can run on separate machines over Tailscale, with remote live-viewing and takeover hitting the same latency targets as local mode.
**Depends on**: Phase 4
**Requirements**: [DIST-01, DIST-02, DIST-03, DIST-04]
**Success Criteria** (what must be TRUE):

  1. Client and server on two machines connected via Tailscale can browse and drive agent tabs
  2. Remote takeover latency stays within the ~30–60ms active-takeover target
  3. Killing the Tailscale link mid-agent-task and restoring it does not lose the agent's tab state
  4. Restarting a crashed distributed-mode server offers to restore the prior session's tabs

**Plans**: TBD

Plans:

- [ ] 05-01: Multiplexed WebSocket protocol (frame/input/control/tabs envelope) over Tailscale
- [ ] 05-02: Adaptive screenshot polling (passive vs. active-takeover rates) wired to the WebSocket stream
- [ ] 05-03: Reconnect/resync logic (session grace period, client tab-list resync)
- [ ] 05-04: Session manifest persistence + restore-on-restart for distributed mode

### Phase 6: Platform Coverage

**Goal**: Talaria is confirmed working on macOS and Windows, alongside the existing Linux baseline.
**Depends on**: Phase 3, Phase 5
**Requirements**: [PLAT-02, PLAT-03]
**Success Criteria** (what must be TRUE):

  1. The full e2e suite passes on macOS
  2. The full e2e suite passes on Windows
  3. Startup and screenshot-capture performance on both are within the same order of magnitude as the measured Linux numbers

**Plans**: TBD

Plans:

- [ ] 06-01: macOS verification + e2e pass (harness currently assumes Xvfb/`xdotool`)
- [ ] 06-02: Windows port + e2e pass

### Phase 7: Release Readiness

**Goal**: Talaria is shippable to the public, with a working update path and a live landing page.
**Depends on**: Phase 6
**Requirements**: [REL-02, REL-03, REL-04]
**Success Criteria** (what must be TRUE):

  1. A running older version of Talaria can successfully update itself to a new release
  2. A screen reader can navigate the browser chrome, manually confirmed by a human
  3. The landing page is live with the agreed hero ("Fly with Talaria" / "The lightest browser you'll ever need."), About copy, install instructions, and GitHub link

**Plans**: TBD

Plans:

- [ ] 07-01: Choose and implement an update mechanism for the egui-on-winit shell (**open decision** — `self_update`, `cargo-dist`, or version-check-and-replace; Tauri's plugin no longer applies)
- [ ] 07-02: Manual accessibility pass against the accesskit wiring
- [ ] 07-03: Deploy the landing page

## Progress

**Execution Order:**
Phases execute in numeric order: 1 → 2 → 3 → 4 → 5 → 6 → 7

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Foundation & Core Engine | — | ✅ Complete | 2026-08-15 |
| 2. Harden the Agent Surface | 8/11 | In Progress|  |
| 3. Table-Stakes Browsing | 0/4 | Not started | - |
| 4. Authenticated Remote Transport | 0/3 | Not started | - |
| 5. Distributed Mode | 0/4 | Not started | - |
| 6. Platform Coverage | 0/2 | Not started | - |
| 7. Release Readiness | 0/3 | Not started | - |

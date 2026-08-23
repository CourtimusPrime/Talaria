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

## Release boundary: v1 is Phases 1–3

**v1 = Phases 1, 2, 3.** v2 = Phases 4, 5, 6, 7.

Phases 1–3 produce the thing described at the top of this file: a Servo-based browser a person can
use every day, that an agent can drive over MCP, with live takeover, a hardened agent surface, and
the table-stakes features (history, bookmarks, search, downloads) whose absence would otherwise make
"daily driver" a false claim. That is shippable, and it is the whole core value proposition.

Phases 4–7 are not phases in the same sense — each is its own product. Phase 4 is an OAuth 2.1
authorization server. Phase 5 is a distributed remote-framebuffer protocol held to a 30–60ms
interaction budget. Phase 6 is a Windows port of a Servo application. Phase 7 depends on REL-02, an
update mechanism that has had no chosen approach since project init. Any one of them could take
longer than Phases 1–3 combined did.

Holding v1 hostage to all four is how a working browser never ships. They keep their numbers and
their detail below; they are simply not v1.

## Phases

**Phase Numbering:**

- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

- [x] **Phase 1: Foundation & Core Engine** — Servo rendering, egui shell, live agent takeover, a fast MCP tool surface, credential vault, and an Xvfb e2e harness. **Complete.**
- [x] **Phase 2: Harden the Agent Surface** — Close the audit's open gaps: scheme allowlist, `evaluate` wedging and the serializing mutex, control-socket peer auth, bounded downloads, real MCP notifications, a usable vault, and CI.
- [x] **Phase 3: Table-Stakes Browsing** *(v1)* — History, bookmarks, configurable search, and a downloads UI, so Talaria works as a real daily driver. (completed 2026-08-20)
- [x] **Phase 4: Authenticated Remote Transport** *(v2)* — An HTTP/SSE MCP transport plus the OAuth 2.1 authorization server that becomes possible once it exists. (completed 2026-08-21)
- [ ] **Phase 5: Distributed Mode** *(v2)* — Client and server split across machines over Tailscale, with remote live-viewing and takeover.
- [ ] **Phase 6: Platform Coverage** *(v2)* — Confirm macOS, then Windows.
- [ ] **Phase 7: Release Readiness** *(v2)* — An update mechanism, manual accessibility verification, and a live landing page.

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
     — **met with a recorded deviation.** Build, clippy and unit tests run on every push
     (`ci.yml`). The e2e suite runs on every merge to `main`, nightly, and on demand
     (`e2e.yml`) rather than on every push. Two reasons, both discovered when CI first ran
     against a real runner: the suite drives a real browser under Xvfb with `xdotool`, so a
     timing-sensitive suite going red would block pushes that changed no Rust at all; and the
     self-hosted runner is one machine with one job slot, so an e2e run on every push
     serialises every later push behind it. The coverage is not reduced — no commit reaches
     `main` without the full suite having run against it. Neither workflow carries a `paths:`
     filter, so no push can produce a green tick by verifying nothing.

**Plans**: 11/11 plans executed — **Phase complete 2026-08-17**

Plans:

- [x] 02-01-PLAN.md — Green e2e baseline for HEAD + overnight-lock PID liveness and a pre-commit gate (wave 1)
- [x] 02-02-PLAN.md — Scheme allowlist on `parse_agent_url`, refusal naming the scheme, human omnibox untouched (wave 2)
- [x] 02-03-PLAN.md — Control-socket peer-UID check, 0600 socket inside a 0700 per-UID dir, Python path alignment (wave 2)
- [x] 02-04-PLAN.md — Bound `download`: byte cap, uniquifying create, request timeout and cancellation (wave 3)
- [x] 02-05-PLAN.md — Pipeline both sides of the control socket and fix the retry that can re-execute a command (wave 3)
- [x] 02-06-PLAN.md — Per-tab `evaluate` in-flight tracking with instant busy refusal; takeover route stays open (wave 4)
- [x] 02-07-PLAN.md — Owner-addressed event queue + deferred tab-work queue; no silent drops in delegate callbacks (wave 5)
- [x] 02-08-PLAN.md — Surface tab crash/close to MCP clients as notifications, owner-scoped (wave 6)
- [x] 02-09-PLAN.md — Vault write API, verified plaintext-import cleanup, checked key permissions, user notices (wave 6)
- [x] 02-10-PLAN.md — Credentials panel and domain-matched toolbar autofill suggestion in the chrome (wave 7)
- [x] 02-11-PLAN.md — CI pipeline (build, clippy, unit tests, Xvfb e2e) plus a scheduled lockfile-drift audit (wave 8)

### Phase 3: Table-Stakes Browsing

**Goal**: Talaria is usable as an actual daily-driver browser, not just an agent automation surface.
**Depends on**: Phase 2
**Requirements**: [BROWSE-01, BROWSE-02, BROWSE-03, BROWSE-04]
**Success Criteria** (what must be TRUE):

  1. Visited pages appear in a history list with URL, title, and time, and survive a restart
  2. A user can bookmark a page and return to it from a bookmarks list
  3. Typing a non-URL into the address bar searches the user's configured engine, not a hardcoded one
  4. A completed download appears in a downloads list and can be opened from it

**Plans**: 4/4 plans executed

Plans:

- [x] 03-01-PLAN.md — Local history store (JSONL append-log) + capture queue + ChromePanel/History UI (wave 1)
- [x] 03-02-PLAN.md — Bookmarks store (flat list) + toggle + Bookmarks UI (wave 2)
- [x] 03-03-PLAN.md — Configurable search engine (settings.rs) replacing the hardcoded DuckDuckGo fallback + Settings UI (wave 3)
- [x] 03-04-PLAN.md — Downloads store + EventLoopProxy plumbing + Downloads UI, wired to the `download` tool's storage (wave 4)

### Phase 4: Authenticated Remote Transport *(v2)*

**Goal**: Talaria's MCP endpoint is reachable over the network and protected by its own OAuth 2.1 authorization server, with per-client tokens a user can revoke individually.
**Depends on**: Phase 2
**Requirements**: [AUTH-03, AUTH-01, AUTH-02]
**Success Criteria** (what must be TRUE):

  1. An MCP client can connect over HTTP/SSE, not only stdio, and drive the same tool surface
  2. A new agent client, given only Talaria's MCP endpoint URL, receives a 401 carrying
     `WWW-Authenticate: Bearer resource_metadata=…`, discovers Talaria's authorization server
     through the protected-resource metadata document, completes Authorization Code + PKCE
     (S256) with the `resource` parameter set to Talaria's canonical URI, and receives an
     access token that Talaria accepts on `/mcp` and rejects on any other audience
     — *reworded during Phase 4 planning; the original wording permitted a plan that minted
     a token and never implemented discovery or audience binding, which are what make the
     flow findable and the token non-transferable*

  3. A user can list connected agents and revoke one, and that agent's next request is rejected while the others keep working
  4. The stdio transport still works unauthenticated for local use

**Plans**: 8/8 plans executed

Plans:

- [x] 04-01-PLAN.md
- [x] 04-02-PLAN.md
- [x] 04-03-PLAN.md
- [x] 04-04-PLAN.md
- [x] 04-05-PLAN.md
- [x] 04-06-PLAN.md
- [x] 04-07-PLAN.md
- [x] 04-08-PLAN.md

*Split expanded from 3 plans to 7 during planning (decision D-04-03 in `04-CONTEXT.md`), then to 8
after plan review. The original 3-plan split put metadata discovery, DCR, PKCE and token issuance in
a single plan. The two additions carrying the most weight are 04-01, which makes "the same tool
surface" structurally true rather than reviewed-for, and 04-02, which lands the dependency and
lockfile change as its own task because `Cargo.lock` is load-bearing here. The eighth plan came from
the plan-checker: the authorization server was still one plan carrying metadata, registration, a
five-step validation chain, a consent panel, the code store, constant-time PKCE, refresh rotation
and an anti-harassment state machine — several times any peer task. It splits at the code: 04-06
**writes** authorization codes, 04-07 **redeems** them.*

- [x] 04-01: Extract the tool surface into `talaria-mcp/src/lib.rs` behind a `CommandSink` trait — no behaviour change (wave 1)
- [x] 04-02: Dependency + lockfile landing; verify `primeorder 0.14.0-rc.14` survives and CI `--locked` stays green; settle the `rust-mcp-axum` routing spike (wave 1)
- [x] 04-03: Streamable-HTTP listener in the shell — off by default, loopback-bound; `parse_agent_url` refuses its own origin (wave 2) — AUTH-03, SC 4
- [x] 04-04: `agents.rs` — client registry and hashed opaque token store, atomic `0600` writes, no HTTP (wave 2)
- [x] 04-05: Resource-server half — `impl AuthProvider`, RFC 9728 metadata, 401 + `WWW-Authenticate`, audience validation (wave 3) — SC 1
- [x] 04-06: Authorization server, front half — RFC 8414 metadata, DCR, `/authorize` validation, the chrome consent panel, and the holding page (wave 4)
- [x] 04-07: Authorization server, back half — `/token` with constant-time PKCE S256, single-use code redemption, refresh rotation (wave 5) — AUTH-01, SC 2
- [x] 04-08: Revocation — `/revoke` (RFC 7009), the Access panel's client list, and terminating a revoked client's open streams (wave 6) — AUTH-02, SC 3

### Phase 5: Distributed Mode *(v2)*

**Goal**: A remote client on another machine can watch and take over an agent's tab over Tailscale, hitting the active-takeover latency target on a direct link.
**Depends on**: Phase 4
**Requirements**: [DIST-01, DIST-02]
**Success Criteria** (what must be TRUE):

  1. A `talaria-client` on a second machine, connected via Tailscale, can list, watch and drive the
     server's **agent** tabs — Me tabs are refused server-side, not merely hidden (decision D-05-02)

  2. Remote takeover latency stays within the ~30–60ms active-takeover target **on a direct
     WireGuard path**, and on a relayed (DERP) path the client degrades the frame rate and tells the
     user rather than silently missing the target or refusing takeover (decision D-05-06)

  3. Local mode is provably unaffected — the existing single-process path keeps Phase 1's measured
     latency

**Plans**: 1/11 plans executed

*Originally one phase covering DIST-01…04. Split during planning: an architectural client/server
split plus a frame pipeline plus reconnect plus persistence was more than one reviewable phase, and
Phase 4 needed eight plans for less. Reconnect and persistence moved to Phase 5.1.*

*`05-RESEARCH.md` sketched ~10 plans for all four requirements and said four was "too few"; this
phase carries roughly half that scope in eleven, because the half it carries is the dense half — a
transport, a codec, an input channel and a whole second front end, plus five of the seventeen
high-severity threats. Three additions over the research's own sketch: the two required spikes are
separate plans rather than one (the WebSocket-through-Serve spike needs the dependency landed first,
the readback spike does not, so they parallelise), the client binary is two plans rather than one
(the graphics-context construction has no analog in this codebase and the research's own pattern map
flags it as the single unmapped step), and the documentation the phase owes gets its own closing plan
rather than riding on the last functional one — the arrangement Phase 4 used, where the changelog
obligation went unaddressed until 04-08.*

Plans:

- [x] 05-01-PLAN.md — Dependency + lockfile landing (`axum` with WebSockets, one added package); spike A2, whether Tailscale Serve proxies an upgrade and what `Host` a proxied request carries; open `deferred-items.md` (wave 1) — DIST-01
- [ ] 05-02-PLAN.md — Spike A1/A8: GL readback cost at cadence under Xvfb and on real hardware, and the tile-diff/PNG figures re-measured against real Servo output (wave 1) — DIST-02
- [ ] 05-03-PLAN.md — `talaria-protocol`: the `wire` module (envelope, frame header, input and view messages), the Unix-only helpers moved into `local`, and the two module headers that call this crate the distributed wire (wave 1) — DIST-01
- [ ] 05-04-PLAN.md — Advertised base URL as one source feeding the issuer, the canonical resource, the four endpoint URLs and the Host allowlist; `https` on every published OAuth URL; `BIND_HOST` unchanged; `scripts/tailscale-serve.sh` (wave 2) — DIST-01
- [ ] 05-05-PLAN.md — The `/view` WebSocket route with **its own** bearer check, registered for revocation; the view session, the agent-only tab lookup and the attachment cap; `remote_view_test.py` and a stdlib WebSocket client in the harness (wave 3) — DIST-01
- [ ] 05-06-PLAN.md — Remote input: the three forwarders generalised, `keyboard_event_from_wire`, `remote_input.rs` with its four structural refusals, and the chrome-unreachable and coordinate-lands assertions (wave 4) — DIST-02
- [ ] 05-07-PLAN.md — `talaria-client`: the crate, its graphics context without Servo, the verified secure WebSocket, the connection state, the agent tab list and the honest first-run copy (wave 4) — DIST-01
- [ ] 05-08-PLAN.md — Frame pump: the visibility hold, unconditional paint on the tick, off-thread tile diff and `Compression::Fast` encode, keyframes; `encode_screenshot` untouched (wave 5) — DIST-02
- [ ] 05-09-PLAN.md — Client present and capture: keyframes and deltas, frame ordering, one fit transform inverted for the pointer; a real client driven by a real pointer, asserted on the server (wave 6) — DIST-01, DIST-02
- [ ] 05-10-PLAN.md — The rung ladder in one ordered table, the hysteretic controller, the honest report, `link_shim.py` and `remote_latency_test.py` (wave 7) — DIST-02
- [ ] 05-11-PLAN.md — `SECURITY.md`'s fifth party and the CSWSH-by-construction property, `CHANGELOG.md`, the corrected wire claim, `scripts/two-machine-check.sh`, and Success Criterion 3's collected evidence (wave 8) — DIST-01, DIST-02

### Phase 5.1: Distributed Resilience *(v2)*

**Goal**: A distributed session survives a dropped link and a server restart without losing the agent's work.
**Depends on**: Phase 5
**Requirements**: [DIST-03, DIST-04]
**Success Criteria** (what must be TRUE):

  1. Killing the Tailscale link mid-agent-task and restoring it does not lose the agent's tab state;
     the client resyncs on reconnect

  2. Restarting a crashed distributed-mode server offers to restore the prior session's tabs

**Plans**: TBD

Plans:

- [ ] 05.1-01: Reconnect/resync logic (session grace period, client tab-list resync)
- [ ] 05.1-02: Session manifest persistence + restore-on-restart for distributed mode

### Phase 6: Platform Coverage *(v2)*

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

### Phase 7: Release Readiness *(v2)*

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
Phases execute in numeric order: 1 → 2 → 3 (**ship v1**) → 4 → 5 → 6 → 7

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Foundation & Core Engine | — | ✅ Complete | 2026-08-15 |
| 2. Harden the Agent Surface | 11/11 | ✅ Complete | 2026-08-17 |
| 3. Table-Stakes Browsing | 4/4 | Complete    | 2026-08-20 |
| 4. Authenticated Remote Transport | 8/8 | Complete    | 2026-08-21 |
| 5. Distributed Mode | 1/11 | In Progress|  |
| 6. Platform Coverage | 0/2 | Not started | - |
| 7. Release Readiness | 0/3 | Not started | - |

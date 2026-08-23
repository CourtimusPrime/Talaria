# Talaria

## What This Is

Talaria is an open-source, non-Chromium web browser built on the Servo engine (via libservo),
designed for both human daily use and AI agents to drive directly through a built-in MCP server.
A single toggle switches the view between the user's own tabs ("Me") and tabs an agent is actively
driving ("Agents"), and the user can take over an agent's live session at any moment — click,
scroll, type — in the same session, not a handoff to a copy.

## Core Value

An agent can drive a real, already-logged-in browsing session, and a human can take over instantly
the moment it hits something only a human can clear (a login wall, a CAPTCHA, a Cloudflare
challenge), without either side sacrificing performance.

## Requirements

### Validated

<!-- Shipped and confirmed. Phase 1 items verified by two overnight test-and-improve loops;
     the rest verified against the working tree on 2026-08-15 (see .planning/codebase/). -->

- ✓ Servo/libservo rendering engine boots, loads pages, and holds sessions reliably — Phase 1
- ✓ egui-on-winit shell (chrome UI) is functional, fast, and keyboard-navigable — Phase 1
- ✓ Me/Agents toggle and live takeover work end-to-end (verified via `xdotool`, real click → real navigation) — Phase 1
- ✓ Session/profile state persists across restarts — Phase 1
- ✓ Core MCP tool surface (`tabs_*`, `navigate`, `evaluate` incl. async/Promises, `screenshot`) works, is fast (screenshot 4–36ms, well under the 30–60ms takeover target), and is hardened against malformed protocol input — Phase 1
- ✓ Multiple concurrent agent sessions are distinguishable in the Agents view — Phase 1
- ✓ Tab-level crash recovery and an Xvfb-based e2e harness both work reliably — Phase 1
- ✓ `window.open` / `target=_blank` popups open a real tab under the parent's owner — commit `5b1f4db`, `crates/talaria-shell/src/app.rs:1282`
- ✓ Credential vault encrypted at rest (ChaCha20-Poly1305, keychain with 0600 key-file fallback) with domain-keyed matching — `crates/talaria-shell/src/vault.rs`
- ✓ `cookies_read` and `download` MCP tools exist and work — `crates/talaria-mcp/src/tools.rs:78-98`
- ✓ `.overnight-lock` preflight blocks a second loop session on the same branch — commit `ff0000a`, `tests/e2e/overnight_lock.py`
- ✓ `LICENSE-MIT` and `LICENSE-APACHE` in repo root, matching `Cargo.toml` `license = "MIT OR Apache-2.0"`

### Active

<!-- Current scope. Building toward these. -->

- [ ] Close the two open safety gaps: `file://` scheme allowlist on agent navigation, and `evaluate` wedging on heavy-JS pages
- [ ] Remove the single-mutex bottleneck in `ShellConnection` that serializes every MCP tool call (turns one wedged `evaluate` into a whole-server stall)
- [ ] Authenticate the control socket (peer-UID check) so another local user can't drive the browser
- [ ] Bound `download` (size cap, overwrite protection)
- [ ] Bridge shell tab events (`TabCrashed`, `TabClosed`) into real MCP notifications instead of discarding them in the proxy
- [ ] Finish the credential vault: a write/capture path, plaintext-import cleanup, and domain-matched autofill in the shell UI
- [ ] Table-stakes browsing: history, bookmarks, configurable search engine, downloads UI
- [x] OAuth 2.1 authorization server (`rust-mcp-sdk`) plus an HTTP/SSE transport — shipped in Phase 4; the listener is off by default, loopback-only and bearer-authenticated
- [x] Distributed mode: Tailscale client/server split, remote live-viewing and takeover over the designed WebSocket protocol — shipped in Phase 5 (DIST-01, DIST-02). Reconnect/resync (DIST-03, DIST-04) are Phase 5.1's
- [ ] Platform coverage: confirm macOS, then Windows (Linux is the de-facto current baseline)
- [ ] Release readiness: an update mechanism, manual accessibility verification, landing page live
- [ ] Continuous integration — there is none, and only three Rust unit tests exist

### Out of Scope

<!-- Explicit boundaries. Includes reasoning to prevent re-adding. -->

- Browser extensions (v1) — Servo's WebExtensions support is immature; revisit once it matures
- OS-level multi-window support (v1) — rests on a part of libservo's embedding API Servo's own tracker lists as still in progress
- Per-agent permission scoping, rate limiting, request-level audit trails — deliberate product philosophy: the browser is infrastructure, not a policy/safety layer. (Note: the control-socket peer-UID check in Active is *not* this — it is local IPC hygiene against other OS users, not a policy layer over an authorized agent.)
- Proactive rendering-correctness or visual-regression testing — treated as Servo's own responsibility (tracked via their WPT suite); this is a visual-judgment problem, a poor fit for code-based validation, and the reactive backstop (live-viewing/takeover) already covers it
- Cloud sync/backup — credential vault and session state are deliberately plain local files, which already work with whatever backup tooling the user has
- Telemetry — none, by design, stated explicitly as a privacy stance

## Context

- Built as a direct reaction against Chromium's resource weight and against the fact that the
  entire current AI-agent browsing ecosystem (ego-lite, OpenBrowser, orion-browser, steel-browser,
  agent-browser) automates Chromium rather than replacing it. Talaria is the only project in this
  space betting on a genuinely different, non-Chromium engine.
- Two prior working names: "OpenBrowser"/"Obie" (dropped — collided with an existing, unrelated
  open-source project) and "Charon" (dropped in favor of the current name). "Talaria" (Hermes'
  winged sandals) was chosen for fitting the messenger/agent theme and the speed/lightness
  positioning more directly.
- Built by Claude Fable 5 working autonomously in structured overnight test-and-improve loops
  (branch-per-run, one commit per fix, hard iteration caps, judgment calls logged and escalated
  rather than decided autonomously). Full record in `OVERNIGHT_LOG.md`.
- A real incident on 2026-08-16: two Claude Code sessions ran against the same branch
  simultaneously without either knowing about the other, causing a silent edit collision and an
  invalid soak-test result. Root-caused and recovered without data loss; the `.overnight-lock`
  preflight that closes the gap has since shipped (`ff0000a`), though nothing auto-enforces it —
  a session must call it.
- Servo's own WPT (Web Platform Test) conformance was ~62% and climbing (from ~45% a year prior)
  as of the last check — accepted as a known, monitored cost rather than a blocker.
- **The brief this project was initialized from was stale in both directions.** A verification pass
  on 2026-08-15 found six items marked unbuilt were already shipped (popups, vault, `cookies_read`,
  `download`, lock-file preflight, license files) and surfaced several unlisted concerns (unauthenticated
  control socket, serializing mutex, unbounded `download`, no CI). See `.planning/codebase/CONCERNS.md`.
  The original brief is preserved verbatim at `.planning/BRIEF.md`.

## Constraints

- **Tech Stack**: Rust throughout — Servo/libservo (rendering engine), egui + winit (shell chrome,
  corrected from an original Tauri+React plan), `rust-mcp-sdk` (MCP server) — established through
  direct implementation, not just design. Four crates: `talaria-shell`, `talaria-mcp`,
  `talaria-protocol`, and `talaria-client` (the remote view client, which links no web engine).
- **Transport**: three of them, and `talaria-protocol` is the **shared vocabulary** they carry rather
  than a wire of its own — it defines no transport, no framing and no connection. (1) the Unix control
  socket, local and peer-UID authenticated (`crates/talaria-shell/src/control.rs`); (2) the loopback
  HTTP/MCP listener, off by default and bearer-authenticated (`crates/talaria-shell/src/http.rs`); and
  (3) the remote view WebSocket for live viewing and takeover
  (`crates/talaria-protocol/src/wire.rs`). The stdio MCP proxy
  (`crates/talaria-mcp/src/main.rs`) rides the first. None of them widens the bind: remote reach comes
  from an overlay-network daemon terminating TLS in front of the unchanged `127.0.0.1` listener, so
  this browser handles no certificate.
- **Performance**: screenshot capture must stay well under the ~30–60ms active-takeover latency
  target (currently 4–36ms); startup-to-socket-ready ~21ms; page load ~219ms.
- **Platform**: Linux is the de-facto current baseline — the e2e harness runs under Xvfb with
  `xdotool` and all measured runs are Linux. The original plan named macOS first; macOS status is
  now unverified and Windows is untouched.
- **Dependency risk**: Servo itself was accepted as a not-fully-mature dependency. The egui-on-winit
  shell decision removed the experimental Servo-backed WRY dependency entirely — a larger risk
  reduction than the originally planned fallback.
- **Licensing**: dual MIT OR Apache-2.0 (standard Rust ecosystem convention), compatible with
  Servo/SpiderMonkey's MPL 2.0 (file-level weak copyleft, doesn't force Talaria's own code into any
  particular license).

## Planning artifact policy

**A PLAN.md up front is required only for work touching the security or protocol surface.
Everything else gets a SUMMARY.md after the fact and no plan.**

The security-or-protocol surface means: `crates/talaria-protocol/`, `control.rs`, `vault.rs`, the
MCP tool surface in `crates/talaria-mcp/src/tools.rs`, `parse_agent_url` and the navigation policy
around it, and anything that would carry a threat register. For those, the plan is doing real work —
it is where a threat gets named before the code exists, and Phase 2's plans repeatedly caught
problems at that stage rather than in review.

For UI work, mechanical refactors, docs, and CI changes, it is not. Phase 2 produced roughly 10,000
lines of PLAN and SUMMARY markdown against a 4,734-line Rust codebase. The artifacts were good; the
ratio was not, and on a solo project the up-front plan for a chrome tweak mostly restates what the
diff will show anyway.

Keep writing SUMMARY.md for everything. The summaries are what later sessions actually read, and
`deferred-items.md` — the record of what was found and consciously not done — earned its keep.

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Reject Chromium as the engine | Chromium is the incumbent this project exists to replace; the entire current agent-browsing ecosystem is Chromium-only | ✓ Good |
| Servo over Blitz/Ladybird | Servo ships a real, JIT'd JS engine (SpiderMonkey) already wired to its DOM; Blitz has none, Ladybird isn't embeddable yet | ✓ Good |
| Shell: Tauri + Servo-backed WRY, React chrome | Reuse from [[insomniac]]; React was a stated soft preference | Superseded — see next row |
| Shell: egui-on-winit | `tauri-runtime-verso` found dormant during the Servo-WRY readiness check; egui needs no webview at all, eliminating that dependency entirely rather than just falling back to a native one | ✓ Good — built, verified working |
| MCP surface: minimal, `evaluate`-centric, not typed click/type/wait tools | Independently validated by ego-lite (2.5× faster) and OpenBrowser (3.2–6× fewer tokens) benchmarks | ✓ Good — built and verified, including async/Promise support |
| OAuth via `rust-mcp-sdk`, self-hosted auth server | MCP servers are expected to be their own auth server; avoid hand-rolling OAuth | ✓ Good — built in Phase 4: OAuth 2.1 with PKCE `S256`, refresh rotation, live per-request token lookup |
| No gatekeeping / rate limiting / audit trail | Browser is infrastructure, not a policy layer — explicit product philosophy | ✓ Good |
| No budget for rendering-correctness testing | Servo's own WPT conformance is the correctness signal, not something code-based validation can meaningfully check; reactive backstop is takeover/live-viewing | ✓ Good |
| Naming: OpenBrowser → Charon → Talaria | OpenBrowser collided with an existing project; Talaria better fits the speed/messenger-agent theme | ✓ Good |
| Licensing: dual MIT OR Apache-2.0 | Standard Rust ecosystem convention; compatible with Servo/SpiderMonkey's MPL 2.0 | ✓ Good — both license files now in repo |
| Update mechanism: Tauri's updater plugin | Assumed under the original Tauri shell plan | ⚠️ Revisit — shell is no longer Tauri; no replacement chosen (blocks REL-02) |
| Overnight loops: branch-per-run, no ownership lock | Seemed sufficient at design time | Superseded — a real two-session collision occurred; `.overnight-lock` preflight now ships |
| OAuth sequenced after daily-driver browsing, before distributed mode | Local stdio transport needs no auth; auth's real driver is remote access, so it belongs immediately before the Tailscale split rather than in the stabilization phase | ✓ Good — Phase 5's view channel reused Phase 4's token and revocation machinery outright rather than adding a second trust class |

## Evolution

This document evolves at phase transitions and milestone boundaries.

**After each phase transition** (via `/gsd-transition`):
1. Requirements invalidated? → Move to Out of Scope with reason
2. Requirements validated? → Move to Validated with phase reference
3. New requirements emerged? → Add to Active
4. Decisions to log? → Add to Key Decisions
5. "What This Is" still accurate? → Update if drifted

**After each milestone** (via `/gsd-complete-milestone`):
1. Full review of all sections
2. Core Value check — still the right priority?
3. Audit Out of Scope — reasons still valid?
4. Update Context with current state

---
*Last updated: 2026-08-15 after initialization (brief reconciled against verified working tree)*

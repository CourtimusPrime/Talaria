---
gsd_state_version: 1.0
milestone: v1.0
milestone_name: milestone
status: executing
last_updated: "2026-08-23T13:59:03.805Z"
progress:
  total_phases: 8
  completed_phases: 3
  total_plans: 34
  completed_plans: 28
  percent: 38
---

# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-08-15)

**Core value:** An agent can drive a real, already-logged-in browsing session, and a human can take over instantly the moment it hits something only a human can clear.
**Current focus:** Phase 05 — distributed-mode

## Current Position

Phase: 5 — Distributed Mode *(v2)*
Plan: 5 of 11 executed (waves 1-3 complete)
Status: Ready to execute
Next: `/gsd-execute-phase 5`

Phase 4 is closed. AUTH-01, AUTH-02 and AUTH-03 are Complete; verification passed after four
code-review blockers were fixed. 296 unit tests, e2e 22/22. Phases 1–3 (v1) remain
feature-complete.

**Phase 5 is planned:** 11 plans across 8 waves, plan-checker APPROVED at revision 3. Scope was cut
from four requirements to two — DIST-03 and DIST-04 moved to a new **Phase 5.1** — because an
architectural client/server split plus a frame pipeline plus reconnect plus persistence was more
than one reviewable phase. ROADMAP and REQUIREMENTS reflect the split.

Six decisions locked up front in `05-CONTEXT.md`: a thin `talaria-client` binary with the server
unchanged; the remote client sees **agent tabs only**; Tailscale Serve terminates TLS so
`BIND_HOST` stays a constant; the 5/5.1 split; frame encoding reuses `png` at `Compression::Fast`
over tile diffs with **no new codec**; and on a relayed link the client degrades and reports rather
than refusing takeover.

**Both required spikes have landed (wave 1).** 05-01 settled A2 — Tailscale Serve proxies a
WebSocket upgrade cleanly, and forwards the tailnet `Host` with the Serve port, which makes 05-04's
advertised-base-URL work required rather than optional. 05-02 settled the phase's largest unknown:
**A1 CONFIRMED** — `read_to_image` at a 30 ms cadence costs 1.19–1.56 ms mean with a p95 never above
1.86 ms and zero failures over 900 ticks, so the capture model stands. **A8 REFUTED** — the synthetic
codec frames predicted encode *time* well and *bytes* badly, and `encode_screenshot` was never
running at `png::Compression::Default` at all (png 0.17's `Info::default()` is `Fast`+`Sub`), which
voids the 21 ms / 175 ms premise while leaving D-05-05's choice standing. Only one readback figure
exists and it is software-rendered: this machine has no non-llvmpipe GL path, so the hardware
re-measure is a `VERIFICATION.md` manual item.

**Two open product questions, recorded together in `05-CONTEXT.md`** because they are one question —
what can a remote human do that requires being at the server machine? First pairing needs someone at
the server (OAuth consent is a server-chrome panel), and a remote human at a login wall has no
address bar, since the view channel deliberately carries no `Command` and remote input cannot reach
the chrome. The second sits against the project's stated core value and should be revisited the
first time a real takeover dead-ends.

**Note on history:** the 111 unpushed commits from Phases 3 and 4 were squashed to one commit per
phase at the user's request; the pre-squash history is preserved on `backup/pre-squash-phases-3-4`.
Phase 5 keeps per-task commits.

## Performance Metrics

**Velocity:**

- Total plans completed: 12 (Phase 1 predates GSD tracking — see `OVERNIGHT_LOG.md`)
- Average duration: —
- Total execution time: —

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| 1 | — | — | — |
| 03 | 4 | - | - |
| 04 | 8 | - | - |

**Recent Trend:**

- Last 5 plans: —
- Trend: —

*Updated after each plan completion*
**Per-Plan Metrics:**

| Plan | Duration | Tasks | Files |
|------|----------|-------|-------|
| Phase 02 P01 | 15m | 3 tasks | 3 files |
| Phase 02 P02 | 23m | 2 tasks | 3 files |
| Phase 02 P03 | 24min | 3 tasks | 9 files |
| Phase 02 P04 | 26min | 3 tasks | 4 files |
| Phase 02 P05 | 49min | 3 tasks | 4 files |
| Phase 02 P06 | 41min | 3 tasks | 3 files |
| Phase 02 P07 | 47min | 3 tasks | 2 files |
| Phase 02 P08 | 29min | 3 tasks | 3 files |
| Phase 02 P09 | 33m | 3 tasks | 3 files |
| Phase 02 P10 | 50m | 3 tasks | 5 files |
| Phase 02 P11 | 45m | 4 tasks | 8 files |
| Phase 03 P01 | 34 min | 3 tasks | 8 files |
| Phase 03 P02 | 22 min | 3 tasks | 7 files |
| Phase 03 P03 | 25 min | 3 tasks | 5 files |
| Phase 03 P04 | 32 min | 3 tasks | 8 files |
| Phase 04 P01 | 27min | 3 tasks | 6 files |
| Phase 04 P02 | 31min | 3 tasks | 6 files |
| Phase 04 P03 | 48min | 3 tasks | 15 files |
| Phase 04 P04 | 21min | 2 tasks | 3 files |
| Phase 04 P05 | 34min | 3 tasks | 10 files |
| Phase 04 P06 | 64 min | 3 tasks | 8 files |
| Phase 04 P07 | 55min | 2 tasks | 3 files |
| Phase 04 P08 | 85 min | 3 tasks | 10 files |
| Phase 05 P01 | 50min | 3 tasks | 6 files |
| Phase 05 P02 | 45min | 2 tasks | 2 files |
| Phase 05 P03 | 30min | 2 tasks | 6 files |
| Phase 05 P04 | 48 min | 3 tasks | 7 files |
| Phase 5 P5 | 54 | 3 tasks | 9 files |

## Accumulated Context

### Decisions

Decisions are logged in PROJECT.md Key Decisions table.
Recent decisions affecting current work:

- Init: OAuth resequenced from Phase 2 to Phase 4 — stdio needs no auth; auth's driver is remote access and depends on an HTTP transport that doesn't exist yet
- Init: SEC-01 (control-socket peer-UID check) is local IPC hygiene, deliberately distinct from the per-agent policy layer that stays out of scope
- Phase 1: egui-on-winit shell replaced the Tauri + Servo-WRY plan, removing the WRY dependency entirely
- Phase 2: D-28: overnight lock records os.getppid() (or explicit --pid) instead of os.getpid(), so PID liveness is meaningful; dead owners self-clear, live ones still refuse
- Phase 2: D-29: .githooks/pre-commit calls overnight_lock.py check via core.hooksPath, making the branch-lock preflight non-optional
- Phase 2: Phase 2 baseline verdict GREEN — all 9 e2e suites pass at 16411ee, so later failures in this phase are attributable
- [Phase ?]: 02-02: agent URL allowlist is http, https, data, and the exact about:blank literal; the about: scheme as a whole is not admitted because popup adoption depends on that one string
- [Phase ?]: 02-02: resolve_location now names file: alongside about: so the human omnibox can open a local file — D-02's unrestricted human path did not actually work before
- [Phase ?]: SEC-01: control socket peer check fails closed — a credential-lookup error rejects the connection, and a rejected peer gets a closed connection with no reply
- [Phase ?]: Fallback control socket lives in a 0700 per-UID directory as talaria-$UID/talaria.sock; the XDG path is unchanged and ensure_socket_dir is a no-op there
- [Phase ?]: download cap and byte count are computed from bytes read/written, never from content-length (attacker-controlled)
- [Phase ?]: an unparseable TALARIA_MAX_DOWNLOAD_BYTES falls back to the 2 GiB default, never to zero or unbounded
- [Phase ?]: tokio oneshot Sender::is_closed() is the cancellation handle for off-thread work — the control socket already drops the receiver on command timeout
- [Phase ?]: 02-05: the MCP retry is expressed as a two-state Attempt enum — NotSent is reachable only from a failed send on the outbound channel, so a request a live connection accepted is structurally un-retryable
- [Phase ?]: 02-05: bounding each control-socket write by the command timeout is what makes awaiting the writer at teardown safe instead of stranding it
- [Phase ?]: 02-05: MCP tool calls are dispatched concurrently by rust-mcp-sdk, so a back-to-back ordering assertion passes against a serialising connection half the time — the slow call needs a head start for the test to bind
- [Phase ?]: 02-06: the evaluate in-flight completion flag is an Rc<Cell<bool>> — a Cell set is infallible and borrow-free, so a servo callback cannot silently drop it and leave a healthy tab permanently refused
- [Phase ?]: 02-06: the in-flight entry expires on its own deadline as well as its flag, so a lost engine callback self-heals; that deadline is registered with next_capture_deadline because a deadline nothing wakes for is not a deadline
- [Phase ?]: 02-06: MCP-10 stays In Progress — the second evaluate now fails fast, but the first still burns the timeout; completing it needs an upstream libservo slow-script interrupt
- [Phase ?]: 02-07: lifecycle events are addressed to the tab's owning session, not broadcast; PendingEvent carries a plain session id so a human-owned tab produces no entry at all
- [Phase ?]: 02-07: the four tab-table delegate callbacks defer to a pending_tab_work queue on a busy table instead of skipping; the two marking callbacks log at error level, since their queues are never held across a servo call
- [Phase ?]: 02-07: AGENT-04 stays In Progress — the shell half is done, plan 02-08 owns the MCP notification half
- [Phase ?]: 02-08: MCP tab lifecycle notifications ride notifications/message via McpServer::notify_log_message, with ServerCapabilities.logging declared; the payload is the serialized protocol Event so no follow-up tabs_list is needed
- [Phase ?]: 02-08: The proxy's event sink lives on ShellConnection and the reader forwards without filtering — the shell already addressed each event to the owning session, and a fan-out here would re-broaden it
- [Phase ?]: 02-08: AGENT-04 stays In Progress: close and crash reach MCP clients, but talaria_protocol::Event has no tab-open variant, so a popup adopted under an agent's tab is still poll-only (deferred-items.md)
- [Phase ?]: Vault write path keys on the entry URL's host plus username, not the whole URL, so re-saving the same login from a different page updates it (02-09)
- [Phase ?]: Plaintext vault import removes the source only after re-reading, decrypting and entry-count-matching the encrypted file; a failed verification chmods the source 0600 and logs at error level (02-09)
- [Phase ?]: Vault one-shot notices (plaintext import, keychain downgrade) live on Vault itself with read/clear accessors, so app.rs needed no change; the chrome that renders them is plan 02-10 (02-09)
- [Phase ?]: 02-10: the credentials panel's own open state goes through the UI-intent round trip, not an inline field write — a single sanctioned exception is how the egui anti-pattern returns
- [Phase ?]: 02-10: autofill is a chrome-side suggestion with copy controls, never page-DOM injection (D-23) — injection would collide with an agent's own evaluate and expose the password to every script on the page
- [Phase ?]: 02-10: a typed bare hostname is completed to https before it becomes a vault entry, otherwise it can never be domain-matched or deleted again
- [Phase ?]: 02-10: e2e panel input is driven from the focus point the panel itself sets, not from row coordinates — a row's y depends on whether the machine has a usable keychain
- [Phase ?]: 02-11: the seven pre-existing clippy lints were fixed rather than the gate narrowed — CI runs the full --all-targets -D warnings with no command-line allow-list
- [Phase ?]: 02-11: #[allow] on a macro invocation is silently ignored (rustc unused_attributes), so the enum_variant_names allow for tool_box! had to be module-scoped
- [Phase ?]: 02-11: CI builds --locked so an ordinary push never re-resolves; re-resolution is the scheduled lockfile-audit job's exclusive business
- [Phase ?]: 02-11: TEST-03 stays In Progress — both workflows are config-only and have never executed, because this repository has no git remote
- [Phase ?]: Bookmarks::upsert refuses to be the toggle (add-if-absent, never overwrite); the add-or-remove decision lives only in apply_ui_actions
- [Phase ?]: Every bookmarks mutation is an atomic whole-array write (.tmp sibling + fs::rename) — the shape downloads.rs and settings.rs should copy, not vault.rs's plain fs::write
- [Phase ?]: vault_ui_test.py's CREDENTIALS_BUTTON moved 283 -> 341, measured from the button's own rect under Xvfb rather than guessed
- [Phase ?]: A data: URL in the human address bar stays a search, not a navigation — the human/agent trust-root asymmetry is deliberate and now tested (03-03)
- [Phase ?]: SearchEngine carries no id: exactly one engine is configured at a time, so {name, url_template} is the whole identity (03-03)
- [Phase ?]: config.json is the project's first config file, crossing CLAUDE.md's stated no-config-file-format boundary deliberately (03-03)
- [Phase ?]: 03-04: AppEvent::DownloadCompleted carries no owning-session field — the store records every download regardless of owner (Pitfall 5), so RESEARCH.md's drafted field would have had no reader
- [Phase ?]: 03-04: downloads.rs calls nothing that deletes a file — it drops even bookmarks.rs's stale-staging cleanup — so remove() has no lever to grow into delete()
- [Phase ?]: 03-04: Shared::event_proxy is the first EventLoopProxy on Shared and the sanctioned route from any background thread back onto the main loop; Rc-not-Send makes the discipline compiler-enforced
- [Phase ?]: 03-04: UiAction::OpenDownload (xdg-open) has exactly one construction site and one consumer, both in chrome — the browser's only process spawn is unreachable from the MCP/control-socket surface
- [Phase ?]: 03-04: vault_ui_test.py's CREDENTIALS_BUTTON moved 370 -> 399, measured at [[388.3 2.0] - [409.3 20.0]]; 03-03's ~29pt/button prediction confirmed exactly on the fourth move
- [Phase ?]: dispatch takes `&dyn CommandSink` (trait object) rather than a generic parameter — object-safe, and the HTTP transport can hold the sink behind an Arc
- [Phase ?]: The CommandSink impl for ShellConnection forwards to the inherent request and adds nothing: retry, pipelining and reconnect stay the connection's (plan 02-05's double-execution fix)
- [Phase ?]: The nine-tool surface is pinned by a unit test that names 04-UI-SPEC.md's consent grant bullets — a tool change obliges the consent screen to change with it
- [Phase ?]: 04-02: Resolved the lockfile with cargo metadata against edited manifests, never cargo update; primeorder 0.14.0-rc.14 survived and is now asserted by grep plus a --locked build rather than assumed
- [Phase ?]: 04-02: The rust-mcp-sdk feature list is exactly server/macros/stdio/streamable-http/auth — the legacy SSE transport feature is deliberately omitted, with the reason recorded at the manifest line
- [Phase ?]: 04-02: A1 CONFIRMED — rust-mcp-axum routes a self-hosted authorization server's declared endpoints to the provider's handle_request, so 04-05 and 04-06 need no re-planning
- [Phase ?]: 04-02: The SDK never calls AuthProvider::validate_allowed_methods — 04-06 must call it itself or wrong-verb requests fall into the token-issuance path
- [Phase 4]: Fixed listener port, default 8779 (04-CONTEXT open question closed): a human configures an MCP client with a URL once, and 8779 avoids both the common dev-server defaults and this platform's ephemeral range
- [Phase 4]: Built the MCP router via rust-mcp-axum's BYO-server mcp_routes path rather than create_axum_server: the Origin refusal has no middleware slot in AxumServerOptions, the bound address becomes a fact, and the process keeps its own SIGTERM handling
- [Phase 4]: The remote-access config key has no bind-address field at all — D-04-04's loopback-only constraint is structural, not validated
- [Phase ?]: A corrupt agents.json degrades to an empty client SET, never an empty CHECK — enforced by shape (every decision is a list search; no store-is-empty branch exists), not by care
- [Phase ?]: The agent token store stays out of the credential vault: hash-only persistence removes the cipher, which removes the key, which removes the keychain — so Vault::load()'s D-Bus hang cannot become 'no agent can connect'
- [Phase ?]: TokenRecord carries consumed_at_ms — the plan's field list could not express reuse detection, which the same plan requires
- [Phase ?]: Access 1h / refresh 30d / 32-client cap are conventional defaults, parameters at every call site, not specification requirements
- [Phase ?]: AUTH-01 and AUTH-02 deliberately left unmarked: 04-05/06/07 and 04-08 close them; a store without an endpoint is not an authorization server
- [Phase 04]: 04-05: the canonical resource identifier is http://{bound}/mcp, built by one function from the address actually bound — compared byte for byte, so a second construction site would be a way for a token to validate against one spelling and not another
- [Phase 04]: 04-05: every token-verification failure returns one REFUSAL constant, so unknown, expired and wrong-audience are byte-identical to the caller and the endpoint is not an oracle for which tokens exist
- [Phase 04]: 04-05: an HTTP client's tab-owner label is the verified client_id, not the self-asserted clientInfo.name; stdio sessions keep their Hello string and the two are deliberately different
- [Phase 04]: 04-05: Shared::agents is an Arc<Mutex<Agents>> — the one non-RefCell field on Shared — because the chrome's revoke and the listener's verifier must see one store, not two
- [Phase 04]: 04-05: AUTH-01 stays open; the resource-server half landed, the OAuth authorization server (Authorization Code + PKCE) is 04-06's
- [Phase 04]: The consent timing override is a clamp, not a value: each of the three durations is clamped to its compiled constant as a ceiling and to a non-zero 200 ms floor, so a test-hooks-gated variable can only ever make a security window smaller — never longer, never zero, never off. The grant path takes its timings as a parameter and reads no environment variable. — At the shipped values an expiry plus the cooldown it arms is two and a half minutes, and an assertion that slow is one somebody quietly deletes — which would leave the expiry path, the one an attacker uses to hold the chrome hostage, the least-tested thing in the phase. Unit-tested by setting every override to its floor and confirming an unanswered request still resolves denied and no client is marked authorized.
- [Phase 04]: ChromePanel::Consent is raised only by Gui::raise_consent, and UiAction::SetPanel(Consent) is refused by both apply_ui_actions and Gui::set_panel. The page served at /authorize carries no control of any kind and no request-derived value. — http is in the agent navigation allowlist, so an agent holding evaluate can open Talaria own authorization endpoint in a tab it owns. Adding the enum variant made SetPanel(Consent) representable; the two refusals turn "no control constructs it" from an invariant every future panel button must maintain into a property a reviewer confirms in two functions.
- [Phase 04]: The holding page carries no script at all — a meta refresh instead — resolving a contradiction inside 04-UI-SPEC.md, which asks for a status-poll script and fixes default-src none in the same table. — The two cannot both hold: default-src none forbids inline script. Kept the header and dropped the script, so the page ships stricter than the approved contract describing it.
- [Phase ?]: Token endpoint refuses with 400 invalid_grant (RFC 6749 §5.2), not 401 — one grant_refused() gives indistinguishability
- [Phase ?]: A failed code redemption still consumes the code, so an intercepted code cannot be retried after a wrong verifier
- [Phase ?]: CodeRecord records code_challenge_method so the S256 gate at redemption is a check, not an assumption
- [Phase ?]: PKCE challenges are re-spelled (base64url -> hex) by challenge_as_digest rather than hashed a second time; comparison stays agents::digests_match
- [Phase 04]: Ending an MCP session does not close a stream already open on it — measured in 04-08 by asserting the closure from the client end and watching it fail. The SDK cancels only the reader; the response body is fed from a duplex half its transport still owns. The shell closes the connection instead, at an immediate bound.
- [Phase 04]: Revoking either half of a token pair takes the whole family (RFC 7009 leaves one direction open); the standard /revoke endpoint drops tokens and leaves the registration, while the human's Access-panel revoke removes both.
- [Phase 04]: A revoked agent's tabs stay open, visible in the Agents view and available for takeover — closing them destroys state the human may want and buys nothing, since the agent can no longer drive them.
- [Phase 04]: Any arm-then-confirm control acting on one row of a list that can reorder is keyed on the row's identifier, never on its index.
- [Phase ?]: axum 0.8.9 declared directly with its ws feature — a unification onto the node rust-mcp-axum already resolved, not a new resolution
- [Phase ?]: A2 CONFIRMED: Tailscale Serve proxies a WebSocket upgrade to loopback and holds it across a 90s idle
- [Phase ?]: Serve forwards the tailnet Host with the Serve port, so 05-04's advertised base URL is required work and must come from configuration
- [Phase ?]: 05-07's client TLS costs zero packages on either rustls spelling; rustls-tls-native-roots recommended
- [Phase ?]: First pairing requiring local access to the server is recorded as an open developer decision, not answered by silence
- [Phase 5]: A1 CONFIRMED: read_to_image at a 30 ms cadence costs 1.19-1.56 ms mean, p95 never above 1.86 ms, 0 failures over 900 ticks — the capture model stands and 05-08 may be executed as planned
- [Phase 5]: A8 REFUTED: encode_screenshot never ran at png::Compression::Default — png 0.17's Info::default() is Fast+Sub — so the 21 ms/175 ms premise is void, the frame encoder diverges on one line not three, and the synthetic frames predicted encode time well but bytes badly (3.2x on the photo case)
- [Phase 5]: One readback figure, not two: this machine has no non-llvmpipe GL path, and the Xvfb number is plausibly optimistic rather than conservative — the hardware re-measure becomes a VERIFICATION.md manual item
- [Phase ?]: The view channel carries no agent tool vocabulary — D-05-02 expressed as an absence in the wire types, so a remote click is unreachable from the MCP tool surface by construction
- [Phase ?]: The view wire's channel tags are 0x01 control, 0x02 tabs, 0x03 event, 0x04 input, 0x10 frame — a low JSON block and a high binary block, with the gap deliberate
- [Phase ?]: The frame header is a fixed 51-byte little-endian layout carrying last_applied_input, which makes input-to-photon latency measurable with no clock shared between the two machines
- [Phase ?]: talaria-protocol's Unix socket helpers were moved into a cfg(unix) local module and deliberately not re-exported: a re-export would leave the root surface being corrected exactly as it was
- [Phase 05]: One advertised origin feeds every string this browser publishes; the bind host stays a module constant and Talaria handles no transport identity (D-05-03)
- [Phase 05]: A validated advertised URL is refused rather than normalised — a normalisation is a second spelling, and RFC 8707 audience validation compares byte for byte
- [Phase 05]: Standing the Tailscale Serve proxy up is an executable, port-scoped, Funnel-refusing script rather than a recipe in a document
- [Phase ?]: The /view route authenticates in its own right: the SDK middleware chain is composed only for its own transport handlers, so a merged axum route inherits axum layers and nothing else
- [Phase ?]: View-socket termination is the stream registry's second half — a WebSocket is not an SDK session, so terminate_matching cannot reach one
- [Phase ?]: A remote viewer sees every agent's tabs (T-05-10), matching the local Agents view, decided rather than defaulted

### Pending Todos

None yet.

### Blockers/Concerns

- **`Vault::load()` can hang the shell's main thread where no session D-Bus exists** — found by the
  first real CI run on 2026-08-17. The keychain lookup falls into D-Bus autolaunch and blocks
  startup indefinitely; the control thread keeps answering `hello`, so the shell looks alive while
  serving nothing. CI works around it with `dbus-run-session`. **This is a real user-facing hang on
  any headless box, container, or SSH session**, and the real fix changes `vault.rs`. See
  `.planning/phases/02-harden-the-agent-surface/deferred-items.md`. Candidate v1 blocker.

- **REL-02 has no chosen approach** — Tauri's updater plugin no longer applies to the egui shell.
  Phase 7, which is now v2. Does not block v1.

- **Verification weight sits almost entirely in the Python e2e suite** — 14 Xvfb suites against 9
  Rust unit tests. TEST-04 (meaningful Rust unit coverage) remains deferred to v2.

- ~~**`.overnight-lock` is advisory**~~ — RESOLVED by plan 02-01.
- ~~**MCP-09 / MCP-10 half closed**~~ — RECLASSIFIED 2026-08-17. Neither is blocked on effort:
  MCP-09 needs navigation provenance designed (it conflicts with D-02's human-trust-root rule) and
  MCP-10 needs a SpiderMonkey interrupt libservo does not expose. Both are now published in
  `SECURITY.md` as known limitations rather than carried as open work.

- ~~**Neither workflow has ever run — no git remote**~~ — RESOLVED 2026-08-17. Remote added, `main`
  pushed, `ci.yml` green (run `32019859735`), `e2e.yml` green 14/14 (run `32019859744`). The
  predicted hosted-runner risks were real: a 42 GB `target/` cannot enter a 10 GB `actions/cache`,
  so the gate moved to a self-hosted runner and the cold hosted build became a weekly canary.

## Deferred Items

| Category | Item | Status | Deferred At |
|----------|------|--------|-------------|
| Testing | TEST-04 — meaningful Rust unit coverage across `talaria-shell` | v2 | 2026-08-15 |
| Agent UX | AGENT-05 — per-agent session naming in the Agents view | v2 | 2026-08-15 |
| Agent UX | AGENT-04 — wire-level `TabOpened` event so adopted popups are not poll-only | v2 | 2026-08-17 |
| Reliability | `Vault::load()` D-Bus autolaunch hang — needs the keychain lookup off the startup path | Phase 3 | 2026-08-17 |

## Quick Tasks Completed

| Date | Task | Outcome |
|------|------|---------|
| 2026-08-17 | `260817-jec` unblock CI and close Phase 2 | Remote + self-hosted runner; CI split into fast/e2e/cold-canary; `SECURITY.md`; Phase 2 closed on green runs `32019859735` and `32019859744` (14/14 e2e); v1 declared as Phases 1-3. Found a real `Vault::load()` D-Bus startup hang. |

## Session Continuity

Last session: 2026-08-23T13:58:51.151Z
Stopped at: Completed 05-05-PLAN.md
Resume file: None

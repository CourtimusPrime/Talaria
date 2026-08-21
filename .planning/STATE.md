---
gsd_state_version: 1.0
milestone: v1.0
milestone_name: milestone
current_phase: 4
current_phase_name: v2
status: planning
stopped_at: Completed 03-04-PLAN.md — Phase 3 complete
last_updated: "2026-08-20T16:47:21.523Z"
last_activity: 2026-08-20
last_activity_desc: Phase 03 complete, transitioned to Phase 4
progress:
  total_phases: 2
  completed_phases: 2
  total_plans: 15
  completed_plans: 15
---

# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-08-15)

**Core value:** An agent can drive a real, already-logged-in browsing session, and a human can take over instantly the moment it hits something only a human can clear.
**Current focus:** Phase 03 — table-stakes-browsing

## Current Position

Phase: 4 — Authenticated Remote Transport *(v2)*
Plan: Not started
Status: Ready to plan
Next: /gsd-verify-work 3, then close the phase. **v1 (Phases 1-3) is feature-complete.**

All four plans landed sequentially, each depending on the previous, because every one of
them touches app.rs and gui.rs. BROWSE-01 through BROWSE-04 are all delivered; the e2e
suite is at 18/18 and `cargo test` at 83.

Planned without a CONTEXT.md — /gsd-discuss-phase was not run, so the design decisions it
would have locked were resolved by 03-RESEARCH.md and 03-UI-SPEC.md instead, each recording
its discretionary calls explicitly.

Three documentation debts are queued for the phase close, listed at the end of
03-04-SUMMARY.md: CLAUDE.md's "no config file format" claim (false since 03-03), CLAUDE.md's
component table missing the four Phase 3 stores and Shared::event_proxy, and CHANGELOG.md
missing BROWSE-01/02/03 entries (BROWSE-04's is written).

Last activity: 2026-08-20 — Phase 03 complete, transitioned to Phase 4

v1 (Phases 1-3): [██████████] 3 of 3 phases complete
All phases (1-7): [████░░░░░░] 3 of 7 complete

## Performance Metrics

**Velocity:**

- Total plans completed: 4 (Phase 1 predates GSD tracking — see `OVERNIGHT_LOG.md`)
- Average duration: —
- Total execution time: —

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| 1 | — | — | — |
| 03 | 4 | - | - |

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

Last session: 2026-08-20T09:21:53.268Z
Stopped at: Completed 03-04-PLAN.md — Phase 3 complete
Resume file: None

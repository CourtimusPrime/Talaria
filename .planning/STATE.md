---
gsd_state_version: 1.0
milestone: v1.0
milestone_name: milestone
current_phase: 02
current_phase_name: harden-the-agent-surface
status: executing
stopped_at: Completed 02-07-PLAN.md
last_updated: "2026-08-16T08:14:50.693Z"
last_activity: 2026-08-16
last_activity_desc: Phase 02 execution started
progress:
  total_phases: 1
  completed_phases: 0
  total_plans: 11
  completed_plans: 7
---

# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-08-15)

**Core value:** An agent can drive a real, already-logged-in browsing session, and a human can take over instantly the moment it hits something only a human can clear.
**Current focus:** Phase 02 — harden-the-agent-surface

## Current Position

Phase: 02 (harden-the-agent-surface) — EXECUTING
Plan: 8 of 11
Status: Ready to execute
Last activity: 2026-08-16 — Phase 02 execution started

Progress: [██████░░░░] 64%

## Performance Metrics

**Velocity:**

- Total plans completed: 0 (Phase 1 predates GSD tracking — see `OVERNIGHT_LOG.md`)
- Average duration: —
- Total execution time: —

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| 1 | — | — | — |

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

### Pending Todos

None yet.

### Blockers/Concerns

- **REL-02 has no chosen approach** — Tauri's updater plugin no longer applies to the egui shell. Decide before Phase 7; does not block Phases 2–6.
- ~~**`.overnight-lock` is advisory**~~ — RESOLVED by plan 02-01. `.githooks/pre-commit` enforces the lock at commit time via `core.hooksPath`, and a lock whose owning PID is dead now self-clears instead of refusing. The stale `c85a1ef3` / pid 2796577 lock was cleared.
- **Verification weight sits almost entirely in the Python e2e suite** — only three Rust unit tests exist, and there is no CI (TEST-03, Phase 2).
- MCP-09 half closed: parse_agent_url refuses file:/javascript:/blob:, but an agent with an evaluate handle still reaches the filesystem via location.href='file://...' or window.open('file://...') — proven readable end to end. Needs WebViewDelegate::request_navigation + request_create_new policy on agent-owned tabs; out of 02-02's scope. Do not mark MCP-09 complete until a follow-up plan lands.
- MCP-10 half closed: 02-06 added per-tab in-flight tracking, so a SECOND evaluate on a wedged tab is refused instantly and screenshot/tabs_close/tabs_focus/tabs_list keep working. The FIRST evaluate still runs to the command timeout and never completes — that needs a SpiderMonkey slow-script interrupt exposed through libservo (upstream, out of Phase 2). Do not mark MCP-10 complete in this phase.

## Deferred Items

| Category | Item | Status | Deferred At |
|----------|------|--------|-------------|
| Testing | TEST-04 — meaningful Rust unit coverage across `talaria-shell` | v2 | 2026-08-15 |
| Agent UX | AGENT-05 — per-agent session naming in the Agents view | v2 | 2026-08-15 |

## Session Continuity

Last session: 2026-08-16T08:14:50.688Z
Stopped at: Completed 02-07-PLAN.md
Resume file: None

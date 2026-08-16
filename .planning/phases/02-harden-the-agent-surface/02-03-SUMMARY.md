---
phase: 02-harden-the-agent-surface
plan: 03
subsystem: infra
tags: [unix-socket, so_peercred, ipc, permissions, tokio, e2e]

# Dependency graph
requires:
  - phase: 02-01
    provides: green e2e baseline (10/10) and the enforcing pre-commit hook, so any suite failure here is attributable to this plan
provides:
  - "talaria_protocol::socket_dir / ensure_socket_dir / current_uid"
  - "Fallback control socket moved from a bare /tmp/talaria-$UID.sock into a 0700 per-UID directory"
  - "Control socket chmodded 0600 after bind, checked, with a refusal to serve on failure"
  - "peer_uid_ok: SO_PEERCRED gate on accept, fail-closed, before any session id is allocated"
  - "tests/e2e/harness.py::socket_path mirroring the Rust path logic including the fallback"
  - "Socket mode / dir mode / owner assertions plus an explicit cross-uid skip line in control_socket_test"
affects: [02-11 CI, 04 HTTP transport and OAuth, 05 distributed mode]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Fail-closed OS-level IPC gate: reject on credential-lookup error, not just on mismatch"
    - "Path logic split into dir + filename so the socket path and its directory cannot drift"
    - "Python e2e suites mirror the Rust socket-path expression inline rather than importing it, preserving standalone runnability"

key-files:
  created: []
  modified:
    - crates/talaria-protocol/src/lib.rs
    - crates/talaria-shell/src/control.rs
    - tests/e2e/harness.py
    - tests/e2e/control_socket_test.py
    - tests/e2e/scheme_refusal_test.py
    - tests/e2e/crash_recovery_test.py
    - tests/e2e/crash_event_test.py
    - tests/e2e/timeout_session_test.py
    - tests/e2e/popup_test.py

key-decisions:
  - "The peer check fails closed: a credential-lookup error rejects the connection rather than admitting it"
  - "A rejected peer gets a closed connection and no reply — no hint about what the check was"
  - "ensure_socket_dir skips create/chmod when XDG_RUNTIME_DIR is set; the OS already owns that directory at 0700"
  - "Python suites carry the socket expression inline instead of importing harness, keeping each suite runnable standalone against an already-running shell"
  - "std::env::temp_dir() on the Rust side is mirrored as TMPDIR-or-/tmp on the Python side, and os.environ.get with a default mirrors Rust's env::var Ok(\"\") edge case exactly"

patterns-established:
  - "Fail-closed IPC gate: reject before spawn, log at warn, continue the accept loop"
  - "Checked chmod on a security-relevant file, with refusal-to-serve on failure, replacing the let _ = discard"

requirements-completed: [SEC-01]

coverage:
  - id: D1
    description: "Another local OS user cannot drive the control socket: SO_PEERCRED gate rejects a mismatched peer UID before the hello handshake"
    requirement: SEC-01
    verification:
      - kind: e2e
        ref: "scratchpad probe: sudo -n -u nobody connect to a deliberately widened socket -> ConnectionResetError; shell logs 'control connection refused: peer uid 65534 is not ours (1001)'"
        status: pass
      - kind: e2e
        ref: "tests/e2e/control_socket_test.py (same-uid peer completes hello and drives every command)"
        status: pass
    human_judgment: false
  - id: D2
    description: "A rejected peer consumes no session id and does not close or stall any other connection"
    requirement: SEC-01
    verification:
      - kind: e2e
        ref: "scratchpad probe: session 1 accepted -> intruder refused -> three concurrent connections get sessions 2,3,4 and connection 1 still answers tabs_list"
        status: pass
    human_judgment: false
  - id: D3
    description: "The control socket is mode 0600 and owner-owned on both the XDG path and the temp fallback; the fallback directory is 0700"
    requirement: SEC-01
    verification:
      - kind: e2e
        ref: "tests/e2e/control_socket_test.py#socket permissions block (0600 + owner uid; 0700 dir on the fallback branch)"
        status: pass
      - kind: e2e
        ref: "scratchpad probe: shell started with and without XDG_RUNTIME_DIR, both sockets 0600, fallback dir 0700, both handshake"
        status: pass
    human_judgment: false
  - id: D4
    description: "A shell that cannot establish those permissions logs at error level and does not serve"
    verification:
      - kind: other
        ref: "code inspection: crates/talaria-shell/src/control.rs serve() returns after log::error! on ensure_socket_dir and set_permissions failure"
        status: pass
    human_judgment: true
    rationale: "Forcing a chmod/mkdir failure in-process needs a hostile filesystem (immutable or foreign-owned directory) that no automated suite sets up today; only the success path is machine-verified"
  - id: D5
    description: "The Python e2e suites resolve the socket path the same way the Rust does, fallback shape included"
    requirement: SEC-01
    verification:
      - kind: e2e
        ref: "TALARIA_E2E_DISPLAY=:98 XDG_RUNTIME_DIR=/tmp/talaria-e2e-rt python3 tests/e2e/run_all.py -> exit 0, 10/10 PASS"
        status: pass
      - kind: e2e
        ref: "scratchpad probe: control_socket_test.py run against a shell on the fallback path (XDG_RUNTIME_DIR unset) -> exit 0"
        status: pass
    human_judgment: false

# Metrics
duration: 24min
completed: 2026-08-16
status: complete
---

# Phase 2 Plan 03: Control Socket Peer Authentication Summary

**SO_PEERCRED gate on accept plus a 0600 socket inside a 0700 per-UID directory, closing SEC-01: another local user can no longer connect, handshake, and drive the browser.**

## Performance

- **Duration:** 24 min
- **Started:** 2026-08-16T05:47:00Z
- **Completed:** 2026-08-16T06:11:00Z
- **Tasks:** 3
- **Files modified:** 9

## Accomplishments

- `peer_uid_ok` compares the kernel-reported peer UID against `talaria_protocol::current_uid()` on every accept and rejects before the `tokio::spawn`, so a refused peer never reaches the handshake, never consumes a session id, and never touches another connection. A credential-lookup error also rejects — the check fails closed.
- The fallback socket moved from a bare `/tmp/talaria-$UID.sock` (world-connectable) into `talaria-$UID/talaria.sock` inside a directory created at mode 0700, and the socket itself is chmodded 0600 on both paths with the result checked.
- `serve()` refuses to serve when either the directory or the chmod cannot be established, logging at error level — the fail-closed posture the whole plan exists for.
- The six standalone Python suites now resolve the socket exactly as the Rust does, and `control_socket_test` pins the socket mode, the owner UID, the fallback directory mode, and prints an explicit line naming the cross-UID half as un-exercisable at the same UID.
- Cross-UID rejection was proven for real, not just inferred: a `sudo -u nobody` client against a deliberately widened socket was refused with `control connection refused: peer uid 65534 is not ours (1001)`.

## Task Commits

Each task was committed atomically:

1. **Task 1: Owner-only socket directory and 0600 socket (D-16)** - `5fd1985` (feat)
2. **Task 2: Reject peers whose UID is not the shell's (D-15)** - `3308c4b` (feat)
3. **Task 3: Align the Python suites and assert the permissions** - `935d4ae` (test)

**Plan metadata:** see the `docs(02-03)` commit following this summary.

## Files Created/Modified

- `crates/talaria-protocol/src/lib.rs` - `socket_dir`, `ensure_socket_dir` (0700, checked), `current_uid`; `socket_path` is now `socket_dir().join(SOCKET_FILE)`; module docstring states the owner-only invariant and why. No new dependency — still serde-only.
- `crates/talaria-shell/src/control.rs` - `ensure_socket_dir()` before bind, checked 0600 chmod after bind, both refusing to serve on failure; `peer_uid_ok` helper and its call in the accept loop before the spawn.
- `tests/e2e/harness.py` - `socket_path()` mirroring the Rust logic; `SOCK` now assigned from it; isolation-knob docstring describes the fallback.
- `tests/e2e/control_socket_test.py` - permissions block (socket 0600, owner UID, fallback dir 0700), cross-UID skip line, docstring sentence.
- `tests/e2e/{scheme_refusal,crash_recovery,crash_event,timeout_session,popup}_test.py` - socket constant replaced with the mirrored expression, nothing else.

## Decisions Made

- **Fail closed on credential-lookup error.** `Err` from `peer_cred()` is treated as a rejection, not a soft admit. A peer the kernel will not vouch for is precisely this gate's case.
- **Silent rejection to the peer, loud rejection to the operator.** Nothing is written back on the socket (no probe oracle); every rejection logs at `warn` naming both UIDs, per CONVENTIONS' error-vs-warn split.
- **`ensure_socket_dir` is a no-op under XDG.** Creating or chmodding `$XDG_RUNTIME_DIR` is the OS's job and doing it ourselves would be wrong on systems that manage it; the XDG socket path is byte-identical to what it was before this plan.
- **`SOCKET_FILE` constant + `socket_dir().join(...)`.** Splitting the path means the directory that gets chmodded and the socket that gets bound cannot drift apart.
- **Exact mirroring in Python, including the degenerate case.** `os.environ.get("XDG_RUNTIME_DIR", <fallback>)` reproduces Rust's `env::var` returning `Ok("")` for an empty-but-set variable, which `or`-style fallback would not.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] A sixth suite carried the stale socket constant**

- **Found during:** Task 3
- **Issue:** The plan enumerates five suites with the hardcoded `os.environ.get("XDG_RUNTIME_DIR", "/tmp") + "/talaria.sock"` line, but plan 02-02 added `tests/e2e/scheme_refusal_test.py` after this plan was written, and it carries the same line. The plan's own acceptance criterion demands zero matches across `tests/e2e/`, so leaving it would both fail acceptance and leave a suite pointing at a path the shell no longer uses on the fallback.
- **Fix:** Applied the identical one-line replacement to `scheme_refusal_test.py`.
- **Files modified:** `tests/e2e/scheme_refusal_test.py`
- **Verification:** `grep -rn 'XDG_RUNTIME_DIR", "/tmp"' tests/e2e/` returns nothing; `scheme_refusal_test` PASSes in the full run.
- **Committed in:** `935d4ae` (Task 3 commit)

**2. [Rule 2 - Missing Critical] Unlink the socket when the chmod fails**

- **Found during:** Task 1
- **Issue:** The plan says to return without serving when the 0600 chmod fails, but a bound-then-abandoned socket file would be left behind at whatever (possibly permissive) mode the chmod failed to correct.
- **Fix:** Drop the listener and `let _ = std::fs::remove_file(&path)` before returning, so the refusal leaves nothing reachable behind.
- **Files modified:** `crates/talaria-shell/src/control.rs`
- **Verification:** Build + clippy clean; the success path is exercised by the full e2e run.
- **Committed in:** `5fd1985` (Task 1 commit)

---

**Total deviations:** 2 auto-fixed (1 blocking, 1 missing critical)
**Impact on plan:** Both are inside the plan's stated intent (zero stale socket-path resolvers; fail closed). No scope creep, no new dependency, no protocol change.

## Issues Encountered

- The first cross-UID probe asserted the intruder would read `b''`. It actually got `ConnectionResetError` — the server drops the stream while the client's `hello` write is still in flight, so the kernel resets rather than delivering a clean EOF. That *is* the rejection; the probe assertion was widened to accept either shape. Worth knowing for any future peer-rejection e2e suite: assert "refused", not "clean EOF".
- The filesystem alone refuses another user (0700 dir, 0600 socket), which masks the peer check. The probe deliberately widened the directory to 0755 and the socket to 0666 first, so the refusal observed is attributable to `peer_uid_ok` and nothing else. The widened directory was removed afterwards.

## Verification Evidence

- `cargo build --release` — exit 0.
- `cargo test` — 3 passed, 0 failed (2 protocol round-trip, 1 vault).
- `cargo clippy --all-targets -- -D warnings` — still fails with **exactly the 7 pre-existing lints** owned by plan 02-11 (`talaria-mcp/src/tools.rs:97,132`; `talaria-shell/src/app.rs:708 ×2, 733 ×2`; `talaria-shell/src/tabs.rs:93`). Lint locations were captured before and after each task and are byte-identical: **no new lint was introduced** by this plan. The plan lists this command as acceptance "exits 0", which it cannot at this phase baseline; the honest reading is "no new lints", and that holds.
- `TALARIA_E2E_DISPLAY=:98 XDG_RUNTIME_DIR=/tmp/talaria-e2e-rt python3 tests/e2e/run_all.py` — **exit 0, 10/10 PASS**, matching the 02-01 baseline verdict with no suite regressed.
- Fallback-path run of `control_socket_test.py` against a shell with `XDG_RUNTIME_DIR` unset — exit 0, with `SOCKET PERMS ok: /tmp/talaria-1001/talaria.sock 0o600` and the 0700 directory assertion exercised (the branch `run_all.py` never takes).

## Known Stubs

None. No placeholder, empty-collection, or TODO path was introduced.

## Threat Flags

None. No new network endpoint, auth path, file-access pattern, or schema change at a trust boundary was introduced beyond the ones the plan's own threat register already covers.

## User Setup Required

None - no external service configuration required.

Note for anyone with a running Talaria and no `XDG_RUNTIME_DIR`: the fallback socket path changed, so a stale `/tmp/talaria-$UID.sock` from a pre-this-plan build is now inert and can be deleted. All three Rust consumers and every Python suite follow the new path automatically.

## Next Phase Readiness

- SEC-01 is fully closed: both halves (peer authentication and filesystem reachability) shipped and verified, so the requirement moves to Complete rather than In Progress.
- Plan 02-11 (CI) inherits an unchanged clippy baseline of 7 lints — this plan added none.
- Phase 4's HTTP transport will need its own authentication story; `peer_uid_ok` is Unix-socket-specific and deliberately does not generalize. The scoping comment above the helper says so in-file.

## Self-Check: PASSED

All five key files exist on disk; all three task commits (`5fd1985`, `3308c4b`, `935d4ae`) are present in git history.

---
*Phase: 02-harden-the-agent-surface*
*Completed: 2026-08-16*

---
phase: 02-harden-the-agent-surface
plan: 04
subsystem: security
tags: [rust, ureq, download, filesystem, dos, tokio-oneshot, e2e, xvfb]

# Dependency graph
requires:
  - phase: 02-01
    provides: GREEN e2e baseline at 16411ee and the enforcing .githooks/pre-commit
  - phase: 02-02
    provides: agent URL scheme allowlist (declared depends_on; no code overlap in practice)
provides:
  - "Byte-capped download loop with an env-tunable ceiling (TALARIA_MAX_DOWNLOAD_BYTES, default 2 GiB)"
  - "Exclusive-create uniquifying destination (create_unique) — an agent download can never clobber a user file"
  - "Connect/read timeouts plus an overall deadline on the download request, all under the command timeout"
  - "Cancellation via the reply oneshot's closed state, so the detached thread cannot outlive the request"
  - "Partial-file cleanup on every failure path (cap, read, write, timeout, cancel)"
  - "Download tool description disclosing the size cap and the session-cookie limitation"
  - "tests/e2e/download_bounds_test.py — standalone suite proving all of the above"
affects: [phase-3-downloads-ui, phase-3-servo-fetch-rerouting, 02-11-ci]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "ureq::AgentBuilder with timeout_connect/timeout_read instead of the bare module-level ureq::get"
    - "tokio oneshot Sender::is_closed() as a zero-cost cancellation handle for off-thread work"
    - "std::fs::OpenOptions::create_new() collision loop for no-clobber file creation"
    - "In-process ThreadingHTTPServer fixture with a deliberately stalling route"

key-files:
  created:
    - tests/e2e/download_bounds_test.py
  modified:
    - crates/talaria-shell/src/app.rs
    - crates/talaria-mcp/src/tools.rs
    - tests/e2e/run_all.py

key-decisions:
  - "The cap and the reported byte count are both computed from bytes actually read and written, never from content-length — the header is attacker-controlled"
  - "An unparseable TALARIA_MAX_DOWNLOAD_BYTES falls back to the 2 GiB default, never to zero and never to unbounded, so a typo can neither disable downloads nor disable the cap"
  - "download_timeout() reuses promise_wait()'s derivation (command timeout minus 2s) and is applied three ways: connect, per-read, and as an overall deadline"
  - "A leading dot is treated as part of the name rather than an extension separator, so .bashrc uniquifies as '.bashrc (1)'"
  - "create_unique stops after 1000 attempts and returns AlreadyExists rather than looping forever"
  - "abandon_download was extracted as a free function rather than a closure, because a closure capturing the writer by move cannot be called from inside a loop"

patterns-established:
  - "Cancellation handle: borrow the reply oneshot Sender into off-thread work and poll is_closed() at each loop iteration — the control socket already drops the receiver on command timeout, so no new type is needed"
  - "No-clobber file creation: create_new() in a counter loop, advancing only on AlreadyExists, so concurrent writers each get their own file"
  - "Own-shell e2e isolation: point HOME and XDG_CONFIG_HOME at a temp dir and write a user-dirs.dirs so dirs::download_dir() resolves inside the fixture"

requirements-completed: [MCP-12]

coverage:
  - id: D1
    description: "A download body is capped in bytes; at-cap succeeds reporting the exact byte count, cap-plus-one is refused naming the cap and leaves no partial file"
    requirement: MCP-12
    verification:
      - kind: e2e
        ref: "tests/e2e/download_bounds_test.py#cap boundary"
        status: pass
    human_judgment: false
  - id: D2
    description: "A name collision uniquifies via exclusive create instead of clobbering; two same-name downloads produce two distinct, intact files"
    requirement: MCP-12
    verification:
      - kind: e2e
        ref: "tests/e2e/download_bounds_test.py#no clobber + concurrent same name"
        status: pass
    human_judgment: false
  - id: D3
    description: "The download carries its own connect/read timeout and overall deadline plus a cancellation handle, so a stalling peer ends inside the command-timeout window and leaves nothing behind"
    requirement: MCP-12
    verification:
      - kind: e2e
        ref: "tests/e2e/download_bounds_test.py#timeout and cleanup"
        status: pass
    human_judgment: false
  - id: D4
    description: "The download tool description states the size cap, its env knob, the uniquifying rule, and the session-cookie limitation (D-20 deferral disclosed rather than hidden)"
    verification:
      - kind: e2e
        ref: "tests/e2e/mcp_client_test.py (tool-list assertion still passes — name and schema unchanged)"
        status: pass
      - kind: other
        ref: "grep -c 'TALARIA_MAX_DOWNLOAD_BYTES' crates/talaria-mcp/src/tools.rs == 1"
        status: pass
    human_judgment: false

# Metrics
duration: 26min
completed: 2026-08-16
status: complete
---

# Phase 2 Plan 04: Download Bounds Summary

**`download` is now capped at a tunable 2 GiB, uniquifies instead of clobbering, times out under the command timeout, cancels when the agent stops waiting, and deletes its partial file on every failure path — with an 11th e2e suite pinning all of it.**

## Performance

- **Duration:** 26 min
- **Started:** 2026-08-16T10:00:00+04:00 (approx; first task commit 10:18)
- **Completed:** 2026-08-16T10:26:00+04:00
- **Tasks:** 3
- **Files modified:** 4 (3 modified, 1 created)

## Accomplishments

- **Disk-fill closed (T-02-04-01).** The whole-body `std::io::copy` is gone. A 64 KiB chunked loop keeps a running total and aborts the moment it would exceed `max_download_bytes()` — `TALARIA_MAX_DOWNLOAD_BYTES` or 2 GiB. The count is from bytes actually read and written, so a lying `Content-Length` buys nothing (T-02-04-05).
- **Silent clobber closed (T-02-04-02, T-02-04-04).** `create_unique()` opens the destination with `create_new(true)` and walks `name`, `name (1)`, `name (2)` … up to 1000, advancing only on `AlreadyExists`. Exclusive creation is what makes this hold under a race: the loser retries the next counter rather than opening the winner's file. The returned path is the one actually written.
- **Unbounded detached thread closed (T-02-04-03, T-02-04-06).** The request now goes through a `ureq::AgentBuilder` with `timeout_connect` and `timeout_read` set to `download_timeout()` (command timeout minus 2s), and the copy loop checks an overall deadline plus `reply.is_closed()` on every iteration. The control socket already drops the oneshot receiver when the command timeout fires, so a closed sender is a real, pre-wired cancellation signal.
- **Partial files never survive.** `abandon_download()` drops the writer, removes the file, and returns the error — used by the cap, read-error, write-error, timeout and cancellation paths alike.
- **Deferral disclosed, not hidden (T-02-04-07 / D-20).** The download tool description now tells an agent that the fetch bypasses the browser's network stack and carries no session cookies (so a URL behind a login returns the login page), names the two workarounds, and states the cap, its knob, and the uniquifying rule. Tool name, struct, schema and `tool_box!` registration are untouched.
- **Everything above is pinned by a committed suite** that runs the real release binary against a real HTTP server, including a route that stalls mid-body.

## Task Commits

1. **Task 1: Cap the body, uniquify the destination, bound the request (D-17/D-18/D-19)** — `7e7c623` (feat)
2. **Task 2: State the download tool's real limits to agents (D-20)** — `6967738` (docs)
3. **Task 3: Prove the cap, the cleanup, and the no-clobber rule end to end** — `76eaa9e` (test)

**Plan metadata:** see the final `docs(02-04)` commit.

## Files Created/Modified

- `crates/talaria-shell/src/app.rs` — added `DEFAULT_MAX_DOWNLOAD_BYTES`, `max_download_bytes()`, `download_timeout()`, `create_unique()`, `abandon_download()`; rewrote `download()` as a capped, cancellable chunked loop taking `&tokio::sync::oneshot::Sender<Outcome>`; `Command::Download` now passes the reply sender in as the cancellation handle. Added `use std::io::{Read, Write};`.
- `crates/talaria-mcp/src/tools.rs` — download tool description extended with the cap, the env knob, the uniquifying rule and the session-cookie limitation.
- `tests/e2e/download_bounds_test.py` — new standalone suite (own Xvfb + shell, `HOME`/`XDG_CONFIG_HOME` in a temp dir, `TALARIA_MAX_DOWNLOAD_BYTES=1024`, `TALARIA_COMMAND_TIMEOUT_SECS=5`, in-process `ThreadingHTTPServer` with at-cap / over-cap / small / stall routes).
- `tests/e2e/run_all.py` — registered `download_bounds_test` in the phase-2 tuple; docstring updated.

## Decisions Made

- **`abandon_download` is a free function, not a closure.** A closure capturing the `BufWriter` by move cannot be called from multiple points inside a loop. A free function taking the writer by value works because every call site immediately `return`s, so the move is on a diverging path.
- **Results are bound to a local before `if let`.** `if let Err(e) = writer.write_all(..)` would hold the `&mut writer` autoref temporary for the whole `if let` block in edition 2021, conflicting with moving `writer` into `abandon_download` in the body. `let write_result = writer.write_all(..); if let Err(e) = write_result { … }` sidesteps it.
- **A leading dot is part of the name.** `create_unique` only splits at a `.` found at index > 0, so `.bashrc` uniquifies as `.bashrc (1)` rather than ` (1).bashrc`.
- **`download_timeout()` is applied three ways** — connect, per-read, and as an overall wall-clock deadline. The per-read timeout alone cannot catch a peer that drips one byte just inside the window; the deadline can.
- **The e2e suite writes a `user-dirs.dirs`.** `dirs::download_dir()` resolves through `dirs-sys::user_dir("DOWNLOAD")`, which parses `$XDG_CONFIG_HOME/user-dirs.dirs` and returns `None` when that file is absent — in which case the shell would fall back to its CWD, not the temp `Downloads`. Writing the file makes the fixture deterministic.
- **The stall route sends headers and a few bytes before stalling**, rather than accepting and sending nothing. Stalling before headers would fail inside `.call()` and never create a file, so the partial-file cleanup path would go untested. Stalling mid-body exercises it.

## Deviations from Plan

None — plan executed exactly as written. All three tasks landed with their stated acceptance criteria met on the first build, and the new e2e suite passed on its first run.

## Issues Encountered

None. The pre-existing clippy state was re-confirmed rather than fixed:

**Known RED, not introduced here:** `cargo clippy --all-targets -- -D warnings` still fails with exactly the 7 baseline lints — `crates/talaria-mcp/src/tools.rs` (97, 132), `crates/talaria-shell/src/app.rs` (709 ×2, 734 ×2), `crates/talaria-shell/src/tabs.rs` (93). The `app.rs` line numbers shifted from 708/733 to 709/734 purely because this plan added one import line; they are the same four `unnecessary_cast` lints. No new lint was introduced. Plan 02-11 (CI) owns fixing these.

## Verification

| Check | Result |
|---|---|
| `cargo build --release` | exit 0 |
| `cargo clippy --all-targets -- -D warnings` | 7 pre-existing lints only; **zero new** |
| `cargo test` | 3 passed, 0 failed |
| `python3 tests/e2e/download_bounds_test.py` standalone | exit 0, `DOWNLOAD BOUNDS CHECKS PASSED` |
| Isolated full regression (`run_all.py`) | **11/11 PASS**, `failed: none`, exit 0 |

Full regression detail — the 10 suites at the 02-01 baseline all still PASS (control_socket, scheme_refusal, crash_recovery, crash_event, timeout_session, mcp_client, single_instance, popup, keyboard_nav, takeover), plus the new `download_bounds_test` (17s).

Observed suite output, worth recording because it is the behavioural proof:

```
AT CAP ok: …/Downloads/at-cap.bin 1024 bytes
OVER CAP refused: download exceeds the 1024 byte cap (raise TALARIA_MAX_DOWNLOAD_BYTES to allow more)
NO CLOBBER: note.txt + note (1).txt
CONCURRENT same name -> race.dat + race (1).dat
STALL bounded at 3.1s (< 5s): read failed: timed out reading response
STALL left nothing behind
PATH SEPARATOR still refused
```

## Requirements

**MCP-12** (`download` is bounded — enforced size cap and no silent overwrite of an existing file) is **fully closed**. Both halves of the requirement text ship here and both are proven end to end. Nothing is left In Progress.

## Threat Model Disposition

| Threat ID | Disposition | Status |
|---|---|---|
| T-02-04-01 unbounded body fills the disk | mitigate | Closed — running-total cap, partial removed |
| T-02-04-02 silent overwrite | mitigate | Closed — `create_new` + uniquified name |
| T-02-04-03 thread outlives the command timeout | mitigate | Closed — `is_closed()` + deadline, both delete |
| T-02-04-04 race on the uniquified name | mitigate | Closed — exclusive create; e2e asserts two distinct files |
| T-02-04-05 lying content-length | mitigate | Closed — count from bytes read/written |
| T-02-04-06 slow-drip defeating per-read timeout | mitigate | Closed — overall deadline checked per iteration |
| T-02-04-07 login-wall page saved as the resource | transfer | Deferred to Phase 3 per D-20; **disclosed** in the tool description |
| T-02-04-08 path traversal via filename | mitigate | Preserved unchanged; e2e asserts `../` and `sub/` are still refused |
| T-02-04-09 package installs | accept | No dependency added; `ureq` was already a workspace dependency |

No new security surface was introduced beyond the plan's threat register, so there are no threat flags to raise.

## User Setup Required

None — no external service configuration required. The new `TALARIA_MAX_DOWNLOAD_BYTES` knob is optional and defaults to 2 GiB.

## Next Phase Readiness

- Plan 02-04 was wave 3 and is self-contained; nothing downstream in phase 2 is blocked by it.
- `crates/talaria-shell/src/app.rs` gained ~150 lines near the bottom of the file. Later `app.rs` plans in this phase (02-06 in-flight tracking, 02-07 event fan-out) work in different regions, but they must re-locate by symbol rather than by the pre-change line numbers in 02-PATTERNS.md — those are now off by one at the top of the file and by ~150 below `capture_now`.
- Plan 02-11 (CI) inherits the same 7 clippy lints it already owned; this plan added none.
- Phase 3's downloads UI / progress model now has a bounded, cancellable copy loop to build a progress channel onto, and the D-20 Servo-fetch rerouting is recorded in the tool description as the visible follow-on.

## Self-Check: PASSED

All three task commits verified present in `git log --all` (`7e7c623`, `6967738`, `76eaa9e`). All four claimed files verified present on disk.

---
*Phase: 02-harden-the-agent-surface*
*Completed: 2026-08-16*

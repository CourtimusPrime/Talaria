---
phase: 02-harden-the-agent-surface
plan: 01
subsystem: test-infrastructure
tags: [baseline, e2e, branch-lock, git-hooks, pid-liveness]
requires: []
provides:
  - attributable-e2e-baseline-for-HEAD
  - pid-liveness-aware-overnight-lock
  - overnight_lock-check-subcommand
  - pre-commit-branch-lock-enforcement
affects:
  - tests/e2e/overnight_lock.py
  - .githooks/pre-commit
  - tests/e2e/README.md
tech-stack:
  added: []
  patterns:
    - "_pid_alive copied verbatim from harness.py — zombie counts as dead"
    - "POSIX sh hook fails open only for missing python3 / missing script"
key-files:
  created:
    - .githooks/pre-commit
  modified:
    - tests/e2e/overnight_lock.py
    - tests/e2e/README.md
decisions:
  - "D-28 implemented by changing what is recorded (ppid or explicit --pid) before adding liveness, since the old os.getpid() field was dead by design"
  - "D-29 implemented as a pre-commit hook calling a new read-only `check` subcommand, rather than as a loop-preflight convention"
  - "pid_source key added so a reader can distinguish a loop-supplied PID from the ppid fallback"
metrics:
  duration: ~15 min
  completed: 2026-08-16
status: complete
---

# Phase 2 Plan 01: Green Baseline and Branch-Lock Liveness Summary

Established an attributable all-green e2e baseline for HEAD, then closed the audit's
"refused by a corpse" finding by recording an owning PID that actually outlives the acquiring
script and enforcing the lock through a pre-commit hook.

## Task 1 — Green e2e baseline for HEAD

**Built commit SHA:** `16411ee673ff5b9a228333e0768564767c09b03e`
**Toolchain:** `cargo 1.97.1 (c980f4866 2026-06-30)`
**Invocation:** `TALARIA_E2E_DISPLAY=:98 XDG_RUNTIME_DIR=/tmp/talaria-e2e-rt TALARIA_E2E_OUT=/tmp/talaria-e2e-baseline python3 tests/e2e/run_all.py`
**Total exit code:** `0` (exit code is the count of failed suites)

`cargo build --release` exited 0; both `target/release/talaria` and `target/release/talaria-mcp`
are present. The build was already current for this source tree, so cargo reported no work to do.

Per-suite verdict, first run, no reruns needed:

| Suite | Verdict | Duration |
|-------|---------|----------|
| control_socket_test | PASS | 1s |
| crash_recovery_test | PASS | 11s |
| crash_event_test | PASS | 4s |
| timeout_session_test | PASS | 9s |
| mcp_client_test | PASS | 7s |
| single_instance_test | PASS | 0s |
| popup_test | PASS | 1s |
| keyboard_nav_test | PASS | 32s |
| takeover_test | PASS | 27s |

Baseline verdict: GREEN

No suite failed, so no environmental determination was made and no remediation was run. There is
no code defect at HEAD, so the phase proceeds and Tasks 2 and 3 were executed as planned.

Prerequisites were confirmed present before the run rather than assumed: `Xvfb`, `xdotool`,
`xdpyinfo`, `xwd`, `convert`, `pkill`, `python3` (3.14.6). A stale `/tmp/.X98-lock` was on disk;
`harness._clear_stale_x_lock` handled it as designed, so it required no intervention.

`tests/e2e/README.md` gained a `## Baseline` section pinning this exact invocation, including the
statement that a skipped run is reported as a failure and never as a pass.

## Task 2 — Real owning PID and self-clearing dead locks (D-28)

The recorded `pid` was `os.getpid()` of the acquiring python process, which exits the moment
`acquire` returns — dead by design. A liveness check bolted onto that field would have cleared
every lock, including live ones, so what gets recorded changed first:

- `acquire` gained `--pid <n>`, defaulting to `os.getppid()` — the shell or loop driver that
  outlives the script.
- New sibling key `pid_source`, valued `argv` or `ppid`, so a reader can tell a deliberate
  identity from the fallback. `owner`, `branch` and `started` are unchanged.
- `_pid_alive` copied verbatim from `tests/e2e/harness.py` including its docstring; no import,
  since the two files are independent by design. Verified empirically that a killed-but-unreaped
  process (`/proc` state `Z (zombie)`) returns `False`.
- `held_is_live` treats a missing or unparseable `pid` as dead, so an unattributable lock cannot
  block a session forever.
- A foreign lock that is not live falls through to the write and prints
  `CLEARED stale lock: session <owner> (pid <n>) is no longer running.` ahead of the ACQUIRED
  line. A foreign lock that *is* live still prints REFUSED and returns 1. The same-owner
  "HELD by this session" branch is untouched.
- New `check` subcommand: exits 1 when a live lock is held by an owner other than
  `TALARIA_OVERNIGHT_OWNER`, 0 otherwise. An unset variable counts as not-the-owner. It never
  writes the lock file. Wired into `__main__` alongside `status`, taking no owner argument.
- Module docstring extended with the new flag and subcommand, plus a paragraph explaining why the
  recorded PID had to change before liveness could mean anything, and a note that the lock is a
  cooperative marker rather than a security control (threat T-02-01-02).

**The fixture lock cleared.** The `.overnight-lock` that sat in the working tree (owner
`c85a1ef3`, pid 2796577) was restored verbatim and run against the new `acquire`, which reported
`CLEARED stale lock: session c85a1ef3 (pid 2796577) is no longer running.` and exited 0 rather
than refusing. `/proc/2796577` does not exist. It was then released; the working tree ends with no
`.overnight-lock`.

Verified behaviors:

- dead-PID foreign lock → `CLEARED stale lock`, exit 0
- live-PID foreign lock → REFUSED, exit 1
- `check` with matching `TALARIA_OVERNIGHT_OWNER` → exit 0; unset → exit 1
- `acquire probe --pid 1` → JSON `pid` 1, `pid_source` `argv`; default → `pid_source` `ppid`
- `grep -c '_pid_alive'` → 2; `ast.parse` of the file → exit 0

## Task 3 — Non-optional preflight via pre-commit hook (D-29)

`.githooks/pre-commit` created, mode 0755, POSIX sh (`sh -n` clean). It resolves the repo root,
changes into it, runs `python3 tests/e2e/overnight_lock.py check`, and exits with that status.
`git config core.hooksPath .githooks` was run in this repo, so the hook is live here.

The header comment states the incident it prevents (two loops on one branch and working tree, one
soaking a binary the other rebuilt underneath it), names `TALARIA_OVERNIGHT_OWNER` as how a loop
identifies itself, and documents `--no-verify` as the escape hatch for a human who has confirmed
the other session is stopped.

Per threat T-02-01-03 the hook fails open — one-line stderr warning, exit 0 — in exactly two
cases: `python3` not on PATH, or the lock script absent from the checkout. Every other non-zero
status from `check` fails the commit.

**Proven against a real commit, not just the hook script:** with a live foreign lock held,
`git commit` exited 1, HEAD did not advance, and the staged files stayed staged. After releasing
the lock the same commit succeeded — and that commit itself ran through the hook, so the
no-false-positive case is proven too.

`tests/e2e/README.md` gained a `## Branch lock (overnight loops)` section next to Baseline,
documenting `core.hooksPath`, the `TALARIA_OVERNIGHT_OWNER` convention, the acquire/release shape
with `--pid $$`, and the `--no-verify` escape hatch.

## Deviations from Plan

None — the plan executed as written. No deviation rule was invoked.

One judgement worth recording, well inside the plan's letter: the plan's Task 3 verify used
`overnight_lock.py acquire other-session` with no `--pid`. Under the new default that records the
invoking shell's PID, which is alive, so the lock is correctly live for the test. The verify was
run with an explicit `--pid $$` as well, to make the liveness deliberate rather than incidental.

## Threat Flags

None. This plan added no network endpoint, auth path, or schema at a trust boundary. The one new
trust surface — the lock file's `pid` field now being trusted for a liveness decision — was
already enumerated in the plan's threat model (T-02-01-01, T-02-01-02, T-02-01-05) and its
dispositions are implemented: `pid_source` records provenance, the stale-clear path prints the
previous owner and pid so takeover is visible (T-02-01-04), and PID reuse fails safe toward
refusal with the existing `--force` as recovery.

## Known Stubs

None.

## Requirements Progress

TEST-03 — partially advanced. This plan carried the branch-lock half (D-28, D-29). The CI half
(D-26, D-27) belongs to plan 02-11 and is untouched here.

## Verification

1. `cargo build --release` exits 0 — confirmed.
2. Isolated baseline ran to completion; per-suite verdicts, commit SHA, toolchain and the single
   branch line are recorded above.
3. All three task-level automated verifies exit 0.
4. The working tree carries no `.overnight-lock`.

## Commits

| Task | Commit | Description |
|------|--------|-------------|
| 1 | `0f6752f` | docs(02-01): record the isolated e2e baseline invocation |
| 2 | `f6b42c5` | feat(02-01): self-clear dead overnight locks and add a check subcommand |
| 3 | `45e5b9f` | feat(02-01): enforce the overnight branch lock at commit time |

## Self-Check: PASSED

All three commits exist in `git log`. All three files exist on disk. No tracked file was deleted
by any commit in this plan.

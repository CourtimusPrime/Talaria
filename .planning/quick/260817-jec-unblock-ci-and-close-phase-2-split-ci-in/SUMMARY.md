---
quick_id: 260817-jec
slug: unblock-ci-and-close-phase-2-split-ci-in
date: 2026-08-17
status: complete
---

# Summary: unblock CI and close Phase 2

All five tasks done. Phase 2 is closed, with a green CI run and a green 14/14 e2e
run behind it rather than a local assertion that it would have been green.

## What changed

**Remote and runner (prerequisite, done before the tasks).**
`git@github.com:CourtimusPrime/Talaria.git`, public, 105 commits pushed after a
secret and personal-infrastructure scan came back clean. A self-hosted runner
named `thinkpad` is registered as a systemd service. GitHub Actions was hardened
first: fork PRs from **all** external contributors require approval, and
`GITHUB_TOKEN` defaults to read-only. That ordering was deliberate — a
self-hosted runner on a public repository executes strangers' pull requests on a
personal machine, and the runner should not exist before the controls do.

**Task 1 — the CI split.** One `ubuntu-latest` job became three workflows:

| Workflow | Runner | Trigger | Contents |
|---|---|---|---|
| `ci.yml` | self-hosted | every push, every PR | build, clippy, unit tests |
| `e2e.yml` | self-hosted | merge to `main`, nightly, manual | 14 Xvfb suites |
| `cold-build.yml` | `ubuntu-latest` | weekly | from-scratch build on a stock image |

The measurements behind it: `target/` is 42 GB against a 10 GB `actions/cache`
cap, so the cache never wrote and every hosted run rebuilt Servo cold on a
180-minute budget; a warm rebuild of the two workspace crates is 2m47s. The
first self-hosted run came in at **2m54s**.

Both self-hosted jobs carry an `if:` gate admitting only pushes and same-repo
pull requests — a second control that, unlike the repository setting, cannot be
changed without a reviewable commit. No workflow has a `paths:` filter, because a
filtered-out run reports as a green tick.

The runner shares the warm dependency build through a machine-local
`CARGO_TARGET_DIR` (set in `~/actions-runner/.env`, not in the repository, so no
home path is published). `harness.py` and `mcp_client_test.py` were taught to
resolve the binary through that variable instead of assuming `./target`.

**Task 2 — `SECURITY.md`.** Reporting instructions, the three-party trust model
stated plainly, what is enforced today, and the two known limitations with their
mechanism, their blocking reason, and what closing each would take. It also
records *why* `parse_agent_url` and `resolve_location` are deliberately separate
code paths, since that duplication is the kind a future reader removes.

**Task 3 — Phase 2 closed.** MCP-09 and MCP-10 moved from Pending / In Progress
to a new **Documented Limitation** status, with a status vocabulary added to
`REQUIREMENTS.md` so the distinction between "not scheduled" and "cannot
currently be built" survives. AGENT-04 became **Deferred (v2)**. TEST-03 was
marked complete only after real green runs, and cites their ids.

**Task 4 — v1 is Phases 1–3.** Phases 4–7 keep their numbers and detail and are
labelled v2.

**Task 5 — planning artifact policy** recorded in `PROJECT.md`: PLAN.md up front
only for the security and protocol surface; SUMMARY.md always.

## The thing worth remembering

The first real CI run failed 11 of 14 suites, and the failure was not a CI
problem.

Every suite died on the command timeout — including `tabs_list`, which touches no
network and no page. The shell answered `hello` and served nothing. Direct
inspection of the wedged process showed **one thread**, in `sigsuspend`, with
`dbus-1/`, `at-spi/`, `dconf/` and `keyring/` directories freshly created inside
the runtime dir.

`Vault::load()` runs on the main thread during `Shared` construction and asks the
OS keychain for the vault key through `keyring`, which on Linux is the Secret
Service over D-Bus. With `DBUS_SESSION_BUS_ADDRESS` unset, libdbus falls back to
autolaunch and tries to start a bus itself — and blocks the main thread
indefinitely when it cannot. The control socket is on its own thread, which is
exactly why the process looked healthy.

This could not have been found locally. Interactively, autolaunch finds the
desktop session's bus at `/run/user/$UID/bus` and returns at once; every prior
e2e run inherited it. It needs `XDG_RUNTIME_DIR` pointed at a directory with no
bus, which is what a CI job does and what a headless server, a container, or an
SSH session also does.

CI is worked around with `dbus-run-session`. The underlying hang is a real
user-facing bug and is logged in `deferred-items.md` as a candidate v1 blocker;
fixing it means the keychain lookup must not be able to block startup, which
changes `vault.rs` and therefore needs its own plan under the policy written in
this same task.

Three hypotheses were tested and discarded before that one landed (network
reachability, path length on the Unix socket, and D-Bus alone without the
runtime-dir interaction). The first D-Bus test was a false negative because the
repro inherited a working session bus.

## Verification

- `ci.yml` run `32019859735` — success, 2m54s, build + clippy + `cargo test`.
- `e2e.yml` run `32019859744` — success, **14/14 PASS, 0 FAIL**.
- Workflow YAML parsed and asserted: no `paths:` filters, both self-hosted jobs
  carry the fork gate, hosted jobs are schedule-only.

## Not done

- **AGENT-04's `TabOpened` wire variant.** Deferred to v2 rather than
  implemented — it is a protocol change and was not in scope for this task.
- **The `Vault::load()` hang itself.** Worked around in CI, not fixed.
- `lockfile-audit.yml` was left untouched: schedule-only on `ubuntu-latest`, no
  fork exposure, and fresh dependency resolution is exactly what it should do on
  a stock image.

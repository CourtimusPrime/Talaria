---
quick_id: 260817-kbw
slug: fix-the-vault-d-bus-startup-hang-and-del
date: 2026-08-17
status: complete
---

# Quick task: clean the platform before Phase 3

Two open items survive Phase 2. Both touch the security or protocol surface, so
under the policy in `PROJECT.md` both get a plan.

## Task 1 — `Vault::load()` must not be able to block startup

**The bug.** `Shared` construction calls `Vault::load()` on the main thread
(`app.rs:734`), which calls `load_or_create_key` (`vault.rs:103`), which calls
`keyring::Entry::new("talaria", "vault").get_password()`. On Linux that is the
Secret Service over D-Bus. With no `DBUS_SESSION_BUS_ADDRESS`, libdbus falls
back to *autolaunch* and tries to start a session bus itself — and blocks
indefinitely when it cannot. The control-socket thread is separate, so the shell
answers `hello` and serves nothing: every command dies on the command timeout,
including `tabs_list`. Verified stuck at both a 3s and a 40s timeout, with the
process sitting at one thread in `sigsuspend`.

Affects any headless machine, container, or SSH session — not just CI.

**The fix, in two layers.**

1. *Do not provoke autolaunch when there is demonstrably no bus.* On Linux, if
   `DBUS_SESSION_BUS_ADDRESS` is unset **and** `$XDG_RUNTIME_DIR/bus` does not
   exist, there is no session bus to find; skip the keychain and take the key
   file path immediately. Costs a `stat`, and turns the common headless case
   into an instant, correct downgrade instead of a hang.
2. *Bound the call anyway.* A bus address that is set but dead, or a Secret
   Service that accepts and never answers, still hangs — and layer 1 cannot see
   either. Run the keychain lookup on a worker thread and wait with a timeout.
   On expiry, log and fall back.

Layer 2 is what makes this correct rather than merely better; layer 1 is what
keeps the common case instant and avoids stranding a thread. Neither weakens the
key handling: every path out of `load_or_create_key` already reports a downgrade
through `KeyOutcome.downgraded`, and both new paths set it.

**Accepted cost.** A worker thread that never returns is detached and leaks. It
is one thread, blocked, for the process lifetime, and the alternative is the
whole browser hanging. Recorded in the code, not hidden.

**Timeout value.** 1500ms. It must sit below the shortest command timeout the
e2e suite uses (3s, set by `run_all.py`), or a slow keychain turns into failing
tests rather than a warning; and it must be far above a healthy keychain
lookup, which is single-digit milliseconds. Overridable with
`TALARIA_KEYCHAIN_TIMEOUT_MS` so a machine with a genuinely slow keychain is not
forced into a downgrade.

**Acceptance:**
- The shell starts and answers `tabs_list` with no session bus and a fresh
  `XDG_RUNTIME_DIR`, well inside the 3s command timeout.
- `dbus-run-session` is **removed** from `e2e.yml`, so CI exercises the no-bus
  path for real instead of papering over it.
- A new e2e suite pins the behaviour so it cannot regress silently.
- Unit tests cover the bounded-call helper in both directions.

## Task 2 — AGENT-04: a `TabOpened` wire event

`talaria_protocol::Event` has `TabCrashed` and `TabClosed` and no open variant,
so a popup adopted into an agent's session (`Shared::adopt_popup`) is
discoverable only by polling `tabs_list`. The proxy and notification plumbing
from plan 02-08 need no change — a new variant flows through the reader's event
arm as-is.

Add `Event::TabOpened { tab_id, opener_tab_id }`, raise it at the adoption site,
and assert it end to end. `opener_tab_id` is included because the event is only
raised for adoption: an agent's own `tabs_open` already learns the tab from its
own reply, so the opener is the whole point of the notification.

**Acceptance:** an agent whose page calls `window.open` receives a
`notifications/message` naming the new tab and its opener, without polling.
AGENT-04 moves to Complete.

## Verification

Full `cargo build --release`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`, and the full e2e suite locally, then green `ci.yml` and `e2e.yml`
runs on the self-hosted runner.

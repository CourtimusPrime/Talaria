---
quick_id: 260817-kbw
slug: fix-the-vault-d-bus-startup-hang-and-del
date: 2026-08-19
status: complete
---

# Summary: clean the platform before Phase 3

Both items landed. Full verification is green: `cargo build --release`,
`cargo clippy --all-targets --release` clean, 12 unit tests, and 15/15 e2e
suites — the suite run **without** `dbus-run-session`, which is the point of
task 1.

## Task 1 — `Vault::load()` can no longer block startup

Delivered as the plan's two layers, in `crates/talaria-shell/src/vault.rs`.

1. **`session_bus_reachable()`** — on Linux, `DBUS_SESSION_BUS_ADDRESS` unset
   *and* no `$XDG_RUNTIME_DIR/bus` means there is no bus to find, so the
   keychain is skipped rather than provoking D-Bus autolaunch. Costs a `stat`.
   Non-Linux keychains do not route through D-Bus, so the function is `true`
   there and only the timeout applies.
2. **`with_timeout()`** — the keychain conversation (`ask_keychain`) runs on a
   worker thread with a bounded wait, so a bus address that is set but dead, or
   a Secret Service that accepts and never answers, degrades instead of hanging.
   Default 1500ms, overridable with `TALARIA_KEYCHAIN_TIMEOUT_MS`.

Every new path sets `KeyOutcome.downgraded`, so the fallback is still reported
rather than silently taken.

**Accepted cost, recorded in the code:** a timed-out worker thread is detached
and leaked. It is one blocked thread for the process lifetime, on a machine that
is already misconfigured, and the alternative is the whole browser hanging.

**Deviation from the plan:** the plan named "unit tests cover the bounded-call
helper in both directions", which is delivered
(`with_timeout_returns_work_that_finishes_in_time`,
`with_timeout_gives_up_on_work_that_never_finishes`). A third test was renamed
during execution: it was called `keychain_timeout_default_is_used_for_nonsense_overrides`
but asserts the default sits inside its two bounds, and does not exercise the
override at all — reading the override from the process environment is racy
against parallel tests. It is now
`keychain_timeout_default_stays_inside_its_bounds`, which is what it checks.

### CI

`dbus-run-session` is removed from `.github/workflows/e2e.yml`. The long comment
that justified it is replaced by one explaining why the job now runs bare: a
headless server, a container, an SSH session and this job all look the same to
the vault, and running without a bus is what proves the fallback works. The
comment ends with an explicit "do not reintroduce".

`tests/e2e/vault_nobus_test.py` is new and wired into `run_all.py` as the last
standalone suite — last because it is the only suite that rewrites
`XDG_RUNTIME_DIR` and unsets `DBUS_SESSION_BUS_ADDRESS`. It asserts three
things: `tabs_list` answers well inside the 3s command timeout, a second command
is served (so the event loop is running rather than draining a queue), and the
runtime dir contains no bus scaffolding (so autolaunch was never provoked).

## Task 2 — `Event::TabOpened` (AGENT-04)

`talaria_protocol::Event::TabOpened { tab_id, opener_tab_id }` added and raised
at `Shared::adopt_popup`, mapped to `LoggingLevel::Info` in the MCP proxy. The
proxy's reader and drain task needed no change, exactly as plan 02-08 predicted.

`opener_tab_id` is carried because adoption is the only case that raises the
event: a tab the agent opened itself is already named in the reply to its own
`tabs_open`, so the opener is the whole point of the notification.

Asserted at both levels, and both assertions were confirmed to actually fire:

- `tests/e2e/popup_test.py` — on the raw control socket, for both `window.open`
  and `target=_blank`: `{"event": "tab_opened", "tab_id": 3, "opener_tab_id": 2}`.
- `tests/e2e/mcp_client_test.py` — as an MCP `notifications/message`, matched by
  opener (the new tab's id is knowable no other way, which is the point). The
  second session's silence assertion now also covers this variant, so it cannot
  fan out.

AGENT-04 moves from **Deferred (v2)** to **Complete** in `REQUIREMENTS.md`, and
the corresponding entry in the Phase 2 `deferred-items.md` is marked resolved.

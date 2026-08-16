# Deferred items — Phase 02

Out-of-scope discoveries logged during execution. Not fixed here.

## `cargo clippy --all-targets -- -D warnings` is red at the phase baseline

Found during plan 02-02, Task 1. Several plans in this phase carry
`cargo clippy --all-targets -- -D warnings` as an acceptance criterion. That
command **already fails on unmodified code** at commit `16411ee` (plan 02-01's
green baseline). Seven lints, none in any line plan 02-02 touched:

| Lint | Location |
|------|----------|
| `clippy::enum_variant_names` — "all variants have the same postfix: `Tool`" | `crates/talaria-mcp/src/tools.rs:97` |
| `clippy::redundant_closure` | `crates/talaria-mcp/src/tools.rs:132` |
| `clippy::unnecessary_cast` (`f32` -> `f32`) ×2 | `crates/talaria-shell/src/app.rs:708` |
| `clippy::unnecessary_cast` (`f32` -> `f32`) ×2 | `crates/talaria-shell/src/app.rs:733` |
| `clippy::too_many_arguments` (8/7) | `crates/talaria-shell/src/tabs.rs:93` |

`cargo build --release` and `cargo test` are both green; only the `-D warnings`
clippy gate is red.

**Why deferred:** these are pre-existing lints in files and lines outside plan
02-02's diff (`crates/talaria-shell/src/app.rs` lines 838-872, 955, 995 only).
Fixing them from a hardening plan would mix an unrelated lint sweep into a
security change and make the diff hard to review.

**Where it belongs:** plan 02-11 (CI, TEST-03, decision D-26) wires
`cargo clippy -- -D warnings` into GitHub Actions. That plan cannot go green
until these seven are resolved, so it should either fix them or narrow the gate.
Whoever executes 02-11 must decide; every plan in this phase that lists the
clippy gate as acceptance inherits the same red baseline.

## No wire-level "tab opened" event, so AGENT-04's `open` slice is not delivered

Found during plan 02-08 (the flagged assumption that plan carried, now resolved
as a decline rather than left implicit).

AGENT-04 reads "Tab **open**/close/crash events reach MCP clients as MCP
notifications". Plan 02-08 delivers close and crash. It does **not** deliver
open, because `talaria_protocol::Event` has exactly two variants and no producer
raises anything on tab creation.

For an agent's *own* `tabs_open`, this costs nothing — the reply to its own call
carries the tab. The real gap is the **popup adopted under an agent's tab**:
`Shared::adopt_popup` (`crates/talaria-shell/src/app.rs`) puts a
`window.open`/`target=_blank` tab into the agent's session, and the agent only
learns it exists by polling `tabs_list`.

**What closing it costs:** a third `Event` variant (`TabOpened { tab_id, opener_tab_id }`
or similar) in `crates/talaria-protocol/src/lib.rs`, a `queue_event` call at the
adoption site, and an assertion in both `tests/e2e/crash_event_test.py` and
`tests/e2e/mcp_client_test.py`. The proxy and notification plumbing plan 02-08
built need no change — a new variant flows through the reader's event arm and
the drain task as-is.

**Why deferred:** it is a wire-format change, and plan 02-08's own threat register
and its "the wire format between shell and proxy is unchanged" truth both forbid
one. Inventing a protocol variant inside the plan that was scoped to the proxy
half would have made the diff unreviewable against its own constraints.

**Consequence:** AGENT-04 stays **In Progress**, not Complete. See the plan
02-08 SUMMARY, "Requirements Status".

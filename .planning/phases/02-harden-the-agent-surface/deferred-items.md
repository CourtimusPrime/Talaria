# Deferred items — Phase 02

Out-of-scope discoveries logged during execution. Not fixed here.

## ~~`cargo clippy --all-targets -- -D warnings` is red at the phase baseline~~ — RESOLVED by plan 02-11

**Resolved 2026-08-16 in commit `b29b32e`.** Plan 02-11 took the "fix them"
branch, not the "narrow the gate" branch: the CI job runs the full
`cargo clippy --all-targets -- -D warnings` with no allow-list at the command
line, and the command exits 0 at the phase tip. Two lints were fixed in code
(the redundant closure became point-free; four `f32 -> f32` casts were dropped),
and two were accepted in place with a justifying comment — the `tool_box!`
variant postfix, whose variant names the macro generates rather than this crate
choosing them, and `TabManager::open`'s arity, which has one caller and whose
arguments are all engine handles. Behaviour was held constant: `cargo test` 9/9
and the full e2e suite 14/14 PASS, including `takeover_test` and
`keyboard_nav_test`, which drive the exact mouse paths the cast removal touched.

The original entry follows for the record.

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

## Autofill suggests, it does not populate the page's form fields

Found during plan 02-10 — the flagged assumption that plan carried, resolved as
the plan proposed rather than left implicit.

CRED-02 reads "stored credentials are **suggested for autofill** by domain match
in the shell UI". Plan 02-10 delivers that as a chrome-side offer: a toolbar
control naming the matching username, with copy controls for the username and
the password. It does **not** populate the page's form fields, per D-23.

**What closing it would cost:** form-field detection plus a page-scripting path
from the chrome. Both are new surface, and neither is the expensive part.

**Why deferred:** it needs its own decision, not just work. Injecting into the
page would (a) collide with an agent's own `evaluate` scripting the same form —
two writers, no arbitration — and (b) place the user's password inside a
document every script on that page can read, which is the disclosure the vault's
encryption exists to prevent. Plan 02-10's threat T-02-10-01 is written against
exactly this, and `crates/talaria-shell/src/gui.rs` carries the reasoning in a
comment at the control, because "just fill the form, it's friendlier" is the
change most likely to be made by someone who has not read this.

**Consequence:** none for CRED-02, which is marked complete on the "suggested"
reading. Recorded so a later reader knows the narrower reading was chosen
deliberately.

## A credential entry whose URL has no parseable host is unmatchable and undeletable

Found during plan 02-10, Task 1.

`Vault::matching` and `Vault::delete` both key on `entry_host`, which is `None`
when the entry's URL does not parse into a host. `Vault::upsert` deliberately
*appends* such an entry rather than discarding it — so it can be stored, but
never surfaced by a domain match and never removed.

Plan 02-10 closed the reachable half: `credential_url` in
`crates/talaria-shell/src/gui.rs` completes a typed bare hostname to `https://…`
before it becomes an entry, so nothing the panel creates can land in this state.
The remaining case is a hand-written `vault.json` carrying a bare host, imported
on first run.

**What closing it would cost:** a fallback in `entry_host` (or in `delete` and
`matching` separately) treating an unparseable URL as its own host, plus unit
tests for the new behaviour.

**Why deferred:** it changes the key semantics plan 02-09 defined, documented and
unit-tested three plans ago, from inside a UI plan. It belongs in a vault change,
not a chrome change.

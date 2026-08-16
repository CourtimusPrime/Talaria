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

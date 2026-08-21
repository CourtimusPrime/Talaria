---
phase: 04-authenticated-remote-transport-v2
plan: 01
subsystem: api
tags: [mcp, rust, rust-mcp-sdk, async-trait, cargo-lib-target, refactor, tool-surface]

# Dependency graph
requires:
  - phase: 02-agent-control
    provides: "the nine-tool MCP surface, `dispatch`, and `ShellConnection`'s narrow-retry control-socket wire (plan 02-05's double-execution fix is what the sink impl deliberately does not re-implement)"
  - phase: 03-browsing-essentials
    provides: "`ResultPayload::ChromeRects` — the control-socket-only payload the wildcard arm exists to keep off the tool surface"
provides:
  - "`talaria_mcp` library crate target — the MCP tool surface defined once, linkable by any transport"
  - "`talaria_mcp::CommandSink` — a one-method async trait abstracting how a `Command` reaches a running shell"
  - "`dispatch(sink: &dyn CommandSink, client: &str, tool: TalariaTools)` — no transport type in the signature"
  - "`impl CommandSink for ShellConnection` — the stdio sink, a one-line forward"
  - "`crates/talaria-mcp/src/tools.rs` `#[cfg(test)] mod tests` — the crate's first unit tests, pinning the nine-tool surface as data"
affects: [04-02, 04-03, 04-04, 04-05, 04-06, 04-07, 04-08, http-transport, oauth, consent-screen]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "One tool surface, two transports: the surface is a library item with no transport in its signature, so a second transport links it instead of redefining it"
    - "A sink is one hop — no retry, batching, or policy above the connection that already decides what is safe to re-send"
    - "Source-comment coupling from code to UI contract: the `tool_box!` invocation names the consent screen's grant list, and a unit test enforces the obligation"

key-files:
  created:
    - crates/talaria-mcp/src/lib.rs
  modified:
    - crates/talaria-mcp/Cargo.toml
    - crates/talaria-mcp/src/tools.rs
    - crates/talaria-mcp/src/socket.rs
    - crates/talaria-mcp/src/main.rs
    - CHANGELOG.md

key-decisions:
  - "`dispatch` takes `&dyn CommandSink` (a trait object) rather than a generic `S: CommandSink` — the plan's shape, and it keeps `dispatch` object-safe and monomorphisation-free for the two callers it will have"
  - "The `CommandSink` impl for `ShellConnection` forwards to the inherent `request` and adds nothing: retry, pipelining and reconnect stay the connection's, because a sink that retried on top of them would re-create the double-execution bug plan 02-05 fixed"
  - "Task 1's commit does not build the binary. The trait's only impl and the binary's rewiring are Task 2's by plan assignment, and the binary still compiled `tools.rs` as its own module at that point — Task 1 and Task 2 are jointly the smallest compiling unit"
  - "`every_tool_carries_a_description_and_an_input_schema` asserts `input_schema.type_() == \"object\"` rather than mere presence — `ToolInputSchema` is not an `Option`, so presence is a type-level tautology and would assert nothing"

patterns-established:
  - "Pattern 1 (04-RESEARCH.md): one tool surface, two transports — `CommandSink` is the only coupling point between the MCP tool surface and any way of reaching the shell"
  - "Pattern: a UI contract obligation recorded at the code site that triggers it, with a unit test as the tripwire (the `tool_box!` comment plus `the_surface_is_the_nine_tools_the_consent_screen_describes`)"

requirements-completed: [AUTH-03]

coverage:
  - id: D1
    description: "The nine-tool MCP surface, its schemas, `TalariaTools` and `dispatch` live in a `talaria_mcp` library crate target that both the stdio binary and a future HTTP transport can link"
    requirement: "AUTH-03"
    verification:
      - kind: unit
        ref: "crates/talaria-mcp/src/tools.rs#tools::tests::the_surface_is_the_nine_tools_the_consent_screen_describes"
        status: pass
      - kind: other
        ref: "cargo build --release (workspace); grep -c '^\\[lib\\]' crates/talaria-mcp/Cargo.toml == 1"
        status: pass
    human_judgment: false
  - id: D2
    description: "`dispatch` is generic over how a `Command` reaches the shell — it takes `&dyn CommandSink`, and names no concrete transport type"
    requirement: "AUTH-03"
    verification:
      - kind: other
        ref: "grep -c 'ShellConnection' crates/talaria-mcp/src/tools.rs == 0; grep -c 'dyn crate::CommandSink' crates/talaria-mcp/src/tools.rs == 1"
        status: pass
    human_judgment: false
  - id: D3
    description: "The stdio proxy's externally observable behaviour is unchanged: same nine tools with the same names, descriptions and schemas, same protocol revision, same instructions, same notification behaviour, same error text"
    requirement: "AUTH-03"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/mcp_client_test.py (unmodified) — exit 0, 'ALL MCP CLIENT CHECKS PASSED'"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/run_all.py — 19/19 PASS, 'failed: none'"
        status: pass
    human_judgment: false
  - id: D4
    description: "`ShellConnection` implements `CommandSink` by forwarding to its existing `request` — no retry, pipelining, or reconnect semantics re-implemented, moved, or altered"
    requirement: "AUTH-03"
    verification:
      - kind: other
        ref: "grep -c 'impl crate::CommandSink for ShellConnection' crates/talaria-mcp/src/socket.rs == 1; git diff HEAD~4 -- crates/talaria-mcp/src/socket.rs shows only the appended impl block"
        status: pass
      - kind: e2e
        ref: "tests/e2e/mcp_client_test.py concurrency check — tabs_list returned in 0.00s while a slow evaluate was outstanding"
        status: pass
    human_judgment: false
  - id: D5
    description: "The `ResultPayload` wildcard arm survives verbatim, so a control-socket-only payload (`ChromeRects`, and anything a later phase adds) still cannot leak onto the tool surface"
    requirement: "AUTH-03"
    verification:
      - kind: other
        ref: "grep -c 'the shell answered with a result this tool does not understand' crates/talaria-mcp/src/tools.rs == 1; git diff HEAD~4 -- crates/talaria-mcp/src/tools.rs shows no change inside either match block"
        status: pass
    human_judgment: false
  - id: D6
    description: "The consent screen's grant bullets are coupled to `tools.rs`: a comment on the `tool_box!` invocation records that adding, removing, or widening a tool obliges 04-UI-SPEC.md's grant list to change with it"
    requirement: "AUTH-03"
    verification:
      - kind: unit
        ref: "crates/talaria-mcp/src/tools.rs#tools::tests::the_surface_is_the_nine_tools_the_consent_screen_describes — its doc comment names 04-UI-SPEC.md and the test fails on any surface change"
        status: pass
    human_judgment: false

# Metrics
duration: 27min
completed: 2026-08-21
status: complete
---

# Phase 4 Plan 01: Tool-Surface Extraction Summary

**The nine-tool MCP surface now lives in a `talaria_mcp` library crate behind a one-method `CommandSink` trait, so `dispatch` names no transport — the stdio proxy is a consumer of its own library and behaves identically, proven by an unmodified e2e suite.**

## Performance

- **Duration:** 27 min
- **Started:** 2026-08-20T19:43:00Z
- **Completed:** 2026-08-20T20:10:18Z
- **Tasks:** 3
- **Files modified:** 6 (1 created, 5 modified)

## Accomplishments

- **Success Criterion 1 became a type-system property.** `dispatch` no longer mentions
  `ShellConnection`; it takes `&dyn CommandSink`. A second transport now serves the identical nine
  tools by linking the library and implementing one method, rather than by copying nine tool
  definitions that a reviewer would have to re-diff every phase.
- **The extraction is a move plus one parameter type, and nothing else.** The `Command` match, the
  `ResultPayload` match, its wildcard arm and comment, `text_result`, all nine `#[mcp_tool]`
  attributes, the struct definitions, the `tool_box!` invocation and the module-scoped
  `#![allow(clippy::enum_variant_names)]` are byte-identical. `git diff` on `tools.rs` touches only
  the import line, the signature, the `.request(...)` receiver, the new `tool_box!` comment and the
  appended test module.
- **The stdio path is proven unchanged end to end.** `tests/e2e/mcp_client_test.py` — a real
  `initialize`, `tools/list`, every tool called against a live shell, a pipelining check and the
  cross-session notification-addressing checks — passes with zero edits to that file. The full
  suite is 19/19, `failed: none`.
- **The nine-tool surface is now pinned as data.** Two unit tests, the crate's first, assert the
  exact sorted nine names and that every tool carries a non-empty description and an object input
  schema. `cargo test` went 117 → 119.
- **The consent screen's grant list is coupled to the code that obliges it.** A comment on the
  `tool_box!` invocation names 04-UI-SPEC.md's Consent panel and says a tool added, removed or
  widened here obliges those bullets to change; the nine-name test is the tripwire that makes the
  obligation impossible to skip silently.

## Task Commits

1. **Task 1: lib.rs, the CommandSink trait, and a transport-agnostic dispatch** — `7e99122` (refactor)
2. **Task 2: The stdio sink and a main.rs that consumes its own library** — `7fdf77e` (refactor)
3. **Task 3: Prove the stdio path did not move** — `2dba925` (test)

**CHANGELOG:** `95ccedf` (docs)

## Files Created/Modified

- `crates/talaria-mcp/src/lib.rs` — **created.** The library root: `//!` header stating why the
  surface is defined once, `pub mod socket; pub mod tools;`, the
  `dispatch`/`ShellConnection`/`TalariaTools` re-exports, and the `CommandSink` trait with a doc
  comment recording what a sink is not (no retry, batching, or policy), the two implementations that
  will exist, the "never connect to your own control socket" anti-pattern from 04-RESEARCH.md
  Pattern 1, and why `Send + Sync` is load-bearing.
- `crates/talaria-mcp/Cargo.toml` — a `[lib]` target `talaria_mcp` above the existing `[[bin]]`. No
  dependency line added, moved, or removed.
- `crates/talaria-mcp/src/tools.rs` — `dispatch` takes `&dyn crate::CommandSink`; the one
  `.request(...)` call goes through it; the `use crate::socket::ShellConnection;` import is gone; a
  comment above `tool_box!` records the consent-screen coupling; a `#[cfg(test)] mod tests` with two
  I/O-free tests is appended.
- `crates/talaria-mcp/src/socket.rs` — `impl crate::CommandSink for ShellConnection`, placed
  immediately after the inherent `impl` block, forwarding to `ShellConnection::request`. No existing
  code moved, reordered, or reformatted.
- `crates/talaria-mcp/src/main.rs` — `mod socket;`/`mod tools;` removed; `use talaria_mcp::{dispatch,
  ShellConnection, TalariaTools};`; the handler calls `dispatch(&self.connection, ...)`, coercing to
  `&dyn CommandSink`. The `//!` header now describes a thin stdio front end over the library and
  keeps the sentence about a locally-spawned stdio server not going through OAuth.
  `handle_list_tools_request`, the protocol version pin, `ServerCapabilities`, the `instructions`
  string, `notify_tab_events` and the event drain are untouched.
- `CHANGELOG.md` — a `### Changed` entry under Unreleased describing the extraction and stating that
  stdio behaviour is unchanged.

## Decisions Made

- **`&dyn CommandSink`, not `impl CommandSink` or a generic parameter.** The plan's shape. It keeps
  `dispatch` object-safe, avoids monomorphising the whole tool surface twice, and lets the future
  HTTP transport hold the sink behind an `Arc<dyn CommandSink>` without a second generic threading
  through every handler.
- **The sink impl adds nothing.** `ShellConnection::request` already decides what is safe to re-send
  — a command the wire accepted is never re-sent — and the impl's doc comment records that a sink
  retrying on top of that would re-create the double-execution bug plan 02-05 fixed.
- **`the_surface_is_the_nine_tools…` asserts sorted names, not a set.** A sorted `Vec<String>`
  comparison gives a readable diff naming exactly which tool appeared or vanished, which is what
  makes the consent-screen obligation actionable rather than merely tripped.
- **`input_schema.type_() == "object"` rather than an `is_some()` check.** `Tool::input_schema` is
  `ToolInputSchema`, not `Option<ToolInputSchema>`, so a presence assertion would be a type-level
  tautology asserting nothing. Checking the rendered `type` is the assertion with content.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Task 1's tree does not build the binary; the workspace gate was met at Task 2 instead**

- **Found during:** Task 1 (lib.rs, the CommandSink trait, and a transport-agnostic dispatch)
- **Issue:** Task 1's `<verify>` requires `cargo build --release` to exit 0, but Task 1's file set
  cannot produce a compiling workspace. Once `tools.rs` says `&dyn crate::CommandSink`, the *binary*
  crate — which at that point still declared `mod socket;` and `mod tools;` in `main.rs` — compiles
  its own copy of `tools.rs` against a crate root that has no `CommandSink`:
  `error[E0405]: cannot find trait CommandSink in the crate root`. The trait's only impl
  (`socket.rs`) and the binary's rewiring (`main.rs`) are Task 2's by explicit plan assignment, so
  Task 1 and Task 2 are jointly the smallest compiling unit.
- **Fix:** No code papering-over. Per the phase's own sequencing note ("Do NOT paper over it with a
  throwaway `#[allow(dead_code)]` — note it and confirm clippy is green at the plan tip"), Task 1
  was gated on `cargo build --release -p talaria-mcp --lib` and
  `cargo clippy --release -p talaria-mcp --lib -- -D warnings` (both clean), all seven of its source
  acceptance greps, and the empty manifest diff. Its commit message states plainly that the binary
  does not build at that SHA and why. The full workspace gate — build, `clippy --all-targets -D
  warnings`, `cargo test` — was met at Task 2's commit and at every commit after it.
- **Files modified:** none beyond the plan's own file sets.
- **Verification:** `7fdf77e` and every later commit pass `cargo build --release`,
  `cargo clippy --all-targets -- -D warnings` and `cargo test`.
- **Committed in:** `7e99122` (documented in the commit message body).

**2. [Rule 2 - Missing Critical] CHANGELOG entry for the extraction**

- **Found during:** Task 3 (post-verification)
- **Issue:** The project's standing instruction is that all changes are logged in `CHANGELOG.md`.
  The plan's `<output>` names only the SUMMARY.
- **Fix:** Added a `### Changed` bullet under Unreleased describing the library extraction and
  stating explicitly that stdio behaviour is unchanged, so a reader does not mistake a structural
  change for a behavioural one.
- **Files modified:** `CHANGELOG.md`
- **Verification:** Entry sits under the existing `### Changed` heading; no other section touched.
- **Committed in:** `95ccedf`

---

**Total deviations:** 2 (1 blocking-sequencing, disclosed rather than patched; 1 missing project convention)
**Impact on plan:** No scope creep. Neither deviation added, removed or altered a tool, a dependency,
or a line of runtime behaviour. Every prohibition in the plan's frontmatter holds — see below.

## Prohibitions Verified

All three of the plan's `flagged-unverified` prohibitions were checked and now hold:

| Prohibition | Check | Result |
|---|---|---|
| The tool surface no longer names a concrete transport type | `grep -c 'ShellConnection' crates/talaria-mcp/src/tools.rs` | `0` |
| No tool is added, removed, or renamed by this plan | the nine `tool_box!` names unchanged; `git diff` on the `#[mcp_tool]` attributes empty; tool-name grep count `27`, identical to the pre-task baseline | pass |
| No new dependency is added to any Cargo.toml | `git diff --stat Cargo.lock Cargo.toml crates/talaria-shell/Cargo.toml crates/talaria-protocol/Cargo.toml` | empty |

The `[lib]` block is the only edit to `crates/talaria-mcp/Cargo.toml`; `Cargo.lock` is byte-identical,
so 04-02 (same wave) owns every dependency change in this phase without a collision.

**Threat register:** all three mitigations landed. T-04-01-01 — the wildcard arm and its comment moved
verbatim, pinned by an exact-text source assertion. T-04-01-02 — `dispatch` and `TalariaTools::tools()`
are library items with no transport in their signatures, and the nine-name unit test fails against any
second surface that differs. T-04-01-03 — the sink impl is a one-line forward with the reasoning in its
doc comment. No new threat surface: this plan opens no port, adds no route, and touches no trust
boundary beyond the two it already documented.

## Issues Encountered

- **A 10-minute apparent hang running `mcp_client_test.py` standalone was my throwaway wrapper, not
  the suite.** `harness.stop` takes `(shell, xvfb)`; the scratch runner called it as
  `harness.stop(shell); harness.stop(xvfb)`, so teardown raised a `TypeError` *after* the suite had
  already printed `ALL MCP CLIENT CHECKS PASSED`, leaving Xvfb and the shell alive until the outer
  timeout fired. Fixed in the scratch script (kept in `/tmp`, never the repo), leftover processes
  reaped, and the suite re-run to a clean exit 0. Worth recording because the first symptom looked
  exactly like a regression in the thing this plan was supposed to leave untouched.
- **`cargo test`'s output gained a `9 ignored` doc-test line.** The `[lib]` target means the crate's
  doc comments are now collected, and `#[mcp_tool]` generates one non-Rust JSON block per tool. Nine
  ignored doc-tests, zero failures; the passing total went 117 → 119 from the two new unit tests. Not
  a regression, but a reader diffing test output between phases will see the extra line.

## User Setup Required

None — no external service configuration, no new env var, no new config key, no new file on disk at
runtime, no HTTP route.

## Next Phase Readiness

**Ready.** What the rest of the phase consumes from here:

- **04-02** (same wave) owns every dependency change. `Cargo.lock` and all three other manifests are
  untouched by this plan, so there is nothing to collide with.
- **04-03** implements the second `CommandSink` — an in-process sink in `talaria-shell` that sends an
  `AppEvent` down the `EventLoopProxy` and awaits the `oneshot`, per 04-RESEARCH.md Pattern 1. The
  anti-pattern it must avoid (the shell connecting to its own Unix control socket) is recorded in the
  trait's doc comment, at the place someone writing that impl will read it. 04-03 also lifts
  `ProtocolVersion::V2025_11_25` out of `main.rs` into a `lib.rs` constant both transports read — the
  literal is still in `main.rs` at this plan's tip.
- **The plan that builds the consent screen** should read the comment above the `tool_box!`
  invocation in `crates/talaria-mcp/src/tools.rs`: the seven grant bullets in 04-UI-SPEC.md's Consent
  panel are written against exactly the nine tools listed there, and
  `tools::tests::the_surface_is_the_nine_tools_the_consent_screen_describes` is the tripwire in the
  other direction. That coupling is the one thing in this plan that a future contributor can break
  without breaking a build, which is why it is both commented and tested.

**No blockers.** `cargo build --release`, `cargo clippy --all-targets -- -D warnings` and `cargo test`
(119) are clean at the plan tip; the e2e suite is 19/19 with `failed: none`.

---
*Phase: 04-authenticated-remote-transport-v2*
*Completed: 2026-08-21*

## Self-Check: PASSED

All five source artifacts, the SUMMARY and the CHANGELOG exist on disk; all four commit hashes
(`7e99122`, `7fdf77e`, `2dba925`, `95ccedf`) resolve in `git log`.

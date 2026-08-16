---
phase: 02-harden-the-agent-surface
plan: 11
subsystem: ci
tags: [github-actions, xvfb, clippy, lockfile-pin, unexecuted]
requires:
  - green-e2e-baseline-from-02-01
  - all-phase-02-suites-present-in-run_all
provides:
  - push-and-pr-pipeline-config
  - scheduled-lockfile-drift-audit-config
  - clippy-clean-tree-at-phase-tip
  - in-manifest-record-of-the-primeorder-pin
affects:
  - .github/workflows/ci.yml
  - .github/workflows/lockfile-audit.yml
  - Cargo.toml
  - README.md
  - crates/talaria-mcp/src/tools.rs
  - crates/talaria-shell/src/app.rs
  - crates/talaria-shell/src/tabs.rs
tech-stack:
  added:
    - "GitHub Actions (config only — never executed; this repository has no git remote)"
  patterns:
    - "first-party actions only, pinned to a major tag: checkout@v5, cache@v6, upload-artifact@v7"
    - "run_all.py's exit code used directly as the gate — unpiped, unwrapped, no || true"
    - "job-private XDG_RUNTIME_DIR at 0700 plus a pinned display, so concurrent runs cannot collide"
    - "expected-to-fail audit job kept off push, with a failure-gated diagnosis step"
key-files:
  created:
    - .github/workflows/ci.yml
    - .github/workflows/lockfile-audit.yml
  modified:
    - Cargo.toml
    - README.md
    - crates/talaria-mcp/src/tools.rs
    - crates/talaria-shell/src/app.rs
    - crates/talaria-shell/src/tabs.rs
    - .planning/phases/02-harden-the-agent-surface/deferred-items.md
decisions:
  - "The seven pre-existing clippy lints were fixed rather than the gate narrowed — CI runs the full --all-targets -D warnings with no command-line allow-list"
  - "Two of the seven are accepted in place with justifying comments (macro-generated variant names, an 8-argument constructor with one caller); the enum_variant_names allow had to be module-scoped because an attribute on a macro invocation is ignored"
  - "CI builds --locked, so an ordinary push never re-resolves; re-resolution is the scheduled audit job's exclusive business"
  - "TEST-03 stays In Progress — the config exists and is shape-verified, but the requirement's verb is 'runs' and nothing has run"
metrics:
  duration: ~45 min
  completed: 2026-08-16
status: complete
---

# Phase 2 Plan 11: Continuous Integration Summary

A push-and-pull-request pipeline that runs build, clippy, unit tests and the full Xvfb e2e suite
in that fixed order, a weekly from-scratch resolution audit that surfaces the `primeorder` pin
with its recovery command attached, and the clippy-clean tree the `-D warnings` gate needs —
written against a repository that has no remote, so the pipeline has never executed.

## The headline caveat, stated once and plainly

**Neither workflow has ever run.** `git remote -v` returns nothing; there are zero remotes
configured in this checkout, and `.github/` did not exist before this plan. Everything below that
describes CI behaviour describes *configured* behaviour, verified by parsing and by shape and
mutation checks, never by a job. Specifically **not** established:

- that the workflow is accepted by GitHub's own parser (only that it is valid YAML and that every
  key used is a documented Actions key)
- that the apt package list is sufficient for a Servo build on a hosted runner
- that Servo renders under a hosted runner's software GL well enough for the e2e suite to pass
- that the job fits in a hosted runner's disk or its 180-minute timeout
- any run identifier or job duration — the plan's Task 3 asks for both and neither exists

The plan's Task 3 `done` condition — "satisfied by an actual green run, not by a workflow file
that has never executed" — is therefore **not met**, and TEST-03 stays In Progress. What *is*
established is the local half: the four steps the workflow runs all exit 0 on this tree, in the
workflow's own order and environment.

## Task 0 (deviation) — The seven clippy lints

Every plan from 02-02 onward deferred these to 02-11; `deferred-items.md` recorded the choice as
"fix them or narrow the gate". **Fixed**, not narrowed: the CI job runs
`cargo clippy --all-targets --locked -- -D warnings` with no allow-list at the command line.

| Lint | Location | Resolution |
|------|----------|------------|
| `redundant_closure` | `talaria-mcp/src/tools.rs` | `.map_err(\|m\| CallToolError::from_message(m))` → point-free `.map_err(CallToolError::from_message)` |
| `unnecessary_cast` ×4 | `talaria-shell/src/app.rs:991,1016` | dropped four `f32 as f32` casts in the two viewport constructions |
| `enum_variant_names` | `talaria-mcp/src/tools.rs` | module-scoped `#![allow]` with rationale |
| `too_many_arguments` (8/7) | `talaria-shell/src/tabs.rs` | `#[allow]` on `TabManager::open` with rationale |

Two are accepted rather than restructured, and the reasons are recorded in the code, not only here:

- **`enum_variant_names`** — `tool_box!` expands to one enum variant per tool struct, so the
  variants inherit the crate's documented `<Command>Tool` struct naming. The macro generates the
  variant identifiers; this crate does not choose them, and renaming nine structs to satisfy a
  style lint would break the convention CLAUDE.md states. **A wrinkle worth recording:**
  `#[allow(...)]` placed on the `tool_box!` invocation is silently ignored, and rustc says so
  under `-D warnings` (`unused_attributes`: "the built-in attribute `allow` will be ignored, since
  it's applied to the macro invocation"). The first attempt at this fix therefore traded one lint
  for two. The allow is module-scoped (`#![allow]` at the top of `tools.rs`) as a result.
- **`too_many_arguments`** — `TabManager::open` has exactly one caller, `Shared::open_tab`
  (`app.rs:645`), and every argument past `size` is an engine handle owned by `Shared`. A parameter
  struct would move the same list one line up at that one call site.

**Behaviour was held constant, and the check is not incidental.** The only lints that changed
generated code are the four `f32 as f32` removals, which sit in `forward_mouse_move` and
`forward_mouse_button` — and `takeover_test` and `keyboard_nav_test` drive real `xdotool` clicks
and keystrokes through exactly those two functions. The safety net covers the changed lines rather
than merely running beside them. `cargo test` 9/9 and the e2e suite 14/14 PASS after the change.

The `deferred-items.md` entry is marked RESOLVED with the commit, and the original text kept below
it for the record.

## Task 1 — `.github/workflows/ci.yml` (D-26)

One job on `ubuntu-latest`, `permissions: contents: read`, `timeout-minutes: 180`, eleven steps:

| # | Step | Notes |
|---|------|-------|
| 1 | Free disk space | removes dotnet/android/ghc/boost/docker images |
| 2 | `actions/checkout@v5` | |
| 3 | Install system packages | Servo prerequisites + every binary the harness shells out to |
| 4 | Record toolchain versions | rustc, cargo, python3, Xvfb, xdotool |
| 5 | `actions/cache@v6` | registry + git db + `target/`, keyed on `hashFiles('**/Cargo.lock')` |
| 6 | `cargo build --release --locked` | |
| 7 | Assert both release binaries exist | `test -x` on `talaria` and `talaria-mcp` |
| 8 | `cargo clippy --all-targets --locked -- -D warnings` | |
| 9 | `cargo test --locked` | |
| 10 | End-to-end suite under Xvfb | `python3 tests/e2e/run_all.py` |
| 11 | Upload e2e output | `if: failure()` only |

Design points that carry a guarantee rather than a preference:

- **No path filter and no branch filter**, with the reason stated at the trigger block. A
  `paths:` filter is what lets a push produce zero jobs and still show a green tick — GitHub
  reports "no runs required" as success. This is probe row 18 (the empty-changeset input case): a
  push that changes no Rust source runs the whole pipeline.
- **Fixed step order** (probe row 19). Build precedes clippy precedes test precedes e2e, and no
  step carries `continue-on-error`, so a clippy failure stops the run rather than letting a broken
  build's e2e results confuse the diagnosis.
- **The orchestrator's exit code is the gate directly.** `python3 tests/e2e/run_all.py` is the last
  line of its step — not piped, not in a conditional, not followed by `|| true`. `run_all.py` exits
  with the count of failed suites, so exit 1 (one failed suite) fails the job.
- **Step 7 exists so step 10 cannot fail for a misleading reason.** `harness.py` launches
  `target/release/talaria` and `mcp_client_test` drives `target/release/talaria-mcp`; asserting both
  before the suite runs turns a future reorganisation into a clear failure instead of a puzzling one.
- **Concurrency isolation.** `TALARIA_E2E_DISPLAY: ":98"` pins the display rather than relying on
  the harness's `:99` default, and `XDG_RUNTIME_DIR` is a job-private directory `chmod 700`. Two
  hosted-runner jobs are separate machines, but a self-hosted runner executing two jobs at once
  would otherwise have them fight over the default display and the shared socket path.
- **`--locked` on all three cargo steps.** The committed lockfile is load-bearing; re-resolution is
  the audit job's exclusive business.

### The package list is grounded, not guessed

This machine is **Ubuntu 24.04.4 noble** — the same release `ubuntu-latest` resolves to — and it
builds this workspace successfully. The package names in the workflow are the names installed
here, which is the only real evidence available without a runner:

```
pkg-config cmake clang python3 libssl-dev libdbus-1-dev
libfontconfig-dev libfreetype-dev libgl1-mesa-dev libegl1-mesa-dev
libx11-dev libxcb1-dev libgl1-mesa-dri libxcursor1 libxi6 libxrandr2
libxkbcommon-x11-0 xvfb x11-utils xdotool x11-apps imagemagick procps
```

`apt-get install --simulate --no-install-recommends <the 22 names>` exits **0** with "0 newly
installed" — every name resolves on noble. The control: the same command with one bogus name
appended exits **100**, so the check discriminates rather than always passing.

Note the names are `libfontconfig-dev` and `libfreetype-dev`, not the `libfontconfig1-dev` /
`libfreetype6-dev` that the README and older Servo docs give — those were renamed. The transitional
names may still resolve on noble, but the current ones are what is verified here.

**Still unproven:** that this list is *sufficient* on a runner. A hosted image starts from a
different package set than this workstation, and Servo may need something present here for an
unrelated reason. A missing package is at least a loud failure — `apt-get install` exits non-zero
when a package cannot be located, and the step runs under `set -euo pipefail`.

### Artifact upload cannot publish the vault (T-02-11-01)

The upload path is `${{ runner.temp }}/talaria-e2e-out` and nothing else, `if: failure()`, with a
DO-NOT-WIDEN comment naming the reason: the home and config directories hold the credential vault,
its key file, and the engine profile. Verified by grep and by mutation (below).

## Task 2 — `.github/workflows/lockfile-audit.yml` and the manifest note (D-27)

Weekly (`cron: "17 4 * * 1"`, Mondays 04:17 UTC) and `workflow_dispatch`. **No push trigger** — the
job's failure is its expected outcome, and a job expected to fail must never sit in an ordinary
commit's path, or everyone learns to ignore a red tick.

The job frees disk, checks out, installs build prerequisites only, `rm -f Cargo.lock`,
`cargo generate-lockfile`, prints `cargo tree --invert --package primeorder`, then
`cargo build --release`. A final step gated on `failure() && steps.fresh_build.outcome == 'failure'`
prints the diagnosis: `primeorder` resolves to 0.14.0 final, which no longer satisfies the trait
bounds p256/p384/p521 0.14.0-rc.14 are written against; the working combination is held only by the
committed lockfile; and the recovery command
`cargo update -p primeorder --precise 0.14.0-rc.14`. The message also says explicitly what to do if
the build failed for a *different* reason — treat it as new information and read the log, rather
than reflexively applying the recovery command.

Two omissions are deliberate and commented so a later reader does not "fix" them:

- **No X packages.** A resolution failure surfaces at build time and this job never runs the e2e
  suite; adding them for symmetry would only make a job expected to fail slower to fail.
- **No cache.** A target directory built against the pinned resolution would mask the very failure
  the job exists to find.

The job never commits and never writes a lockfile back.

`Cargo.toml` gained a comment block above `[workspace.dependencies]` recording which crate is held
back, that no manifest field pins it, which crates break without it, the recovery command, and a
pointer to the audit workflow as the thing that watches it. Comment only:
`cargo metadata --format-version 1 --no-deps` exits 0, `git diff Cargo.lock` is **0 lines**, and
`cargo build --release` exits 0 (cargo fingerprints the parsed manifest, so a comment-only change
triggered no recompilation at all — it finished in 1.11s).

## Task 3 — Local rehearsal

Run at commit `cba03b8` in the workflow's own order and environment: `TALARIA_E2E_DISPLAY=:98`,
a job-private `XDG_RUNTIME_DIR=/tmp/talaria-ci-rt` created and `chmod 700` (verified `700` by
`stat`), `TALARIA_E2E_OUT=/tmp/talaria-ci-out`.

Toolchain: `rustc 1.97.1 (8bab26f4f 2026-07-14)`, `cargo 1.97.1 (c980f4866 2026-06-30)`,
`Python 3.14.6` — the same toolchain as plan 02-01's baseline.

| Step | Command | Exit |
|------|---------|------|
| 1 | `cargo build --release --locked` | 0 |
| 2 | `test -x target/release/talaria && test -x target/release/talaria-mcp` | 0 |
| 3 | `cargo clippy --all-targets --locked -- -D warnings` | **0** |
| 4 | `cargo test --locked` | 0 — 9 passed (2 protocol + 7 shell), 0 failed |
| 5 | `python3 tests/e2e/run_all.py` | **0** — 14/14 PASS, 244s |

### Per-suite verdict against the 02-01 baseline

| Suite | 02-01 baseline | Now | |
|-------|----------------|-----|---|
| control_socket_test | PASS 1s | PASS 1s | |
| scheme_refusal_test | — | PASS 0s | added this phase (02-02) |
| crash_recovery_test | PASS 11s | PASS 11s | |
| crash_event_test | PASS 4s | PASS 5s | |
| timeout_session_test | PASS 9s | PASS 9s | |
| wedge_fastfail_test | — | PASS 10s | added this phase (02-06) |
| mcp_client_test | PASS 7s | PASS 18s | |
| single_instance_test | PASS 0s | PASS 0s | |
| popup_test | PASS 1s | PASS 1s | |
| keyboard_nav_test | PASS 32s | PASS 32s | |
| takeover_test | PASS 27s | PASS 27s | |
| download_bounds_test | — | PASS 17s | added this phase (02-04) |
| vault_test | — | PASS 55s | added this phase (02-09) |
| vault_ui_test | — | PASS 42s | added this phase (02-10) |

All nine baseline suites still pass — **no regression**. All five suites this phase added are
present and passing. `mcp_client_test` went 7s → 18s across the phase, which is the pipelining and
notification work of 02-05/02-08 adding assertions, not a slowdown in a fixed test.

### Timeout headroom — a measured number, and what it is measured against

The plan asks for headroom measured rather than guessed. A hosted job duration does not exist, so
the next best measurement was taken directly: **a genuinely cold release build into an empty target
directory** (`CARGO_TARGET_DIR=/tmp/talaria-coldbuild`, leaving the repo's `target/` untouched).

| Quantity | Measured |
|----------|----------|
| Cold `cargo build --release` from an empty target dir | **7m02s** wall clock |
| Cores used | 20 |
| Peak RSS | 5.4 GB |
| Resulting target directory | **3.5 GB** |
| e2e suite | 244s |
| build + clippy + test + e2e, warm | ~250s |

Extrapolating honestly: a hosted `ubuntu-latest` runner has 4 vCPU, so the build alone is plausibly
5–7× the local figure (~35–50 min), plus a cold crates.io download the local run did not pay
(the registry was already populated), plus apt and disk cleanup (~5 min), plus an e2e suite that
will be slower under software GL. A cold job in the region of **50–75 minutes** is the expectation.
**180 minutes is roughly 2.5–3.5× that** — real headroom, from a measurement, with the
extrapolation shown so it can be corrected once a real run exists.

The disk risk (T-02-11-06's sibling) looks smaller than feared: a clean **release-only** target
directory is 3.5 GB, not the 42 GB this repo's `target/` occupies — that figure is debug plus
release plus incremental artifacts accumulated over the project. 3.5 GB fits a hosted runner
comfortably even before the free-disk step.

### Badge

`README.md` gained the badge above all other content, targeting
`https://github.com/court/talaria/actions/workflows/ci.yml` — derived from `Cargo.toml`'s
`repository` field, the only source of that path in the tree. An HTML comment beside it records
that no remote is configured, so it reads "no status" until the repository is pushed, and warns
that a badge pointing at the wrong owner/repo renders identically to one that has simply not run
yet. That is the single most likely thing to be silently wrong on first push.

## Verification performed

**Shape checks — all 20 acceptance criteria across Tasks 1 and 2 pass.** Counts exactly as the plan
specifies them: `run_all.py` ×1, `cargo build --release` ×1, `cargo clippy` ×1, `cargo test` ×1,
`XDG_RUNTIME_DIR` ≥1, `TALARIA_E2E_DISPLAY` ≥1, path filters 0, `continue-on-error` 0,
`workflow_dispatch` ×1, `schedule` ×1, audit `push:` 0, recovery command ×1, `primeorder` in
`Cargo.toml` on comment lines only (3 lines, 0 non-comment).

Two of those counts drove edits rather than being observed passively: a comment reading
"run_all.py exits with…" and a `cargo clippy --version` in the toolchain step each pushed their
count to 2, and both were reworded so the count means what it claims.

**YAML parse and schema walk.** Both files parse under PyYAML (installed into a scratchpad venv,
not the repo). Every step has either `uses:` or `run:`, and no step carries a key outside the
documented Actions step schema. Trigger maps, `permissions`, `runs-on`, `timeout-minutes` and step
order were dumped and read; ci.yml's steps are in the order build → assert binaries → clippy → test
→ e2e. The cron string was validated field by field: 5 fields, Mondays 04:17 UTC.

*(PyYAML parses a bare `on:` key as the boolean `True` under YAML 1.1; GitHub's parser treats it as
the string `on`. The walk handles both. This is a parser quirk, not a defect in the file.)*

**Mutation controls — every shape assertion was proven to discriminate.** Following the pattern
this phase adopted from 02-05 onward, each assertion was checked against a deliberately broken copy
in the scratchpad, confirming it fails for the reason claimed rather than merely failing:

| Mutant | Assertion | Mutant | Real file |
|--------|-----------|--------|-----------|
| `paths: ['crates/**']` added under `push:` | path-filter count | 1 (caught) | 0 |
| `continue-on-error: true` on the e2e step | continue-on-error count | 1 (caught) | 0 |
| `uses: dtolnay/rust-toolchain@stable` added | non-`actions/` uses count | 1 (caught) | 0 |
| `~/.config/talaria` added to the upload path | home/config path in upload block | 1 (caught) | 0 |
| `push:` added to the audit workflow | audit push-trigger count | 1 (caught) | 0 |

The fourth is the one that matters most: it proves the T-02-11-01 assertion would actually notice
someone widening the artifact upload to the directory holding the vault, rather than being a grep
that happens to return 0 for an unrelated reason.

**Package names** validated by `apt-get --simulate` with a bogus-name control (exit 0 vs exit 100).

## Deviations from Plan

### Auto-fixed issues

**1. [Rule 3 - Blocking] The seven pre-existing clippy lints**

- **Found during:** Task 0, before Task 1 — a `-D warnings` gate cannot land red.
- **Issue:** `cargo clippy --all-targets -- -D warnings` failed on unmodified code at the phase
  baseline `16411ee`. Every plan from 02-02 onward deferred them here.
- **Fix:** two fixed in code, two accepted in place with justifying comments. See Task 0 above.
- **Files modified:** `crates/talaria-mcp/src/tools.rs`, `crates/talaria-shell/src/app.rs`,
  `crates/talaria-shell/src/tabs.rs`
- **Commit:** `b29b32e`

This is inside the plan's letter — Task 3's `<verify>` runs the clippy gate — but it is recorded as
a deviation because the work was inherited from other plans rather than described by this one.

### Plan steps that could not be executed

**Task 3's push-and-confirm half.** The plan says "push the branch and confirm the workflow run
itself concludes successfully" and asks the SUMMARY to record the run identifier and duration.
There are zero git remotes. This was not skipped for convenience and there was no alternative: the
work is simply not performable from this checkout. It is recorded as an open item below rather than
papered over, and the plan's explicit prohibitions were honoured — no suite was disabled, no step
swallows a failure, and no path filter was added to route around anything.

### Judgement calls worth recording

- **The README's CI section was split across two commits.** Task 1 wrote it; the paragraph
  describing the audit workflow was held back to Task 2's commit so neither commit references a
  file that does not exist in it.
- **`--locked` was added to the three cargo steps**, which the plan does not name. It follows
  directly from the plan's own instruction that the audit job is where re-resolution happens: if the
  main pipeline could re-resolve, the audit job would be redundant and the pin unenforced in CI.

## Threat Flags

None. This plan adds no network endpoint, auth path, or schema at a trust boundary in the product.
The new surfaces are all CI-side and were enumerated in the plan's own threat model; dispositions
for the four `high` items are implemented and, unusually, each is backed by a mutation control
rather than only by inspection:

- **T-02-11-01** (vault in artifacts) — upload scoped to the e2e output dir; mutation-checked.
- **T-02-11-02** (third-party action) — only `actions/checkout@v5`, `actions/cache@v6`,
  `actions/upload-artifact@v7`; toolchain from the runner image; mutation-checked.
- **T-02-11-03** (green for a pipeline that skipped its verification) — no path filter, no
  `continue-on-error`, binary assertion before e2e, exit code used directly; first two
  mutation-checked.
- **T-02-11-04** (silent resolution drift) — main pipeline `--locked`; audit job resolves fresh,
  never writes back, never commits.
- **T-02-11-05** — `permissions: contents: read` declared explicitly on both workflows; cache key
  includes the lockfile hash.

## Known Stubs

None. Both workflow files are complete configurations, not scaffolds. What is missing is execution,
not content — and that is a property of the repository having no remote, not of these files.

## Requirements Progress

**TEST-03 — still In Progress. Deliberately not marked complete.**

The requirement reads "CI **runs** build, clippy, Rust unit tests, and the e2e suite on every push",
and the phase success criterion adds "and is green". The configuration for all four checks now
exists and is shape-verified, and all four are green locally at the phase tip. But nothing runs:
there is no remote, so there has been no job, no run identifier, and no green tick.

Plan 02-01 landed the branch-lock half (D-28/D-29) and correctly reverted its own premature
completion marking of TEST-03 for exactly this reason — because CI did not exist. Marking it
complete now, when CI exists but has never executed, would repeat that error one step further along.

**The single remaining step:** add a remote, push, and confirm the run concludes successfully.
Realistic risks on that first run, in descending order of likelihood — the apt list being
insufficient on a hosted image; Servo failing to render under the runner's software GL, which would
fail the e2e step for an environmental reason; and the timing-sensitive suites (`vault_test` at 55s,
`vault_ui_test` at 42s, both driving `xdotool` against a real window) being flaky on a slower,
noisier machine. Per the plan's prohibition, the response to any of these is to fix the workflow and
name the failure explicitly — never to disable a suite, swallow a failure, or add a path filter.

MCP-09, MCP-10 and AGENT-04 remain In Progress, untouched by this plan.

## Commits

| Task | Commit | Description |
|------|--------|-------------|
| 0 (Rule 3) | `b29b32e` | fix(02-11): clear the seven clippy lints blocking a -D warnings gate |
| 1 | `fb62f10` | ci(02-11): run build, clippy, tests and the e2e suite on every push (D-26) |
| 2 | `cba03b8` | ci(02-11): audit fresh dependency resolution on a schedule (D-27) |
| 3 | `7508d93` | ci(02-11): add the CI badge and close the deferred clippy item |

## Self-Check: PASSED

All four commits exist in `git log`. All eight touched files exist on disk. No tracked file was
deleted by any commit in this plan (`git diff --diff-filter=D fbafb1e..HEAD` is empty). The two
untracked files in the working tree (`.claude/HANDOFF-xkihS.md`, `.claude/settings.json`) predate
this plan and were left alone. Throwaway probe scripts, the PyYAML venv, the workflow mutants and
the cold-build target directory were all kept in the scratchpad and removed; none entered the repo.

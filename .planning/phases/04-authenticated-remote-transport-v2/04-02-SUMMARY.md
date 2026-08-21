---
phase: 04-authenticated-remote-transport-v2
plan: 02
subsystem: infra
tags: [cargo, lockfile, rust-mcp-sdk, rust-mcp-axum, axum, jsonwebtoken, reqwest, oauth, spike, dependencies]

# Dependency graph
requires:
  - phase: 02-harden-the-agent-surface
    provides: "the `--locked` CI contract (decision 02-11) and `.github/workflows/lockfile-audit.yml`, the scheduled job that resolves from scratch and prints the primeorder diagnosis"
  - phase: 03-table-stakes-browsing
    provides: "the deferred-items.md house format this phase's register follows"
provides:
  - "`rust-mcp-sdk` with `streamable-http` and `auth` enabled, and `rust-mcp-axum 1.0.1`, `sha2 0.11`, `async-trait` and tokio's `rt-multi-thread` available to `talaria-shell`"
  - "A `Cargo.lock` that builds `--locked` with the `primeorder 0.14.0-rc.14` pin intact, asserted rather than assumed"
  - "`04-02-SPIKE.md` — assumption A1 settled CONFIRMED with observed request/response evidence, plus A2 and A8 settled and A6 narrowed"
  - "The published-1.0.1 `AxumServerOptions` field table and `AuthenticationError` status mapping, so 04-03/04-05 write against observed reality"
  - "`deferred-items.md` — the phase's register, opened with D-04-01's protocol migration, D-04-02's CIMD refusal and D-04-04's Phase 5 TLS obligation"
affects: [04-03, 04-04, 04-05, 04-06, 04-07, 04-08, phase-05]

# Tech tracking
tech-stack:
  added:
    - "rust-mcp-axum 1.0.1 (direct)"
    - "sha2 0.11 (direct; already resolved via the servo tree)"
    - "rust-mcp-sdk features streamable-http + auth (widened, not added)"
    - "axum 0.8.9, axum-core 0.5.6, axum-server 0.8.0, jsonwebtoken 10.4.0, reqwest 0.12.28 (transitive, newly resolved)"
    - "tower-http 0.6.11, quinn 0.11.11 (+ quinn-proto, quinn-udp), cookie_store 0.22.1, publicsuffix 2.3.0, psl-types 2.0.11, pem 3.0.6, simple_asn1 0.6.4, serde_urlencoded 0.7.1, serde_path_to_error 0.1.20, matchit 0.8.4, ryu 1.0.23, fs-err 3.3.1, lru-slab 0.1.2, document-features 0.2.12, litrs 1.0.0, rand_pcg 0.10.2, wasm-streams 0.4.2 (transitive)"
  patterns:
    - "A dependency's supply-chain clearance is recorded as a comment at its manifest line, so the audit travels with the crate instead of living only in a planning document"
    - "A feature's deliberate absence is documented at the line that would otherwise gain it, so 'just add the missing feature' has to delete a stated rationale"
    - "A spike is a cargo example inside the crate that will consume the dependency, so it links the real resolved graph; it is deleted in the task that ran it and only its written findings are committed"

key-files:
  created:
    - .planning/phases/04-authenticated-remote-transport-v2/04-02-SPIKE.md
    - .planning/phases/04-authenticated-remote-transport-v2/deferred-items.md
  modified:
    - Cargo.toml
    - Cargo.lock
    - crates/talaria-shell/Cargo.toml
    - CHANGELOG.md

key-decisions:
  - "Resolved with `cargo metadata` against edited manifests rather than `cargo add` or `cargo update` — the minimum action that updates the lock, with no crate named on the command line that could pull a newer line"
  - "The `talaria-mcp` library edge stays out of both manifests; it belongs to 04-03, because 04-01 ran in this same wave and a path dependency on a lib target it had not finished creating would have raced this plan's own `--locked` build"
  - "`rust-mcp-sdk` and `rust-mcp-axum` were re-verified as still-newest at 1.0.1 and pinned there regardless — a version change mid-phase would re-open D-04-01, which is locked"
  - "The spike ran as three separate process invocations rather than one, after three in-process tokio runtimes made server startup nondeterministic; the cause is recorded because a later multi-server test will hit it"
  - "Probe C was added beyond the plan's letter to settle A6 rather than leave it open, and its residual (an already-open stream) is stated as a residual rather than rounded to answered"

patterns-established:
  - "Pattern: assert the lockfile pin by direct grep plus a `--locked` build as acceptance criteria, and review the diff's *shape* (new names, re-versioned names) against a prediction before committing"
  - "Pattern: a spike's verdict line is a machine-checkable literal (`A1: CONFIRMED`) so a plan can gate on it, with the reasoning above it in prose"

requirements-completed: [AUTH-03]

coverage:
  - id: D1
    description: "The workspace builds `--locked` with `streamable-http` and `auth` enabled, `rust-mcp-axum` present, and `sha2` and the SDK available to `talaria-shell`"
    requirement: "AUTH-03"
    verification:
      - kind: other
        ref: "cargo build --release --locked (exit 0); cargo clippy --all-targets --locked -- -D warnings (exit 0)"
        status: pass
      - kind: other
        ref: "grep -c 'rust-mcp-sdk' crates/talaria-shell/Cargo.toml == 1; grep -c 'rust-mcp-axum' crates/talaria-shell/Cargo.toml == 1; grep -c 'rt-multi-thread' crates/talaria-shell/Cargo.toml == 1"
        status: pass
    human_judgment: false
  - id: D2
    description: "`Cargo.lock` still pins `primeorder 0.14.0-rc.14` after the additions — asserted, not assumed"
    requirement: "AUTH-03"
    verification:
      - kind: other
        ref: "grep -A1 'name = \"primeorder\"' Cargo.lock | grep -c '0.14.0-rc.14' == 1; p256/p384/p521 all still 0.14.0-rc.14; cargo build --release --locked exit 0"
        status: pass
    human_judgment: false
  - id: D3
    description: "The legacy Server-Sent-Events transport feature is not enabled: the `rust-mcp-sdk` feature list is exactly server, macros, stdio, streamable-http, auth"
    requirement: "AUTH-03"
    verification:
      - kind: other
        ref: "grep -c 'features = [\"server\", \"macros\", \"stdio\", \"streamable-http\", \"auth\"]' Cargo.toml == 1; grep -c '\"sse\"' Cargo.toml == 0"
        status: pass
    human_judgment: false
  - id: D4
    description: "The lockfile diff is proportionate — the newly-resolved crates are the ones 04-RESEARCH.md predicted, not a whole-tree re-resolution"
    requirement: "AUTH-03"
    verification:
      - kind: other
        ref: "package-set diff of Cargo.lock at HEAD~ vs HEAD: 25 new names, 0 removed, 0 re-versioned (2 gained an additional major line); axum/axum-server/jsonwebtoken/reqwest/rust-mcp-axum all present"
        status: pass
    human_judgment: false
  - id: D5
    description: "Assumption A1 is settled with a compiled, executed answer before 04-05 and 04-06 are executed"
    requirement: "AUTH-03"
    verification:
      - kind: other
        ref: "04-02-SPIKE.md carries `A1: CONFIRMED` with a per-endpoint status table; grep -cE '^A1: (CONFIRMED|REFUTED)' == 1"
        status: pass
      - kind: integration
        ref: "cargo run --example auth_routing_spike -- a|b|c — all six declared endpoints reached handle_request with the right OauthEndpoint; undeclared path 404; /mcp 401 with WWW-Authenticate"
        status: pass
    human_judgment: false
  - id: D6
    description: "The spike leaves no code behind: its throwaway target is deleted in the same task that ran it"
    requirement: "AUTH-03"
    verification:
      - kind: other
        ref: "test ! -f crates/talaria-shell/examples/auth_routing_spike.rs (exit 0); the file appears in no commit — git log --diff-filter=A on that path is empty"
        status: pass
    human_judgment: false
  - id: D7
    description: "D-04-01's protocol-revision migration and D-04-02's CIMD refusal are recorded as tracked deferred work with their reasoning"
    requirement: "AUTH-03"
    verification:
      - kind: other
        ref: "deferred-items.md: 3 `## ` entries; grep '2026-07-28' == 5; grep -i 'SSRF|request forgery' == 2; grep -i 'loopback exception' == 1"
        status: pass
    human_judgment: false
  - id: D8
    description: "`talaria-shell` gaining `rust-mcp-sdk` and `rust-mcp-axum` produces no version conflict with Servo's tree (A8), demonstrated rather than argued"
    requirement: "AUTH-03"
    verification:
      - kind: other
        ref: "the spike was a talaria-shell cargo example, so Servo, egui, winit, rustls and the new HTTP stack linked into one binary; cargo build --release --locked exit 0"
        status: pass
    human_judgment: false

# Metrics
duration: 31min
completed: 2026-08-20
status: complete
---

# Phase 4 Plan 02: Dependencies and the A1 Spike Summary

**The full HTTP and OAuth stack is available to `talaria-shell` with the `primeorder 0.14.0-rc.14`
pin measurably intact, and the phase's largest assumption is settled by a compiled experiment:
`rust-mcp-axum` routes a self-hosted authorization server's declared endpoints to the provider's own
`handle_request`, so 04-05 and 04-06 can be executed as planned.**

## Performance

- **Duration:** 31 min
- **Started:** 2026-08-20T20:14:46Z
- **Completed:** 2026-08-20T20:45:51Z
- **Tasks:** 3
- **Files:** 6 (2 created, 4 modified)

## Accomplishments

- **The lockfile hazard this plan exists for did not fire, and that is now a measurement rather
  than a hope.** `primeorder` is still `0.14.0-rc.14`, and so are `p256`, `p384` and `p521`. The
  documented `cargo update -p primeorder --precise` recovery was never needed. Both facts are wired
  as acceptance criteria — a direct lockfile grep and a `--locked` release build — so the next plan
  to touch a manifest inherits an assertion, not an anecdote.
- **The lockfile diff is proportionate and was reviewed before it was committed.** 25 new package
  names, 0 removed, 0 re-versioned. `signature` and `untrusted` each gained an *additional* major
  line rather than moving. Pitfall 1's warning sign — hundreds of Servo packages re-versioned — did
  not appear.
- **Assumption A1 is CONFIRMED with observed evidence, not a source reading.** All six endpoints a
  co-hosted authorization server declares reached the provider's `handle_request` with the correct
  `OauthEndpoint` discriminant, on a server built by `create_axum_server` with nothing but
  `AxumServerOptions { auth: Some(..), sse_support: false, .. }`. An undeclared path `404`s. Nothing
  upstream assumes a remote issuer, and the specific line that settles it is recorded.
- **A2 is answered, and answered better than expected.** The published `1.0.1` tarballs are
  *byte-identical* to GitHub `main` for `auth_provider.rs`, `rust-mcp-axum`'s `server.rs` and
  `auth_routes.rs`. The research's quotations were accurate. Two deltas still bit the spike, both in
  the research's illustrative examples rather than its quotes, and both are recorded with the fix.
- **A6 moved from "not verified" to "answered for the request path, narrowed to one residual."** A
  live session is addressable by client identity, and dropping it makes the next request on that
  session id `404` immediately. What is *not* settled — whether an already-open stream dies with it
  — is stated as a residual and handed to 04-07, rather than rounded up.
- **A8 is confirmed by construction.** The spike was a `talaria-shell` cargo example, so Servo,
  egui, winit, rustls and the new HTTP stack linked into one binary. The two-process fallback is not
  needed.
- **A finding that changes 04-06's work surfaced early, which is the entire point of a spike.** The
  SDK never calls `AuthProvider::validate_allowed_methods`: `GET /token` returned `200` and reached
  `handle_request`. The trait provides the verb table and builds the `405` for you, but the provider
  must invoke it. Found in 30 minutes here instead of in a security review of 04-06.
- **No regression anywhere.** `cargo test` 119 passing (2 mcp-lib + 3 protocol + 114 shell) with the
  9 pre-existing ignored doc-tests, `cargo clippy --all-targets -- -D warnings` clean, and the full
  e2e suite 19/19 with `failed: none` under the CI-style environment.

## Task Commits

1. **Task 1: Add the dependencies and prove the lockfile pin survived** — `ff89c93` (chore)
2. **Task 2: Settle assumption A1** — `4f8cb0b` (docs)
3. **Task 3: Open the phase's deferred-items register** — `bebfc5d` (docs)

**CHANGELOG:** `4cf6a23` (docs)

## What Actually Resolved

The research predicted `axum`, `axum-server`, `jsonwebtoken` and `reqwest` as the only genuinely new
crates. That was right about the *headline* set and understated the transitive tail. The complete
newly-resolved set, from a package-name diff of `Cargo.lock` at `HEAD~` vs `HEAD`:

| Crate | Version | Why |
|-------|---------|-----|
| `rust-mcp-axum` | 1.0.1 | direct — the HTTP server |
| `axum` / `axum-core` | 0.8.9 / 0.5.6 | the router |
| `axum-server` | 0.8.0 | listener + graceful shutdown |
| `matchit` | 0.8.4 | axum's route matcher |
| `tower-http` | 0.6.11 | axum middleware |
| `serde_path_to_error` | 0.1.20 | axum extractor errors |
| `jsonwebtoken` | 10.4.0 | via the SDK's `auth` feature |
| `simple_asn1` / `pem` | 0.6.4 / 3.0.6 | jsonwebtoken's key parsing |
| `reqwest` | 0.12.28 | via the SDK's `auth` feature |
| `cookie_store` / `publicsuffix` / `psl-types` | 0.22.1 / 2.3.0 / 2.0.11 | reqwest's `cookies` feature |
| `quinn` / `quinn-proto` / `quinn-udp` | 0.11.11 / 0.11.17 / 0.5.15 | reqwest's HTTP/3 path |
| `lru-slab` / `rand_pcg` | 0.1.2 / 0.10.2 | quinn's own |
| `wasm-streams` | 0.4.2 | reqwest's `stream` feature (wasm target) |
| `serde_urlencoded` | 0.7.1 | reqwest form encoding |
| `fs-err` / `document-features` / `litrs` / `ryu` | 3.3.1 / 0.2.12 / 1.0.0 / 1.0.23 | build/util transitives |

**Already resolved and confirmed so, exactly as the research said:** `hyper 1.11.0`, `http 1.5.0`,
`http-body`/`http-body-util 1.1.0`, `tower 0.5.3`, `tokio-stream 0.1.19`, `rustls 0.23.43`,
`aws-lc-rs 1.18.0`, and `sha2` at both `0.10.9` and `0.11.0`. `rust-mcp-sdk` itself was already in
the lock (via `talaria-mcp`); only its features widened. Two packages gained an additional major
line without moving: `signature` (now `2.2.0` and `3.0.0`) and `untrusted` (now `0.7.1` and
`0.9.0`).

**The SDK's version constraints, which are the SDK's and not drift.** `jsonwebtoken` is capped at
`^10.1` while `11.0.0` is published, and `reqwest` at `^0.12` while `0.13.x` is published. Resolution
therefore takes the older major line of each. This is worth knowing before someone reads a
dependency dashboard and "fixes" it: bumping either requires the SDK to widen first.

**Two HTTP clients, accepted.** The workspace now carries `ureq 2` (used by the `download` tool) and
`reqwest 0.12.28` (arrives with the SDK's `auth` feature, not separately selectable). The
alternative is forgoing `auth` and hand-writing the 401 middleware — trading a well-reviewed
dependency for bespoke security code. Talaria never calls `reqwest` itself, and D-04-02's refusal to
implement CIMD means the authorization server has no code path that fetches an attacker-supplied
URL. Recorded rather than mitigated, per the plan's threat register (T-04-02-04, `accept`).

**Feature unification, stated so it is not a surprise later.** Once 04-03 adds the `talaria-mcp`
edge, `talaria-mcp` will ask for `stdio` while `talaria-shell` asks for `streamable-http` and
`auth`; Cargo unifies features across the graph, so the **stdio binary will be built against an SDK
with all five features enabled**. Its behaviour does not change — it never constructs an HTTP
transport — but the binary grows, and a reader diffing binary sizes across phases should know why.

**Freshness re-check.** `04-RESEARCH.md` put a 14-day validity on its SDK facts. Re-queried at
execution: `rust-mcp-sdk` and `rust-mcp-axum` are both still newest at `1.0.1`, published
2026-07-26. No newer version exists, so D-04-01 was never at risk of being re-opened. The Package
Legitimacy Audit's numbers also still hold — `rust-mcp-axum` now shows 607 total downloads against
the audit's ~600, created 2026-06-24, same repository and owner as the SDK. Nothing about the audit
changed; its clearance is now recorded as a comment at the dependency line so it travels with the
crate.

## The Spike, in Short

Full evidence is in
[`04-02-SPIKE.md`](./04-02-SPIKE.md). The verdict line is `A1: CONFIRMED`.

**04-05 and 04-06 do NOT need re-planning.** The routing works as the research's source reading
suggested, so their approach stands.

Three things they will each need, which the spike found and the SPIKE document records in full:

- **04-06 must call `validate_allowed_methods` itself.** Nothing else will. `GET /token` reached
  `handle_request` as `TokenEndpoint` with a `200`. The trait ships the per-endpoint verb table and
  builds the `405`; `handle_auth_requests` goes straight past it to `handle_request`.
- **04-04's token store must never mint an `AuthInfo` with `expires_at: None`.** The middleware
  rejects that outright with `"Token has no expiration time"`. "No expiry" is not "never expires",
  it is invalid.
- **04-03 has a small decision to make.** With `auth_endpoints() -> None` the server starts fine and
  still refuses `/mcp` — but the `401` it emits still advertises
  `resource_metadata="…/.well-known/oauth-protected-resource"`, a path that `404`s until 04-06
  declares it. Either return `None` from `protected_resource_metadata_url()` in the interim or
  declare the metadata endpoint early; advertising a document that does not exist is worse than
  advertising nothing.

Auth endpoints are composed with an **empty** middleware slice
(`compose(&[], final_handler)`), which is why `/token` is reachable without a bearer token while
`/mcp` in the same process returns `401` — the one design detail that makes a co-hosted AS possible
at all, and it is deliberate upstream, not an accident.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] The spike could not use `http` or `{endpoint:?}` as the research's examples wrote them**

- **Found during:** Task 2, first compile of the throwaway.
- **Issue:** Two compile errors, both traceable to `04-RESEARCH.md`'s illustrative code rather than
  to the source it quoted. `http` is not a direct dependency of `talaria-shell`, so
  `http::Request` did not resolve; and `OauthEndpoint` derives `Hash, Eq, PartialEq, Clone` but
  **not** `Debug`, so `format!("{endpoint:?}")` did not compile.
- **Fix:** Used the SDK's own re-export, `rust_mcp_sdk::mcp_http::http` — which is also the only way
  to be certain the `http` version matches the one the trait signatures are written against — and
  replaced the debug format with a hand-written match.
- **Files modified:** the throwaway only; no repository file changed.
- **Verification:** compiled and ran; both facts recorded in `04-02-SPIKE.md` under A2 so 04-03 and
  04-06 do not rediscover them.
- **Committed in:** `4f8cb0b` (as findings; the code was deleted).

**2. [Rule 1 - Bug] Three tokio runtimes in one process made the spike's own results nondeterministic**

- **Found during:** Task 2, second run.
- **Issue:** Running all three probes as three servers inside one process produced a passing run and
  then a run where probe C produced no output at all. The SDK spawns a `shutdown_signal` task per
  server that installs its own ctrl-c and `SIGTERM` handlers (`server.rs:579-607`); three of those
  across three `new_multi_thread` runtimes in one process is the difference.
- **Fix:** The throwaway takes a probe selector argument and was invoked three times, one server per
  process. All three probes are deterministic after that. The cause is recorded in the SPIKE rather
  than left as a curiosity, because a later in-process multi-server test will hit it. Talaria itself
  runs exactly one listener, so 04-03 is unaffected.
- **Files modified:** the throwaway only.
- **Verification:** three consecutive clean runs.
- **Committed in:** `4f8cb0b` (as findings).

**3. [Rule 1 - Bug] A misleading readiness check in the spike's own instrumentation**

- **Found during:** Task 2.
- **Issue:** Probe C's pre-flight `wait_until_listening` reported `false` while the very next lines
  showed a `200` from `initialize` against that same port. Cause: probe C's server lives only as
  long as its own work — a few hundred milliseconds — so the 50 ms-interval poll straddled the
  window and its last recorded error was the post-teardown `ECONNREFUSED`. Left in, it would have
  put a self-contradicting line into the phase's evidence.
- **Fix:** Removed the poll from probe C and stated in the SPIKE that reachability there is proven
  by the `200` on `initialize`, not by a readiness check that races teardown. Probes A and B keep
  their poll, where the server outlives it.
- **Files modified:** the throwaway only.
- **Verification:** final three-probe run is internally consistent.
- **Committed in:** `4f8cb0b` (as findings).

**4. [Rule 2 - Missing Critical] Probe C, beyond the plan's letter, to settle A6**

- **Found during:** Task 2.
- **Issue:** The plan permits marking A6 "still open" if it cannot be answered cheaply. It could be
  answered cheaply — the `SessionStore`/`AxumRuntime` surface was already open in front of me — and
  A6 is what 04-07's whole revocation story rests on.
- **Fix:** Added a third probe opening a genuinely authenticated session, then asking the server's
  own runtime whether that session is findable by client identity and whether deleting it takes
  effect. Both yes. The one part that was *not* observed — an already-open stream — is written down
  as a residual for 04-07 rather than rounded up to answered.
- **Files modified:** the throwaway only.
- **Verification:** probe C output quoted verbatim in the SPIKE.
- **Committed in:** `4f8cb0b`.

**5. [Rule 2 - Missing Critical] CHANGELOG entry**

- **Found during:** post-verification.
- **Issue:** The project's standing instruction is that all changes are logged in `CHANGELOG.md`.
  The plan's `<output>` names only the SUMMARY.
- **Fix:** A `### Changed` bullet under Unreleased describing the widened features, the new
  dependencies, the deliberately-absent legacy transport feature, and the surviving pin — stating
  explicitly that no runtime behaviour changes yet, so a reader does not mistake a dependency
  addition for a feature.
- **Files modified:** `CHANGELOG.md`
- **Committed in:** `4cf6a23`

---

**Total deviations:** 5 — three inside the throwaway (which no longer exists), one deliberate
scope *addition* that closed an open assumption, one project convention.
**Impact on plan:** none negative. No dependency was added, removed, or versioned differently from
what the plan specified. The `talaria-mcp` edge stayed out, as instructed.

## Prohibitions Verified

All four of the plan's `flagged-unverified` prohibitions were checked and hold:

| Prohibition | Check | Result |
|---|---|---|
| `Cargo.lock` still pins `primeorder 0.14.0-rc.14` | `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'`; `cargo build --release --locked` | `1`; exit 0 |
| The legacy Server-Sent-Events transport feature is never enabled | exact-match grep on the five-element feature array; `grep -c '"sse"' Cargo.toml` | `1`; `0` |
| No `cargo update` against the whole tree; only targeted, minimal resolution | lock resolved by `cargo metadata` from edited manifests; package-set diff | 25 added, 0 removed, 0 re-versioned |
| The spike's throwaway target does not survive into the committed tree | `test ! -f crates/talaria-shell/examples/auth_routing_spike.rs`; the path appears in no commit | exit 0; absent from history |

**Threat register:** all three `high` items mitigated as written. **T-04-02-SC** — the Package
Legitimacy Audit's clearance for `rust-mcp-axum` now lives as a comment at its manifest line, with
the publish date, download count, `SUS` flag and the provenance that clears it; download count
re-checked at execution (607, consistent with the audit's ~600). **T-04-02-01** — the pin is
asserted by direct grep and a `--locked` build, and the diff's shape was reviewed against the
prediction before commit. **T-04-02-02** — the feature array is exactly five entries, asserted by
exact-match grep, with the omission's reason at the line. **T-04-02-03** — the throwaway is gone
and was never committed; build and clippy re-run green after the deletion. **T-04-02-04** (`accept`)
— the two-HTTP-client cost is recorded above.

**New threat surface:** none introduced by this plan. No port is opened, no route is served, no
trust boundary moves. The dependencies are present but unreferenced by any Talaria code path; 04-03
is the plan that first opens a listener.

## Verification

| Check | Result |
|---|---|
| `cargo build --release --locked` | exit 0 |
| `cargo clippy --all-targets --locked -- -D warnings` | exit 0, no warnings |
| `cargo test --locked` | exit 0 — **119 passing** (2 mcp-lib + 3 protocol + 114 shell), 9 ignored doc-tests |
| `python3 tests/e2e/run_all.py` (CI-style env) | **19/19 PASS**, `failed: none` |
| `grep -A1 'name = "primeorder"' Cargo.lock` | `0.14.0-rc.14` |
| `04-02-SPIKE.md` verdict line | `A1: CONFIRMED` (grep count 1); A2 and A6 both addressed |
| `crates/talaria-shell/examples/auth_routing_spike.rs` | absent |
| `deferred-items.md` | 3 entries, all required reasoning present |

e2e ran with `XDG_RUNTIME_DIR=/tmp/tal-e2e-rt`, `TALARIA_E2E_DISPLAY=:98`,
`TALARIA_E2E_OUT=/tmp/tal-e2e-out`, both directories `chmod 700`. Baselines held exactly: 119 and
19/19, unchanged from 04-01's tip. Adding dependencies changed neither number, which is what it
should have done.

## Issues Encountered

- **The `wait_until_listening` contradiction cost more time than it should have.** A readiness poll
  reporting `false` next to a successful request against the same port reads like a real anomaly,
  and it was chased through several hypotheses (port mismatch, DNS-rebinding protection,
  loopback weirdness) before the mundane answer — the server's whole lifetime fitted between two
  polls — surfaced. Recorded because the shape of the mistake generalises: a readiness check against
  a server that shuts down when its work is done will lie in whichever direction is least useful.
- **`rust-mcp-axum`'s `create_axum_server` is not in `lib.rs`**, it is in `factory.rs` and re-exported.
  Worth a grep rather than a guess when 04-03 goes looking for it.

## User Setup Required

None. No new environment variable, config key, runtime file, port or HTTP route. `cargo build` will
download the new crates on a fresh checkout; `--locked` builds are unaffected because the lockfile
is committed and self-consistent.

## Next Phase Readiness

**Ready. No blockers.**

- **04-03** has the two manifest lines it owns (`talaria-mcp` in `[workspace.dependencies]` and
  `talaria-mcp = { workspace = true }` in the shell) and nothing else to add. Everything else it
  needs — `rust-mcp-sdk` with `streamable-http` + `auth`, `rust-mcp-axum`, `async-trait`, `sha2`,
  tokio's `rt-multi-thread` — is present and building. `04-02-SPIKE.md` carries the published
  `AxumServerOptions` field table, the verified Pattern 3 thread/runtime shape, and the
  `protected_resource_metadata_url` decision it has to make.
- **04-04** must not mint an `AuthInfo` with `expires_at: None`, and must not memoise `verify_token`
  past the request boundary. Both recorded with the source lines.
- **04-05** should not hand-write the `401` or the `WWW-Authenticate` challenge — the SDK builds
  both. The `AuthenticationError` → status mapping table is in the SPIKE.
- **04-06** proceeds as planned, and must call `validate_allowed_methods` itself.
- **04-07** inherits one narrow open question — whether an already-open stream survives a session
  delete — with the fallback named and `sse_support: false` already removing most of the exposure.
- **Phase 5** inherits the TLS obligation as a named prerequisite in `deferred-items.md`.

---
*Phase: 04-authenticated-remote-transport-v2*
*Completed: 2026-08-20*

## Self-Check: PASSED

All six files exist on disk (`Cargo.toml`, `Cargo.lock`, `crates/talaria-shell/Cargo.toml`,
`CHANGELOG.md`, `04-02-SPIKE.md`, `deferred-items.md`); the deleted throwaway is confirmed absent;
all four commit hashes (`ff89c93`, `4f8cb0b`, `bebfc5d`, `4cf6a23`) resolve in `git log`.

---
phase: 05-distributed-mode
plan: 01
subsystem: infra
tags: [cargo, lockfile, axum, websocket, tokio-tungstenite, tailscale-serve, spike, dependencies]

# Dependency graph
requires:
  - phase: 04-authenticated-remote-transport-v2
    provides: "the hand-edit-then-`cargo metadata` dependency procedure (04-02), the `--locked` CI contract, and `http.rs`'s router with `refuse_page_originated` as its outermost layer"
  - phase: 02-harden-the-agent-surface
    provides: "`.github/workflows/lockfile-audit.yml`, the scheduled job that resolves from scratch and prints the primeorder diagnosis"
provides:
  - "`axum 0.8.9` with its `ws` feature available to `talaria-shell` as a direct dependency, unified onto the node `rust-mcp-axum` already resolved"
  - "A `Cargo.lock` that builds `--locked` with the `primeorder 0.14.0-rc.14` pin intact, asserted by grep rather than assumed"
  - "`05-01-SPIKE.md` — assumption A2 settled CONFIRMED with observed evidence, plus the verbatim header set a Serve-proxied request carries and the measured client-TLS dependency cost"
  - "The observed fact that Serve forwards `Host: <tailnet-name>:<serve-port>`, which makes 05-04's advertised-base-URL work required rather than optional"
  - "`deferred-items.md` — the phase's register, opened with the Phase 4 `AxumServerOptions` correction and the first-pairing question stated as open"
affects: [05-02, 05-03, 05-04, 05-05, 05-07, 05-11, phase-05.1]

# Tech tracking
tech-stack:
  added:
    - "axum 0.8.9 (direct, feature `ws`; already resolved transitively via rust-mcp-axum)"
    - "tokio-tungstenite 0.29.0 (transitive, newly resolved — the only new package)"
  patterns:
    - "A dependency line's provenance comment answers legitimacy with a structural fact (its sibling is already linked by Servo's own network stack) rather than with a download count"
    - "A measurement a later plan will need is taken now against a throwaway edit and restored, so the later plan lands a dependency whose lockfile impact is already known"
    - "A spike that touches machine-level state outside git captures that state first, tears its own change down in the same task, and diffs against the capture as an acceptance criterion"

key-files:
  created:
    - .planning/phases/05-distributed-mode/05-01-SPIKE.md
    - .planning/phases/05-distributed-mode/deferred-items.md
  modified:
    - Cargo.toml
    - Cargo.lock
    - crates/talaria-shell/Cargo.toml
    - CHANGELOG.md

key-decisions:
  - "`axum` is declared at the version already resolved (0.8.9) rather than at the newest available — the goal is unification onto the existing node, not an upgrade, and the lock carries exactly one `axum` package before and after"
  - "Resolved with `cargo metadata` against hand-edited manifests; no `cargo add`, no `cargo update`, no crate named on any command line that could pull a newer line"
  - "`talaria-client` was deliberately not added to the workspace `members` list — a member entry naming a directory that does not exist makes `cargo metadata` fail outright, and 05-07 creates the directory"
  - "The spike's `Sec-WebSocket-Accept` check was validated against RFC 6455 §1.3's worked example before its result was believed; the first run's `match=False` was a mistyped GUID in the probe, and a false REFUTED would have re-planned 05-04 onto the certificate fallback for nothing"
  - "`rustls-tls-native-roots` is the recommended client TLS spelling for 05-07: zero added packages, and Serve's certificate chains to a public root the OS trust store already carries"
  - "The e2e suite was run on display `:97` rather than the `:98` the phase brief named, because `:98` is this machine's self-hosted CI runner's display and the two runs SIGKILL each other"

patterns-established:
  - "Pattern: predict the lock diff's *shape* (one added block, zero re-versioned lines) and gate the commit on it, rather than reading the diff for surprises after the fact"
  - "Pattern: a spike verifies its own instrument against a specification's worked example before reporting a verdict the plan gates on"

requirements-completed: []

coverage:
  - id: D1
    description: "The workspace builds `--locked` with axum's WebSocket feature available to `talaria-shell`, and the lockfile grew by exactly one package with no existing version changed"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "cargo build --release --locked (exit 0); cargo clippy --all-targets --locked -- -D warnings (exit 0); cargo test --locked (exit 0)"
        status: pass
      - kind: other
        ref: "git diff Cargo.lock: 1 added [[package]] block (tokio-tungstenite 0.29.0), 0 removed version lines; grep -c '^name = \"axum\"$' Cargo.lock == 1 (one node, unified)"
        status: pass
      - kind: other
        ref: "grep -c '^axum = ' Cargo.toml == 1; grep -c '\"ws\"' Cargo.toml == 1; grep -c '^axum = { workspace = true }' crates/talaria-shell/Cargo.toml == 1; grep -c 'talaria-client' Cargo.toml == 0"
        status: pass
    human_judgment: false
  - id: D2
    description: "`Cargo.lock` still pins `primeorder 0.14.0-rc.14` after the addition — asserted, not assumed"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "grep -A1 'name = \"primeorder\"' Cargo.lock | grep -c '0.14.0-rc.14' == 1; p256/p384/p521 all still 0.14.0-rc.14; cargo build --release --locked exit 0"
        status: pass
    human_judgment: false
  - id: D3
    description: "Assumption A2 is settled with an executed answer before 05-04 and 05-05 are written: the upgrade, the keep-alive survival, and the exact header set a proxied request carries"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "05-01-SPIKE.md carries `A2: CONFIRMED`; grep -cE '^A2: (CONFIRMED|REFUTED)' == 1"
        status: pass
      - kind: integration
        ref: "tailscale serve --bg --https=8449 http://127.0.0.1:40771 then a stdlib RFC 6455 client: 101 Switching Protocols, Sec-WebSocket-Accept verified against an independently computed SHA-1+base64, frames echoed, connection survived 30s and 90s idles on the same server-side socket"
        status: pass
      - kind: integration
        ref: "proxied GET echoed 10 headers verbatim; host = thinkpad.tailcd3cc6.ts.net:8449 (the client's name and Serve port, not the loopback target); no Origin synthesised; a client-sent Origin passed through unchanged"
        status: pass
    human_judgment: false
  - id: D4
    description: "The Tailscale Serve mappings already on this node are identical in `tailscale serve status` before and after the spike"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "tailscale serve status captured before and after: byte-identical, md5 f3df1875edd9a80b61d460fdb56a7ca0 both times; the :8443 -> http://127.0.0.1:5678 mapping belonging to an unrelated running service survives"
        status: pass
    human_judgment: false
  - id: D5
    description: "The spike leaves no source behind and no listener outliving it"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "test ! -f crates/talaria-shell/examples/serve_ws_spike.rs (exit 0); the file appears in no commit; crates/talaria-shell/examples/ removed; 127.0.0.1:40771 no longer bound; the 8449 Serve mapping torn down in the same task"
        status: pass
    human_judgment: false
  - id: D6
    description: "The cost of the client's own TLS WebSocket dependency is measured now and recorded for 05-07"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "hand-edit + cargo metadata against the post-Task-1 lock: rustls-tls-native-roots adds 0 packages, rustls-tls-webpki-roots adds 0, native-tls adds 7 (OpenSSL); manifests and lockfile restored byte-identical, md5 verified before and after"
        status: pass
    human_judgment: false
  - id: D7
    description: "The `talaria-client` workspace member is deliberately not added here"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "grep -c 'talaria-client' Cargo.toml == 0; the crates/ directory still holds exactly talaria-shell, talaria-protocol and talaria-mcp"
        status: pass
    human_judgment: false
  - id: D8
    description: "The phase's deferred register is opened with the corrections and open questions the locked decisions create"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "deferred-items.md: 5 `## ` entries; grep 'AxumServerOptions' == 4; grep -i '8628|device authorization' == 1; DIST-03 and DIST-04 both named; client_id rebinding fact recorded; jpeg-encoder recorded as cleared-not-adopted"
        status: pass
    human_judgment: false
  - id: D9
    description: "The dependency landing changed no behaviour"
    requirement: "DIST-01"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/run_all.py exit 0, 22/22 PASS, `failed: none` — run at the shipping tree state on an isolated display"
        status: pass
      - kind: unit
        ref: "cargo test --locked: 296 passing (2 + 3 + 291, plus two empty targets and 9 ignored), unchanged from the Phase 4 baseline"
        status: pass
    human_judgment: false

# Metrics
duration: 50min
completed: 2026-08-23
status: complete
---

# Phase 5 Plan 01: The WebSocket Dependency and the A2 Spike Summary

**`axum`'s WebSocket feature is available to `talaria-shell` at a measured cost of exactly one
lockfile package with the `primeorder 0.14.0-rc.14` pin proven intact, and the assumption D-05-03's
whole exposure story rests on is settled by an executed experiment: Tailscale Serve proxies a
WebSocket upgrade to loopback cleanly and holds it across a 90-second idle — while forwarding the
tailnet `Host`, which turns 05-04's advertised-base-URL work from optional into required.**

---

## What was built

### Task 1 — the dependency, landed by hand

`axum` is now declared directly in `[workspace.dependencies]` at `0.8.9` — the version already
resolved in this tree — with `features = ["ws"]` and nothing else, grouped with the
`rust-mcp-sdk` / `rust-mcp-axum` block so the manifest keeps reading as engine / chrome / plumbing.
`talaria-shell` takes it with workspace inheritance beside its existing `rust-mcp-axum` line.

The line carries a provenance comment in the register the two existing ones established. It records
three things: that this is a **unification rather than a resolution** (`axum 0.8.9` was already in
the tree through `rust-mcp-axum`, and `http.rs` already used it by name as `rust_mcp_axum::axum::*`,
so declaring it directly is what lets `http.rs` write `axum::extract::ws::WebSocketUpgrade` without
the re-export); that the feature's entire measured cost is one package whose sibling
`tungstenite 0.29.0` Servo's own network stack already pulls in through `servo-net` →
`async-tungstenite` — **which is the legitimacy answer, and a stronger one than a download count**;
and that D-05-05 chose this over adding a codec crate.

**The freshness-limited facts were re-verified rather than trusted.** `05-RESEARCH.md` measured this
on 2026-08-21 and the lockfile is the authority, so the vendored `axum-0.8.9/Cargo.toml` was read at
execution time to confirm `ws = ["dep:hyper", "tokio", "dep:tokio-tungstenite", "dep:sha1",
"dep:base64"]`, that `tokio-tungstenite` is required at `0.29.0`, and that `sha1 0.10` and
`base64 0.22.1` were already resolved here.

Resolution was a `cargo metadata` run against the edited manifests. **No `cargo add`, no
`cargo update`, no crate named on any command line.** Verbatim:

```
     Locking 1 package to latest compatible version
      Adding tokio-tungstenite v0.29.0
```

The lock diff, gated before the commit rather than read after it:

- **exactly one added `[[package]]` block** — `tokio-tungstenite 0.29.0`;
- **three feature edges** under `axum` (`base64`, `sha1 0.10.7`, `tokio-tungstenite`), plus `axum`
  under `talaria-shell`;
- **zero removed version lines** — nothing already in the lock changed version;
- **one `axum` node**, before and after, so the unification held.

The pin, read back rather than assumed:

```
name = "primeorder"
version = "0.14.0-rc.14"
```

and `p256`, `p384` and `p521` all still on their `0.14.0-rc.14` lines. The recovery command the
manifest header documents was never needed.

### Task 1, second half — the client's TLS cost, measured for 05-07

05-07's client connects over `wss://`, so it needs `tokio-tungstenite` as a **direct** dependency
with a TLS feature. The same procedure was run a second time against a throwaway edit and then
backed out, with the manifests and lockfile confirmed byte-identical afterwards by `md5sum`.

| Feature spelling | Packages added vs the post-Task-1 lock |
|------------------|----------------------------------------|
| `rustls-tls-native-roots` | **0** |
| `rustls-tls-webpki-roots` | **0** |
| `native-tls` | 7 (`openssl`, `openssl-sys`, `openssl-macros`, `native-tls`, `tokio-native-tls`, `foreign-types`, `foreign-types-shared`) — and a C dependency |

Everything a rustls client needs is already resolved here: `rustls 0.23.43`, `tokio-rustls 0.26.4`,
`rustls-pki-types 1.15.1`, `rustls-native-certs 0.8.4`, `webpki-roots`. **So 05-07's client TLS is
free**, and `rustls-tls-native-roots` is the recommendation — Serve's certificate is a real
Let's Encrypt certificate chaining to a public root the OS trust store already carries. The
`native-tls` row is recorded only as the contrast that makes the rustls answer obvious.

### Task 2 — the A2 spike

A throwaway cargo example inside `talaria-shell` (so it linked the workspace's real resolved graph,
which is why it ran after Task 1), driven by a hand-written RFC 6455 client over Python's stdlib
`ssl` and `socket` — no third-party client, because the e2e suites are stdlib-only and
`remote_view_test.py` will carry the same decoder.

**`A2: CONFIRMED`.** The upgrade returned `101 Switching Protocols` with a `Sec-WebSocket-Accept`
that matched an independently computed SHA-1-plus-base64 of the key; text frames echoed both ways;
and the connection survived idles of **30 s and 90 s** — both past `McpAppState`'s 12 s ping
interval — then carried traffic again. The server-side counter continued from `3` to `4` across the
idle, which proves it was the *same* socket rather than a transparent reconnect.

**The more valuable half is the header set.** A Serve-proxied request arrives with:

```
accept-encoding: gzip
host: thinkpad.tailcd3cc6.ts.net:8449
tailscale-headers-info: https://tailscale.com/s/serve-headers
tailscale-user-login: CourtimusPrime@github
tailscale-user-name: Court
tailscale-user-profile-pic: https://avatars.githubusercontent.com/u/190138327?v=4
user-agent: talaria-a2-spike
x-forwarded-for: 100.118.105.121
x-forwarded-host: thinkpad.tailcd3cc6.ts.net:8449
x-forwarded-proto: https
```

Three findings 05-04 is now written against rather than guessing at:

1. **Serve forwards the client's `Host`, name and Serve port, not the loopback target.**
   `refuse_page_originated` compares `Host` against the bound address (`127.0.0.1:8779`), so **every
   proxied request would be refused today**, and `DnsRebindProtector` is seeded from the same value.
   The advertised base URL is required work, it must carry the Serve port, and it must come from
   configuration — never from the request header, or a local page could choose the browser's own
   advertised issuer (Pitfall 8, T-05-09).
2. **No `Origin` is synthesised**, and a client-sent one passes through unchanged. The CSWSH defence
   that makes a browser-based viewer structurally impossible needs no change (T-05-01).
3. **The Tailscale identity headers exist and are named**, together with the reason they are never a
   boundary: the hop from `tailscaled` to Talaria is plain HTTP on loopback, so any local account can
   send whatever it likes for them (T-05-08). They are recorded so a later reader who notices
   `tailscale-user-login` finds that sentence before reaching for it.

The Serve mapping used port **8449**, torn down in the same task. `tailscale serve status` was
captured before and after and is **byte-identical**, `md5` `f3df1875edd9a80b61d460fdb56a7ca0` both
times — the `:8443` → `http://127.0.0.1:5678` mapping belonging to an unrelated running service is
untouched. The throwaway example was deleted and the `examples/` directory removed.

### Task 3 — the deferred register

`deferred-items.md` opened in the Phase 4 house format with five entries: the `AxumServerOptions`
correction; the first-pairing question stated as **open** with its three options and their costs, and
with this phase's choice named as a choice; the lossy codec cleared but not adopted with its revisit
condition; what Phase 5.1 owns, including the two design facts it will need; and the MCP
`2026-07-28` migration's reach into 5.1's session model.

---

## Deviations from Plan

### Auto-fixed issues

**1. [Rule 3 — Blocking] The e2e suite ran on display `:97`, not `:98`**

- **Found during:** Task 1 verification.
- **Issue:** the first e2e run aborted at `shell died early, exit -9`. The cause was not the
  dependency change: this machine's **self-hosted GitHub Actions runner was executing the same e2e
  suite concurrently**, out of `~/actions-runner/_work/Talaria/Talaria/`, on the same display. The
  harness's `start_xvfb` opens with `pkill -f "[X]vfb :98"` and `kill_shells_on_display`, so whichever
  run starts second SIGKILLs the other's Xvfb and its shells. My run killed CI's at 14:19; CI's
  restart killed mine. `exit -9` is exactly `kill_shells_on_display`'s SIGKILL.
- **Fix:** waited for the CI job to finish, then reran with `TALARIA_E2E_DISPLAY=:97` and a private
  `XDG_RUNTIME_DIR`. Isolation is complete — both `pkill` and `kill_shells_on_display` are scoped by
  display.
- **Files modified:** none. This is a runner-environment fact, not a repository change.
- **Worth knowing:** the phase brief named `:98`, which is the display CI uses on this machine.
  Anything running the suite by hand here should pick a different one.

**2. [Rule 1 — Bug] The spike's own `Sec-WebSocket-Accept` check was wrong before it was right**

- **Found during:** Task 2.
- **Issue:** the first proxied run reported `match=False` on the accept key. Taken at face value that
  reads as a Serve finding and would have re-planned 05-04 onto the certificate fallback.
- **Fix:** validated the probe's derivation against RFC 6455 §1.3's worked example
  (`dGhlIHNhbXBsZSBub25jZQ==` → `s3pPLMBiTxaQ9kYGzzhZRbK+xOo=`), which it failed — the probe carried a
  mistyped GUID. The canonical constant was read out of `tungstenite-0.29.0`'s own
  `derive_accept_key`, the probe corrected, the worked example reproduced, and the proxied probe
  rerun: `match=True`. Recorded in `05-01-SPIKE.md` because a false REFUTED is the expensive failure
  here.
- **Files modified:** none in the repository — the probe was a throwaway under `/tmp`.

**3. [Rule 2 — Missing critical] `CHANGELOG.md` updated**

- **Found during:** Task 1.
- **Issue:** the plan's `files_modified` does not list `CHANGELOG.md`, but the global rule is "log all
  changes in a project CHANGELOG.md", and Phase 4's 04-02 — the precedent this plan inherits — logged
  its dependency landing there in a dedicated commit.
- **Fix:** added a `### Changed` entry in the same register as 04-02's, recording the one added
  package, that nothing changed version, that the pin survived, and that no route is mounted yet.
- **Files modified:** `CHANGELOG.md`. **Commit:** `44397c8`.

### Corrections to the plan's stated premises

Neither changed what was done; both are recorded because a later reader would otherwise inherit them.

**4. There are four pre-existing Tailscale Serve mappings on this node, not three.**
`05-CONTEXT.md` and `05-01-PLAN.md` both name three (`:8443`, `:9446`, `:9447`). At execution there
was also a mapping on the default `:443` proxying `http://100.118.105.121:8086`, added since the
2026-08-21 research. The research put a seven-day validity on every Tailscale claim, and this is what
that was for. Port `8449` was still free, so nothing about the plan changed — but the acceptance
criterion "the three pre-existing mappings, no fourth" was evaluated as **all four pre-existing
mappings, no fifth**, which is what it means.

**5. One acceptance criterion is unsatisfiable as literally written.**
Task 1 asks that `grep -c 'axum = { workspace = true }' crates/talaria-shell/Cargo.toml` be 1. It is
**2**, and would have been 1 *before* this plan — because the pre-existing line
`rust-mcp-axum = { workspace = true }` contains that string as a substring. The criterion's intent is
"the shell has a direct `axum` dependency", and the anchored form
`grep -c '^axum = { workspace = true }'` is 1. Verified in the anchored form and flagged rather than
silently reinterpreted.

### Not done, deliberately

The `talaria-client` workspace member was **not** added, per the plan and for the reason it gives: a
`members` entry naming a directory that does not exist makes `cargo metadata` fail outright, and
05-07 creates the directory. `grep -c 'talaria-client' Cargo.toml` is 0.

---

## Verification

| Check | Result |
|-------|--------|
| `cargo build --release --locked` | exit 0 |
| `cargo clippy --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --locked` | exit 0, **296 passing** (2 + 3 + 291; 9 ignored) — unchanged |
| `python3 tests/e2e/run_all.py` | exit 0, **22/22 PASS**, `failed: none` — unchanged |
| `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'` | 1 |
| `git diff Cargo.lock \| grep -c '^+\[\[package\]\]'` | 1 |
| `git diff Cargo.lock \| grep -cE '^-version = '` | 0 |
| `grep -cE '^A2: (CONFIRMED\|REFUTED)' 05-01-SPIKE.md` | 1 |
| `test ! -f crates/talaria-shell/examples/serve_ws_spike.rs` | exit 0 |
| `tailscale serve status` vs baseline | byte-identical, `f3df1875edd9a80b61d460fdb56a7ca0` |
| `grep -c '^## ' deferred-items.md` | 5 |

The e2e and unit runs above were taken at the **shipping tree state**, after the spike example was
deleted.

---

## What this hands to the rest of the phase

- **05-04** knows the exact `Host` a proxied request carries and that the allowlist and the rebind
  protector both need the advertised identity, with the Serve port on it. It also knows the identity
  headers exist and must not become a boundary.
- **05-05** can build an `axum` WebSocket route knowing the upgrade survives Serve and a 90-second
  idle, and that `refuse_page_originated` needs no relaxation.
- **05-07** lands its client TLS dependency knowing it costs zero packages, and which spelling to use.
- **05-11** has the `SECURITY.md` material: the Origin refusal is what makes a browser-based viewer
  structurally impossible, and the Tailscale identity headers are a named non-boundary.
- **Phase 5.1** has the verified-`client_id` rebinding fact and the `process_pending_events` discard
  behaviour written down before it needs them.

## Threat Flags

None. This plan added no network endpoint, no auth path, no file access pattern and no schema at a
trust boundary. The one port it opened and the one Serve mapping it created were both torn down
inside the task that created them, and the teardown is asserted rather than intended.

## Self-Check: PASSED

All created files exist on disk; all four commit hashes are present in `git log`.

---
phase: 05-distributed-mode
plan: 07
subsystem: core
tags: [client, websocket, rustls, surfman, egui, credential, chrome, e2e-harness]

# Dependency graph
requires:
  - phase: 05-distributed-mode
    provides: "05-01's measurement that `tokio-tungstenite`'s `rustls-tls-native-roots` costs **zero** packages against this lock, and that `native-tls` would cost 7 plus a C dependency on OpenSSL — so this plan landed a known number rather than discovering one"
  - phase: 05-distributed-mode
    provides: "05-03's wire vocabulary — `Channel`, `ServerView::Hello`, `TabList`, `FrameHeader`/`FRAME_HEADER_LEN`, `PROTOCOL_VERSION` — and its rule that an unrecognised tag is a refusal rather than a fallthrough"
  - phase: 05-distributed-mode
    provides: "05-05's `GET /view`: the route, the hello-first ordering, the bearer check on the upgrade itself, and `ServerView::Refused`'s deliberate absence of a reason"
  - phase: 05-distributed-mode
    provides: "05-PATTERNS.md's map of what a second GUI binary copies from `talaria-shell` and what it must not, including its warning that the graphics context is the one step with no analog"
  - phase: 04-authenticated-remote-transport-v2
    provides: "`04-UI-SPEC.md`'s copy register and untrusted-claim rules, and `oauth.rs`'s rule that a credential is never interpolated into a log"
provides:
  - "`crates/talaria-client` — a fourth workspace member and a second binary, `talaria-client`, that links no web engine"
  - "`WindowSurface` — a `surfman` window context and `glow` loader built without the engine, the step `05-PATTERNS.md` marked unmapped"
  - "`net::ConnectionState` — nine variants, each with its own copy and its own next step, including `Untrusted` for a certificate that did not verify"
  - "`net::view_endpoint` — the view endpoint derived from the one base URL argument, refusing a non-loopback `http://` rather than downgrading it"
  - "`net::spawn` / `net::Update` — the connection thread and the one-way channel it reaches the event loop through"
  - "`chrome::Chrome` / `chrome::UiAction` — the client's three surfaces, its intent enum, and its geometry hook"
  - "`TALARIA_CLIENT_TOKEN` and an owner-only credential file — the two places a bearer credential may come from, and `argv` is not one"
  - "`harness.start_client` / `client_rects` / `wait_for_client_rect` — a client launcher whose readiness signal is a drawn frame rather than a socket on disk"
  - "`talaria-client-rects` — one JSON line of control geometry per frame under `TALARIA_TEST_HOOKS=1`, the seam 05-09 and 05-10 assert against"
affects: [05-08, 05-09, 05-10, 05-11]

# Tech tracking
tech-stack:
  added:
    - "surfman 0.13.0 (`sm-x11`) — direct, zero added packages: the crate servo-paint-api already resolves at the same version with the same features"
    - "tokio-tungstenite 0.29.0 (`rustls-tls-native-roots`) — zero added packages, exactly 05-01's measurement"
    - "futures-util 0.3 (`default-features = false`, `std` + `sink`) — zero added packages; tokio-tungstenite already depends on it"
    - "log 0.4 / env_logger 0.11 — zero added packages; both already resolved"
  patterns:
    - "A security property enforced by the absence of an argument: `connect_async` with no connector verifies against native roots, so disabling verification would mean writing new code rather than setting a value"
    - "A state enumeration whose doc comment states that each variant exists because the next step differs, so a later collapse has to argue against it"
    - "Test-only geometry emitted on the process's own standard output, where the process owns no socket to answer on"
    - "An empty state gated on the connection being live, because a claim about a server needs a server to have been reached"

key-files:
  created:
    - crates/talaria-client/Cargo.toml
    - crates/talaria-client/src/main.rs
    - crates/talaria-client/src/net.rs
    - crates/talaria-client/src/chrome.rs
  modified:
    - Cargo.toml
    - Cargo.lock
    - tests/e2e/harness.py
    - .planning/phases/05-distributed-mode/deferred-items.md

key-decisions:
  - "The graphics context is `surfman` taken directly, not `glutin`: servo-paint-api's own `[dependencies.surfman]` block names version 0.13.0 with `chains` and `sm-x11`, so a direct dependency at that version unifies onto a node already in the lock and the measured cost is zero packages. `glutin` would have resolved a new subtree for the same capability."
  - "A certificate that did not verify is its own `ConnectionState` variant (`Untrusted`), which is a ninth beyond the eight the plan's action text enumerated. Folding it into `Unreachable` would send a human to check the network when the next step is to check the certificate — the exact failure the enumeration's own doc comment argues against."
  - "An `http://` base URL naming anything but a loopback literal is refused at startup rather than connected to, because the alternative is putting a bearer token on the wire in the clear. A *name* that happens to resolve to loopback is not accepted either: what it resolves to is not this client's decision."
  - "There is no retry loop. Every failed state has a human next step, and a client that silently reconnected forever would hide all of them behind a spinner. `UiAction::Reconnect` is a press, and it is the client's only control."
  - "The tab surface — including its empty state — is drawn only while connected, correcting a first draft that showed 'no agent has opened a tab on this server' underneath 'no credential is configured'."
  - "The client's readiness signal is a line it prints after a frame is actually drawn, not the shell's control-socket check: a client owns no socket, and a drawn frame also proves GL came up where an alive process does not."
  - "`truncate_chars` and `sanitize_claim` are duplicated from the shell rather than shared. `talaria-shell` is a binary crate with the engine linked in, and depending on it would undo this binary's entire reason for existing; `talaria-protocol` is the wire vocabulary and deliberately carries no display concerns."

patterns-established:
  - "Pattern 1: prefer the dependency the engine already resolves over the one the ecosystem defaults to — measure both, and let the lock diff decide"
  - "Pattern 2: make a security property structural rather than configurable, then assert the absence of every spelling of the escape hatch"
  - "Pattern 3: a second front end copies the first's conventions and refuses its workarounds — no `thread_local!` for the chrome, because there is no `Rc<Shared>` to work around"

requirements-completed: [DIST-01]

coverage:
  - id: D1
    description: "A second binary exists, builds without the web engine, and opens a window drawn through its own graphics context"
    requirement: "DIST-01"
    verification:
      - kind: cli
        ref: "cargo tree -p talaria-client | grep -c '^servo\\| servo v' — 0; grep -c servo crates/talaria-client/Cargo.toml — 0"
        status: pass
      - kind: manual
        ref: "recorded one-off run below — the client drew its first frame under Xvfb :95 and printed its control geometry"
        status: pass
    human_judgment: false
  - id: D2
    description: "The client verifies the server's certificate against the platform trust roots, and no flag, environment variable or build feature relaxes it"
    requirement: "DIST-01"
    verification:
      - kind: cli
        ref: "! grep -rqi 'danger|no_verify|noverify|accept_invalid|insecure_skip' crates/talaria-client/src/ — absent in every spelling"
        status: pass
      - kind: unit
        ref: "cargo test -p talaria-client net::tests::a_certificate_that_did_not_verify_is_its_own_state"
        status: pass
    human_judgment: false
  - id: D3
    description: "The bearer credential never reaches the command line and never reaches a log"
    requirement: "DIST-01"
    verification:
      - kind: cli
        ref: "grep -c 'std::env::args' crates/talaria-client/src/net.rs — 0; every log call in the file names the credential's *source* and never its value"
        status: pass
      - kind: unit
        ref: "cargo test -p talaria-client net::tests::the_environment_wins_over_the_file / a_blank_variable_is_absent_rather_than_a_credential / neither_place_holding_one_is_the_missing_credential_case"
        status: pass
    human_judgment: false
  - id: D4
    description: "Every way the connection can fail is a named state the human can read, never a hang, a panic or a silent retry"
    requirement: "DIST-01"
    verification:
      - kind: unit
        ref: "cargo test -p talaria-client net::tests — 17 tests, including two driving a real local WebSocket peer through hello, snapshot and close"
        status: pass
      - kind: cli
        ref: "grep -c 'unwrap()|expect(' crates/talaria-client/src/net.rs — 0"
        status: pass
    human_judgment: false
  - id: D5
    description: "The tab list renders in the server's order, treats a page-supplied title and address as claims, and renders a real empty state at zero tabs"
    requirement: "DIST-01"
    verification:
      - kind: unit
        ref: "cargo test -p talaria-client chrome::tests — 4 tests on the claim treatment; net::tests::a_tab_snapshot_decodes_in_the_order_the_server_sent_it"
        status: pass
      - kind: cli
        ref: "grep -c 'sort_by|sort_unstable|.sort()|.rev()' crates/talaria-client/src/chrome.rs — 0"
        status: pass
      - kind: manual
        ref: "recorded one-off run below — `tabs.empty` with no agent tabs, `tabs.row.0` once one existed"
        status: pass
    human_judgment: false
  - id: D6
    description: "The first-run surface states that authorising this client the first time requires a human at the server machine"
    requirement: "DIST-01"
    verification:
      - kind: manual
        ref: "recorded first-run screenshot below — 'Getting a token needs someone at the server machine.'"
        status: pass
    human_judgment: true
  - id: D7
    description: "The server binary and every local code path are unchanged, and everything Phases 1–4 assert still holds"
    requirement: "DIST-01"
    verification:
      - kind: cli
        ref: "git status --porcelain crates/talaria-shell/src/ — empty; git diff --stat tests/e2e/remote_view_test.py — empty"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/run_all.py — 23/23 PASS, failed: none"
        status: pass
    human_judgment: false

# Metrics
duration: 96min
completed: 2026-08-23
status: complete
---

# Phase 5 Plan 07: The `talaria-client` Crate Summary

**A second front end that links no engine — 222 dependency crates against the shell's 690, a 19 MB binary against 183 MB — and whose certificate verification is enforced by the absence of an argument rather than by the presence of a rule.**

## Performance

- **Duration:** 96 min
- **Started:** 2026-08-23T19:07Z
- **Completed:** 2026-08-23T20:43Z
- **Tasks:** 3 of 3
- **Files modified:** 8 (4 created, 4 modified)

## Accomplishments

- **The one unmapped step cost nothing.** `05-PATTERNS.md` flagged the graphics
  context as the single genuine gap in the shell-as-analog and asked that it be
  treated as a bounded task rather than assumed to fall out of `Gui::new`. It
  does not: `EguiGlow::new` wants an `Arc<glow::Context>`, and the shell's comes
  from the engine's `WindowRenderingContext`. Reading
  `servo-paint-api-0.4.0/rendering_context.rs` showed that type is a thin
  wrapper over `surfman` — connection from the display handle, adapter, device,
  context descriptor, widget surface, `glow::Context::from_loader_function` —
  and its manifest names `surfman 0.13.0` with `chains` and `sm-x11`. Taking
  that crate directly at that version unified onto the node already in the lock
  and added **zero packages**. `WindowSurface` is deliberately smaller than the
  engine's: no swap chain, no offscreen variant, no surface-texture path and no
  `gleam` beside `glow`, because every one of those serves a compositor this
  binary does not have.
- **Certificate verification is structural, not a rule to be followed.**
  `tokio_tungstenite::connect_async` with no explicit connector builds a rustls
  client from `rustls-native-certs` and verifies. There is no argument to pass
  it — so turning verification off would mean *writing new code*, not changing a
  value, which is the strongest form the property can take. The end-to-end path
  needs no escape hatch because it connects to loopback **without** transport
  security rather than **with** it disabled; those are different things and only
  the second is a hole. `! grep -rqi 'danger\|no_verify\|noverify\|accept_invalid\|insecure_skip'`
  holds across the whole crate.
- **A refusal to downgrade, which the plan did not ask for and the threat model
  implies.** `view_endpoint` maps `https` to `wss` always, and `http` to `ws`
  **only** for a loopback literal. An `http://` base naming any other host is
  refused before the window opens, because connecting to it would put the bearer
  token on the wire in the clear. A hostname that merely resolves to loopback is
  not accepted either: what it resolves to is not this client's decision, and a
  plaintext credential is not something to gamble on a resolver.
- **Nine states, and the ninth is the point.** The plan's action text enumerated
  eight; a certificate that did not verify would have had to land in
  `Unreachable`, which sends a human to check the network when the next step is
  to check the certificate. That is precisely the failure the enumeration's own
  doc comment argues against, so `Untrusted` exists and carries its own copy —
  including the sentence that nothing turns the check off, because a human who
  does not know that will go looking for the switch.
- **A first draft told the user two contradictory things at once.** The tab
  surface originally drew unconditionally, so the first-run screen showed "No
  agent has opened a tab on this server." directly beneath "No credential is
  configured, so nothing has been tried yet." — a claim about a server made by a
  client that never reached one, inviting a person to go looking for the agent
  rather than for the token. The tab surface is now gated on `Connected`, and
  the first-run capture below was retaken to prove it.
- **The harness got a readiness signal that means something.** `start_shell`
  waits for the control socket; a client owns none, and checking for one would
  answer "yes" the moment any shell was running. The client instead prints one
  `talaria-client-rects` line per drawn frame under `TALARIA_TEST_HOOKS=1`.
  Waiting for the first is stronger than waiting for a process or even a window:
  a client whose graphics context failed would be alive, would own a window, and
  would never print one. The same lines give 05-09 and 05-10 controls by name.

## Task Commits

1. **Tasks 1–3: the crate, its window and graphics context, its connection, and its interface** — `adcc708` (feat)
2. **Task 3: the harness client launcher** — `b8a558d` (test)
3. **Task 3: the deferred design-contract entry** — `3da809b` (docs)

## Files Created/Modified

- `crates/talaria-client/Cargo.toml` — **created.** Every dependency
  `{ workspace = true }`, no version string anywhere; the shell's trailing
  `log = "0.4"` is a wart this manifest does not copy. The comment block states
  that the engine's absence is the point and names `cargo tree` as the assertion.
- `crates/talaria-client/src/main.rs` — **created.** Module header saying what
  the binary is and what it deliberately is not; `WindowSurface`; the two-state
  `App` with window creation in `resumed`; the drain-then-set-wait `new_events`;
  the crypto-provider install first with a rewritten reason; `env_logger`
  installed with the reason the shell's "no logger here" comment does not
  transfer; one positional argument; the connection thread spawned with a cloned
  proxy before the loop runs; intents applied after the interface's borrows drop.
- `crates/talaria-client/src/net.rs` — **created.** Credential sourcing and
  precedence, the owner-only mode check, endpoint derivation, the nine-variant
  connection state, `classify`, the decode loop and its two discards, and 17
  tests — 15 pure and 2 driving a real local WebSocket peer stood up in the test
  rather than borrowed from the shell.
- `crates/talaria-client/src/chrome.rs` — **created.** Three surfaces, the
  `UiAction` enum, the geometry helper, and the full copy as a table in the
  module's own doc comment so it can be reviewed as copy.
- `Cargo.toml` — the fourth member, and four workspace dependency lines each
  carrying its provenance and its measured cost.
- `Cargo.lock` — one added `[[package]]`, and it is `talaria-client` itself.
- `tests/e2e/harness.py` — `start_client`, `client_rects`,
  `wait_for_client_rect`, `CLIENT_RECTS_PREFIX`.
- `.planning/phases/05-distributed-mode/deferred-items.md` — the eleventh entry.

## The lockfile, measured against 05-01

```
$ git diff Cargo.lock | grep -c '^+\[\[package\]\]'   →  1   (talaria-client itself)
$ git diff Cargo.lock | grep -c '^-\[\[package\]\]'   →  0
$ git diff Cargo.lock | grep -cE '^-version = '       →  0
$ grep -A1 'name = "primeorder"' Cargo.lock | grep -c '0.14.0-rc.14'  →  1
```

The whole third-party diff is four new dependency *edges* on the
`tokio-tungstenite` node — `rustls`, `rustls-native-certs`, `rustls-pki-types`,
`tokio-rustls` — every one of which was already resolved in this tree. **That is
05-01's measurement exactly:** it recorded `rustls-tls-native-roots` at zero
added packages and named those four crates as the reason. No difference to
report. `surfman`, `futures-util`, `log` and `env_logger` were each landed by
the same hand-edit-then-`cargo metadata` procedure and each cost zero as well.

The graphics route taken was **surfman directly**, not the `glutin` fallback
`<interfaces>` allowed for. Its cost in the lockfile is zero, so the fallback
and its legitimacy note were not needed.

## Recorded one-off client run

Per the plan, this task owes a recorded run rather than an edit to
`tests/e2e/remote_view_test.py`, which a sibling plan is extending in this same
wave. A shell was started with remote access on and a seeded authorization
record; the client was started against it with the credential in its
environment. Observed output, verbatim:

```
SHELL: remote access on, listening on 127.0.0.1:44169
CLIENT: first frame drawn; controls = ['connection.state', 'tabs.empty']
VIEW CONNECTION: [2026-08-23T15:55:19Z INFO  talaria::http] view connection 1 opened by client client-oneoff-viewer
EMPTY STATE: controls = ['connection.state', 'tabs.empty']
AGENT TAB opened: {'tab': {'tab_id': 2, 'url': 'about:blank', 'title': '', 'owner': 'agent-oneoff', 'focused': True, 'crashed': False, 'loading': False}}
ROW RENDERED: controls = ['connection.state', 'tabs.row.0']
ROW GEOMETRY: [{'name': 'tabs.row.0', 'x': 8.0, 'y': 56.5, 'width': 10.1875, 'height': 15.0}]
ONE-OFF CLIENT RUN PASSED
```

All three observations the plan asked for:

1. **The shell reports the view connection** — the `INFO talaria::http` line
   above, which is `ViewRoute::run`'s own log. The control socket carries no
   view-session command (05-05 added none), so the shell's log at `RUST_LOG=info`
   is where the server says it happened.
2. **The client rendered a row once an agent tab existed** — `tabs.row.0`
   appeared, with a real rectangle, after `tabs_open` under an agent client.
3. **With no agent tabs it rendered the empty state, not a blank pane** —
   `tabs.empty` was present from the first frame.

The client's window, connected, with one agent tab (`""` is the tab's empty
title, rendered as a quoted claim; `"about:blank"` beneath it):

```
Connected to http://127.0.0.1:38901.
────────────────────────────────────────
""   (as claimed)
"about:blank"
```

And the first-run surface, with no credential configured:

```
No credential is configured, so nothing has been tried yet.
Getting a token needs someone at the server machine.
  Talaria asks for approval in its own window, on the computer running the server, so a
  person has to be there to approve this client the first time. After that the token works
  from here.
  Set TALARIA_CLIENT_TOKEN in this client's environment, or put the token in
  /tmp/talaria-firstrun-home/.config/talaria/client-token and chmod 600 it.
[ Reconnect ]
```

Its controls, from the same hook — note that `tabs.empty` is **absent**, which
is the fix described under Deviations:

```
talaria-client-rects [{"name":"connection.state",...},{"name":"pairing.constraint",...},{"name":"connection.reconnect",...}]
```

05-09 turns all of this into standing assertions; this is the evidence the
binary works before that plan is written against it.

## The connection state enumeration and its copy

| Variant | Fact line | Next step |
|---------|-----------|-----------|
| `NotStarted` | `Not connected yet.` | `This line changes on its own when the connection opens.` |
| `Connecting` | `Connecting to {server}…` | — |
| `Connected` | `Connected to {server}.` | — (the tab list below is the next thing to read) |
| `MissingCredential` | `No credential is configured, so nothing has been tried yet.` | the pairing block |
| `Unreachable` | `Nothing answered at {server}.` | `Check the address, and check that remote access is turned on in the server's own window.` |
| `Untrusted` | `The server's certificate did not verify, so no credential was sent.` | `Talaria checks the certificate against this machine's trust store, and nothing turns that off. Check that {server} is the name the certificate was issued for.` |
| `Refused` | `{server} declined the connection.` | `The server does not say why, on purpose — a refusal that gave a reason would let anyone find out which credentials exist. Check that this client is still approved on the server, and that the token here is the current one.` |
| `VersionMismatch { server, client }` | `The server speaks view protocol {server}; this client speaks {client}.` | `The two were built against different wires. Update whichever is older.` |
| `Dropped` | `The connection to {server} ended.` | `Reconnect opens it again.` |

No variant carries the server address: the chrome already holds the address the
human typed, and a second copy of one fact is a second thing that can disagree.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 — Bug] The empty state was drawn while not connected**

- **Found during:** Task 3, in the first-run screenshot capture
- **Issue:** `Chrome::update` drew the tab surface unconditionally, so the
  first-run screen rendered "No agent has opened a tab on this server."
  immediately beneath "No credential is configured, so nothing has been tried
  yet." That is a claim about a *server*, made by a client that never reached
  one — and it contradicted the module's own comment, which says the empty state
  is the connected-and-empty case specifically and is distinct from every
  not-connected state above it. A human reading it would go looking for the
  agent rather than for the token.
- **Fix:** the separator and the tab surface are drawn only when the state is
  `Connected`, with the reasoning in a comment at the `if`.
- **Files modified:** `crates/talaria-client/src/chrome.rs`
- **Commit:** `adcc708`

**2. [Rule 2 — Missing critical functionality] A ninth connection state for a certificate that did not verify**

- **Found during:** Task 2
- **Issue:** the plan's action text enumerated eight states and none of them was
  a certificate failure, while the same task's behaviour list requires "a
  certificate that does not verify produces a named state". Routing it into
  `Unreachable` would have satisfied the letter and broken the intent: the next
  step for "nothing answered" is to check the address, and the next step for
  "the certificate did not verify" is to check the certificate.
- **Fix:** `ConnectionState::Untrusted`, with its own copy naming what was
  checked and stating that nothing turns the check off.
- **Files modified:** `crates/talaria-client/src/net.rs`, `chrome.rs`
- **Commit:** `adcc708`

**3. [Rule 2 — Missing critical functionality] A non-loopback `http://` base URL is refused rather than downgraded**

- **Found during:** Task 2
- **Issue:** the plan says a secure base yields a secure socket and the loopback
  insecure form yields a plain one, but does not say what an insecure base to a
  *remote* host does. Deriving `ws://` from it would put the bearer credential on
  the wire in the clear — the same class of exposure the argv rule exists to
  close, arrived at from the other direction.
- **Fix:** `view_endpoint` returns `None` for it, and `main` refuses at startup
  with a message naming the reason. Loopback is matched on literals only, never
  on a name that happens to resolve there.
- **Files modified:** `crates/talaria-client/src/net.rs`, `main.rs`
- **Commit:** `adcc708`

**4. [Rule 3 — Blocking] `futures-util` as a fourth workspace dependency line**

- **Found during:** Task 2
- **Issue:** `tokio-tungstenite`'s socket type is written against `Stream` and
  `Sink`, and a trait has to be nameable to be called — `socket.next()` does not
  compile without `StreamExt` in scope.
- **Fix:** declared with `default-features = false, features = ["std", "sink"]`.
  Not a third dependency in 05-01's sense: `tokio-tungstenite` already depends
  on it, so it was in this lock before the line existed and the measured cost is
  zero packages. Recorded here rather than absorbed, as the plan asked.
- **Files modified:** `Cargo.toml`, `crates/talaria-client/Cargo.toml`
- **Commit:** `adcc708`

### Process Deviations

**5. Three tasks landed in one source commit**

The plan asks for a commit per task. `crates/talaria-client/src/main.rs` is
touched by all three — Task 1 creates the entry point and window, Task 2 adds
the connection thread and the event channel, Task 3 adds the chrome and the
intent application — and the work was completed before the first commit was
made. Splitting after the fact would have produced commits whose trees do not
build: a `main.rs` carrying `mod net;` with no `net.rs` beside it is not a
buildable state, and a fabricated intermediate would be worse than an honest
combined one. The commit message names all three pieces and says why they are
together. The harness launcher and the deferred entry are separate commits, as
they touch separate files.

**Correction for future plans in this phase:** commit each task as it completes
rather than at the end of the plan.

**6. One e2e run was abandoned and restarted**

The first full-suite run wedged in `download_bounds_test` after ~20 minutes, with
its shell idle and the suite blocked reading its control socket. The suite passes
standalone in 17 s against the same binaries, and the second full run passed it
in 17 s as part of 23/23. The most likely cause is the hazard this phase's own
`deferred-items.md` already records — a smoke-test `start_xvfb(':95')` of mine
racing the suite's own display setup. **No display work was done while the second
run was in flight**, and the plan's `:95` instruction is exactly the mitigation
for the `:98` collision; the same discipline is needed *within* a session, not
only across them.

## Verification

| Check | Result |
|-------|--------|
| `cargo build --release --locked` | exit 0, `target/release/talaria-client` present |
| `cargo clippy --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --locked` | exit 0, **410 passing** (21 + 2 + 24 + 363; 9 ignored) — baseline 389, +21 from the client |
| `cargo test -p talaria-client --locked` | exit 0, 17 passing |
| `python3 tests/e2e/run_all.py` | exit 0, **23/23 PASS**, `failed: none` |
| `cargo tree -p talaria-client \| grep -c '^servo\| servo v'` | 0 |
| `grep -c 'servo' crates/talaria-client/Cargo.toml` | 0 |
| `grep -c 'crates/talaria-client' Cargo.toml` | 1 |
| `grep -c 'version = "' crates/talaria-client/Cargo.toml` | 0 |
| `grep -c 'forward_to_running_instance\|single.instance' .../main.rs` | 0 |
| `grep -c 'egui_phosphor' .../main.rs` | 1 |
| `! grep -rq 'unwrap()' crates/talaria-client/src/` | absent |
| `! grep -rqi 'danger\|no_verify\|noverify\|accept_invalid\|insecure_skip' crates/talaria-client/src/` | absent |
| `grep -c 'std::env::args' crates/talaria-client/src/net.rs` | 0 |
| `grep -c 'unwrap()\|expect(' crates/talaria-client/src/net.rs` | 0 |
| `grep -c '#\[test\]' crates/talaria-client/src/net.rs` | 15 (plus 2 `#[tokio::test]`) |
| `grep -c '0600\|0o600' crates/talaria-client/src/net.rs` | 2 |
| `grep -ci 'sanitize\|sanitise\|truncate' .../chrome.rs` | 10 |
| `grep -c 'sort_by\|sort_unstable\|\.sort()\|\.rev()' .../chrome.rs` | 0 |
| `grep -ci 'empty' .../chrome.rs` | 8 |
| `grep -ci 'server machine\|at the server' .../chrome.rs` | 3 |
| `grep -c 'start_client' tests/e2e/harness.py` | 1 |
| `grep -c 'os.path.exists(SOCK)' tests/e2e/harness.py` | 1 |
| `grep -c '^## ' deferred-items.md` | 11 |
| `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'` | 1 |
| `git diff Cargo.lock \| grep -cE '^-version = '` | 0 |
| `git status --porcelain crates/talaria-shell/src/` | empty |
| `git diff --stat tests/e2e/remote_view_test.py` | empty |
| `python3 tests/e2e/takeover_test.py` / `panel_click_test.py` | both PASS unmodified, inside `run_all.py` |

## What this hands to the rest of the phase

- **05-08** has a client that connects and lists but sees no frames, which is
  precisely the state its own blocker note describes: a hidden webview answers
  no hit test, and an attachment neither shows one nor ticks a pump until 05-08
  fixes it. Nothing here depends on that being fixed.
- **05-09** has a real binary to write assertions against, `harness.start_client`
  to launch it, `client_rects`/`wait_for_client_rect` to find controls by name,
  and `frame_header` in `net.rs` as the seam the tile presenter grows from. It
  also owns `remote_view_test.py`, which this plan left untouched.
- **05-10** has the connection state and its copy table as the place a
  link-quality surface joins, and the deferred entry's dated condition: the
  fourth surface arrives at 05-10 and the client's design contract is due before
  it.
- **05-11** has the client-side half of the certificate story for `SECURITY.md`:
  the server handles no certificate and the client verifies one, and the phase
  gate's `! grep 'certificate' crates/talaria-shell/src/` is unaffected because
  every mention lives in `crates/talaria-client/`.

## Known Stubs

None. Every surface this plan ships renders live data or a state that is true of
the live connection. The frame and event channels are decoded and discarded
rather than stubbed, and the discard is documented at the `match` arm with the
plan that picks each up.

## Threat Flags

None. This plan added no network *endpoint* — it added a network *client* — no
new auth path on the server, and no schema at a trust boundary. The one file it
reads is the credential file, whose mode is checked before it is read, and the
one new trust boundary (a page-supplied tab title reaching a surface that had
never rendered one) is mitigated in `chrome.rs` by the same treatment the
server's lists use, as `<threat_model>`'s T-05-04-D requires.

## Self-Check: PASSED

All four created source files exist on disk, the SUMMARY exists at the path the
plan names, and all three commit hashes are present in `git log`.

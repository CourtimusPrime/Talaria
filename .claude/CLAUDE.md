<!-- GSD:project-start source:PROJECT.md -->

## Project

**Talaria**

Talaria is an open-source, non-Chromium web browser built on the Servo engine (via libservo),
designed for both human daily use and AI agents to drive directly through a built-in MCP server.
A single toggle switches the view between the user's own tabs ("Me") and tabs an agent is actively
driving ("Agents"), and the user can take over an agent's live session at any moment — click,
scroll, type — in the same session, not a handoff to a copy.

**Core Value:** An agent can drive a real, already-logged-in browsing session, and a human can take over instantly
the moment it hits something only a human can clear (a login wall, a CAPTCHA, a Cloudflare
challenge), without either side sacrificing performance.

### Constraints

- **Tech Stack**: Rust throughout — Servo/libservo (rendering engine), egui + winit (shell chrome,
  corrected from an original Tauri+React plan), `rust-mcp-sdk` (MCP server) — established through
  direct implementation, not just design. Three crates: `talaria-shell`, `talaria-mcp`,
  `talaria-protocol`.

- **Transport**: MCP is stdio-only today (`crates/talaria-mcp/src/main.rs`), which is both why no
  auth exists and why auth work must land alongside an HTTP transport.

- **Performance**: screenshot capture must stay well under the ~30–60ms active-takeover latency
  target (currently 4–36ms); startup-to-socket-ready ~21ms; page load ~219ms.

- **Platform**: Linux is the de-facto current baseline — the e2e harness runs under Xvfb with
  `xdotool` and all measured runs are Linux. The original plan named macOS first; macOS status is
  now unverified and Windows is untouched.

- **Dependency risk**: Servo itself was accepted as a not-fully-mature dependency. The egui-on-winit
  shell decision removed the experimental Servo-backed WRY dependency entirely — a larger risk
  reduction than the originally planned fallback.

- **Licensing**: dual MIT OR Apache-2.0 (standard Rust ecosystem convention), compatible with
  Servo/SpiderMonkey's MPL 2.0 (file-level weak copyleft, doesn't force Talaria's own code into any
  particular license).
<!-- GSD:project-end -->

<!-- GSD:stack-start source:codebase/STACK.md -->

## Technology Stack

## Languages

- Rust (edition 2021) — all shipped code: `crates/talaria-shell/`, `crates/talaria-protocol/`, `crates/talaria-mcp/` (~3,159 lines across `crates/*/src/*.rs`)
- Python 3 (3.14.6 on this machine) — end-to-end test harness only, `tests/e2e/*.py`
- JavaScript — not authored in-repo, but injected into pages at runtime via the `evaluate` MCP tool (`crates/talaria-mcp/src/tools.rs`)

## Runtime

- Native desktop binary. Two executables:
- Rust toolchain: 1.97.1 installed; README requires ≥ 1.88. No `rust-toolchain.toml` is committed — the toolchain is unpinned.
- Async runtime: `tokio` (feature-gated per crate; shell uses `rt`, MCP proxy uses `rt-multi-thread`)
- Cargo, workspace with `resolver = "2"` (`Cargo.toml`)
- Lockfile: `Cargo.lock` present and committed (9,527 lines) — deliberately load-bearing, see Configuration below

## Frameworks

- `servo` 0.4.0 (libservo) — the web engine, compiled in; content pane only. Instantiated via `ServoBuilder` in `crates/talaria-shell/src/app.rs`
- `servo-embedder-traits` `=0.4.0` (aliased `embedder_traits`) — exact-pinned to match the engine
- `winit` 0.30.13 — window + event loop (`crates/talaria-shell/src/main.rs`)
- `egui` / `egui-winit` / `egui_glow` 0.34.3 — browser chrome, drawn into Servo's GL context (`crates/talaria-shell/src/gui.rs`). Versions must match Servo's own workspace so the `glow` context type is shared.
- `glow` 0.17 — GL bindings shared between egui and Servo
- `egui-phosphor` 0.12 — chrome icon font
- `rust-mcp-sdk` 1.0.1 (`server`, `macros`, `stdio` features, `default-features = false`) — MCP server implementation in `crates/talaria-mcp/`
- No Rust test framework in use — no `#[cfg(test)]` modules, no `tests/` Rust integration targets
- Python stdlib (`subprocess`, `socket`, sockets + Xvfb) drives all regression tests: `tests/e2e/run_all.py`, `tests/e2e/harness.py`
- Cargo only. No justfile, Makefile, or CI configuration (`.github/` does not exist)
- Release profile uses `lto = "thin"`; `[profile.dev.package."*"] opt-level = 2` per Servo's recommendation (otherwise debug-build rendering is unusably slow)

## Key Dependencies

- `servo` 0.4.0 — the entire rendering/JS capability; every architectural constraint flows from it
- `rustls` 0.23 — TLS for the engine. `rustls::crypto::aws_lc_rs::default_provider().install_default()` must run before anything else in `crates/talaria-shell/src/main.rs`
- `url` 2.5 — URL parsing and the address-bar/search heuristic (`crates/talaria-shell/src/app.rs`)
- `talaria-protocol` (path dep) — the newline-delimited-JSON control-socket schema shared by shell and MCP proxy (`crates/talaria-protocol/src/lib.rs`)
- `chacha20poly1305` 0.10 — at-rest encryption for the credential vault (`crates/talaria-shell/src/vault.rs`)
- `keyring` 3 (`sync-secret-service`, `apple-native`, `windows-native`) — OS keychain storage of the vault key
- `rand` 0.8, `hex` 0.4 — key generation and key encoding for the vault
- `dirs` 5 — config dir (engine profile, vault) and downloads dir resolution
- `ureq` 2 — blocking HTTP client used solely by the `download` command (`crates/talaria-shell/src/app.rs:1253`)
- `png` 0.17, `base64` 0.22 — screenshot encoding for the MCP `screenshot` tool
- `serde` / `serde_json` 1 — control-socket wire format and MCP tool schemas
- `webrender_api` 0.69, `euclid` 0.22 — geometry/compositor types shared with Servo
- `tracing` 0.1 and `log` 0.4 — logging. No logger is installed by the app; `servo.setup_logging()` installs the global one (honors `RUST_LOG`)
- `async-trait` 0.1 — `ServerHandler` impl in the MCP crate

## Configuration

- No `.env` files. Configuration is environment variables plus CLI argv, plus one JSON config
  file introduced by Phase 3: `config.json` under the `dirs` config dir, holding the address
  bar's search engine (`crates/talaria-shell/src/settings.rs`). It is read at startup and
  written by the Settings panel; a malformed file degrades to the default engine.
- `TALARIA_HISTORY_MAX_ENTRIES` — browsing-history retention cap, default 5000
  (`crates/talaria-shell/src/history.rs`)
- `XDG_RUNTIME_DIR` — determines the control-socket path; falls back to `/tmp/talaria-$UID.sock` (`crates/talaria-protocol/src/lib.rs:15`)
- `TALARIA_COMMAND_TIMEOUT_SECS` — control-socket command timeout (`crates/talaria-shell/src/control.rs:185`, `crates/talaria-shell/src/app.rs:1075`)
- `TALARIA_TEST_HOOKS=1` — enables test-only behavior in the shell (`crates/talaria-shell/src/app.rs:972`)
- `TALARIA_E2E_DISPLAY` — X display the e2e harness creates with Xvfb, default `:99` (`tests/e2e/harness.py`)
- `RUST_LOG` — log filtering, via Servo's logger
- First CLI argument is the startup URL; default `https://servo.org` (`crates/talaria-shell/src/main.rs:17`)
- `Cargo.toml` (workspace root) — all dependency versions are declared once in `[workspace.dependencies]`; member crates use `{ workspace = true }`
- `Cargo.lock` — **must not be blindly updated**. Fresh resolution against servo 0.4.0 pulls `primeorder 0.14.0` (final), which breaks `p256/p384/p521 0.14.0-rc.14`. The lock pins `primeorder 0.14.0-rc.14` (`Cargo.lock:5241`); recovery is `cargo update -p primeorder --precise 0.14.0-rc.14`. Documented in `SPEC.md:98`.
- `.gitignore` excludes `target/`

## Platform Requirements

- Rust ≥ 1.88 plus Servo's Linux/macOS build prerequisites: `python3`, `pkg-config`, `cmake`, `clang`, fontconfig/freetype dev headers
- A GL-capable display; headless runs use Xvfb
- Python 3 for the e2e suite
- `getuid` is called via a raw `extern "C"` shim (`crates/talaria-protocol/src/lib.rs:24`) and the socket path is a Unix domain socket — the control plane is Unix-only in practice, even though `keyring` carries a Windows feature
- Locally built and locally run desktop binaries; no packaging, installer, signing, or release pipeline exists in the repo
- Build: `cargo build --release`; run: `cargo run --release -p talaria-shell -- https://example.com`

<!-- GSD:stack-end -->

<!-- GSD:conventions-start source:CONVENTIONS.md -->

## Conventions

- **Rust** — the workspace crates under `crates/` (`talaria-shell`, `talaria-protocol`, `talaria-mcp`).
- **Python 3** — the e2e harness under `tests/e2e/`, deliberately stdlib-only (no pytest, no third-party deps).

## Naming Patterns

- Rust modules are single lowercase words, one concern each: `crates/talaria-shell/src/app.rs`, `control.rs`, `gui.rs`, `tabs.rs`, `vault.rs`, `keyutils.rs`. No `mod.rs` directories — modules are declared flat in `crates/talaria-shell/src/main.rs`.
- Crates are `talaria-<role>`; the shell crate builds a binary named `talaria` (see `[[bin]]` in `crates/talaria-shell/Cargo.toml`), the MCP crate builds `talaria-mcp`.
- Python e2e suites are `<feature>_test.py` (`tests/e2e/control_socket_test.py`, `tests/e2e/popup_test.py`). Runners are `run_*.py` (`tests/e2e/run_all.py`, `tests/e2e/run_control_socket_e2e.py`). Shared code is `tests/e2e/harness.py`.
- Rust: `snake_case`, verb-first, short. Public API on `TabManager` reads as terse accessors: `open`, `register`, `close`, `set_active`, `displayed`, `displayed_mut`, `find_by_webview`, `me_tabs`, `agent_tabs` (`crates/talaria-shell/src/tabs.rs`).
- The `_mut` suffix pairs with the shared-borrow variant (`get`/`get_mut`, `displayed`/`displayed_mut`). Follow that pairing when adding accessors.
- Private helpers get a leading underscore in Python (`_pid_alive`, `_clear_stale_x_lock` in `tests/e2e/harness.py`); Rust uses plain non-`pub` fns.
- Spell words out. The codebase uses `error`, not `e`; `message`, not `msg`; `attempt`, not `i`. See the match arms in `crates/talaria-shell/src/control.rs` and `crates/talaria-shell/src/vault.rs`.
- Exception: the Python suites are intentionally terse scratch scripts (`r`, `f`, `s`, `rid`, `tid`) — match that when adding to `tests/e2e/*_test.py`, and the verbose style when adding Rust.
- Module-level Python constants are SCREAMING_CASE: `SOCK`, `DISPLAY`, `BINARY`, `REPO`, `OUT`.
- Rust: `PascalCase`. Protocol enums are noun-shaped (`ClientMessage`, `ServerMessage`, `Command`, `Outcome`, `ResultPayload`, `Event` in `crates/talaria-protocol/src/lib.rs`).
- MCP tool structs are `<Command>Tool` (`TabsListTool`, `EvaluateTool`, `ScreenshotTool` in `crates/talaria-mcp/src/tools.rs`).

## Code Style

- No `rustfmt.toml` or `clippy.toml` in the repo; default `cargo fmt` / `cargo clippy` behaviour is the baseline.
- Hand-written style follows Servo's conventions, most visibly the **trailing comma after a block match arm**:
- Lines wrap near 100 columns. Inline format-string captures (`{error}`, `{tab_id}`, `{timeout_secs}`) are used everywhere instead of positional args.
- No CI config and no `.github/` directory. Verification is `cargo build --release` plus `python3 tests/e2e/run_all.py`.
- Python has no linter config; the suites use 2-space-free stdlib style with occasional semicolon-joined one-liners in setup helpers (`tests/e2e/crash_event_test.py`).

## Import Organization

## Error Handling

- **No `anyhow`/`thiserror`.** `main` returns `Result<(), Box<dyn Error>>` (`crates/talaria-shell/src/main.rs`); everything else uses `std::io::Result`, `Option`, or the protocol's own `Outcome::Error { message }`.
- **Agent-facing failures are values, not panics.** Every control-socket command resolves to `Outcome::Ok { result }` or `Outcome::Error { message }`; the message is human-readable and includes context an agent can act on, e.g. `format!("timed out after {timeout_secs}s (script still running?)")` (`crates/talaria-shell/src/control.rs`).
- **`expect()` only for genuine invariants** — installing the crypto provider, spawning the control thread, serialising a type that cannot fail (`serde_json::to_vec(message).expect("serializable")`). `crates/talaria-shell/src/app.rs` contains zero `unwrap()` calls. Do not introduce `unwrap()` in shell code paths.
- **Degrade, never abort, on user-data problems.** `crates/talaria-shell/src/vault.rs` logs a warning and falls back (keychain → key file → empty vault) for every failure mode rather than erroring out.
- **Match on `io::ErrorKind`** to distinguish expected absence from real failure, as in `forward_to_running_instance` where `NotFound`/`ConnectionRefused` mean "we're the first instance" (`crates/talaria-shell/src/control.rs`).
- Ignored results are explicit: `let _ = proxy.send_event(...)`, `let _ = std::fs::remove_file(&path)`.

## Logging

- `log::error!` — an unrecoverable subsystem failure (`control socket bind failed`, `tab crashed`).
- `log::warn!` — a degraded fallback the user should know about (all of `crates/talaria-shell/src/vault.rs`).
- `log::info!` — notable lifecycle events (`control socket listening at …`, `popup from tab N opened as tab M`).
- `log::debug!` — routine churn (`control connection ended`).
- `eprintln!` — reserved for pre-event-loop, user-facing CLI messages in `main` only.

## Comments

- `//!` module headers on every Rust file, stating the module's role in one or two sentences (`crates/talaria-protocol/src/lib.rs`, `crates/talaria-shell/src/control.rs`, `crates/talaria-mcp/src/tools.rs`).
- `///` on public items and on struct fields whose meaning isn't obvious from the name (`TabInfo::loading`, `Shared::toolbar_height`).
- Design decisions reference `SPEC.md` by name in-line ("per SPEC", "per SPEC's crash-recovery decision").
- Python modules all open with a `"""docstring"""` stating what the suite pins down and what preconditions it assumes (running shell, `TALARIA_TEST_HOOKS=1`, etc.).

## Function Design

- Lookups return `Option<&T>` / `Option<&mut T>`.
- Mutations that may be no-ops return `bool` (`TabManager::close`).
- Iteration returns `impl Iterator<Item = &Tab>` rather than allocating a `Vec`.
- The GUI never mutates shared state directly: `Gui::…` returns a `Vec<UiAction>` for the caller to apply once egui's borrows are released (`crates/talaria-shell/src/gui.rs`).

## Module Design

<!-- GSD:conventions-end -->

<!-- GSD:architecture-start source:ARCHITECTURE.md -->

## Architecture

## System Overview

```text

```

## Component Responsibilities

| Component | Responsibility | File |
|-----------|----------------|------|
| `main` (shell) | Arg → URL, crypto provider, single-instance forward, build event loop, spawn control thread | `crates/talaria-shell/src/main.rs` |
| `App` / `Shared` | winit `ApplicationHandler`; owns Servo, window, tabs, sessions, vault, the four Phase 3 stores, an `event_proxy` for off-thread wake-ups, and the pending queues; implements `servo::WebViewDelegate` | `crates/talaria-shell/src/app.rs` |
| `Gui` | egui chrome (toolbar, tab strip, Me/Agents toggle, crash page); records the blit of the displayed tab's offscreen buffer; returns `Vec<UiAction>` | `crates/talaria-shell/src/gui.rs` |
| `TabManager` / `TabOwner` | Tab table, per-view active tab, show/hide invariant, cycling | `crates/talaria-shell/src/tabs.rs` |
| `control` | Unix socket listener, Hello handshake, session ids, request timeout, single-instance forwarding | `crates/talaria-shell/src/control.rs` |
| `Vault` | Encrypted credential store backing `cookies_read` | `crates/talaria-shell/src/vault.rs` |
| `History` | Append-only browsing history for Me tabs; capped, pruned at load | `crates/talaria-shell/src/history.rs` |
| `Bookmarks` | Flat bookmark list; atomic whole-file save via `.tmp` sibling + rename | `crates/talaria-shell/src/bookmarks.rs` |
| `Settings` | `config.json`; the address bar's configurable `SearchEngine` | `crates/talaria-shell/src/settings.rs` |
| `Downloads` | Completed-download list; records the path actually written | `crates/talaria-shell/src/downloads.rs` |
| `keyutils` | winit `KeyEvent` → servo keyboard event translation | `crates/talaria-shell/src/keyutils.rs` |
| `talaria-protocol` | Wire enums (`ClientMessage`, `Command`, `ServerMessage`, `Outcome`, `Event`), `socket_path()` | `crates/talaria-protocol/src/lib.rs` |
| `TalariaTools` | MCP tool structs + JSON schemas + `dispatch` to `Command` | `crates/talaria-mcp/src/tools.rs` |
| `ShellConnection` | One lazily-opened, auto-reconnecting socket wire; id-matched round trips | `crates/talaria-mcp/src/socket.rs` |

## Pattern Overview

- All engine work happens on the winit main thread; every async or off-thread actor reaches it only by `EventLoopProxy::send_event(AppEvent)`.
- Request/reply correlation is by `id`; replies may be out of order, and events are unsolicited on the same stream.
- Deferred work (screenshot of a background tab, load waits, promise polling) lives in `RefCell<Vec<Pending…>>` queues on `Shared`, serviced from the loop — never from inside a Servo callback.
- Tabs carry an owner (`Me` or `Agent { session_id, client }`) that decides which of two views they appear in; the human can display and drive any agent tab (takeover).

## Layers

- Purpose: the only shared vocabulary between shell and clients; also the intended distributed-mode wire.
- Location: `crates/talaria-protocol/src/lib.rs`
- Contains: serde enums with `#[serde(tag = …, flatten)]` envelopes, `TabInfo`, `CredentialEntry`, `Cookie`, `Event`, `socket_path()`.
- Depends on: `serde`, `serde_json` only (deliberately dependency-light).
- Used by: `talaria-shell`, `talaria-mcp`.
- Purpose: stateless translation of MCP tool calls into `Command`s; no browser state of its own.
- Location: `crates/talaria-mcp/src/`
- Depends on: `rust-mcp-sdk`, `talaria-protocol`, tokio.
- Used by: any stdio MCP host.
- Purpose: window, chrome, tab ownership, command execution against real webviews.
- Location: `crates/talaria-shell/src/`
- Depends on: `servo`, `winit`, `egui*`, `talaria-protocol`.

## Data Flow

### Agent command path (e.g. `evaluate`)

### Evaluate specifics

### Unsolicited event path

### Me / Agents ownership and takeover

- All shared mutable state lives in one `Rc<Shared>` with `Cell`/`RefCell` fields; `Gui` lives in a `thread_local!` `RefCell<Option<Gui>>` (`crates/talaria-shell/src/app.rs:423`).
- Session table is `RefCell<BTreeMap<u64, Session>>`, keyed by the socket-assigned session id.

## Key Abstractions

- Purpose: the single inbound channel into the event loop (`Wake`, `SessionStarted`, `SessionEnded`, `Agent`).
- Location: `crates/talaria-shell/src/app.rs:40`
- Purpose: turn asynchronous engine outcomes into socket replies without blocking or re-entering Servo.
- Location: `crates/talaria-shell/src/app.rs:101`–`143`
- Pattern: delegate callback sets a flag → `process_pending_*` on the loop builds and sends the `Outcome`; `next_capture_deadline()` drives `ControlFlow::WaitUntil`.
- Purpose: egui closures cannot mutate borrowed state, so the chrome returns intents that `apply_ui_actions` performs after borrows drop.
- Location: `crates/talaria-shell/src/gui.rs:24`, applied at `crates/talaria-shell/src/app.rs:775`
- Purpose: `ClientMessage`/`ServerMessage` are internally tagged by `type`; `Command` adds a flattened `command` tag, `Outcome` a flattened `outcome` tag, `Event` an `event` tag; `ResultPayload` is `untagged`.
- Location: `crates/talaria-protocol/src/lib.rs:29`–`139`

## Entry Points

- Location: `crates/talaria-shell/src/main.rs`
- Triggers: user launch, optional URL argument.
- Responsibilities: install rustls provider, resolve URL, forward to a running instance via `control::forward_to_running_instance` (single instance), build `EventLoop<AppEvent>`, spawn the control thread, run the app.
- Location: `crates/talaria-mcp/src/main.rs`
- Triggers: spawned by an MCP host over stdio.
- Responsibilities: advertise tools and instructions, translate calls, proxy to the socket.
- Location: `crates/talaria-shell/src/control.rs:90` (`serve`); path from `talaria_protocol::socket_path()` — `$XDG_RUNTIME_DIR/talaria.sock`, else `/tmp/talaria-$UID.sock`.
- Triggers: any client connection; first line must be `Hello`.
- Location: `impl servo::WebViewDelegate for Shared`, `crates/talaria-shell/src/app.rs:1271`
- Triggers: navigation requests, popups, frame/load/title/URL changes, crashes, closes.

## Architectural Constraints

- **Threading:** Servo, winit, egui and all `WebView` calls are main-thread only. Exactly one extra thread (`talaria-control`) runs a `current_thread` tokio runtime; `Download` spawns a short-lived `std::thread`. Cross-thread transfer is `EventLoopProxy` in, `oneshot`/`mpsc` out.
- **Re-entrancy:** Servo invokes delegate callbacks while holding internal borrows (painter, evaluator). Callbacks may only set flags and `request_redraw()`; painting, visibility changes and follow-up `evaluate_javascript` calls must be deferred to a pending queue.
- **Borrow discipline:** `Shared`'s `RefCell`s are borrowed from both callbacks and the loop, so callbacks use `try_borrow`/`try_borrow_mut` and skip work rather than panic (`notify_new_frame_ready`, `request_create_new`).
- **Global state:** `static NEXT_SESSION: AtomicU64` (`control.rs:26`) and the `GUI` thread-local (`app.rs:423`).
- **Rendering invariant:** every tab owns an `OffscreenRenderingContext`, so capturing a background tab cannot disturb the displayed one; `paint()` without a fresh frame is a no-op, which is why `sync_visibility()` must run on every tab-set change.
- **Single instance:** the first process owns both the socket path and the Servo profile dir; a second launch forwards its URL via `Command::OpenForUser` and exits.
- **Timeouts:** socket command timeout (30s default) > `promise_wait()` (timeout − 2s) > `LOAD_WAIT` (20s) > background-capture deadline (1.5s). Keep this ordering when adding waits so the agent gets a specific message, not a generic timeout.

## Anti-Patterns

### Doing work inside a Servo delegate callback

### Mutating shell state from inside an egui closure

### Replying to an agent before the page exists

### Blocking the event loop on network or engine work

### Switching the human's view because an agent acted

## Error Handling

- Unknown tab: `format!("no tab {tab_id}")`.
- Crashed tab rejects `evaluate`/`screenshot` with "navigate it to recover"; `navigate` clears `crashed` and recovers.
- Engine states `DocumentNotFound`/`WebViewNotReady`/`InternalError` are rewritten by `describe_js_error` into a retry hint (`crates/talaria-shell/src/app.rs:1173`).
- Startup-only failures use `expect` (window, GL context, crypto provider); everything post-startup degrades.
- The MCP side surfaces connection failure as "is the Talaria browser running?" and retries once on a stale wire (`crates/talaria-mcp/src/socket.rs:27`).

## Cross-Cutting Concerns

<!-- GSD:architecture-end -->

<!-- GSD:skills-start source:skills/ -->

## Project Skills

No project skills found. Add skills to any of: `.claude/skills/`, `.agents/skills/`, `.cursor/skills/`, `.github/skills/`, or `.codex/skills/` with a `SKILL.md` index file.
<!-- GSD:skills-end -->

<!-- GSD:workflow-start source:GSD defaults -->

## GSD Workflow Enforcement

Before using Edit, Write, or other file-changing tools, start work through a GSD command so planning artifacts and execution context stay in sync.

Use these entry points:

- `/gsd-quick` for small fixes, doc updates, and ad-hoc tasks
- `/gsd-debug` for investigation and bug fixing
- `/gsd-execute-phase` for planned phase work

Do not make direct repo edits outside a GSD workflow unless the user explicitly asks to bypass it.
<!-- GSD:workflow-end -->

<!-- GSD:profile-start -->

## Developer Profile

> Profile not yet configured. Run `/gsd-profile-user` to generate your developer profile.
> This section is managed by `generate-claude-profile` -- do not edit manually.
<!-- GSD:profile-end -->

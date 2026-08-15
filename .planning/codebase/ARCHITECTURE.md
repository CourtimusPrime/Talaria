<!-- refreshed: 2026-08-15 -->
# Architecture

**Analysis Date:** 2026-08-15

## System Overview

```text
┌─────────────────────────────────────────────────────────────┐
│  MCP client (Claude Desktop, any stdio MCP host)             │
└──────────────────────────┬──────────────────────────────────┘
                           │ JSON-RPC over stdio
                           ▼
┌─────────────────────────────────────────────────────────────┐
│  talaria-mcp (proxy process)                                 │
│  `crates/talaria-mcp/src/main.rs`  ServerHandler             │
│  `crates/talaria-mcp/src/tools.rs` tool schemas + dispatch   │
│  `crates/talaria-mcp/src/socket.rs` lazy shell connection    │
└──────────────────────────┬──────────────────────────────────┘
                           │ newline-delimited JSON over
                           │ Unix domain socket
                           │ (`talaria-protocol`)
                           ▼
┌─────────────────────────────────────────────────────────────┐
│  talaria-shell — control thread (tokio current_thread)       │
│  `crates/talaria-shell/src/control.rs`                       │
│  accept → Hello handshake → AgentRequest + oneshot reply     │
└──────────────────────────┬──────────────────────────────────┘
                           │ EventLoopProxy::send_event(AppEvent)
                           ▼
┌─────────────────────────────────────────────────────────────┐
│  talaria-shell — winit main thread (the only Servo thread)   │
├──────────────────┬──────────────────┬───────────────────────┤
│  App / Shared    │  Gui (egui-glow) │  TabManager           │
│  `src/app.rs`    │  `src/gui.rs`    │  `src/tabs.rs`        │
│  command exec,   │  chrome, Me /    │  Me vs Agents tab     │
│  pending queues, │  Agents strip,   │  ownership, active    │
│  WebViewDelegate │  blit callback   │  tab, visibility      │
└────────┬─────────┴────────┬─────────┴──────────┬────────────┘
         │                  │                    │
         ▼                  ▼                    ▼
┌─────────────────────────────────────────────────────────────┐
│  libservo (`servo` 0.4.0): one WebView per tab, each with    │
│  its own OffscreenRenderingContext, blitted into the shared  │
│  WindowRenderingContext GL context egui also draws into.     │
│  Profile on disk: `$XDG_CONFIG_HOME/talaria/servo`           │
└─────────────────────────────────────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────────────────────────────┐
│  Vault (ChaCha20-Poly1305 + OS keychain)                     │
│  `crates/talaria-shell/src/vault.rs`                         │
└─────────────────────────────────────────────────────────────┘
```

## Component Responsibilities

| Component | Responsibility | File |
|-----------|----------------|------|
| `main` (shell) | Arg → URL, crypto provider, single-instance forward, build event loop, spawn control thread | `crates/talaria-shell/src/main.rs` |
| `App` / `Shared` | winit `ApplicationHandler`; owns Servo, window, tabs, sessions, vault, pending queues; implements `servo::WebViewDelegate` | `crates/talaria-shell/src/app.rs` |
| `Gui` | egui chrome (toolbar, tab strip, Me/Agents toggle, crash page); records the blit of the displayed tab's offscreen buffer; returns `Vec<UiAction>` | `crates/talaria-shell/src/gui.rs` |
| `TabManager` / `TabOwner` | Tab table, per-view active tab, show/hide invariant, cycling | `crates/talaria-shell/src/tabs.rs` |
| `control` | Unix socket listener, Hello handshake, session ids, request timeout, single-instance forwarding | `crates/talaria-shell/src/control.rs` |
| `Vault` | Encrypted credential store backing `cookies_read` | `crates/talaria-shell/src/vault.rs` |
| `keyutils` | winit `KeyEvent` → servo keyboard event translation | `crates/talaria-shell/src/keyutils.rs` |
| `talaria-protocol` | Wire enums (`ClientMessage`, `Command`, `ServerMessage`, `Outcome`, `Event`), `socket_path()` | `crates/talaria-protocol/src/lib.rs` |
| `TalariaTools` | MCP tool structs + JSON schemas + `dispatch` to `Command` | `crates/talaria-mcp/src/tools.rs` |
| `ShellConnection` | One lazily-opened, auto-reconnecting socket wire; id-matched round trips | `crates/talaria-mcp/src/socket.rs` |

## Pattern Overview

**Overall:** Single-threaded embedder event loop (winit + libservo) fronted by an async socket boundary; a separate stdio proxy process translates MCP into the socket protocol.

**Key Characteristics:**
- All engine work happens on the winit main thread; every async or off-thread actor reaches it only by `EventLoopProxy::send_event(AppEvent)`.
- Request/reply correlation is by `id`; replies may be out of order, and events are unsolicited on the same stream.
- Deferred work (screenshot of a background tab, load waits, promise polling) lives in `RefCell<Vec<Pending…>>` queues on `Shared`, serviced from the loop — never from inside a Servo callback.
- Tabs carry an owner (`Me` or `Agent { session_id, client }`) that decides which of two views they appear in; the human can display and drive any agent tab (takeover).

## Layers

**Protocol layer (`crates/talaria-protocol`):**
- Purpose: the only shared vocabulary between shell and clients; also the intended distributed-mode wire.
- Location: `crates/talaria-protocol/src/lib.rs`
- Contains: serde enums with `#[serde(tag = …, flatten)]` envelopes, `TabInfo`, `CredentialEntry`, `Cookie`, `Event`, `socket_path()`.
- Depends on: `serde`, `serde_json` only (deliberately dependency-light).
- Used by: `talaria-shell`, `talaria-mcp`.

**Agent-surface layer (`crates/talaria-mcp`):**
- Purpose: stateless translation of MCP tool calls into `Command`s; no browser state of its own.
- Location: `crates/talaria-mcp/src/`
- Depends on: `rust-mcp-sdk`, `talaria-protocol`, tokio.
- Used by: any stdio MCP host.

**Shell layer (`crates/talaria-shell`):**
- Purpose: window, chrome, tab ownership, command execution against real webviews.
- Location: `crates/talaria-shell/src/`
- Depends on: `servo`, `winit`, `egui*`, `talaria-protocol`.

## Data Flow

### Agent command path (e.g. `evaluate`)

1. MCP host calls a tool; `Handler::handle_call_tool_request` reads `clientInfo.name` as the session label (`crates/talaria-mcp/src/main.rs:43`).
2. `tools::dispatch` maps the tool struct to a `Command` (`crates/talaria-mcp/src/tools.rs`).
3. `ShellConnection::request` connects if needed (sending `ClientMessage::Hello { client }`), writes `ClientMessage::Request { id, command }`, then reads lines until a `Reply` with the matching `id` (`crates/talaria-mcp/src/socket.rs:27`).
4. Shell's `handle_connection` decodes the line, builds an `AgentRequest { session_id, client, command, reply: oneshot }` and sends `AppEvent::Agent` through the event-loop proxy, then awaits the oneshot under a timeout (`TALARIA_COMMAND_TIMEOUT_SECS`, default 30s) (`crates/talaria-shell/src/control.rs:166`).
5. `App::user_event` → `execute_agent_command` runs it on the main thread against the `WebView` (`crates/talaria-shell/src/app.rs:890`).
6. Fast commands answer the oneshot inline; slow ones park in `pending_loads` / `pending_captures` / `pending_evals` and are answered from `new_events` / `user_event` / `window_event` via `process_pending_*` (`crates/talaria-shell/src/app.rs:174`).
7. The outcome travels back over the oneshot; `control.rs` wraps it in `ServerMessage::Reply` and pushes it to the per-connection writer task's mpsc channel.

### Evaluate specifics

1. `wrap_script` wraps the agent's script for promise capture, top-level `await`, and CSP detection (`crates/talaria-shell/src/app.rs:1088`).
2. A returned thenable is parked in `window.__talaria_async[slot]`; a `PendingEval { step: Poll }` re-evaluates `poll_script(slot)` every 50 ms until settled or `promise_wait()` expires.
3. A CSP-blocked page yields `{__talaria_csp:1}` and a `PendingEval { step: RunRaw }` re-runs the script unwrapped.
4. `js_value_to_json` flattens `servo::JSValue` into plain JSON before it reaches the agent.

### Unsolicited event path

1. A Servo delegate callback (`notify_crashed`, `notify_closed`) or a UI action marks state.
2. `Shared::broadcast_event` pushes `ServerMessage::Event` into every connected session's unbounded mpsc sender (`crates/talaria-shell/src/app.rs:368`).
3. Each connection's writer task interleaves events with replies on the socket.

### Me / Agents ownership and takeover

1. Every tab has `TabOwner::Me` or `TabOwner::Agent { session_id, client }` (`crates/talaria-shell/src/tabs.rs:21`); `TabOwner::view()` maps that to `ViewMode::Me` / `ViewMode::Agents`.
2. `TabManager` keeps two independent active tabs (`active_me`, `active_agent`) plus a current `mode`; `displayed()` is the active tab of the current mode.
3. Opening an agent tab never switches the human's view and never steals the agent view from a tab the agent is already using (`TabManager::open`, `activate` computation).
4. Takeover is the absence of a gate: the user clicks **Agents**, `UiAction::SwitchMode` flips `tabs.mode`, `sync_visibility()` shows/focuses that tab, and ordinary keyboard/mouse forwarding in `window_event` drives the agent's webview directly. The tab's owner never changes, so the agent keeps operating on the same session afterwards.
5. `sync_visibility()` is the load-bearing invariant: exactly the displayed webview is `show()`+`focus()`, all others `hide()`+`blur()` — the hidden→shown transition is what makes Servo produce a fresh frame.
6. Agent tabs outlive their session; `gui.rs` groups agent tabs by live session and lists tabs of vanished sessions under "disconnected".

**State Management:**
- All shared mutable state lives in one `Rc<Shared>` with `Cell`/`RefCell` fields; `Gui` lives in a `thread_local!` `RefCell<Option<Gui>>` (`crates/talaria-shell/src/app.rs:423`).
- Session table is `RefCell<BTreeMap<u64, Session>>`, keyed by the socket-assigned session id.

## Key Abstractions

**`AppEvent`:**
- Purpose: the single inbound channel into the event loop (`Wake`, `SessionStarted`, `SessionEnded`, `Agent`).
- Location: `crates/talaria-shell/src/app.rs:40`

**Pending queues (`PendingLoad`, `PendingCapture`, `PendingEval`):**
- Purpose: turn asynchronous engine outcomes into socket replies without blocking or re-entering Servo.
- Location: `crates/talaria-shell/src/app.rs:101`–`143`
- Pattern: delegate callback sets a flag → `process_pending_*` on the loop builds and sends the `Outcome`; `next_capture_deadline()` drives `ControlFlow::WaitUntil`.

**`UiAction`:**
- Purpose: egui closures cannot mutate borrowed state, so the chrome returns intents that `apply_ui_actions` performs after borrows drop.
- Location: `crates/talaria-shell/src/gui.rs:24`, applied at `crates/talaria-shell/src/app.rs:775`

**Envelope types:**
- Purpose: `ClientMessage`/`ServerMessage` are internally tagged by `type`; `Command` adds a flattened `command` tag, `Outcome` a flattened `outcome` tag, `Event` an `event` tag; `ResultPayload` is `untagged`.
- Location: `crates/talaria-protocol/src/lib.rs:29`–`139`

## Entry Points

**`talaria` binary:**
- Location: `crates/talaria-shell/src/main.rs`
- Triggers: user launch, optional URL argument.
- Responsibilities: install rustls provider, resolve URL, forward to a running instance via `control::forward_to_running_instance` (single instance), build `EventLoop<AppEvent>`, spawn the control thread, run the app.

**`talaria-mcp` binary:**
- Location: `crates/talaria-mcp/src/main.rs`
- Triggers: spawned by an MCP host over stdio.
- Responsibilities: advertise tools and instructions, translate calls, proxy to the socket.

**Control socket:**
- Location: `crates/talaria-shell/src/control.rs:90` (`serve`); path from `talaria_protocol::socket_path()` — `$XDG_RUNTIME_DIR/talaria.sock`, else `/tmp/talaria-$UID.sock`.
- Triggers: any client connection; first line must be `Hello`.

**Servo delegate callbacks:**
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

**What happens:** Calling `paint()`, `show()/hide()`, or `evaluate_javascript` from `notify_new_frame_ready` / an evaluate callback.
**Why it's wrong:** Servo holds a mutable borrow of the painter/evaluator during the callback; the nested call panics or silently no-ops.
**Do this instead:** Set a flag on the matching pending entry and `request_redraw()`, then act in `process_pending_captures` / `process_pending_evals` (`crates/talaria-shell/src/app.rs:174`, `:196`).

### Mutating shell state from inside an egui closure

**What happens:** Calling `state.open_tab(...)` or `tabs.close(...)` inside `Gui::update`'s `ctx.run` closure while `shared.tabs` is borrowed.
**Why it's wrong:** `RefCell` double-borrow panic mid-frame.
**Do this instead:** Push a `UiAction` and let `apply_ui_actions` run it after the closure returns (`crates/talaria-shell/src/gui.rs:98`, `crates/talaria-shell/src/app.rs:775`).

### Replying to an agent before the page exists

**What happens:** Answering `tabs_open` / `navigate` as soon as `load()` is called.
**Why it's wrong:** The agent's next `evaluate` runs in the previous document or none at all, which Servo reports as an opaque `InternalError`.
**Do this instead:** `Shared::reply_after_load(tab_id, needs_start, reply)` and let `process_pending_loads` answer (`crates/talaria-shell/src/app.rs:326`).

### Blocking the event loop on network or engine work

**What happens:** Performing an HTTP fetch or a busy wait inside `execute_agent_command`.
**Why it's wrong:** The whole browser — chrome, rendering, other agents — stalls.
**Do this instead:** Off-thread with a oneshot reply (see `Command::Download`, `crates/talaria-shell/src/app.rs:1058`) or a deadline-based pending entry.

### Switching the human's view because an agent acted

**What happens:** Activating an agent-opened tab or flipping `tabs.mode` on agent commands.
**Why it's wrong:** It yanks the user out of the Me view mid-task and breaks the ownership model.
**Do this instead:** Only human-driven `UiAction::SwitchMode` (and the explicit `Command::OpenForUser`) may change `mode`; `TabManager::open` gates activation on `!owner.is_agent() || self.active_agent.is_none()`.

## Error Handling

**Strategy:** Every agent-facing failure becomes `Outcome::Error { message }` with an actionable, human-readable string; the shell never panics on agent input.

**Patterns:**
- Unknown tab: `format!("no tab {tab_id}")`.
- Crashed tab rejects `evaluate`/`screenshot` with "navigate it to recover"; `navigate` clears `crashed` and recovers.
- Engine states `DocumentNotFound`/`WebViewNotReady`/`InternalError` are rewritten by `describe_js_error` into a retry hint (`crates/talaria-shell/src/app.rs:1173`).
- Startup-only failures use `expect` (window, GL context, crypto provider); everything post-startup degrades.
- The MCP side surfaces connection failure as "is the Talaria browser running?" and retries once on a stale wire (`crates/talaria-mcp/src/socket.rs:27`).

## Cross-Cutting Concerns

**Logging:** `servo.setup_logging()` installs the global logger (honors `RUST_LOG`); the shell uses `log::` macros and must not install another logger (`crates/talaria-shell/src/main.rs:19`).
**Validation:** URLs via `parse_agent_url` (agent: no search fallback) vs `resolve_location` (human omnibox, DuckDuckGo fallback); `Download` rejects filenames containing `/` or `..`.
**Authentication:** None on the local stdio/socket path by design — the ability to spawn the proxy implies access. Credentials live encrypted in the vault and are only exposed through `cookies_read` by domain.
**Test hooks:** `TALARIA_TEST_HOOKS=1` enables the `__talaria_sim_crash__` evaluate hook used by `tests/e2e/crash_recovery_test.py`.

---

*Architecture analysis: 2026-08-15*

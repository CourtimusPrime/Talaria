# Codebase Structure

**Analysis Date:** 2026-08-15

## Directory Layout

```
talaria/
├── Cargo.toml                          # Workspace root: members, shared dep pins, profiles
├── Cargo.lock                          # Committed (workspace ships binaries)
├── README.md                           # Usage, MCP wiring, keyboard map
├── SPEC.md                             # Product + architecture decisions of record
├── OVERNIGHT_LOG.md                    # Latest autonomous test-and-fix run log
├── LICENSE-MIT, LICENSE-APACHE         # Dual license (MPL headers on servo-derived files)
├── crates/
│   ├── talaria-shell/                  # The browser binary (`talaria`)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs                 # Entry: URL parse, single-instance, event loop
│   │       ├── app.rs                  # Shared state, command execution, WebViewDelegate
│   │       ├── gui.rs                  # egui chrome + servo blit
│   │       ├── tabs.rs                 # TabManager, TabOwner, ViewMode
│   │       ├── control.rs              # Unix control socket server
│   │       ├── vault.rs                # Encrypted credential vault
│   │       └── keyutils.rs             # winit → servo key translation
│   ├── talaria-protocol/               # Shared wire types (no engine deps)
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   └── talaria-mcp/                    # Stdio MCP server binary (`talaria-mcp`)
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs                 # MCP ServerHandler + server_details
│           ├── tools.rs                # Tool structs, schemas, dispatch
│           └── socket.rs               # Lazy shell connection
├── tests/
│   └── e2e/                            # Python end-to-end suite (Xvfb-driven)
└── target/                             # Cargo build output (ignored)
```

## Directory Purposes

**`crates/talaria-shell/src/`:**
- Purpose: everything that runs on the winit main thread plus the socket server.
- Contains: one module per concern, flat (no submodule directories).
- Key files: `app.rs` (1425 lines — the hub), `tabs.rs`, `gui.rs`, `control.rs`.

**`crates/talaria-protocol/src/`:**
- Purpose: the only crate both other crates depend on; keep it serde-only.
- Contains: a single `lib.rs` with envelope enums, payload structs, `socket_path()`, and round-trip unit tests.

**`crates/talaria-mcp/src/`:**
- Purpose: MCP adapter; holds no browser state.
- Key files: `tools.rs` (tool schemas + `dispatch`), `socket.rs` (wire).

**`tests/e2e/`:**
- Purpose: black-box tests that launch the real binary under Xvfb and drive the control socket / MCP stdio.
- Contains: `harness.py` (shared launch helpers), `run_all.py` (suite runner), `*_test.py` cases, `README.md`, plus captured artifacts (`*.png`, `*.xwd`, `talaria.log`) and `overnight_lock.py` (one overnight loop per branch).

## Key File Locations

**Entry Points:**
- `crates/talaria-shell/src/main.rs`: `talaria` binary.
- `crates/talaria-mcp/src/main.rs`: `talaria-mcp` binary.
- `crates/talaria-shell/src/control.rs`: control-socket listener (`serve`) and single-instance forwarder.

**Configuration:**
- `Cargo.toml` (root): workspace member list, `[workspace.dependencies]` version pins (egui/glow versions must match Servo's own workspace), `[profile.dev.package."*"] opt-level = 2`.
- Runtime config dirs (created at startup, not in-repo): `$XDG_CONFIG_HOME/talaria/servo` (engine profile), `$XDG_CONFIG_HOME/talaria/vault*` (credentials).
- Env vars: `TALARIA_COMMAND_TIMEOUT_SECS`, `TALARIA_TEST_HOOKS`, `XDG_RUNTIME_DIR`, `RUST_LOG`.

**Core Logic:**
- `crates/talaria-shell/src/app.rs`: `Shared`, `execute_agent_command`, pending queues, `WebViewDelegate` impl, script wrapping.
- `crates/talaria-shell/src/tabs.rs`: ownership and visibility invariants.
- `crates/talaria-shell/src/gui.rs`: chrome and the offscreen→egui blit.
- `crates/talaria-protocol/src/lib.rs`: wire contract.
- `crates/talaria-mcp/src/tools.rs`: agent-facing tool surface and descriptions.

**Testing:**
- `crates/talaria-protocol/src/lib.rs` (`mod tests`): the only in-tree Rust unit tests.
- `tests/e2e/run_all.py`: runs the Python e2e suite.

## Naming Conventions

**Crates:**
- `talaria-<role>` (`talaria-shell`, `talaria-mcp`, `talaria-protocol`); binaries are `talaria` and `talaria-mcp`.

**Files:**
- Rust modules: lowercase single word, no prefixes (`app.rs`, `tabs.rs`, `vault.rs`); flat module tree declared in `main.rs`/`lib.rs`.
- E2E tests: `<feature>_test.py` (`popup_test.py`, `takeover_test.py`); runners are `run_*.py`; shared code is `harness.py`.
- Repo-root docs: UPPERCASE (`SPEC.md`, `README.md`, `OVERNIGHT_LOG.md`).

**Code:**
- Protocol variants are `PascalCase` in Rust, serialized `snake_case` via `rename_all`; MCP tool names are `snake_case` (`tabs_open`, `cookies_read`) and match their `Command` variant.
- Pending work types are `Pending<Thing>` with a `process_pending_<things>()` servicing method.
- UI intents are `UiAction::<Verb>`.

## Where to Add New Code

**A new agent capability (tool):**
1. Add a variant to `Command` (and a `ResultPayload` variant if the reply shape is new) in `crates/talaria-protocol/src/lib.rs`.
2. Handle it in `execute_agent_command` in `crates/talaria-shell/src/app.rs`; use a pending queue if it cannot answer synchronously.
3. Add the `#[mcp_tool]` struct, register it in the `tool_box!` list, and extend `dispatch` in `crates/talaria-mcp/src/tools.rs`.
4. Add an e2e case under `tests/e2e/` and list it in `run_all.py`; document the tool in `README.md`.

**New chrome / UI:**
- Draw in `Gui::update` (`crates/talaria-shell/src/gui.rs`), emit a new `UiAction`, and handle it in `apply_ui_actions` (`crates/talaria-shell/src/app.rs`). Never mutate `Shared` inside the egui closure.

**New keyboard shortcut:**
- `handle_browser_shortcut` in `crates/talaria-shell/src/app.rs`; key translation belongs in `crates/talaria-shell/src/keyutils.rs`.

**New tab-ownership or view behaviour:**
- `crates/talaria-shell/src/tabs.rs`; anything that changes the tab set must end in `sync_visibility()`.

**New unsolicited notification:**
- Add an `Event` variant in `crates/talaria-protocol/src/lib.rs` and call `Shared::broadcast_event`.

**New shared wire type:**
- `crates/talaria-protocol/src/lib.rs` only — do not add engine, tokio, or MCP dependencies to that crate.

**Tests:**
- Pure serialization/logic: `#[cfg(test)] mod tests` in the same Rust file.
- Anything requiring a live browser: `tests/e2e/<feature>_test.py` using `harness.py`.

## Special Directories

**`target/`:**
- Purpose: Cargo build artifacts (Servo builds are large).
- Generated: Yes. Committed: No.

**`tests/e2e/__pycache__/`:**
- Purpose: Python bytecode cache.
- Generated: Yes. Committed: No.

**`.planning/`:**
- Purpose: GSD planning artifacts, including this codebase map (`.planning/codebase/`).
- Generated: Yes (by GSD commands). Committed: project-dependent.

**`tests/e2e/` artifacts (`*.png`, `*.xwd`, `talaria.log`):**
- Purpose: captured evidence from the last e2e/soak run.
- Generated: Yes. Committed: currently present in the working tree.

---

*Structure analysis: 2026-08-15*

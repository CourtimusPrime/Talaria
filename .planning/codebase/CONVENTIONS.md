# Coding Conventions

**Analysis Date:** 2026-08-15

Two languages, two rule sets:
- **Rust** — the workspace crates under `crates/` (`talaria-shell`, `talaria-protocol`, `talaria-mcp`).
- **Python 3** — the e2e harness under `tests/e2e/`, deliberately stdlib-only (no pytest, no third-party deps).

## Naming Patterns

**Files:**
- Rust modules are single lowercase words, one concern each: `crates/talaria-shell/src/app.rs`, `control.rs`, `gui.rs`, `tabs.rs`, `vault.rs`, `keyutils.rs`. No `mod.rs` directories — modules are declared flat in `crates/talaria-shell/src/main.rs`.
- Crates are `talaria-<role>`; the shell crate builds a binary named `talaria` (see `[[bin]]` in `crates/talaria-shell/Cargo.toml`), the MCP crate builds `talaria-mcp`.
- Python e2e suites are `<feature>_test.py` (`tests/e2e/control_socket_test.py`, `tests/e2e/popup_test.py`). Runners are `run_*.py` (`tests/e2e/run_all.py`, `tests/e2e/run_control_socket_e2e.py`). Shared code is `tests/e2e/harness.py`.

**Functions:**
- Rust: `snake_case`, verb-first, short. Public API on `TabManager` reads as terse accessors: `open`, `register`, `close`, `set_active`, `displayed`, `displayed_mut`, `find_by_webview`, `me_tabs`, `agent_tabs` (`crates/talaria-shell/src/tabs.rs`).
- The `_mut` suffix pairs with the shared-borrow variant (`get`/`get_mut`, `displayed`/`displayed_mut`). Follow that pairing when adding accessors.
- Private helpers get a leading underscore in Python (`_pid_alive`, `_clear_stale_x_lock` in `tests/e2e/harness.py`); Rust uses plain non-`pub` fns.

**Variables:**
- Spell words out. The codebase uses `error`, not `e`; `message`, not `msg`; `attempt`, not `i`. See the match arms in `crates/talaria-shell/src/control.rs` and `crates/talaria-shell/src/vault.rs`.
- Exception: the Python suites are intentionally terse scratch scripts (`r`, `f`, `s`, `rid`, `tid`) — match that when adding to `tests/e2e/*_test.py`, and the verbose style when adding Rust.
- Module-level Python constants are SCREAMING_CASE: `SOCK`, `DISPLAY`, `BINARY`, `REPO`, `OUT`.

**Types:**
- Rust: `PascalCase`. Protocol enums are noun-shaped (`ClientMessage`, `ServerMessage`, `Command`, `Outcome`, `ResultPayload`, `Event` in `crates/talaria-protocol/src/lib.rs`).
- MCP tool structs are `<Command>Tool` (`TabsListTool`, `EvaluateTool`, `ScreenshotTool` in `crates/talaria-mcp/src/tools.rs`).

**Wire format:** all protocol enums carry `#[serde(rename_all = "snake_case")]` with an internal tag (`type`, `command`, `outcome`, `event`) and `#[serde(flatten)]` for the payload. Wire names are therefore `snake_case` (`tabs_open`, `hello_ack`, `tab_crashed`) and MCP tool names match the `Command` variant exactly.

## Code Style

**Formatting:**
- No `rustfmt.toml` or `clippy.toml` in the repo; default `cargo fmt` / `cargo clippy` behaviour is the baseline.
- Hand-written style follows Servo's conventions, most visibly the **trailing comma after a block match arm**:
  ```rust
  Err(error) => {
      log::error!("control socket bind failed at {}: {error}", path.display());
      return;
  },
  ```
  (`crates/talaria-shell/src/control.rs`). Preserve this; plain `rustfmt` will not add it.
- Lines wrap near 100 columns. Inline format-string captures (`{error}`, `{tab_id}`, `{timeout_secs}`) are used everywhere instead of positional args.

**Linting:**
- No CI config and no `.github/` directory. Verification is `cargo build --release` plus `python3 tests/e2e/run_all.py`.
- Python has no linter config; the suites use 2-space-free stdlib style with occasional semicolon-joined one-liners in setup helpers (`tests/e2e/crash_event_test.py`).

## Import Organization

**Rust order** (blank line between groups), as in `crates/talaria-shell/src/app.rs`:
1. `std::*`
2. third-party crates (`base64`, `servo`, `url`, `webrender_api`, `winit`)
3. workspace crates (`talaria_protocol::{...}`)
4. local modules (`crate::control::AgentRequest`, `crate::gui::Gui`, …)

**Python order:** stdlib imports alphabetised at the top; `harness` imported only after `sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))`. Heavy/optional imports (`http.server`, `threading`, `base64`) are imported inline at the point of use in some suites (`tests/e2e/control_socket_test.py`).

**Path aliases:** none. Rust dependencies are declared once in the root `[workspace.dependencies]` (`Cargo.toml`) and pulled into crates with `{ workspace = true }`. Add new deps at the workspace root, never directly in a crate manifest.

## Error Handling

- **No `anyhow`/`thiserror`.** `main` returns `Result<(), Box<dyn Error>>` (`crates/talaria-shell/src/main.rs`); everything else uses `std::io::Result`, `Option`, or the protocol's own `Outcome::Error { message }`.
- **Agent-facing failures are values, not panics.** Every control-socket command resolves to `Outcome::Ok { result }` or `Outcome::Error { message }`; the message is human-readable and includes context an agent can act on, e.g. `format!("timed out after {timeout_secs}s (script still running?)")` (`crates/talaria-shell/src/control.rs`).
- **`expect()` only for genuine invariants** — installing the crypto provider, spawning the control thread, serialising a type that cannot fail (`serde_json::to_vec(message).expect("serializable")`). `crates/talaria-shell/src/app.rs` contains zero `unwrap()` calls. Do not introduce `unwrap()` in shell code paths.
- **Degrade, never abort, on user-data problems.** `crates/talaria-shell/src/vault.rs` logs a warning and falls back (keychain → key file → empty vault) for every failure mode rather than erroring out.
- **Match on `io::ErrorKind`** to distinguish expected absence from real failure, as in `forward_to_running_instance` where `NotFound`/`ConnectionRefused` mean "we're the first instance" (`crates/talaria-shell/src/control.rs`).
- Ignored results are explicit: `let _ = proxy.send_event(...)`, `let _ = std::fs::remove_file(&path)`.

## Logging

**Framework:** the `log` crate façade. The shell installs **no** logger of its own — Servo's `setup_logging()` owns the global logger and honours `RUST_LOG`, covering `log::` macros too (documented in the comment at the top of `fn main`, `crates/talaria-shell/src/main.rs`).

**Level discipline:**
- `log::error!` — an unrecoverable subsystem failure (`control socket bind failed`, `tab crashed`).
- `log::warn!` — a degraded fallback the user should know about (all of `crates/talaria-shell/src/vault.rs`).
- `log::info!` — notable lifecycle events (`control socket listening at …`, `popup from tab N opened as tab M`).
- `log::debug!` — routine churn (`control connection ended`).
- `eprintln!` — reserved for pre-event-loop, user-facing CLI messages in `main` only.

Messages are lowercase, no trailing period, with the cause appended in parentheses or after a colon.

## Comments

**When to comment:** explain *why*, and specifically the non-obvious constraint. The house style is a short prose paragraph above the tricky code, e.g. the `pending_captures` field comment in `crates/talaria-shell/src/app.rs` explaining that captures must be serviced from the event loop and never inside a servo callback (painter borrow), or `_clear_stale_x_lock` in `tests/e2e/harness.py` explaining the leftover `/tmp/.X<n>-lock`.

**Doc comments:**
- `//!` module headers on every Rust file, stating the module's role in one or two sentences (`crates/talaria-protocol/src/lib.rs`, `crates/talaria-shell/src/control.rs`, `crates/talaria-mcp/src/tools.rs`).
- `///` on public items and on struct fields whose meaning isn't obvious from the name (`TabInfo::loading`, `Shared::toolbar_height`).
- Design decisions reference `SPEC.md` by name in-line ("per SPEC", "per SPEC's crash-recovery decision").
- Python modules all open with a `"""docstring"""` stating what the suite pins down and what preconditions it assumes (running shell, `TALARIA_TEST_HOOKS=1`, etc.).

**Licensing:** `crates/talaria-shell/src/app.rs` carries an MPL-2.0 file header because it derives from Servo's `servoshell`. Original Talaria code is `MIT OR Apache-2.0`. Any new file copied from Servo must carry the MPL header.

## Function Design

**Size:** small and single-purpose except for the winit/servo event-loop plumbing in `crates/talaria-shell/src/app.rs` (1425 lines), which is large by necessity — new logic belongs in `tabs.rs`, `gui.rs`, `vault.rs`, or `control.rs`, not appended there.

**Parameters:** plain positional args; no builder/options structs. Async socket helpers take `&mut (impl AsyncWriteExt + Unpin)` rather than a concrete type (`write_line`, `crates/talaria-shell/src/control.rs`).

**Return values:**
- Lookups return `Option<&T>` / `Option<&mut T>`.
- Mutations that may be no-ops return `bool` (`TabManager::close`).
- Iteration returns `impl Iterator<Item = &Tab>` rather than allocating a `Vec`.
- The GUI never mutates shared state directly: `Gui::…` returns a `Vec<UiAction>` for the caller to apply once egui's borrows are released (`crates/talaria-shell/src/gui.rs`).

**Configuration via env vars,** read at the point of use with a documented default:
```rust
std::env::var("TALARIA_COMMAND_TIMEOUT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(30)
```
Known knobs: `TALARIA_COMMAND_TIMEOUT_SECS`, `TALARIA_TEST_HOOKS`, `XDG_RUNTIME_DIR`, `RUST_LOG`, plus the e2e-only `TALARIA_E2E_DISPLAY` and `TALARIA_E2E_OUT`.

## Module Design

**Exports:** `talaria-protocol` is the only crate with a real public API — it re-exports everything from `crates/talaria-protocol/src/lib.rs` flat (no submodules, no prelude). Shell and MCP modules are `pub` only within their binary crate.

**Barrel files:** not used. `crates/talaria-shell/src/main.rs` declares the module list and nothing else re-exports.

**Shared types live in `talaria-protocol`.** Anything crossing the shell↔MCP boundary (commands, results, tab info, socket path) is defined once there; neither side redefines wire shapes. Adding a capability means: new `Command` variant + `ResultPayload` case in `crates/talaria-protocol/src/lib.rs`, handling in `crates/talaria-shell/src/app.rs`, a `<Name>Tool` in `crates/talaria-mcp/src/tools.rs`, and a case in the `tool_box!` registration.

**Threading:** the winit event loop owns all engine state on the main thread; the control socket runs on a dedicated named thread (`talaria-control`) with a `current_thread` Tokio runtime, and communicates only via `EventLoopProxy<AppEvent>` + `oneshot`/`mpsc` channels (`crates/talaria-shell/src/control.rs`). Never touch `Servo`/`WebView` off the main thread.

---

*Convention analysis: 2026-08-15*

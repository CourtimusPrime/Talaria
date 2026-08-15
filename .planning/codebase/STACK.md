# Technology Stack

**Analysis Date:** 2026-08-15

## Languages

**Primary:**
- Rust (edition 2021) — all shipped code: `crates/talaria-shell/`, `crates/talaria-protocol/`, `crates/talaria-mcp/` (~3,159 lines across `crates/*/src/*.rs`)

**Secondary:**
- Python 3 (3.14.6 on this machine) — end-to-end test harness only, `tests/e2e/*.py`
- JavaScript — not authored in-repo, but injected into pages at runtime via the `evaluate` MCP tool (`crates/talaria-mcp/src/tools.rs`)

## Runtime

**Environment:**
- Native desktop binary. Two executables:
  - `talaria` — the browser shell (`crates/talaria-shell/src/main.rs`, `[[bin]]` in `crates/talaria-shell/Cargo.toml`)
  - `talaria-mcp` — stdio MCP proxy (`crates/talaria-mcp/src/main.rs`)
- Rust toolchain: 1.97.1 installed; README requires ≥ 1.88. No `rust-toolchain.toml` is committed — the toolchain is unpinned.
- Async runtime: `tokio` (feature-gated per crate; shell uses `rt`, MCP proxy uses `rt-multi-thread`)

**Package Manager:**
- Cargo, workspace with `resolver = "2"` (`Cargo.toml`)
- Lockfile: `Cargo.lock` present and committed (9,527 lines) — deliberately load-bearing, see Configuration below

## Frameworks

**Core:**
- `servo` 0.4.0 (libservo) — the web engine, compiled in; content pane only. Instantiated via `ServoBuilder` in `crates/talaria-shell/src/app.rs`
- `servo-embedder-traits` `=0.4.0` (aliased `embedder_traits`) — exact-pinned to match the engine
- `winit` 0.30.13 — window + event loop (`crates/talaria-shell/src/main.rs`)
- `egui` / `egui-winit` / `egui_glow` 0.34.3 — browser chrome, drawn into Servo's GL context (`crates/talaria-shell/src/gui.rs`). Versions must match Servo's own workspace so the `glow` context type is shared.
- `glow` 0.17 — GL bindings shared between egui and Servo
- `egui-phosphor` 0.12 — chrome icon font
- `rust-mcp-sdk` 1.0.1 (`server`, `macros`, `stdio` features, `default-features = false`) — MCP server implementation in `crates/talaria-mcp/`

**Testing:**
- No Rust test framework in use — no `#[cfg(test)]` modules, no `tests/` Rust integration targets
- Python stdlib (`subprocess`, `socket`, sockets + Xvfb) drives all regression tests: `tests/e2e/run_all.py`, `tests/e2e/harness.py`

**Build/Dev:**
- Cargo only. No justfile, Makefile, or CI configuration (`.github/` does not exist)
- Release profile uses `lto = "thin"`; `[profile.dev.package."*"] opt-level = 2` per Servo's recommendation (otherwise debug-build rendering is unusably slow)

## Key Dependencies

**Critical:**
- `servo` 0.4.0 — the entire rendering/JS capability; every architectural constraint flows from it
- `rustls` 0.23 — TLS for the engine. `rustls::crypto::aws_lc_rs::default_provider().install_default()` must run before anything else in `crates/talaria-shell/src/main.rs`
- `url` 2.5 — URL parsing and the address-bar/search heuristic (`crates/talaria-shell/src/app.rs`)
- `talaria-protocol` (path dep) — the newline-delimited-JSON control-socket schema shared by shell and MCP proxy (`crates/talaria-protocol/src/lib.rs`)

**Infrastructure:**
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

**Environment:**
- No `.env` files, no config file format. All configuration is environment variables plus CLI argv.
- `XDG_RUNTIME_DIR` — determines the control-socket path; falls back to `/tmp/talaria-$UID.sock` (`crates/talaria-protocol/src/lib.rs:15`)
- `TALARIA_COMMAND_TIMEOUT_SECS` — control-socket command timeout (`crates/talaria-shell/src/control.rs:185`, `crates/talaria-shell/src/app.rs:1075`)
- `TALARIA_TEST_HOOKS=1` — enables test-only behavior in the shell (`crates/talaria-shell/src/app.rs:972`)
- `TALARIA_E2E_DISPLAY` — X display the e2e harness creates with Xvfb, default `:99` (`tests/e2e/harness.py`)
- `RUST_LOG` — log filtering, via Servo's logger
- First CLI argument is the startup URL; default `https://servo.org` (`crates/talaria-shell/src/main.rs:17`)

**Build:**
- `Cargo.toml` (workspace root) — all dependency versions are declared once in `[workspace.dependencies]`; member crates use `{ workspace = true }`
- `Cargo.lock` — **must not be blindly updated**. Fresh resolution against servo 0.4.0 pulls `primeorder 0.14.0` (final), which breaks `p256/p384/p521 0.14.0-rc.14`. The lock pins `primeorder 0.14.0-rc.14` (`Cargo.lock:5241`); recovery is `cargo update -p primeorder --precise 0.14.0-rc.14`. Documented in `SPEC.md:98`.
- `.gitignore` excludes `target/`

## Platform Requirements

**Development:**
- Rust ≥ 1.88 plus Servo's Linux/macOS build prerequisites: `python3`, `pkg-config`, `cmake`, `clang`, fontconfig/freetype dev headers
- A GL-capable display; headless runs use Xvfb
- Python 3 for the e2e suite
- `getuid` is called via a raw `extern "C"` shim (`crates/talaria-protocol/src/lib.rs:24`) and the socket path is a Unix domain socket — the control plane is Unix-only in practice, even though `keyring` carries a Windows feature

**Production:**
- Locally built and locally run desktop binaries; no packaging, installer, signing, or release pipeline exists in the repo
- Build: `cargo build --release`; run: `cargo run --release -p talaria-shell -- https://example.com`

---

*Stack analysis: 2026-08-15*

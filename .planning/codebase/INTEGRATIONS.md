# External Integrations

**Analysis Date:** 2026-08-15

## APIs & External Services

Talaria has **no SaaS or cloud-service integrations**. There are no API keys, no vendor SDKs, and no telemetry. Every external interaction is either the web itself (through Servo) or a local IPC surface.

**Agent control surface (MCP):**
- `talaria-mcp` — a stdio MCP server implemented with `rust-mcp-sdk` 1.0.1 (`crates/talaria-mcp/src/main.rs`)
  - Transport: `StdioTransport` — the MCP client spawns the binary; there is no listening port
  - Auth: **none by design.** Per `crates/talaria-mcp/src/main.rs:5`, a locally-spawned stdio server does not go through OAuth; access is implied by the ability to spawn the process. (The README's "OAuth'ed MCP server" describes the intended distributed mode, not what is implemented.)
  - Client identity: taken from MCP `clientInfo.name` (`crates/talaria-mcp/src/main.rs:48-51`) and sent as the `Hello { client }` control-socket frame to label the agent's tabs in the Agents view
  - Tools exposed (`crates/talaria-mcp/src/tools.rs`): `tabs_list`, `tabs_open`, `tabs_close`, `tabs_focus`, `navigate`, `evaluate`, `screenshot`, `cookies_read`, `download`
  - Intended client: Claude Desktop via `claude_desktop_config.json` (`mcpServers.talaria.command` → the built `talaria-mcp` binary)

**Web content:**
- Servo (libservo 0.4.0) performs all page fetches, TLS (`rustls` + `aws_lc_rs`), cookies, and storage — Talaria itself never proxies page traffic

**Direct outbound HTTP from the shell:**
- `ureq::get(url).call()` in `crates/talaria-shell/src/app.rs:1253` — the only HTTP request made outside the engine, serving the `download` tool/command. It writes to the OS downloads directory (`dirs::download_dir()`, falling back to `.`).

**Default endpoints referenced in code:**
- `https://servo.org` — startup URL default and new-tab URL (`crates/talaria-shell/src/main.rs:17`, `crates/talaria-shell/src/app.rs:798`)
- `https://duckduckgo.com/?q={query}` — address-bar fallback search (`crates/talaria-shell/src/app.rs:867`)
- `https://accounts.google.com/signin`, `https://github.com/login` — sample vault entries only (`crates/talaria-shell/src/vault.rs:190-191`), not live integrations

## Data Storage

**Databases:**
- No application database. Servo's own SQLite-backed ClientStorage lives inside the engine profile at `$XDG_CONFIG_HOME/talaria/servo/` — set explicitly via `servo::Opts { config_dir }` in `crates/talaria-shell/src/app.rs:451-459`, because the default temp dir both discarded session state and failed to initialize the sqlite store.
- This directory holds localStorage, IndexedDB, and cookies. It is the reason single-instance enforcement exists — two shells must not share it.

**File Storage:**
- Local filesystem only:
  - Engine profile: `$XDG_CONFIG_HOME/talaria/servo/`
  - Credential vault: `$XDG_CONFIG_HOME/talaria/` (`config_dir()` in `crates/talaria-shell/src/vault.rs:28`)
  - Vault key fallback file: `$XDG_CONFIG_HOME/talaria/vault.key`, mode `0600`
  - Downloads: `dirs::download_dir()`

**Caching:**
- None beyond Servo's internal caches.

## Authentication & Identity

**Auth Provider:**
- None. There is no user account, no identity provider, and no login flow owned by Talaria.
- Site credentials live in a local **credential vault** (`crates/talaria-shell/src/vault.rs`):
  - Format: JSON list of `CredentialEntry` (type defined in `crates/talaria-protocol/src/lib.rs`), sealed with ChaCha20-Poly1305, file prefixed with the magic `TALARIA1`
  - Key storage: OS keychain via `keyring::Entry::new("talaria", "vault")` — Secret Service on Linux, Keychain on macOS, Credential Manager on Windows
  - Fallback: a hex 32-byte key in `vault.key` at mode `0600`, used when the keychain is unavailable (headless/CI); logged as a warning. If the key file exists but is malformed, the vault refuses to open rather than re-keying.
  - Consumers: autofill suggestion in the chrome, and the `cookies_read` MCP tool for reusing existing sessions
  - Explicit non-goal: agents do not perform fresh logins. Fresh logins go through **human takeover** — the user switches to the Agents view and drives the live session themselves (e.g. an SSO screen or a Cloudflare challenge), then hands control back.

**Local control-socket trust model:**
- Unix domain socket at `$XDG_RUNTIME_DIR/talaria.sock`, falling back to `/tmp/talaria-$UID.sock` (`crates/talaria-protocol/src/lib.rs:15-21`)
- Authorization is filesystem permissions on that socket; there is no handshake secret. The `Hello { client }` frame is a display label, not a credential.

## Monitoring & Observability

**Error Tracking:**
- None. No Sentry, no crash reporter, no analytics or telemetry of any kind.

**Logs:**
- `log` + `tracing` macros throughout; the global logger is installed by `servo.setup_logging()` (`crates/talaria-shell/src/app.rs`) rather than `env_logger`, because Servo panics if a logger is already set. Filtering is via `RUST_LOG`.
- The e2e harness captures shell output to `tests/e2e/talaria.log`.

## CI/CD & Deployment

**Hosting:**
- Not applicable — a locally built desktop application. No deployment target, packaging, or update channel exists.

**CI Pipeline:**
- None. There is no `.github/` directory and no CI configuration anywhere in the repo. All regression runs are manual: `python3 tests/e2e/run_all.py` after `cargo build --release`.
- Long-running verification is done by an "overnight loop" convention guarded by `.overnight-lock` and `tests/e2e/overnight_lock.py`, with results recorded in `OVERNIGHT_LOG.md`.

## Environment Configuration

**Required env vars:**
- None are required. All have defaults.
- Optional: `XDG_RUNTIME_DIR`, `TALARIA_COMMAND_TIMEOUT_SECS`, `TALARIA_TEST_HOOKS`, `TALARIA_E2E_DISPLAY`, `RUST_LOG`, `DISPLAY`

**Secrets location:**
- OS keychain (service `talaria`, account `vault`), with a `0600` key file fallback at `$XDG_CONFIG_HOME/talaria/vault.key`. No secrets are stored in the repo, and no `.env` file exists.

## Webhooks & Callbacks

**Incoming:**
- No HTTP server, no webhook endpoints. The only inbound channel is the Unix control socket, which accepts newline-delimited JSON `ClientMessage` frames (`Hello`, `Request { id, command }`) and replies with `ServerMessage` frames carrying the request `id`. Replies may arrive out of order because `evaluate` and `screenshot` complete asynchronously inside the engine.
- A second inbound use of the same socket is single-instance handoff: a newly launched `talaria` first calls `control::forward_to_running_instance(url)` (`crates/talaria-shell/src/control.rs:28`) and exits if an existing instance accepts the URL, rather than stealing the socket path.

**Outgoing:**
- None.

---

*Integration audit: 2026-08-15*

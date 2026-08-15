# Codebase Concerns

**Analysis Date:** 2026-08-15

Scope: the three-crate Rust workspace (`crates/talaria-shell`, `crates/talaria-protocol`, `crates/talaria-mcp`) plus the Python e2e harness in `tests/e2e/`. Each open item recorded in `OVERNIGHT_LOG.md` and `SPEC.md` was re-checked against the code; the verdict is stated inline.

## Tech Debt

**Credential vault is read-only — no write or capture path (`crates/talaria-shell/src/vault.rs`):**
- Issue: `Vault` exposes only `load()`, `save()` and `matching()`. Grepping the rest of the shell shows the vault is touched in exactly three places — construction at `crates/talaria-shell/src/app.rs:477`, the field at `:72`, and the read at `:1042` (`Command::CookiesRead`). Nothing ever pushes a `CredentialEntry`, so `save()` only ever rewrites what was loaded. There is no autofill UI in `crates/talaria-shell/src/gui.rs` either, despite SPEC listing "autofill suggestion during human-driven moments" as the vault's primary role.
- Files: `crates/talaria-shell/src/vault.rs`, `crates/talaria-shell/src/app.rs:1041-1044`, `crates/talaria-shell/src/gui.rs`
- Impact: the only way to populate the vault is to hand-write `~/.config/talaria/vault.json` in plaintext. The `cookies_read` MCP tool always returns `{"entries": []}` on a fresh install, so agents cannot reuse the user's sessions — one of the SPEC's headline features is inert.
- Fix approach: add `Vault::upsert(entry)` + a capture path (a "save credentials?" prompt on form submit, and/or reading Servo's cookie jar for the active domain), and an autofill suggestion popup in the toolbar/page-form path. Encryption at rest already works, so this is additive.
- **OVERNIGHT_LOG/SPEC claim status: still holds.** SPEC's "Credential vault implementation — resolved" describes design, not shipped behaviour.

**Plaintext `vault.json` import leaves the plaintext file on disk (`crates/talaria-shell/src/vault.rs:115-127`):**
- Issue: when `vault.enc` is missing, `Vault::load()` reads `~/.config/talaria/vault.json`, logs `importing plaintext vault.json into encrypted vault`, and then `save()`s the encrypted copy — but never deletes, renames, or chmods the plaintext source. On every subsequent start the encrypted file exists, so the plaintext file is quietly ignored and left sitting there indefinitely with default permissions.
- Files: `crates/talaria-shell/src/vault.rs:115-131`
- Impact: passwords in cleartext on disk, contradicting SPEC's "encrypted at rest, not plaintext on disk" decision. Worse, the user believes the import migrated them.
- Fix approach: after a successful `save()`, delete (or rename to `vault.json.imported` with `0600`) the plaintext file, and surface the import in the UI rather than only in the log.
- **Claim status: confirmed, and worse than logged** — the log noted the import path exists; it did not note that the source file survives.

**Key-file fallback silently downgrades data-at-rest protection (`crates/talaria-shell/src/vault.rs:33-86`):**
- Issue: if the OS keychain is unavailable (headless Linux, no Secret Service — i.e. the exact environment the e2e suite runs in), a 32-byte key is written to `~/.config/talaria/vault.key` at `0600`, next to `vault.enc`. The `set_permissions` result is discarded (`let _ =`), so a failed chmod is invisible.
- Files: `crates/talaria-shell/src/vault.rs:60-85`
- Impact: encryption at rest becomes obfuscation at rest — key and ciphertext in the same directory. Acceptable per the module doc, but it is a `log::warn!` the user never sees because the shell has no log surface in the UI.
- Fix approach: surface the downgrade in the UI once, and check the chmod result before writing the key.

**No `history`, `bookmarks`, or downloads UI despite being resolved as v1 scope in SPEC:**
- Issue: grep for `history`/`bookmark` across `crates/talaria-shell/src/` returns nothing. Downloads exist only as the agent-facing `Command::Download` (`crates/talaria-shell/src/app.rs:1058-1067`, `crates/talaria-shell/src/app.rs:1250-1269`) — no UI, no download list, no progress.
- Files: `crates/talaria-shell/src/gui.rs`, `crates/talaria-shell/src/app.rs`
- Impact: the "Me" side of the browser is below the table-stakes bar SPEC set for itself. Omnibox search does work (`resolve_location`, `crates/talaria-shell/src/app.rs:853-868`) but the engine is hardcoded to DuckDuckGo, not configurable as SPEC states.
- Fix approach: a small local store (SQLite or JSON) under the existing `~/.config/talaria` dir, plus egui panels; configurable search engine is a one-line setting on top of `resolve_location`.

**Distributed mode does not exist:**
- Issue: SPEC's centerpiece — a multiplexed WebSocket over Tailscale, JPEG frame streaming, adaptive 30–500ms polling, session manifest persistence, restore-on-restart — has no code. The only transport is the local Unix socket (`crates/talaria-shell/src/control.rs`), and `crates/talaria-protocol/src/lib.rs` documents itself as "the in-process precursor to the distributed protocol."
- Files: `crates/talaria-protocol/src/lib.rs:1-8`, `crates/talaria-shell/src/control.rs`
- Impact: sizeable unbuilt surface. Two SPEC decisions (OAuth for HTTP connections, crash-recovery session manifest) are blocked behind it and read as done when they are not.
- Fix approach: treat as its own milestone; the `ClientMessage`/`ServerMessage` envelope is already transport-agnostic, so the socket layer is the only thing that changes shape.

## Known Bugs

**Tab crash/close events reach the control socket but the MCP proxy discards them:**
- Symptoms: an agent whose tab crashes or is closed by the page gets no notification. It finds out only when its next `evaluate`/`screenshot` returns `tab {id} crashed — navigate it to recover` (`crates/talaria-shell/src/app.rs:984-989`, `:998-1002`).
- Files: `crates/talaria-mcp/src/socket.rs:70-80` (the `ServerMessage::Event` arm is an explicit no-op comment: "Not ours (event or stray reply) — keep reading"), producer side at `crates/talaria-shell/src/app.rs:368-376`, `:1392`, `:1404`, `crates/talaria-protocol/src/lib.rs:134-139`
- Trigger: any `Event::TabCrashed` / `Event::TabClosed` while an MCP session is connected.
- Workaround: none for MCP clients; a raw control-socket client (e.g. `tests/e2e/crash_event_test.py`) does receive them.
- **Claim status: confirmed exactly as logged.** SPEC's "Tab/process crash recovery — resolved… surfaces as an MCP tool error/event" is half-true: the error path works, the event path stops at the proxy. Fixing it needs the `ShellConnection` reader split into a background task that can raise `notifications/message` (or a resource-changed notification) through the `rust-mcp-sdk` runtime, which the current request/reply `Mutex<Option<Wire>>` design cannot do.

**`ShellConnection` serialises every MCP tool call behind one mutex (`crates/talaria-mcp/src/socket.rs:27-46`):**
- Symptoms: a wedged `evaluate` blocks *all* other tool calls from the same agent for the full command timeout — including `tabs_list`, `screenshot`, and `tabs_close` on unrelated tabs. The same serialisation exists shell-side: `handle_connection` awaits each request's outcome before reading the next line (`crates/talaria-shell/src/control.rs:166-215`).
- Files: `crates/talaria-mcp/src/socket.rs:27-46`, `crates/talaria-shell/src/control.rs:166-215`
- Trigger: `evaluate` on a heavy-JS page (see below), or any 30s timeout.
- Workaround: shorten `TALARIA_COMMAND_TIMEOUT_SECS`; open a second MCP session.
- Fix approach: the protocol already tolerates out-of-order replies (documented at `crates/talaria-protocol/src/lib.rs:5-8`) and IDs every request — make both sides pipeline instead of blocking.

**Retry-on-stale-connection can re-execute a non-idempotent command (`crates/talaria-mcp/src/socket.rs:29-45`):**
- Symptoms: if the write succeeds but the read fails, the command is resent on a fresh connection. `tabs_open` or `download` could run twice.
- Files: `crates/talaria-mcp/src/socket.rs:34-42`
- Trigger: shell restart, or a socket error mid-round-trip.
- Fix approach: only retry when the failure happened before any byte was written, or when the connection was never established this call.

**Popup adoption can be silently dropped (`crates/talaria-shell/src/app.rs:1290-1293`):**
- Symptoms: a `window.open()` fired while the tab table is already mutably borrowed logs `dropping popup request: tab table busy` and the popup never appears. Same shape at `broadcast_event` (`:369`, a failed `try_borrow` silently drops crash/close events for all sessions) and `notify_load_status_changed` (`:1339`).
- Files: `crates/talaria-shell/src/app.rs:369`, `:1290`, `:1328`, `:1339`, `:1364`, `:1385`, `:1398`
- Trigger: reentrancy from Servo delegate callbacks during an in-flight command.
- Fix approach: queue the request into a `RefCell<Vec<_>>` deferred-work list drained from the event loop (the pattern already used by `pending_captures`/`pending_loads`/`pending_evals`) instead of dropping it.

## Security Considerations

**No scheme allowlist on agent-supplied URLs — agents can read local files:**
- Risk: `parse_agent_url` (`crates/talaria-shell/src/app.rs:841-849`) accepts anything `Url::parse` accepts and only adds an `https://` prefix to bare hosts. `file://`, `data:`, `about:`, and any other scheme Servo loads go straight through to `Command::TabsOpen` (`:914-927`) and `Command::Navigate` (`:948-967`). An agent can then read the loaded document via `evaluate`, i.e. exfiltrate any file the user can read that Servo will render (`.html`, `.txt`, `.json`, …; extensionless paths like `/etc/hostname` come back as an octet-stream error page only because of content sniffing, which is not a security boundary).
- Files: `crates/talaria-shell/src/app.rs:841-849`, `:915`, `:949`
- Current mitigation: none. `resolve_location` (the human URL bar path, `:853`) is a separate function, so restricting agents does not affect the user.
- Recommendations: allowlist `http`/`https`/`about`/`data` in `parse_agent_url` and return a clear tool error otherwise, or make it an explicit opt-in setting. This is the SPEC "no-gatekeeping" philosophy colliding with local-filesystem read — worth an explicit decision either way rather than defaulting by omission.
- **Claim status: confirmed, unchanged since the log entry.**

**Control socket has no authentication and, on the fallback path, weak filesystem protection:**
- Risk: `socket_path()` (`crates/talaria-protocol/src/lib.rs:15-21`) uses `$XDG_RUNTIME_DIR/talaria.sock` when set (mode-0700 dir, fine) and otherwise `/tmp/talaria-$UID.sock`. `UnixListener::bind` (`crates/talaria-shell/src/control.rs:93`) never chmods the socket, so on the `/tmp` fallback any local user can connect, send `Hello`, and get full control of the browser — the user's cookies, sessions, `cookies_read` vault contents, arbitrary JS in logged-in pages. `handle_connection` accepts any `client` string with no credential check.
- Files: `crates/talaria-protocol/src/lib.rs:15-21`, `crates/talaria-shell/src/control.rs:90-156`
- Current mitigation: `XDG_RUNTIME_DIR` is normally set on desktop Linux; the MCP transport is stdio-only, so remote access is not exposed.
- Recommendations: `SO_PEERCRED` check that the peer UID matches, plus `set_permissions(0o600)` on the socket after bind, plus creating the `/tmp` fallback inside a `0700` per-UID directory.

**No OAuth / no auth layer of any kind:**
- Risk: `crates/talaria-mcp/src/main.rs` builds only a `StdioTransport` (`:90`) and its own doc comment states stdio "does not go through OAuth — connection access is implied by the ability to spawn it." That is correct per the MCP spec, but it means SPEC's entire resolved OAuth design (authorization server, PKCE, per-client tokens, revocable "connected agents" list) is unimplemented and untested. Session identity is a self-asserted `clientInfo.name` string (`crates/talaria-mcp/src/main.rs:48-51`) that anything can spoof, and it is what the Agents view labels sessions with.
- Files: `crates/talaria-mcp/src/main.rs:90`, `crates/talaria-shell/src/control.rs:128-156`
- Current mitigation: local-only transport.
- Recommendations: OAuth becomes mandatory the moment the HTTP/distributed transport lands — sequence it with that work, not after.
- **Claim status: confirmed.**

**`download` has no size cap, no overwrite protection, and bypasses the browser session:**
- Risk: `Command::Download` validates only that the filename contains no `/` and no `..` (`crates/talaria-shell/src/app.rs:1058-1062`), then a detached thread `ureq::get`s the URL and `std::io::copy`s it into `~/Downloads/<filename>` (`:1250-1269`). No content-length limit (an agent can fill the disk), no existing-file check (silently clobbers a user file of the same name), no timeout (the thread outlives the 30s command timeout and keeps writing after the agent got an error), and no concurrency limit. It also uses a bare `ureq` client, so it carries none of the page's cookies — downloads behind a login silently fetch the login page instead.
- Files: `crates/talaria-shell/src/app.rs:1058-1067`, `:1250-1269`
- Current mitigation: the filename check does block path traversal on Unix.
- Recommendations: cap bytes, uniquify the filename (`file (1).pdf`), give the request a timeout and a cancellation handle, and route through Servo's network stack so session cookies apply.

**Crash and close events are broadcast to every connected session (`crates/talaria-shell/src/app.rs:368-376`):**
- Risk: agent A is told about agent B's tab IDs closing/crashing. Minor, but the events carry no owner filter.
- Files: `crates/talaria-shell/src/app.rs:368-376`
- Recommendations: filter by tab owner when the MCP notification path is built — cheap to do at the same time.

**Agent scripts run with full page authority and shared cookie jar — by design, flagged for visibility:**
- Risk: SPEC's "shared session state by design, no isolation layer" means any agent with MCP access can act as the logged-in user anywhere. `request_navigation` allows every navigation unconditionally (`crates/talaria-shell/src/app.rs:1272-1280`), and `request_create_new` does no popup blocking (`:1282-1323`).
- Current mitigation: intentional, documented in SPEC's product philosophy.
- Recommendations: none technically; keep it explicit in user-facing docs so the blast radius of granting MCP access is understood.

## Performance Bottlenecks

**`evaluate` wedges on heavy-JS pages, with only a blanket command timeout:**
- Problem: on JS-heavy sites (github.com was the logged case) every `evaluate` — even `document.title` — burns the full 30s timeout because Servo's script thread is busy with the page's own JS and never runs the callback.
- Files: timeout is the only defence, at `crates/talaria-shell/src/control.rs:184-204`; the eval path itself is `crates/talaria-shell/src/app.rs:1118-1168` with promise polling in `pending_evals`.
- Cause: no per-tab isolation and no slow-script interrupt. `WebView::evaluate_javascript` is fire-and-forget from the shell's side — the callback either fires or it does not, and `PendingEval` entries are only advanced from the event loop.
- Improvement path: the real fix is upstream (a SpiderMonkey slow-script interrupt exposed through libservo). Shell-side mitigations worth having now: per-tab in-flight tracking so a wedged tab fails fast with "tab busy" instead of burning 30s each time, and pipelining (above) so one wedged tab does not stall the session.
- **Claim status: confirmed — no per-tab isolation exists in code; `TALARIA_COMMAND_TIMEOUT_SECS` (`crates/talaria-shell/src/control.rs:185`) is the only knob.**

**~12MB per tab from per-webview framebuffers, ~31MB allocator retention after closing 10 tabs:**
- Problem: every tab gets its own offscreen rendering context (`crates/talaria-shell/src/app.rs:1303-1304`, and the same in `crates/talaria-shell/src/tabs.rs`), allocated eagerly at open. A heavy page costs ~200MB engine-side. Soak RSS sat flat at 476–479MB, so this is cost, not leak.
- Files: `crates/talaria-shell/src/tabs.rs`, `crates/talaria-shell/src/app.rs:1303`
- Improvement path: allocate the framebuffer lazily on first display/capture. Recorded as an observation in `OVERNIGHT_LOG.md`, still not done.

**Background-tab screenshots cost up to 1.5s (`crates/talaria-shell/src/app.rs:1023-1034`):**
- Problem: capturing a background tab shows it into its own framebuffer and waits for `notify_new_frame_ready`, with a 1500ms deadline fallback for pages that never repaint.
- Files: `crates/talaria-shell/src/app.rs:1004-1039`, `:1325-1334`
- Improvement path: acceptable today (measured 2–82ms typical); the deadline is the worst case, not the norm. PNG encode is ~30ms at default compression — `Compression::Fast` cuts it to ~9ms at +50% bytes if capture ever becomes hot.

**Software GL dominates on animated pages:** WebGL/rAF pages under Xvfb hit 100–640% CPU, entirely in Mesa `llvmpipe`. Nothing shell-side; the main thread stays ~17%. Non-issue on a real GPU.

## Fragile Areas

**`crates/talaria-shell/src/app.rs` is 1425 lines and holds everything:**
- Files: `crates/talaria-shell/src/app.rs`
- Why fragile: the winit `ApplicationHandler`, the `WebViewDelegate`, agent command dispatch, JS wrapping, PNG encode, downloads, URL parsing and the three pending-work queues (`pending_captures`, `pending_loads`, `pending_evals`) all live in one file, all reached through `Rc<Shared>` with `RefCell` interior mutability. Servo delegate callbacks reenter this state, which is why the file is littered with `try_borrow` guards that silently drop work on contention.
- Safe modification: never take a long-lived `borrow_mut()` across a call into Servo; mark state and drain from the event loop, following `PendingCapture`/`PendingLoad`/`PendingEval`.
- Test coverage: the command-dispatch half is covered end-to-end by `tests/e2e/`; the delegate half is not directly testable and is where the silent-drop bugs live.

**The `wrap_script` JS trampoline (`crates/talaria-shell/src/app.rs:1088-1115`):**
- Files: `crates/talaria-shell/src/app.rs:1088-1115`, `:1109-1115`
- Why fragile: a hand-built JS string with three nested fallbacks (indirect eval → async-IIFE-return retry → async-body retry), promise parking on `window.__talaria_async`, and CSP detection by matching `e instanceof EvalError`. It depends on SpiderMonkey's error taxonomy and on the exact wording of syntax errors (`/await/.test(String(e.message))`). A Servo version bump can change either.
- Safe modification: extend the 30-case probe matrix described in `OVERNIGHT_LOG.md` iteration 7 before touching the string; also note the CSP fallback path (`EvalStep::RunRaw`) does *not* await promises, a documented behavioural difference agents can hit without warning.
- Test coverage: e2e covers promise, `await fetch`, rejection, globals, and a CSP page. There is no Rust unit test for `wrap_script` output.

**Load-completion detection for popups (`crates/talaria-shell/src/app.rs:872-888`, `:1336-1356`, `:1363-1380`):**
- Why fragile: a tab's `loading` flag combines `load_status()` with a wall-clock `initial_blank_until` grace window, because an adopted popup runs a full Started→Complete cycle on its own `about:blank` document before the opener's navigation begins. Time-based heuristics in a load state machine break under load or on slow machines.
- Safe modification: prefer additional signals from `notify_url_changed` over widening the grace window.

**The three-crate test story is Xvfb-shaped:**
- Why fragile: there are exactly three `#[test]` functions in the whole workspace (`crates/talaria-protocol/src/lib.rs:145`, `:165`; `crates/talaria-shell/src/vault.rs:184`). All real coverage is the Python suite in `tests/e2e/`, which needs Xvfb, a built release binary, and a private `XDG_RUNTIME_DIR`. `cargo test` proves almost nothing.
- Files: `tests/e2e/run_all.py`, `tests/e2e/harness.py`
- Safe modification: any refactor of `app.rs` must be validated by `python3 tests/e2e/run_all.py`, not by `cargo test`.

## Scaling Limits

**One request in flight per session, one shell process, one window:**
- Current capacity: N concurrent MCP sessions, each strictly sequential (see the mutex bug above). 200 rapid connect/disconnect churns were clean — sessions map drains, no thread growth.
- Limit: an agent that wants parallel tab work must open parallel MCP sessions; tab count is bounded by the ~12MB/tab framebuffer plus page memory.
- Scaling path: pipeline the socket, lazily allocate framebuffers.

**Unbounded event channel per connection (`crates/talaria-shell/src/control.rs:150`):**
- A client that stops reading its socket while tabs churn grows the `mpsc::unbounded_channel` without limit. Low risk today (only two event types), but it is unbounded memory driven by a peer's behaviour.

**Tab volume is uncapped:** SPEC explicitly defers tab-volume controls, and `request_create_new` does no popup blocking — a runaway agent or a popup loop can open tabs until memory runs out.

## Dependencies at Risk

**`servo` 0.4.0 with a hand-pinned `primeorder`:**
- Risk: a fresh dependency resolution pulls `primeorder 0.14.0` (final) which breaks `p256/p384/p521 0.14.0-rc.14` with an E0277. The workaround lives only in the committed `Cargo.lock` (`cargo update -p primeorder --precise 0.14.0-rc.14`) and in a SPEC note.
- Impact: any `cargo update` silently breaks the build; a contributor with a clean checkout and a lockfile refresh hits a confusing RustCrypto error.
- Migration plan: record the pin as a `[patch]`/explicit dependency with a comment in `Cargo.toml`, or add a CI job that builds from a regenerated lockfile so drift is caught deliberately.

**`servo` is a fast-moving 0.x with recent APIs at the core:** `WebView::take_screenshot`, offscreen rendering contexts, `evaluate_javascript`, and the `JavaScriptEvaluationError` taxonomy that `describe_js_error` (`crates/talaria-shell/src/app.rs:1173-1193`) matches exhaustively are all recent, churn-prone surfaces. `egui`/`egui-winit`/`egui_glow` must stay pinned to whatever Servo's own workspace uses (`Cargo.toml` comment: "shared glow context type") — an independent egui bump breaks the shared GL context.

**`ureq 2`** is used only for downloads and is a second HTTP stack alongside Servo's; keeping it means downloads will always diverge from browser session behaviour.

## Missing Critical Features

- **MCP notifications for tab lifecycle** — see Known Bugs. Blocks agents reacting to crashes without polling.
- **Vault write path and autofill** — see Tech Debt. Makes `cookies_read` useless in practice.
- **OAuth + HTTP/WebSocket transport** — blocks distributed mode and any non-local agent.
- **History, bookmarks, downloads UI, configurable search engine** — SPEC-resolved v1 items with no code.
- **Session manifest persistence / restore-on-restart** — SPEC's crash-recovery answer for distributed mode; nothing persists tab state today.
- **Update/distribution mechanism** — SPEC explicitly reopened this on 2026-08-16 when the Tauri updater stopped applying; still undecided, and there is no packaging in the repo (no CI, no release workflow, no `cargo-dist` config).
- **macOS build** — SPEC names macOS as target platform #1; everything built and soaked so far is Linux. `crates/talaria-shell/src/vault.rs` has `#[cfg(unix)]` handling and `keyring` carries `apple-native`, but no macOS run has happened.

## Test Coverage Gaps

**`.overnight-lock` preflight is advisory with nothing enforcing it:**
- What's not tested/enforced: `tests/e2e/overnight_lock.py` is a well-documented cooperative lock, but nothing calls it. No git hook, no `run_all.py` integration, no CI. The file is gitignored, and `acquire` does not check whether the recorded PID is alive — the lock currently on disk (`.overnight-lock`, pid 2796577, owner `c85a1ef3`) references a process that no longer exists, so the next loop is refused by a corpse and has to `--force`.
- Files: `tests/e2e/overnight_lock.py`, `.overnight-lock`, `.gitignore`
- Risk: the exact failure it was written to prevent — two sessions on one branch and working tree, one soaking a binary the other rebuilt underneath it — is still possible. `OVERNIGHT_LOG.md` documents that this already happened once and cost a soak result plus two silently-dropped log entries.
- Priority: High — cheap to fix. Add a PID/`kill -0` liveness check so stale locks self-clear, and call `acquire` from the loop's own preflight (or a `pre-commit` hook) so it is not optional.
- **Claim status: confirmed exactly as logged.**

**Delegate-callback paths have no direct tests:** `notify_crashed`, `notify_closed`, `request_create_new`, and the `try_borrow`-drop branches (`crates/talaria-shell/src/app.rs:1282-1410`) are only exercised indirectly. The crash path is tested via the `__talaria_sim_crash__` hook (`crates/talaria-shell/src/app.rs:971-983`), which sets `tab.crashed` directly and therefore never exercises real `notify_crashed`. Priority: Medium.

**A test-only backdoor ships in release builds:** `__talaria_sim_crash__` is gated on `TALARIA_TEST_HOOKS=1` at runtime, not on `#[cfg(test)]` or a cargo feature (`crates/talaria-shell/src/app.rs:971-972`). Harmless in effect (it only marks a tab crashed), but the pattern invites a more dangerous hook later. Priority: Low — gate it behind a `test-hooks` cargo feature.

**No CI:** no `.github/`, no workflow files. Every check in `OVERNIGHT_LOG.md` was run by hand. The Xvfb dependency of `tests/e2e/` is the main obstacle, and it is solvable (Xvfb runs fine in CI). Priority: High — nothing currently prevents a regression from being committed.

**Final full e2e regression was deliberately skipped at the close of the 2026-08-16 loop** (`OVERNIGHT_LOG.md`, wind-down entry) because the suites would have killed a parallel session's processes. The current `HEAD` (`5b1f4db`, the `window.open`/`target=_blank` popup work) has probe verification but no full-suite pass on record. Priority: High — run `python3 tests/e2e/run_all.py` against a fresh release build before treating `HEAD` as green.

---

*Concerns audit: 2026-08-15*

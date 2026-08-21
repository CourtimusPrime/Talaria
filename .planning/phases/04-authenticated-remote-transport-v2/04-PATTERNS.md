# Phase 4: Authenticated Remote Transport (v2) - Pattern Map

**Mapped:** 2026-08-20
**Files analyzed:** 16 new/modified (7 Rust source, 3 Cargo/config, 3 Python e2e, 3 docs)
**Analogs found:** 13 / 16 (3 with no in-repo analog)

Every excerpt below is real code from this tree, with `file:line`. Where a plan's file has no analog,
that is said explicitly rather than papered over with a generic pattern.

---

## File Classification

| New/Modified File | Role | Data Flow | Closest Analog | Match Quality |
|-------------------|------|-----------|----------------|---------------|
| `crates/talaria-mcp/src/lib.rs` (new) | library / dispatch | request-response | `crates/talaria-mcp/src/tools.rs` (whole file moves) | exact (it *is* the file) |
| `crates/talaria-mcp/src/tools.rs` (modified) | service | request-response | itself, `tools.rs:120-179` | exact |
| `crates/talaria-mcp/src/main.rs` (modified) | entry point | request-response | itself, `main.rs:26-56` | exact |
| `crates/talaria-mcp/src/socket.rs` (modified: `impl CommandSink`) | service | request-response | `socket.rs:36-61` | exact |
| `Cargo.toml` + `Cargo.lock` (modified) | config | — | existing `[workspace.dependencies]` block | exact |
| `crates/talaria-shell/src/agents.rs` (new) | model / store | CRUD + file-I/O | `crates/talaria-shell/src/bookmarks.rs` | exact |
| `crates/talaria-shell/src/agents.rs` — token secret handling | model | file-I/O | `crates/talaria-shell/src/vault.rs` (`OsRng`, 0600, degrade) | role-match |
| `crates/talaria-shell/src/http.rs` (new) | service / listener thread | streaming + request-response | `crates/talaria-shell/src/control.rs` | role-match |
| `crates/talaria-shell/src/oauth.rs` (new) | middleware / auth provider | request-response | **no analog** (see No Analog Found) | none |
| `crates/talaria-shell/src/settings.rs` (modified: listener toggle) | config | file-I/O | itself, `settings.rs:186-275` | exact |
| `crates/talaria-shell/src/gui.rs` (modified: `Consent` + `Access` panels) | component | event-driven | `gui.rs:33-113`, `:409-424`, `:682-700`, `:1240-1273` | exact |
| `crates/talaria-shell/src/app.rs` (modified: `AppEvent`, `apply_ui_actions`, `parse_agent_url`) | controller | event-driven | `app.rs:44-83`, `:908-949`, `:1253-1295`, `:1476-1492` | exact |
| `crates/talaria-protocol/src/lib.rs` (modified: any new test-only command) | model / wire | request-response | `Command::ChromeRects`, `lib.rs:123`, `:183` | exact |
| `tests/e2e/http_transport_test.py` (new) | test | request-response | `tests/e2e/panel_click_test.py` | role-match |
| `tests/e2e/oauth_flow_test.py` (new) | test | request-response + event-driven | `tests/e2e/panel_click_test.py` + `tests/e2e/vault_test.py` | role-match |
| `tests/e2e/revocation_test.py` (new) | test | streaming | `tests/e2e/panel_click_test.py` (rects) + `mcp_client_test.py` (raw socket read) | partial |
| `tests/e2e/run_all.py` (modified) | test runner | batch | `run_all.py:67-70` | exact |

---

## Pattern Assignments

### `crates/talaria-mcp/src/lib.rs` + `tools.rs` + `main.rs` (04-01, tool-surface extraction)

**Analog:** the existing files themselves. This plan is a *move*, and the exact seam to cut is one
parameter.

**The single line that couples the surface to stdio** (`crates/talaria-mcp/src/tools.rs:120-124`):

```rust
pub async fn dispatch(
    connection: &ShellConnection,
    client: &str,
    tool: TalariaTools,
) -> Result<CallToolResult, CallToolError> {
```

and its one use (`crates/talaria-mcp/src/tools.rs:137-140`):

```rust
    let outcome = connection
        .request(client, command)
        .await
        .map_err(CallToolError::from_message)?;
```

**Everything else in `dispatch` is transport-agnostic already** — the `Command` match
(`tools.rs:125-135`) and the `ResultPayload` match (`tools.rs:147-178`) touch nothing but
`talaria_protocol` types. So exactly two things move behind the trait: the parameter type and that
`.request(...)` call. Do not restructure the two match blocks.

**Preserve the wildcard arm verbatim** (`tools.rs:167-178`) — it is the reason a control-socket-only
payload (`ResultPayload::ChromeRects`) cannot leak onto the tool surface:

```rust
        // Every payload a tool above can actually produce is named. ...
        _ => Err(CallToolError::from_message(
            "the shell answered with a result this tool does not understand".to_owned(),
        ))?,
```

**The stdio consumer that must keep compiling unchanged** (`crates/talaria-mcp/src/main.rs:44-55`):

```rust
    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        runtime: Arc<dyn McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        let client = runtime
            .client_info()
            .map(|info| info.client_info.name.clone())
            .unwrap_or_else(|| "unknown-agent".to_owned());
        let tool: TalariaTools = TalariaTools::try_from(params).map_err(CallToolError::new)?;
        tools::dispatch(&self.connection, &client, tool).await
    }
```

After extraction this line becomes `talaria_mcp::dispatch(&self.connection, &client, tool)` with
`ShellConnection: CommandSink`. `handle_list_tools_request` (`main.rs:32-42`) is what makes SC 1
structurally true — the HTTP handler must call the **same** `TalariaTools::tools()`.

**Crate-level note:** `talaria-mcp` has `serde` with derive and `rust-mcp-sdk`
(`crates/talaria-mcp/Cargo.toml`); `talaria-shell` has neither today. Adding a `[lib]` target to
`talaria-mcp` and depending on it from `talaria-shell` is what pulls the SDK into the shell — name
that in the plan, it is the real cost of Pattern 1.

---

### `crates/talaria-shell/src/agents.rs` (new; store, CRUD + file-I/O)

**Analog:** `crates/talaria-shell/src/bookmarks.rs` (whole-array atomic store) for the shape;
`crates/talaria-shell/src/settings.rs` for the degrade-to-default reasoning;
`crates/talaria-shell/src/vault.rs` for the secret-material discipline.

**Imports + module head** (`crates/talaria-shell/src/bookmarks.rs:29-32`):

```rust
use std::fs;
use std::path::PathBuf;

use crate::permissions;
```

**Hand-written JSON, no `serde` derive** (C-2) (`bookmarks.rs:55-83`):

```rust
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "url": self.url,
            "title": self.title,
            "created_at_ms": self.created_at_ms,
        })
    }

    fn from_json(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            url: value.get("url")?.as_str()?.to_owned(),
            title: value
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            created_at_ms: value
                .get("created_at_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
        })
    }
```

`to_json`/`from_json` sit adjacent on purpose (`bookmarks.rs:50-54`) so a field added to one is
visibly missing from the other. For `agents.rs` the load-bearing field — the one whose absence makes
the row corrupt, the `?` case — is the **token hash**, not the client name.

**Per-store `config_dir()`, copied not shared** (`bookmarks.rs:92-106`, identical at
`settings.rs:145-155`):

```rust
fn config_dir() -> PathBuf {
    let path = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("talaria");
    // Owner-only, and created here rather than in the write paths below.
    permissions::create_dir_owner_only(&path);
    path
}
```

The comment at `bookmarks.rs:92-95` states the convention explicitly: each store owns its own copy
rather than sharing a `paths` module. Follow it; do not introduce a shared helper for the fifth store.

**Degrade-never-abort load** (`bookmarks.rs:115-144`):

```rust
    fn load_from(path: PathBuf) -> Self {
        let stored: Option<Vec<serde_json::Value>> = fs::read(&path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok());
        let Some(stored) = stored else {
            // Absent and unreadable are the same answer — an empty list — but
            // only the second is worth saying out loud ...
            if path.exists() {
                log::warn!("bookmarks: {} is unreadable; starting empty", path.display());
            }
            return Self { path, entries: Vec::new() };
        };
        let mut entries = Vec::with_capacity(stored.len());
        let mut skipped = 0usize;
        for value in &stored {
            match Bookmark::from_json(value) {
                Some(entry) => entries.push(entry),
                None => skipped += 1,
            }
        }
        if skipped > 0 {
            log::warn!("bookmarks: skipped {skipped} unreadable entry(s) in {}", path.display());
        }
        Self { path, entries }
    }
```

For `agents.rs` the degraded state is an **empty client set** (every agent must re-authorize) — never
an empty *allow*. C-5.

**Atomic `.tmp`-sibling save — copy this verbatim** (`bookmarks.rs:196-227`):

```rust
    fn save(&mut self) {
        let array: Vec<serde_json::Value> =
            self.entries.iter().map(Bookmark::to_json).collect();
        let Ok(data) = serde_json::to_vec(&array) else {
            log::warn!("bookmarks could not be serialised; this change will not survive a restart");
            return;
        };
        // Staged beside the target, never in the temp directory: `fs::rename`
        // is only atomic within one filesystem ...
        let Some(name) = self.path.file_name().map(|name| name.to_string_lossy().into_owned())
        else {
            log::warn!("bookmarks path has no file name; not saving");
            return;
        };
        let staged = self.path.with_file_name(format!("{name}.tmp"));
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // Restricted on the staging file, not on the target: a rename swaps
        // the destination's inode for the staged one, so the mode that lands
        // is the staged file's.
        if let Err(error) = permissions::write_owner_only(&staged, &data) {
            log::warn!("could not stage bookmarks: {error}");
            return;
        }
        if let Err(error) = fs::rename(&staged, &self.path) {
            log::warn!("could not replace bookmarks with the new list: {error}");
            let _ = fs::remove_file(&staged);
        }
    }
```

**The owner-only helper this store must use** (`crates/talaria-shell/src/permissions.rs:75-90`):

```rust
pub fn write_owner_only(path: &Path, data: &[u8]) -> io::Result<()> {
    use io::Write;

    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(FILE_MODE);
    }
    let mut file = options.open(path)?;
    file.write_all(data)?;
    file.flush()?;
    restrict_file(path);
    Ok(())
}
```

The mode is set twice on purpose (`permissions.rs:62-74`): at `open` time, and again after, because
`mode` is ignored for a file that already exists — exactly the stale `.tmp` sibling case.

**Do NOT use `vault.rs`'s `restrict_to_owner`** (`vault.rs:89-101`). It is private, logs at
`log::error!` rather than `log::warn!`, and is deliberately separate because it guards a file holding
*plaintext human passwords*. `agents.json` holds hashes; `permissions` is its helper.

**Random token generation** — the precedent is `vault.rs`'s key/nonce generation
(`crates/talaria-shell/src/vault.rs:258`, `:417`):

```rust
    let key = ChaCha20Poly1305::generate_key(&mut OsRng);
```

with the import at `vault.rs:20`: `use chacha20poly1305::aead::{Aead, KeyInit, OsRng};`. For 32 raw
bytes, `rand` is already a direct dependency of `talaria-shell`
(`crates/talaria-shell/Cargo.toml`) — use `rand::rngs::OsRng`, never a time-seeded PRNG.

**`sha2` availability, checked:** `sha2` is **not** a dependency of `talaria-shell`
(`crates/talaria-shell/Cargo.toml` has no `sha2` line). It *is* already resolved in the committed
lockfile at both `0.10.9` (`Cargo.lock:7651`) and `0.11.0` (`Cargo.lock:7662`), pulled in by the
Servo tree. So adding it is a `Cargo.toml` line against an already-resolved version, not a new
resolution — but it is still a lockfile touch, and C-1 applies.

**Test module shape** (`bookmarks.rs:230-245`) — a `TempPath` guard built from pid + an
`AtomicU64` counter, because `tempfile` is not a dependency:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A bookmarks path in the temp directory that removes itself, and the
    /// `.tmp` sibling [`Bookmarks::save`] writes, when a test ends.
    struct TempPath(PathBuf);
```

The mode assertion helper exists for exactly this: `permissions::mode_of`
(`permissions.rs:142-146`), `#[cfg(all(unix, test))]`. `bookmarks.rs` has 19 tests
(`:289-483`), `settings.rs` has 22 (`:337-611`) — that is the density RESEARCH.md means by "at
`vault.rs` density."

---

### `crates/talaria-shell/src/http.rs` (new; off-thread listener)

**Analog:** `crates/talaria-shell/src/control.rs`. This is the **third** off-main-thread actor, and
it should follow `control::spawn`'s shape, with **one deliberate divergence** noted below.

**The thread-spawn precedent** (`crates/talaria-shell/src/control.rs:78-89`):

```rust
pub fn spawn(proxy: EventLoopProxy<AppEvent>) {
    std::thread::Builder::new()
        .name("talaria-control".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("control socket runtime");
            runtime.block_on(serve(proxy));
        })
        .expect("spawn control thread");
}
```

**Divergence to make explicit in the plan:** `http.rs` uses `new_multi_thread`, not
`new_current_thread`, and a thread named `talaria-http`. Reason (RESEARCH Pattern 3): the control
socket must never be starved by an HTTP request, and `rust-mcp-axum` expects tokio `full`. Also,
unlike `control::spawn` which is unconditional (`crates/talaria-shell/src/main.rs` calls it at
startup), `http::spawn` is called **only when the listener is enabled in settings** — a default
install pays nothing.

**Bind-failure handling: log at `error!` and return, never panic** (`control.rs:103-109`):

```rust
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) => {
            log::error!("control socket bind failed at {}: {error}", path.display());
            return;
        },
    };
```

...followed by the "listening at" line at `control.rs:126`:

```rust
    log::info!("control socket listening at {}", path.display());
```

**Fail-closed-on-a-weaker-boundary precedent** (`control.rs:110-125`) — when the socket cannot be
made owner-only, the listener is *dropped and removed* rather than served. The TCP analogue: if the
listener cannot bind `127.0.0.1` specifically, do not fall back to anything.

**The peer check that does NOT transfer** (`control.rs:154-172`) — quote it in the plan as the thing
being lost:

```rust
/// Accept only peers running as the same OS user as the shell.
/// ...
/// Fails closed. A peer whose credentials the kernel will not vouch for is
/// exactly the case this gate exists for, so a lookup error is a rejection.
fn peer_uid_ok(stream: &UnixStream) -> Result<(), String> {
    let ours = current_uid();
    match stream.peer_cred() {
        Ok(credential) if credential.uid() == ours => Ok(()),
        Ok(credential) => {
            Err(format!("peer uid {} is not ours ({ours})", credential.uid()))
        },
        Err(error) => Err(format!("peer credentials unavailable: {error}")),
    }
}
```

Note `control.rs:132-139`: the check runs **before** the spawn, so a rejected peer never consumes a
session id and gets a closed connection with no hint. The HTTP 401 path should mirror the "say
nothing about what the check was" discipline where the spec permits.

**The request→main-thread→reply round trip to copy** (`control.rs:241-272`):

```rust
            Ok(ClientMessage::Request { id, command }) => {
                let (tx, rx) = oneshot::channel();
                let request = AgentRequest {
                    session_id,
                    client: client.clone(),
                    command,
                    reply: tx,
                };
                if proxy.send_event(AppEvent::Agent(request)).is_err() {
                    break; // Event loop is gone; shell is shutting down.
                }
                let replies = out_tx.clone();
                inflight.spawn(async move {
                    let timeout_secs = command_timeout_secs();
                    let outcome = match tokio::time::timeout(
                        std::time::Duration::from_secs(timeout_secs),
                        rx,
                    )
                    .await
                    {
                        Ok(Ok(outcome)) => outcome,
                        Ok(Err(_)) => Outcome::Error {
                            message: "shell dropped the request".into(),
                        },
                        Err(_) => Outcome::Error {
                            message: format!(
                                "timed out after {timeout_secs}s (script still running?)"
```

The shell-side `impl CommandSink` is *this block*, minus the socket framing: build an `AgentRequest`,
`proxy.send_event(AppEvent::Agent(request))`, await the `oneshot` bounded by
`command_timeout_secs()`. The `AgentRequest` struct is already public
(`crates/talaria-shell/src/control.rs:20-25`):

```rust
pub struct AgentRequest {
    pub session_id: u64,
    pub client: String,
    pub command: Command,
    pub reply: oneshot::Sender<Outcome>,
}
```

**Proxy-clone-before-spawn discipline** (`crates/talaria-shell/src/app.rs:1852-1856`) — the rule the
HTTP thread lives under, because `Shared` is `Rc` and not `Send`:

```rust
            // Cloned out here, before the spawn: `state` is an `Rc<Shared>`
            // and `Rc` is not `Send`, so the closure below can never borrow
            // it. The proxy is the whole of what crosses the thread boundary.
            let proxy = state.event_proxy.clone();
```

The field's docstring (`app.rs:104-114`) already narrates this as "the third holder of an
`EventLoopProxy`"; the HTTP listener makes it the fourth. Update that comment.

---

### `crates/talaria-shell/src/app.rs` (modified; `AppEvent` + `apply_ui_actions` + `parse_agent_url`)

**Analog:** the file itself.

**Adding an `AppEvent` variant** — `DownloadCompleted` is the precedent for a variant raised from off
the main thread (`crates/talaria-shell/src/app.rs:44-83`):

```rust
#[derive(Debug)]
pub enum AppEvent {
    Wake,
    SessionStarted { ... },
    SessionEnded { session_id: u64 },
    Agent(AgentRequest),
    /// One download finished successfully, raised from the background thread
    /// `download()` runs on so the main loop can record it.
    DownloadCompleted {
        path: String,
        filename: String,
        url: String,
        bytes: u64,
        requested_by_agent: bool,
    },
}
```

Phase 4's new variant is the consent request: `/authorize` parks the request off-thread and raises an
event that opens `ChromePanel::Consent`. It carries only owned `String`/`u64` data plus, if a reply
channel is needed, a `oneshot::Sender` (which is why `AgentRequest` has a hand-written `Debug` at
`app.rs:91-98` — `oneshot::Sender` is not `Debug`; copy that trick).

**The `user_event` arm to extend** (`app.rs:908-949`), noting the comment at `:930-933`:

```rust
                // The one arm raised from off the main thread. The store is
                // touched here and only here, because `Shared` is `Rc`-based
                // and the thread that learned about the download cannot hold
                // any of it.
                AppEvent::DownloadCompleted { path, filename, url, bytes, requested_by_agent } => {
                    state.downloads.borrow_mut().append(
                        path, filename, url, bytes, now_ms(), requested_by_agent,
                    );
                    state.window.request_redraw();
                },
```

Every arm ends with `state.window.request_redraw()`; the fn ends with `state.servo.spin_event_loop()`
and `set_wait(event_loop, state)` (`app.rs:946-947`). Match that.

**`Shared` gains an `agents: RefCell<Agents>` field**, documented like its four siblings
(`app.rs:115-134`):

```rust
    pub tabs: RefCell<TabManager>,
    pub sessions: RefCell<BTreeMap<u64, Session>>,
    pub vault: RefCell<Vault>,
    pub history: RefCell<History>,
    pub bookmarks: RefCell<Bookmarks>,
    pub settings: RefCell<Settings>,
    pub downloads: RefCell<Downloads>,
```

**`apply_ui_actions` — where every panel mutation lands** (`app.rs:1253-1262`):

```rust
fn apply_ui_actions(state: &Rc<Shared>, actions: Vec<UiAction>) {
    for action in actions {
        match action {
            UiAction::SwitchMode(mode) => {
                let mut tabs = state.tabs.borrow_mut();
                tabs.mode = mode;
                tabs.sync_visibility();
                drop(tabs);
                state.window.request_redraw();
            },
```

Note the `drop(tabs)` before `request_redraw` — borrows are released before anything that may re-enter.
The Approve / Deny / Revoke arms go here and nowhere else.

**`parse_agent_url` — the hardening this phase adds** (`app.rs:1471-1492`):

```rust
fn parse_agent_url(input: &str) -> Result<Url, String> {
    let parsed = match Url::parse(input) {
        Ok(url) => Ok(url),
        Err(url::ParseError::RelativeUrlWithoutBase) if !input.contains(' ') => {
            Url::parse(&format!("https://{input}"))
        },
        Err(error) => Err(error),
    };
    match parsed {
        Ok(url) if agent_scheme_allowed(&url) => Ok(url),
        Ok(url) => Err(format!(
            "scheme {} is not allowed for agents — use http, https, data:, or about:blank",
            url.scheme()
        )),
        Err(error) => Err(format!("bad url: {error}")),
    }
}
```

Add a host:port refusal for the listener's own origin alongside `agent_scheme_allowed`, worded the
same way — naming the reason so an agent can tell policy from a typo. The refusal-message style to
match is `validate_download_filename` (`app.rs:1510-1523`), whose docstring explains *why* each
refusal is worded separately.

**Test-hook gating — the pattern for any new `TALARIA_TEST_HOOKS=1` affordance**
(`app.rs:1892-1909`):

```rust
        Command::ChromeRects => {
            // Test hook: ... Gated at the point of use on the
            // same variable the `evaluate` crash hook checks.
            //
            // Refused as an unrecognised command rather than as a forbidden
            // one, because that is what it is outside a test run ... what
            // matters is that the refusal says nothing about a feature being
            // withheld.
            if std::env::var("TALARIA_TEST_HOOKS").as_deref() != Ok("1") {
                let _ = reply.send(Outcome::Error { message: "unknown command".into() });
                return;
            }
```

The wire side is `Command::ChromeRects` (`crates/talaria-protocol/src/lib.rs:123`) and
`ResultPayload::ChromeRects { rects, scale }` (`lib.rs:183`), with round-trip tests at
`lib.rs:263-293`. Any Phase 4 hook (e.g. "report the listener's bound address") is minted the same
way — and stays off the MCP surface automatically via the `tools.rs:175` wildcard arm.

---

### `crates/talaria-shell/src/gui.rs` (modified; `Consent` + `Access` panels)

**Analog:** the five existing panels. RESEARCH is right that this is a sixth and seventh instance of
existing machinery, not new machinery.

**The enum to extend** (`crates/talaria-shell/src/gui.rs:26-40`):

```rust
/// Which chrome panel, if any, has replaced the page.
///
/// This generalises what was a single `credentials_open: bool`. Later phases
/// add their own variants (bookmarks, downloads, settings); the panels are
/// mutually exclusive by construction rather than by a set of booleans that
/// could all be true at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromePanel {
    None,
    Credentials,
    History,
    Bookmarks,
    Downloads,
    Settings,
}
```

Name the new variants `Consent` and `Access` — **not** `Agents`, which would collide with
`ViewMode::Agents` (`crate::tabs::ViewMode`, imported at `gui.rs:24`).

**`UiAction` and its "even this goes through the round trip" rule** (`gui.rs:53-68`):

```rust
    /// Store a credential the user typed into the credentials panel. The
    /// panel cannot write the vault itself — a `borrow_mut` inside the egui
    /// closure is the named anti-pattern, so the write leaves as an intent
    /// and lands in `apply_ui_actions` once egui's borrows are released.
    SaveCredential(CredentialEntry),
    /// Show a chrome panel, or [`ChromePanel::None`] to go back to the page.
    /// Even this goes through the round trip: teaching a later reader that
    /// "some" mutations are legal inline is how the anti-pattern comes back.
    SetPanel(ChromePanel),
```

The closest precedent for the new actions is `UiAction::OpenDownload` (`gui.rs:89-103`), because it
is the one action documented as a *security property of where it can be raised from*:

```rust
    /// This is the one action in this enum that starts an external process, so
    /// where it can be raised from is the security property (T-03-04-01): a
    /// Downloads panel button, applied in `apply_ui_actions`, and nowhere
    /// else. No control-socket command reaches it, no `talaria_protocol`
    /// variant carries it, and no MCP tool exists for it ...
    OpenDownload(String),
```

`UiAction::ApproveConsent` / `DenyConsent` / `RevokeClient` need that same paragraph: reachable from
one panel button, applied in `apply_ui_actions`, no protocol variant, no MCP tool.

**Panel-local state resets on every switch — the Approve button must too** (`gui.rs:400-424`):

```rust
    pub fn set_panel(&mut self, panel: ChromePanel) {
        self.panel = panel;
        self.revealed_entry = None;
        self.confirm_clear_history = false;
        self.focus_credentials = panel == ChromePanel::Credentials;
        ...
        self.settings_draft_stale = true;
        ...
        self.download_open_error = None;
    }
```

**Arm-then-confirm, for Revoke** (`gui.rs:1240-1273`):

```rust
                            // Two clicks, in place: no modal exists anywhere
                            // in this chrome and this is not the surface to
                            // introduce the first one. The confirming state
                            // dies with the panel (see `Gui::set_panel`), so
                            // it cannot be completed by a later, unrelated
                            // click.
                            let clear = match confirm_clear_history {
                                true => ui.button(
                                    egui::RichText::new("Confirm clear?")
                                        .color(ui.visuals().error_fg_color),
                                ),
                                false => ui.button("Clear history"),
                            };
                            // Named for which of the two states it is in, so
                            // "the confirm armed" and "the confirm reset" are
                            // observable rather than inferred ...
                            record_rect(
                                chrome_rects.as_mut(),
                                match confirm_clear_history {
                                    true => "history.confirm-clear",
                                    false => "history.clear",
                                },
                                None,
                                clear.rect,
                            );
                            if clear.clicked() {
                                match confirm_clear_history {
                                    true => actions.push(UiAction::ClearHistory),
                                    false => confirm_clear_history = true,
                                }
                            }
```

Note the *two rect names for one control* — that is what lets the e2e suite prove the reset rather
than assume it. `RemoveBookmark` / `DeleteCredential` (`gui.rs:58-61`, `:79-82`) establish the
counter-precedent: a row Trash button is immediate, no confirming click. Decide which Revoke is and
say why.

**Toolbar button + rect, for the Access panel** (`gui.rs:682-700`):

```rust
                    let settings_open = panel == ChromePanel::Settings;
                    let settings_button = ui
                        .add(
                            egui::Button::new(egui_phosphor::regular::GEAR)
                                .selected(settings_open),
                        )
                        .on_hover_text("Settings");
                    record_rect(
                        chrome_rects.as_mut(),
                        "toolbar.settings",
                        None,
                        settings_button.rect,
                    );
                    if settings_button.clicked() {
                        actions.push(UiAction::SetPanel(match settings_open {
                            true => ChromePanel::None,
                            false => ChromePanel::Settings,
                        }));
                    }
```

`ChromePanel::Consent` is transient and raised by an `AppEvent`, so it gets **no** toolbar button —
that is the one way it diverges from all five existing panels.

**Rect recording, and why it is free in production** (`gui.rs:176-204`):

```rust
/// `rects` is `Some` only when the shell was started with
/// `TALARIA_TEST_HOOKS=1` — see [`Gui::chrome_rects`]. The absence of the
/// collection *is* the off switch, so a production frame does not build a
/// name, does not push a rect, and has nothing to hand out.
///
/// `index` names one row of a list panel; `None` is a control there is only
/// ever one of.
fn record_rect(
    rects: Option<&mut Vec<ChromeRect>>,
    name: &str,
    index: Option<usize>,
    rect: egui::Rect,
) {
    let Some(rects) = rects else { return };
    rects.push(ChromeRect {
        name: match index {
            Some(index) => format!("{name}.{index}"),
            None => name.to_owned(),
        },
        ...
```

So `access.revoke.N` comes from `record_rect(..., "access.revoke", Some(index), rect)` — the `.N`
suffix is the macro's, not the call site's.

**Untrusted-label handling — directly applicable to `client_name` on the consent screen.** The
existing helpers are `truncate_chars` (`gui.rs:115-126`, counted in *characters* because
`String::truncate` panics mid-codepoint) and `is_display_unsafe` (`gui.rs:128-...`, control
characters and bidi overrides). `client_name` arrives from DCR — attacker-controlled — and goes
straight onto a consent screen above an Approve button. Run it through the same two.

**Panel body shape for a short form (the Consent panel)** (`gui.rs:1510-1526`):

```rust
            } else if panel == ChromePanel::Settings {
                // The one panel that is not a scrolling list: a short,
                // fixed-height form, so no `ScrollArea` wraps it.
                egui::CentralPanel::default().show(ctx, |ui| {
```

The Access panel is a list → `egui::ScrollArea::vertical()` like Bookmarks (`gui.rs:1277-1281`).

---

### `crates/talaria-shell/src/settings.rs` (modified; listener toggle in `config.json`)

**Analog:** the file itself, whose whole design is "one object, one key" (`settings.rs:1-29`) — this
phase makes it two keys, and the module docstring's claim about the smallest possible shape needs
updating along with it.

**The document write to extend** (`settings.rs:242-248`):

```rust
    pub fn save(&mut self, engine: SearchEngine) {
        self.search_engine = engine;
        let document = serde_json::json!({ "search_engine": self.search_engine.to_json() });
        let Ok(data) = serde_json::to_vec(&document) else {
            log::warn!("settings could not be serialised; this change will not survive a restart");
            return;
        };
```

**Validated, not merely parsed** (`settings.rs:208-224`) — the exact reasoning that applies to a
listener port and bind address read off disk:

```rust
        // Validated, not merely parsed. A template that names no place to put
        // the query would send every search to the same fixed URL, so it is
        // discarded whole ...
        //
        // This is also the gate a hand-edited file meets. A `javascript:` or
        // `file:` template that never went through the panel's Save button
        // degrades to the default here rather than being honoured for having
        // arrived by the back door.
        if !is_valid_template(&engine.url_template) {
            log::warn!( ... );
            return Self::defaults_at(path);
        }
```

**Critical adaptation:** a hand-edited `config.json` naming a non-loopback bind address must be
refused *here*, at load, and degrade to disabled — D-04-04 and C-7. That is the direct analogue of
the `javascript:`/`file:` back-door paragraph. Note the divergence in the degrade direction: the
whole `Settings` degrades to defaults today (`return Self::defaults_at(path)`); a bad listener key
must degrade **that key only** to "off", not silently reset the human's search engine too. Say so.

**A save that cannot reach disk still applies to the session** (`settings.rs:228-241`) — carry that
property to the toggle: turning the listener *off* must take effect immediately even if the write
fails. (Turning it *on* when the write fails is the safer failure; state which you chose.)

---

### `tests/e2e/http_transport_test.py`, `oauth_flow_test.py`, `revocation_test.py` (new)

**Analog:** `tests/e2e/panel_click_test.py` — the newest and richest suite, and the only one that
drives real chrome clicks *and* builds a runtime PATH shim.

**Module docstring convention** (`tests/e2e/panel_click_test.py:1-54`): what the suite pins down as a
bulleted list, then the honest limits, then the standalone/isolation note. Copy the structure; the
"Two honest limits" paragraph (`:36-42`) is the part most suites skip and shouldn't.

**Isolated home + config, written before launch** (`panel_click_test.py:103-118`):

```python
tmp = tempfile.mkdtemp(prefix="talaria-panel-click-e2e-")
config = os.path.join(tmp, ".config")
talaria = os.path.join(config, "talaria")
downloads = os.path.join(tmp, "Downloads")
shim_dir = os.path.join(tmp, "bin")
```

**Per-suite `start()` helper wrapping the harness** (`panel_click_test.py:142-149`):

```python
def start(path=SHIM_PATH, log=subprocess.DEVNULL, hooks="1"):
    """`hooks` is what gates `chrome_rects`; the last section starts a shell
    ...
    return harness.start_shell("about:blank", log=log, rust_log="warn",
                               HOME=tmp, XDG_CONFIG_HOME=config, PATH=path,
```

This is exactly where the OAuth suites seed `config.json` with the listener enabled at a fixed test
port — write the file under `talaria/` *before* calling `start()`. `vault_test.py:58-76` is the
minimal version of the same idea (`make_home` + `write_plaintext` + `start(home, config)`), and
`vault_test.py:162` is the restart-survival precedent the token store needs.

**`stop_shell` that waits for the socket to go** (`panel_click_test.py:151-153`,
`vault_test.py:79-81`) — required whenever a suite restarts the shell, or the next `start_shell`
returns early on a stale socket. The revocation suite restarts; it needs this.

**Real chrome clicks by name** (`tests/e2e/harness.py:215-231`):

```python
def click_rect(name, wid, env, settle=1.5, timeout=8.0):
    """Click the centre of the named chrome control, for real.

    Looks the control up with ``wait_for_rect``, converts its logical centre
    to screen pixels through the window's own origin and scale factor, and
    drives a genuine pointer click at it with ``xdotool`` — the same input
    path a person's mouse takes. Returns the screen point it clicked."""
    rects, scale = wait_for_rect(name, timeout=timeout)
    x, y, width, height = rects[name]
    origin_x, origin_y = window_origin(wid, env)
    point = (str(round(origin_x + (x + width / 2) * scale)),
             str(round(origin_y + (y + height / 2) * scale)))
    subprocess.run(["xdotool", "windowfocus", "--sync", wid], env=env)
    time.sleep(0.3)
    subprocess.run(["xdotool", "mousemove", *point, "click", "1"], env=env)
    time.sleep(settle)
    return point
```

**Assert-absence as well as presence** (`harness.py:189-204`, used at
`panel_click_test.py:454-457`):

```python
    click("downloads.dismiss-error")
    rects, _ = harness.wait_for_rect("downloads.dismiss-error", present=False)
```

`present=False` is how the revocation suite proves the Access panel's row is gone, and how the
consent suite proves the Consent panel closed.

**The hook-gating assertion to extend, not replace** (`panel_click_test.py:463-481`):

```python
    stop_shell(tal)
    tal = start(hooks=None)
    refused = rpc("chrome_rects")
    assert refused["outcome"] == "error", \
        ("chrome_rects answered a shell started without TALARIA_TEST_HOOKS",
         refused)
    assert refused["message"] == "unknown command", \
        ("the refusal advertises the hook rather than denying it exists",
         refused)
    # And the rest of the surface is unaffected — this is a gate on one
    # command, not a shell that stopped answering.
    assert rpc("tabs_list")["outcome"] == "ok", "the gated shell stopped answering"
```

**Local HTTP fixture server, stdlib only** (`panel_click_test.py:55-101`) — a
`http.server.BaseHTTPRequestHandler` subclass with a silenced `log_message`, served on a daemon
thread (`:138`). The OAuth suite's loopback *redirect receiver* is the same class on port 0.

**Registration** (`tests/e2e/run_all.py:67-70`) — the three new suites go in the **standalone**
block, not the shared-shell block above it, because each needs its own `config.json`:

```python
for name in ("keyboard_nav_test", "takeover_test", "download_bounds_test", "vault_test",
             "vault_ui_test", "history_test", "bookmarks_test", "downloads_list_test",
             "panel_click_test", "vault_nobus_test"):
    run(name, [])
```

The docstring at `run_all.py:2-13` lists both phases by name and must be updated in the same edit.
Note `run_all.py:64-66`: `vault_nobus_test` is deliberately last because it rewrites
`XDG_RUNTIME_DIR` — the new suites bind a fixed TCP port, so they must not run concurrently with
each other; keep them sequential in this list.

**Reading an SSE stream** (revocation suite): `mcp_client_test.py` is the precedent for raw
`socket` + `select` reads, and RESEARCH gives the reason — a buffered reader that has timed out once
refuses every later read. Use that, not `urllib`.

---

## Shared Patterns

### Error handling — no `anyhow`/`thiserror`, agent-facing failures are values

**Source:** `crates/talaria-shell/src/control.rs:264-272`, `crates/talaria-mcp/src/tools.rs:142-145`
**Apply to:** `http.rs`, `oauth.rs`, `agents.rs`, `lib.rs`

```rust
    let result = match outcome {
        Outcome::Error { message } => return Err(CallToolError::from_message(message)),
        Outcome::Ok { result } => result,
    };
```

The OAuth error type is a plain enum or a `String` message (C-3). Every failure on the request path
becomes a 400/401/403 response, never a panic (C-4).

### Logging levels

**Source:** `control.rs:106` (`error!` for bind failure), `control.rs:126` (`info!` for listening),
`bookmarks.rs:128` (`warn!` for a degraded store), `control.rs:143` (`debug!` for routine churn)
**Apply to:** `http.rs`, `agents.rs`, `oauth.rs`

`vault.rs` is the one module that logs at `error!` for a permissions failure
(`vault.rs:94`); `permissions.rs:116` logs the same class at `warn!`. `agents.rs` uses
`permissions`, so it warns.

### Ignored results are explicit

**Source:** `app.rs:1875` (`let _ = proxy.send_event(...)`), `control.rs:102`
(`let _ = std::fs::remove_file(&path)`), `bookmarks.rs:225`
**Apply to:** every new module

`app.rs:1871-1874` states the convention outright: a closed receiver means the event loop is already
gone, which is shutdown rather than an error worth reporting from a background thread.

### Comment discipline

**Source:** `//!` module header on every file (`control.rs:1-4`, `socket.rs:1-19`,
`bookmarks.rs` head, `permissions.rs` head); `///` on public items and non-obvious fields
**Apply to:** all new Rust files

Comments in this tree explain *why the alternative was rejected*, not what the line does — see
`bookmarks.rs:203-205` (why `.tmp` is a sibling), `gui.rs:62-64` (why even `SetPanel` round-trips),
`app.rs:1852-1854` (why the proxy is cloned before the spawn). Match that register; a plan that
produces "// write the file" comments is off-house-style.

### Naming

**Source:** `.claude/CLAUDE.md` conventions, visible in `control.rs`'s match arms
**Apply to:** all new Rust

Spell words out: `error` not `e`, `message` not `msg`. Modules are single lowercase words
(`agents.rs`, `http.rs`, `oauth.rs` all conform). Trailing comma after a block match arm
(`control.rs:73`, `app.rs:944`). Declare new modules flat in
`crates/talaria-shell/src/main.rs` — there is no `mod.rs` anywhere.

---

## No Analog Found

Files with no close match in this codebase. The planner should lean on RESEARCH.md's SDK-sourced
patterns for these, and budget accordingly.

| File | Role | Data Flow | Reason |
|------|------|-----------|--------|
| `crates/talaria-shell/src/oauth.rs` | middleware / auth provider | request-response | No `AuthProvider`, no OAuth, no PKCE, no token minting exists anywhere in the tree. The only auth-adjacent code is `peer_uid_ok` (`control.rs:163`), which is a different mechanism entirely. Use RESEARCH.md's `AuthProvider` trait quote and the SDK source it cites. The *shape* of the file (module docstring, `///` on public items, `Result<_, String>` errors, `permissions`-backed persistence via `agents.rs`) still follows the house patterns above. |
| The `CommandSink` trait itself (in `talaria-mcp/src/lib.rs`) | abstraction | — | No trait-based abstraction over a transport exists in this workspace; `WebViewDelegate` (impl'd at `app.rs:1271`) is the only trait impl of consequence and it is Servo's. The trait definition is new design; only its two *implementations* have analogs (`socket.rs:36-61` for stdio, `control.rs:241-272` for the shell). |
| `tests/e2e/revocation_test.py`'s open-stream assertion | test | streaming | No suite in `tests/e2e/` reads a long-lived HTTP stream. `mcp_client_test.py`'s raw-socket-plus-`select` technique on the stdio fd is the closest technique, and is the right one to port, but it is a technique transplant rather than a suite analog. |

---

## Metadata

**Analog search scope:** `crates/talaria-shell/src/`, `crates/talaria-mcp/src/`,
`crates/talaria-protocol/src/`, `tests/e2e/`, workspace and crate `Cargo.toml`, `Cargo.lock`
**Files scanned:** 16 Rust source files (13,595 lines across the workspace and e2e suite), 6 Python
suites, 3 manifests
**Pattern extraction date:** 2026-08-20

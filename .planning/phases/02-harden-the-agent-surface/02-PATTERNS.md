# Phase 2: Harden the Agent Surface - Pattern Map

**Mapped:** 2026-08-15
**Files analyzed:** 13 (9 modified Rust, 1 new Rust-adjacent CI, 3 new Python e2e)
**Analogs found:** 12 / 13

This phase is almost entirely **modification of existing files**, not creation. So "analog" here
usually means *the pattern already living in the same file that the change must match*, and
occasionally a sibling file. Excerpts below are verbatim from the current tree — copy their shape.

## File Classification

| File (M = modify, N = new) | Role | Data Flow | Closest Analog | Match Quality | Decisions |
|---|---|---|---|---|---|
| M `crates/talaria-shell/src/app.rs` — `parse_agent_url` | utility (validation) | transform | `resolve_location` same file `:853-868` | exact | D-01..D-03 |
| M `crates/talaria-shell/src/app.rs` — in-flight tracking | service (state) | request-response | `pending_evals` queue `:98`,`:196-291` | exact | D-04..D-07 |
| M `crates/talaria-shell/src/app.rs` — `broadcast_event` + delegate drops | service (event fan-out) | pub-sub | `pending_captures` deferral `:87`,`:174-193` | exact | D-12, D-13 |
| M `crates/talaria-shell/src/app.rs` — `download()` | utility (file I/O) | streaming/file-I/O | `capture_now`/`encode_screenshot` outcome shape `:150-170` | role-match | D-17..D-19 |
| M `crates/talaria-mcp/src/socket.rs` | service (transport) | request-response → pipelined | `control.rs` writer-task + mpsc `:146-164` | exact (mirror side) | D-08..D-11 |
| M `crates/talaria-shell/src/control.rs` — accept/bind | middleware (auth/IPC) | request-response | `forward_to_running_instance` io::ErrorKind matching `:32-75` | role-match | D-15, D-16 |
| M `crates/talaria-shell/src/control.rs` — `handle_connection` loop | middleware | request-response | writer task `:158-164` (spawn-per-unit) | exact | D-08 |
| M `crates/talaria-protocol/src/lib.rs` — `socket_path()` | config | transform | itself `:15-27` | exact | D-16 |
| M `crates/talaria-shell/src/vault.rs` — `upsert`, import cleanup, chmod check | model (store) | CRUD/file-I/O | `Vault::save` + `matching` `:134-168` | exact | D-21, D-24, D-25 |
| M `crates/talaria-shell/src/gui.rs` — credentials panel + autofill suggestion | component (UI) | event-driven | `egui::Panel::top("toolbar")` `:107-184` + `UiAction` `:24-35` | exact | D-21, D-23 |
| M `crates/talaria-mcp/src/tools.rs` — download description note | config (schema) | request-response | `#[mcp_tool(...)]` blocks `:14-95` | exact | D-20 |
| N `tests/e2e/{scheme_refusal,wedge_fastfail,peer_uid}_test.py` | test | request-response | `tests/e2e/crash_event_test.py` (whole file) | exact | D-01, D-05, D-15 |
| M `tests/e2e/overnight_lock.py` | utility | transform | `harness.py::_pid_alive` `:47-57` | exact | D-28, D-29 |
| N `.github/workflows/ci.yml` | config (CI) | batch | **none — no `.github/` exists** | none | D-26, D-27 |

---

## Pattern Assignments

### `crates/talaria-shell/src/app.rs` — `parse_agent_url` scheme allowlist (D-01..D-03)

**Analog:** `resolve_location` in the same file — proves the agent/human split D-01 depends on.

**Current agent path** (`app.rs:838-849`) — this is the *only* function to change:
```rust
/// Agent-supplied URLs: accept scheme-less hosts ("example.com") by assuming
/// https, but never fall back to a search query — an agent that meant to
/// search should do so explicitly.
fn parse_agent_url(input: &str) -> Result<Url, url::ParseError> {
    match Url::parse(input) {
        Ok(url) => Ok(url),
        Err(url::ParseError::RelativeUrlWithoutBase) if !input.contains(' ') => {
            Url::parse(&format!("https://{input}"))
        },
        Err(error) => Err(error),
    }
}
```

**Human path — DO NOT TOUCH** (`app.rs:851-868`), quoted so the planner can assert it stays intact:
```rust
/// Omnibox behavior: URL if it parses (or looks like a host), search query
/// otherwise.
pub fn resolve_location(input: &str) -> Url {
    let input = input.trim();
    if let Ok(url) = Url::parse(input) {
        if !url.scheme().is_empty() && url.host().is_some() || url.scheme() == "about" {
            return url;
        }
    }
    ...
}
```

**Signature-change ripple.** `parse_agent_url` returns `Result<Url, url::ParseError>` and both
call sites format the error identically. A scheme refusal is not a `ParseError`, so the return
type must become `Result<Url, String>` (message-shaped, per CONVENTIONS "agent-facing failures are
values"). Both call sites (`app.rs:915` `Command::TabsOpen`, `app.rs:949` `Command::Navigate`)
currently read:
```rust
Err(error) => {
    let _ = reply.send(Outcome::Error { message: format!("bad url: {error}") });
},
```
Keep the `bad url: ` prefix for parse failures; use a distinct message naming the scheme for
refusals (Claude's discretion per CONTEXT). `Command::Navigate` uses a tuple match — note the arm
ordering at `:960-966`.

**`about:blank` caveat (D-01).** Popup adoption registers the literal string `"about:blank"`
(`app.rs:1316`) and `notify_url_changed` compares `url.as_str() != "about:blank"` (`:1374`). The
allowlist must admit `about:blank` specifically, not the whole `about:` scheme.

---

### `crates/talaria-shell/src/app.rs` — per-tab in-flight tracking (D-04..D-07)

**Analog:** the pending-queue trio on `Shared`. This is also the *field-comment* style to match.

**Field declaration + doc-comment style** (`app.rs:87-98`):
```rust
    /// Screenshot requests for tabs that are not currently displayed: the
    /// target must be shown and produce a fresh frame before its pixels exist
    /// in the shared framebuffer. Serviced from the event loop (never inside
    /// servo callbacks, which run under a painter borrow).
    pub pending_captures: RefCell<Vec<PendingCapture>>,
    ...
    /// `evaluate` follow-ups that must be issued from the event loop: a
    /// promise result being polled until it settles, or a raw re-run when
    /// the page's CSP forbids the eval-based wrapper. Servo invokes evaluate
    /// callbacks while its evaluator is mutably borrowed, so a follow-up
    /// evaluate can never be started from inside a callback.
    pub pending_evals: RefCell<Vec<PendingEval>>,
```

**Where the fast-fail check goes.** `execute_agent_command` (`app.rs:890-907`) already has two
guard closures; the in-flight guard is a third of the same shape:
```rust
    // Crashed tabs reject page-level commands with a tool error (per SPEC's
    // crash-recovery decision); `navigate` recovers the tab instead.
    let crashed = |tab_id: u64| -> bool {
        state.tabs.borrow().get(tab_id).is_some_and(|t| t.crashed)
    };
```

**D-07 constraint, verifiable in code:** `Command::TabsList` (`:909`), `TabsClose` (`:928`),
`TabsFocus` (`:939`) and `Screenshot` never touch `pending_evals` — they must keep working. Only
the `Command::Evaluate` arm (`:968`) gets the busy check.

**Placement (discretion, per CONTEXT).** `Tab` already carries per-tab mutable flags in `tabs.rs`
alongside `crashed` / `initial_blank_until` (`tabs.rs:44-65`) — a `pub evaluating: bool` there
matches conventions better than a fourth `Shared` queue. Note `Tab`'s field docs explain *why*,
per CONVENTIONS.

**Clearing the flag:** every `PendingEval` terminus in `process_pending_evals` (`app.rs:196-291`)
— the deadline branch `:212-219`, the settled branch `:261`, the unexpected-shape branch `:264`,
the error branch `:269`, and the `RunRaw` branch `:281` — must clear it. Note the re-queue branch
at `:230-239` must *not*.

---

### `crates/talaria-shell/src/app.rs` — deferred-work queue for event drops (D-13)

**Analog:** `pending_captures`, verbatim below. D-13 says: use this, do not invent a second
mechanism.

**Producer side — a Servo callback may only mark state** (`app.rs:1325-1334`):
```rust
    fn notify_new_frame_ready(&self, webview: WebView) {
        // Runs inside servo's painter borrow: only mark state, never paint
        // or toggle visibility here.
        if let Ok(mut pending) = self.pending_captures.try_borrow_mut() {
            for capture in pending.iter_mut().filter(|c| c.webview == webview) {
                capture.ready = true;
            }
        }
        self.window.request_redraw();
    }
```

**Consumer side — drain, then act after the borrow drops** (`app.rs:174-193`). The
`while index < pending.len()` / `remove(index)` split-scope idiom is the house pattern; reuse it:
```rust
    pub fn process_pending_captures(&self) {
        let mut due = Vec::new();
        {
            let mut pending = self.pending_captures.borrow_mut();
            let now = std::time::Instant::now();
            let mut index = 0;
            while index < pending.len() {
                if pending[index].ready || pending[index].deadline <= now {
                    due.push(pending.remove(index));
                } else {
                    index += 1;
                }
            }
        }
        for capture in due {
            let outcome = self.capture_now(&capture.webview, &capture.context, true);
            let _ = capture.reply.send(outcome);
        }
        self.process_pending_loads();
    }
```

**Scheduling hook — a new queue must be added here** (`app.rs:342-349`), or its work never wakes
the loop:
```rust
    /// Earliest deadline among queued captures/loads/evals, for WaitUntil
    /// scheduling.
    pub fn next_capture_deadline(&self) -> Option<std::time::Instant> {
        let captures = self.pending_captures.borrow().iter().map(|c| c.deadline).min();
        let loads = self.pending_loads.borrow().iter().map(|l| l.deadline).min();
        let evals = self.pending_evals.borrow().iter().map(|e| e.next).min();
        [captures, loads, evals].into_iter().flatten().min()
    }
```

**The seven drop sites to convert.** Each is a `let Ok(..) = try_borrow*()` / `if let Ok(..)`
that silently skips:

| Line | Site | What is dropped |
|---|---|---|
| `:369` | `broadcast_event` | every crash/close event, all sessions |
| `:1290-1293` | `request_create_new` | the whole popup (`log::warn!("dropping popup request: tab table busy")`) |
| `:1328` | `notify_new_frame_ready` | capture-ready marks |
| `:1338-1339` | `notify_load_status_changed` | load-complete marks |
| `:1364` | `notify_url_changed` | URL + blank-grace clear |
| `:1385` | `notify_crashed` | the `crashed = true` mark **and** the event |
| `:1398-1401` | `notify_closed` | the close **and** the event (double `try_borrow` → `try_borrow_mut`) |

**Current broadcast (D-12 owner filtering lands here)** (`app.rs:365-376`):
```rust
    /// Push an unsolicited event to every connected control client.
    /// Non-blocking (unbounded channel), so this is safe to call from servo
    /// delegate callbacks.
    pub fn broadcast_event(&self, event: talaria_protocol::Event) {
        if let Ok(sessions) = self.sessions.try_borrow() {
            for session in sessions.values() {
                let _ = session
                    .events
                    .send(talaria_protocol::ServerMessage::Event { event.clone() });
            }
        }
    }
```
(Owner data for the filter: `TabOwner::Agent { session_id, client }`, `tabs.rs:21-25`; sessions
are keyed by the same `session_id` in `RefCell<BTreeMap<u64, Session>>`, `app.rs:71`. Filtering is
a `sessions.get(&session_id)` instead of `values()`. Note `broadcast_event` is also called from
non-delegate paths where the tab is already gone — `app.rs:931` after `tabs.close()` and
`app.rs:803` from `UiAction::CloseTab` — so the owner must be captured *before* the close.)

---

### `crates/talaria-shell/src/app.rs` — `download` bounds (D-17..D-19)

**Analog:** none ideal — it is the only off-thread network path. Closest is the outcome/error
shape of the other leaf helpers.

**Current implementation, complete** (`app.rs:1250-1269`):
```rust
fn download(url: &str, filename: &str) -> Outcome {
    let dir = dirs::download_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let path = dir.join(filename);
    let response = match ureq::get(url).call() {
        Ok(response) => response,
        Err(error) => return Outcome::Error { message: format!("request failed: {error}") },
    };
    let mut reader = response.into_reader();
    let file = match std::fs::File::create(&path) {
        Ok(file) => file,
        Err(error) => return Outcome::Error { message: format!("create failed: {error}") },
    };
    let mut writer = std::io::BufWriter::new(file);
    match std::io::copy(&mut reader, &mut writer) {
        Ok(bytes) => Outcome::Ok {
            result: ResultPayload::Download { path: path.display().to_string(), bytes },
        },
        Err(error) => Outcome::Error { message: format!("write failed: {error}") },
    }
}
```
D-17 replaces `std::io::copy` with a capped copy (`reader.take(max)` plus a post-check, or a
manual loop) and `std::fs::remove_file(&path)` on exceed. D-18 uniquifies before `File::create`.

**Its caller — the detached thread D-19 must bound** (`app.rs:1058-1067`):
```rust
        Command::Download { url, filename } => {
            if filename.contains('/') || filename.contains("..") {
                let _ = reply.send(Outcome::Error { message: "bad filename".into() });
                return;
            }
            std::thread::spawn(move || {
                let outcome = download(&url, &filename);
                let _ = reply.send(outcome);
            });
        },
```

**Env-var knob pattern to copy for `TALARIA_MAX_DOWNLOAD_BYTES`** (`app.rs:1071-1080`) — note the
doc comment stating the relationship to the surrounding timeouts, which ARCHITECTURE calls a
load-bearing ordering:
```rust
/// How long a returned promise may take to settle: just under the control
/// socket's command timeout, so the agent gets our message rather than a
/// generic timeout.
fn promise_wait() -> std::time::Duration {
    let timeout_secs: u64 = std::env::var("TALARIA_COMMAND_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    std::time::Duration::from_secs(timeout_secs.saturating_sub(2).max(1))
}
```
D-19's `ureq` timeout should sit under the command timeout the same way (`ureq::AgentBuilder`
with `.timeout_read(...)`/`.timeout(...)`, since `ureq::get()` is used bare today).

---

### `crates/talaria-mcp/src/socket.rs` — pipelining + retry fix (D-08..D-11)

**Analog:** `control.rs`'s writer task — the shell side already does exactly the split D-08 wants
on the MCP side. Copy its shape (own task + channel + `break` on send failure).

**Analog excerpt — dedicated task draining a channel** (`control.rs:146-164`):
```rust
    let session_id = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
    // Outbound messages (replies AND unsolicited events, e.g. tab_crashed)
    // funnel through one channel so a dedicated writer task can interleave
    // them safely on the socket.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ServerMessage>();
    ...
    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            if write_line(&mut write_half, &message).await.is_err() {
                break;
            }
        }
    });
```

**What D-08 replaces — the whole current `request` + `round_trip`** (`socket.rs:10-46`, `:68-80`):
```rust
pub struct ShellConnection {
    inner: Mutex<Option<Wire>>,
    next_id: AtomicU64,
}

struct Wire {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: BufWriter<tokio::net::unix::OwnedWriteHalf>,
}

impl ShellConnection {
    /// Send one command, await its reply. Connects (with the given client
    /// identity) on first use; reconnects once if the shell restarted.
    pub async fn request(&self, client: &str, command: Command) -> Result<Outcome, String> {
        let mut guard = self.inner.lock().await;          // ← serialises everything
        for attempt in 0..2 {
            if guard.is_none() {
                *guard = Some(connect(client).await?);
            }
            let wire = guard.as_mut().expect("connected above");
            match round_trip(wire, self.next_id.fetch_add(1, Ordering::Relaxed), &command).await {
                Ok(outcome) => return Ok(outcome),
                Err(error) if attempt == 0 => {
                    // Stale connection (shell restarted): drop and retry once. ← D-10 bug
                    *guard = None;
                    let _ = error;
                },
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }
}
```
```rust
async fn round_trip(wire: &mut Wire, id: u64, command: &Command) -> Result<Outcome, String> {
    write_message(wire, &ClientMessage::Request { id, command: command.clone() }).await?;
    loop {
        match read_message(wire).await? {
            ServerMessage::Reply { id: reply_id, outcome } if reply_id == id => {
                return Ok(outcome);
            },
            ServerMessage::Reply { .. } | ServerMessage::Event { .. } | ServerMessage::HelloAck { .. } => {
                // Not ours (event or stray reply) — keep reading.   ← D-11 drops events here
            },
        }
    }
}
```

**Preserve these two things verbatim** — the connect error string is asserted by users/tests, and
the handshake shape is fixed (`socket.rs:48-66`):
```rust
    let stream = UnixStream::connect(&path).await.map_err(|error| {
        format!(
            "cannot reach Talaria at {} ({error}) — is the Talaria browser running?",
            path.display()
        )
    })?;
    ...
    write_message(&mut wire, &ClientMessage::Hello { client: client.to_owned() }).await?;
    match read_message(&mut wire).await? {
        ServerMessage::HelloAck { .. } => Ok(wire),
        other => Err(format!("unexpected handshake reply: {other:?}")),
    }
```

**D-10 retry rule, concretely:** the loop retries on *any* `round_trip` error including a
post-write read failure. Split `round_trip` so the write result and the read result are
distinguishable, and only retry when `connect` failed or the write failed before any byte landed.

**Shell-side counterpart (D-08).** `handle_connection`'s request loop awaits each timeout inline
(`control.rs:166-215`); the fix is to `tokio::spawn` the per-request await-and-reply, since
`out_tx` is already `Clone` and the writer task already interleaves. Excerpt of the body to move:
```rust
                let timeout_secs = std::env::var("TALARIA_COMMAND_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(30);
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
                        ),
                    },
                };
```
Careful: the teardown at `control.rs:217-220` (`SessionEnded`, `drop(out_tx)`, `writer.abort()`)
must not fire while spawned requests are still outstanding.

**D-11 notification sink.** MCP-side, the wire `Event` enum is fixed and needs no change
(`talaria-protocol/src/lib.rs:134-139`):
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    TabCrashed { tab_id: u64 },
    TabClosed { tab_id: u64 },
}
```

---

### `crates/talaria-shell/src/control.rs` + `talaria-protocol/src/lib.rs` — socket auth (D-15, D-16)

**Analog:** `forward_to_running_instance` — the file's own model for OS-level socket handling and
`io::ErrorKind` discrimination.

**Bind site D-16 modifies** (`control.rs:90-101`) — note `let _ = std::fs::remove_file(&path)`
and the trailing-comma-after-block-arm style CONVENTIONS calls out:
```rust
async fn serve(proxy: EventLoopProxy<AppEvent>) {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) => {
            log::error!("control socket bind failed at {}: {error}", path.display());
            return;
        },
    };
    log::info!("control socket listening at {}", path.display());
```

**Accept site D-15 hooks into** (`control.rs:103-117`) — the peer-UID check goes between
`accept()` and `tokio::spawn`; `_addr` is the currently-unused second element:
```rust
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let proxy = proxy.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(stream, proxy).await {
                        log::debug!("control connection ended: {error}");
                    }
                });
            },
            Err(error) => {
                log::warn!("control accept error: {error}");
            },
        }
    }
```
Tokio's `UnixStream::peer_cred()` returns `io::Result<UCred>` with `.uid()` — no new dependency.
Reject by dropping the stream and `log::warn!` (level discipline: a rejected peer is a degraded
condition the user should know about, not an unrecoverable failure).

**Path construction D-16 modifies** (`talaria-protocol/src/lib.rs:13-27`) — note the deliberate
libc shim comment; `getuid` is already available for the per-UID `0700` directory:
```rust
/// Default socket location: `$XDG_RUNTIME_DIR/talaria.sock`, falling back to
/// `/tmp/talaria-$UID.sock`.
pub fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("talaria.sock");
    }
    let uid = unsafe { libc_getuid() };
    PathBuf::from(format!("/tmp/talaria-{uid}.sock"))
}

// Tiny libc shim so we don't pull the libc crate for one call.
extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}
```
**Blast radius warning:** `socket_path()` has three consumers — `control.rs:37` and `:91`, and
`talaria-mcp/src/socket.rs:49` — plus **`tests/e2e/harness.py:21`** and every `*_test.py` that
hardcodes `os.environ.get("XDG_RUNTIME_DIR", "/tmp") + "/talaria.sock"`. Changing the fallback
shape means the Python constant must change too, or the e2e suite must keep setting
`XDG_RUNTIME_DIR` (it does — see harness docstring). `talaria-protocol` depends only on
`serde`/`serde_json`; keep it that way (CONVENTIONS: "deliberately dependency-light"), so the
`0700` dir creation uses `std::fs` + the existing shim, not a new crate.

---

### `crates/talaria-shell/src/vault.rs` — `upsert`, import cleanup, chmod check (D-21, D-24, D-25)

**Analog:** `Vault::save` and `Vault::matching` in the same file.

**`upsert` must match `matching`'s conventions** (`vault.rs:151-168`) — `&self`/`&mut self`
split, URL-host normalisation via `url::Url::parse(&entry.url)`, `to_ascii_lowercase`:
```rust
    /// Entries whose URL host matches `domain` (exact or subdomain).
    pub fn matching(&self, domain: &str) -> Vec<CredentialEntry> {
        let domain = domain.trim().trim_start_matches('.').to_ascii_lowercase();
        self.entries
            .iter()
            .filter(|entry| { /* host == domain || subdomain either way */ })
            .cloned()
            .collect()
    }
```
`upsert(&mut self, entry: CredentialEntry)` should replace on (host, username) match and then call
`self.save()` — `save` is already `&mut self` and already infallible-by-degradation
(`vault.rs:134-149`):
```rust
    pub fn save(&mut self) {
        let Some(cipher) = &self.cipher else { return };
        let Ok(plain) = serde_json::to_vec(&self.entries) else { return };
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let Ok(ciphertext) = cipher.encrypt(&nonce, plain.as_slice()) else { return };
        ...
        if let Err(error) = fs::write(&self.path, data) {
            log::warn!("could not write vault: {error}");
        }
    }
```
Note: `Shared::vault` is `RefCell<Vault>` (`app.rs:72`), so `upsert` is reached via
`state.vault.borrow_mut().upsert(entry)` — mirror the read at `app.rs:1042`
(`state.vault.borrow().matching(&domain)`).

**D-24 import site** (`vault.rs:115-127`) — the plaintext file to delete/rename is `plain_path`:
```rust
            Err(_) => {
                // First run: also accept a plaintext vault.json the user may
                // have hand-written, and encrypt it going forward.
                let plain_path = dir.join("vault.json");
                match fs::read(&plain_path) {
                    Ok(plain) => {
                        log::warn!("importing plaintext vault.json into encrypted vault");
                        serde_json::from_slice(&plain).unwrap_or_default()
                    },
                    Err(_) => Vec::new(),
                }
            },
```
The deletion must happen **after** the `vault.save()` at `:130`, not inside this match arm — the
save is what makes the import durable.

**D-25 chmod site** (`vault.rs:77-81`) — the `let _ =` to check:
```rust
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
    }
```

**Surfacing D-24/D-25 in the UI.** The vault currently has no UI channel at all; both need a
one-shot flag on `Vault` (e.g. `pub imported_plaintext: bool`, `pub key_file_fallback: bool`) read
by `gui.rs`. `Vault` is constructed once at `app.rs:477`.

**Unit-test analog** — the workspace has only three `#[test]`s; this is the vault's
(`vault.rs:171-198`). Construct `Vault` by struct literal with `cipher: None` to test pure logic
without touching disk:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn entry(url: &str) -> CredentialEntry {
        CredentialEntry { url: url.to_owned(), username: "u".into(), password: "p".into(), cookies: Vec::new() }
    }

    #[test]
    fn domain_matching() {
        let vault = Vault { path: PathBuf::new(), cipher: None, entries: vec![...] };
        assert_eq!(vault.matching("google.com").len(), 1);
    }
}
```

---

### `crates/talaria-shell/src/gui.rs` — credentials panel + autofill suggestion (D-21, D-23)

**Analog:** the existing `toolbar` and `strip` panels. **Copy the `UiAction` discipline exactly** —
ARCHITECTURE names mutating shell state inside an egui closure as an explicit anti-pattern.

**The intent enum to extend** (`gui.rs:24-35`):
```rust
pub enum UiAction {
    SwitchMode(ViewMode),
    Go(String),
    NewTab,
    CloseTab(u64),
    SelectTab(u64),
    Back,
    Forward,
    Reload,
    /// Reload a crashed tab: clears the crashed flag and reloads the page.
    ReloadCrashed(u64),
}
```
Add e.g. `SaveCredential(CredentialEntry)`, `DeleteCredential(String)`, `FillCredential(u64, CredentialEntry)`.

**Panel + collect-intent pattern** (`gui.rs:98-118`) — `actions` accumulates inside the closure;
`tabs` is borrowed once at the top and explicitly `drop(tabs)`-ed at `:310` before any
central-panel work:
```rust
    pub fn update(&mut self, shared: &Rc<Shared>) -> Vec<UiAction> {
        let _ = self.rendering_context.make_current();
        let mut actions: Vec<UiAction> = Vec::new();
        ...
        self.context.run(&shared.window, |ctx| {
            let mut tabs = shared.tabs.borrow_mut();
            let mode = tabs.mode;

            egui::Panel::top("toolbar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let me = mode == ViewMode::Me;
                    if ui.selectable_label(me, "Me").clicked() && !me {
                        actions.push(UiAction::SwitchMode(ViewMode::Me));
                    }
```
Reading another `Shared` field inside the closure is already done for sessions (`gui.rs:214`,
`let sessions = shared.sessions.borrow();`) — `shared.vault.borrow()` for the autofill suggestion
follows the same shape. **Do not `borrow_mut()` the vault inside the closure** — that is what
`UiAction::SaveCredential` is for.

**Modal/overlay analog for the credentials panel** — the crash page (`gui.rs:312-325`) is the only
non-strip surface; it shows how a `CentralPanel` is conditionally substituted and how a button
turns into an action:
```rust
            if let Some(tab_id) = crashed_tab {
                // Crashed state replaces the page (same shape as "Aw, Snap").
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() * 0.35);
                        ui.heading("💥 This tab crashed");
                        ui.label("The page's rendering process went away.");
                        ui.add_space(8.0);
                        if ui.button("Reload").clicked() {
                            actions.push(UiAction::ReloadCrashed(tab_id));
                        }
                    });
                });
            }
```
A credentials panel that replaces the page this way avoids the framebuffer-blit interaction at
`:327-349` entirely. Text entry: copy the `egui::TextEdit::singleline` + `.id(egui::Id::new(...))`
+ `response.lost_focus()` / `key_pressed(Enter)` idiom from the location bar (`gui.rs:146-179`).
Icons come from `egui_phosphor::regular::*` (`X`, `PLUS`, `ROBOT`, `PLUGS` are in use).

**Where actions are performed** (`app.rs:775-808`) — the new arms go here, and this file is where
`state.vault.borrow_mut()` is legal:
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
            ...
            UiAction::CloseTab(id) => {
                if state.tabs.borrow_mut().close(id) {
                    state.broadcast_event(talaria_protocol::Event::TabClosed { tab_id: id });
                }
            },
```

**MPL header:** `gui.rs` carries the MPL-2.0 header (derived from servoshell). Edits stay in-file;
if any new file is split out from it, carry the header (CONVENTIONS §Licensing).

---

### `crates/talaria-mcp/src/tools.rs` — download limitation note (D-20)

**Analog:** the existing `#[mcp_tool]` attribute blocks. D-20 is a description edit only.

**Current** (`tools.rs:86-95`):
```rust
#[mcp_tool(
    name = "download",
    description = "Download a URL to the user's downloads directory under the given filename."
)]
#[derive(Debug, ::serde::Deserialize, ::serde::Serialize, JsonSchema)]
pub struct DownloadTool {
    pub url: String,
    /// Plain filename, no directory components.
    pub filename: String,
}
```
**Tone/length analog** — `EvaluateTool`'s description (`tools.rs:56-58`) shows the house style for
documenting a known behavioural limitation inside a tool description: a parenthetical caveat at
the end, e.g. `"(On pages whose CSP forbids eval, scripts still run but promises are not
awaited...)"`. Write D-20's "does not carry the browser session's cookies; downloads behind a
login may fetch the login page" the same way, plus the D-17 size cap.

**If D-11 needs a protocol/tool addition**, CONVENTIONS §Module Design states the full checklist:
new `Command` variant + `ResultPayload` case in `talaria-protocol/src/lib.rs`, handling in
`app.rs`, a `<Name>Tool` in `tools.rs`, and a case in `tool_box!` (`tools.rs:97-110`) **and** in
`dispatch`'s match (`:117-127`).

---

### New e2e suites: `scheme_refusal_test.py`, `wedge_fastfail_test.py`, `peer_uid_test.py`

**Analog:** `tests/e2e/crash_event_test.py` — the shortest complete socket-driven suite. Copy it
whole and swap the assertions. Note CONVENTIONS explicitly sanctions terse variable names here
(`f`, `rid`, `tid`) — opposite of the Rust rule.

```python
#!/usr/bin/env python3
"""Unsolicited events over the control socket: a second client observes
tab_crashed and tab_closed events for a tab it doesn't own. Expects a running
shell with TALARIA_TEST_HOOKS=1."""
import json, os, socket, time

SOCK = os.environ.get("XDG_RUNTIME_DIR", "/tmp") + "/talaria.sock"

def conn(name):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(SOCK)
    f = s.makefile("rw")
    f.write(json.dumps({"type": "hello", "client": name}) + "\n"); f.flush(); f.readline()
    return s, f

rid = 0
def rpc(f, command, **params):
    global rid
    rid += 1
    f.write(json.dumps({"type": "request", "id": rid, "command": command, **params}) + "\n")
    f.flush()
    while True:
        m = json.loads(f.readline())
        if m.get("type") == "reply" and m.get("id") == rid:
            return m

_, actor = conn("event-actor")
tid = rpc(actor, "tabs_open", url="https://example.com/")["result"]["tab"]["tab_id"]
...
assert event == {"type": "event", "event": "tab_crashed", "tab_id": tid}, event
print("CRASH EVENT CHECKS PASSED")
```

Per-suite notes:
- **scheme_refusal:** `rpc(f, "tabs_open", url="file:///etc/hostname")` → assert
  `m["outcome"] == "error"` and the scheme name appears in `m["message"]`. Also assert
  `about:blank` and `https://` still succeed (D-01 allowlist, not blanket refusal).
- **wedge_fastfail:** needs two `evaluate`s in flight, which the synchronous `rpc()` helper cannot
  do. Write both request lines before reading either (the protocol allows it —
  `talaria-protocol/src/lib.rs:5-8`), and assert the second reply arrives well under
  `TALARIA_COMMAND_TIMEOUT_SECS` (`run_all.py` sets it to `3`). This suite is also the pipelining
  (D-08) regression test.
- **peer_uid:** cannot be tested as the same UID. Either assert the socket's mode is `0600`
  (`os.stat(SOCK).st_mode & 0o777 == 0o600`) and the fallback dir is `0700`, or skip the UID half
  with a printed reason when not root. Do not add a dependency to do this.

**Registration** — new suites that reuse the shared shell go in the Phase-1 list in
`tests/e2e/run_all.py:49-55`; suites needing their own shell go in the Phase-2 tuple at `:60`:
```python
    env = dict(os.environ, TALARIA_E2E_OUT=OUT)
    run("control_socket_test", [OUT], env)
    run("crash_recovery_test", [], env)
    run("crash_event_test", [], env)
    ...
for name in ("keyboard_nav_test", "takeover_test"):
    run(name, [])
```
`run_all.py` starts that shared shell with `TALARIA_TEST_HOOKS=1` and
`TALARIA_COMMAND_TIMEOUT_SECS=3` (`:42-43`) — assume both.

---

### `tests/e2e/overnight_lock.py` — PID liveness (D-28, D-29)

**Analog:** `harness.py:47-57`. Copy it verbatim into `overnight_lock.py` (the two files are
independent by design; `overnight_lock.py` imports nothing from `harness`):
```python
def _pid_alive(pid):
    """True for a live process; False for none or a zombie (killed but not
    yet reaped by its parent — still listed in /proc)."""
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("State:"):
                    return "Z" not in line.split()[1]
    except OSError:
        return False
    return False
```

**Insertion point** (`overnight_lock.py:48-57`) — the liveness check goes into the first branch,
so a dead owner falls through to the write instead of printing REFUSED:
```python
def acquire(owner, force=False):
    held = read()
    if held and held.get("owner") != owner and not force:
        print(f"REFUSED: {held.get('branch')} is owned by session {held.get('owner')} "
              f"since {held.get('started')} (pid {held.get('pid')}).")
        print(f"Take it over only if that session is confirmed stopped: "
              f"{sys.argv[0]} acquire {owner} --force")
        return 1
    if held and held.get("owner") == owner:
        print(f"HELD by this session since {held.get('started')} — continuing.")
        return 0
```
Note `held.get("pid")` is `os.getpid()` of the *acquiring* python process (`:64`), which exits
immediately — so the recorded PID is already dead by design. **D-28 requires changing what is
recorded** (the loop's own PID, passed as an argument, or a PPID) before a liveness check means
anything. Flag this to the planner; a naive `kill -0` on the current field would clear every lock.

The module docstring (`:1-23`) is the style to extend: it narrates the incident the file prevents.
Keep that voice.

---

### `.github/workflows/ci.yml` — **no analog** (D-26, D-27)

No `.github/` directory exists anywhere in the repo (verified). Build from the documented
verification commands instead:

**Verification steps, from CONVENTIONS §Linting** — `cargo build --release` plus
`python3 tests/e2e/run_all.py`.

**Xvfb contract the job must satisfy, from `harness.py:1-21`:**
```python
"""Shared launcher for the e2e suites that start their own shell.

Isolation knobs (so a suite can run next to a long soak on the default
display without killing it):

- ``TALARIA_E2E_DISPLAY``  X display to create with Xvfb (default ``:99``).
- ``XDG_RUNTIME_DIR``      inherited by the shell — point it at a private
  directory to get a private control socket (``$XDG_RUNTIME_DIR/talaria.sock``).
"""
REPO = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
DISPLAY = os.environ.get("TALARIA_E2E_DISPLAY", ":99")
BINARY = os.path.join(REPO, "target", "release", "talaria")
SOCK = os.environ.get("XDG_RUNTIME_DIR", "/tmp") + "/talaria.sock"
```
So the job needs: `Xvfb` + `xdpyinfo` (`x11-utils`) + `pkill` (`procps`) on PATH, a writable
`XDG_RUNTIME_DIR`, a **release** build at `target/release/talaria`, and `python3` (stdlib only —
no pip step). `run_all.py` exits with the count of failed suites, so a nonzero exit is the gate.

**D-27's regenerated-lockfile job** must reproduce the pin recorded in CONCERNS:
`cargo update -p primeorder --precise 0.14.0-rc.14` — the scheduled job deletes `Cargo.lock`,
resolves fresh, and is *expected* to fail with the RustCrypto E0277 until the pin is expressed in
`Cargo.toml`. Servo builds are long; caching `~/.cargo` and `target/` is discretionary per
CONTEXT.

---

## Shared Patterns

### Error handling — agent-facing failures are values

**Source:** `crates/talaria-shell/src/app.rs` (throughout), `crates/talaria-shell/src/control.rs:196-204`
**Apply to:** every D-01, D-05, D-15, D-17, D-19 failure path

```rust
Outcome::Error { message: format!("timed out after {timeout_secs}s (script still running?)") }
```
Rules from CONVENTIONS, verified in code: messages are lowercase, no trailing period, cause after
a colon or in parentheses, inline format captures (`{tab_id}`, `{error}`), and they name the
action the agent can take. `app.rs` contains **zero `unwrap()`** — do not add one. No
`anyhow`/`thiserror`. `expect()` only for genuine invariants (`"static url"`, `"serializable"`).

### Reentrancy — never work inside a Servo delegate callback

**Source:** `crates/talaria-shell/src/app.rs:1325-1334` (`notify_new_frame_ready`)
**Apply to:** D-13 (all seven sites), D-04's flag clearing, D-12's owner lookup

Mark state + `self.window.request_redraw()`; act from `process_pending_*`. Never hold a
`borrow_mut()` across a call into Servo. `try_borrow` that *skips* work is the bug being fixed —
`try_borrow` that *defers* work is the fix.

### Ignored results are explicit

**Source:** `control.rs:92`, `app.rs:190`, `:261`
```rust
let _ = std::fs::remove_file(&path);
let _ = capture.reply.send(outcome);
let _ = proxy.send_event(AppEvent::Agent(request));
```
Keep `let _ =` on genuinely fire-and-forget sends. D-25 is the counter-case: a `let _ =` that
should have been checked.

### Env-var knobs, read at point of use with a documented default

**Source:** `control.rs:185-188`, `app.rs:1074-1079`
```rust
std::env::var("TALARIA_COMMAND_TIMEOUT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(30)
```
**Apply to:** D-17's `TALARIA_MAX_DOWNLOAD_BYTES`. Existing knobs: `TALARIA_COMMAND_TIMEOUT_SECS`,
`TALARIA_TEST_HOOKS`, `XDG_RUNTIME_DIR`, `RUST_LOG`, `TALARIA_E2E_DISPLAY`, `TALARIA_E2E_OUT`.
Respect the timeout ordering (ARCHITECTURE): command timeout 30s > `promise_wait()` 28s >
`LOAD_WAIT` 20s > capture deadline 1.5s. D-19's download timeout belongs under the command timeout.

### Log-level discipline

**Source:** CONVENTIONS §Logging, verified across `control.rs` / `vault.rs`
`error!` = unrecoverable subsystem failure (`control socket bind failed`); `warn!` = degraded
fallback the user should know about (all of `vault.rs`; also D-15's rejected peer); `info!` =
lifecycle (`control socket listening at …`, `popup from tab N opened as tab M`); `debug!` =
routine churn. No second logger — Servo's `setup_logging()` owns the global one.

### Formatting — trailing comma after a block match arm

**Source:** `control.rs:93-99`, everywhere
```rust
        Err(error) => {
            log::error!("control socket bind failed at {}: {error}", path.display());
            return;
        },
```
`rustfmt` will not add this; hand-write it. Lines wrap near 100 columns. D-26 runs
`cargo clippy -- -D warnings` — there is no `clippy.toml`, so defaults apply and the phase must
end clippy-clean.

### Import grouping (Rust)

**Source:** `crates/talaria-shell/src/app.rs:9-37`
`std::*` → third-party → `talaria_protocol::{...}` → `crate::*`, blank line between groups. New
dependencies go in the root `[workspace.dependencies]` and are pulled in with
`{ workspace = true }` — never declared directly in a crate manifest.

### Wire-format additions

**Source:** CONVENTIONS §Module Design; `talaria-protocol/src/lib.rs:29-139`
All protocol enums are internally tagged (`type` / `command` / `outcome` / `event`) with
`rename_all = "snake_case"` and `#[serde(flatten)]` payloads; `ResultPayload` is `untagged`. D-09
says no wire change is needed — if a plan finds one, that is a signal to re-read D-09.

---

## No Analog Found

| File | Role | Data Flow | Reason |
|---|---|---|---|
| `.github/workflows/ci.yml` | config (CI) | batch | No `.github/` directory exists; no workflow, packaging, or release config anywhere in the repo. Build from `harness.py`'s environment contract and CONVENTIONS §Linting (both quoted above). |

Partial-analog caveats worth flagging to the planner:

- **`download()` (D-17..D-19)** has no in-repo analog for bounded/cancellable network I/O — it is
  the only `ureq` call site and the only detached `std::thread`. The excerpts above give the
  outcome/error shape and the env-knob shape, but the cancellation handle is new construction.
- **`ShellConnection` background reader (D-08)** — `control.rs`'s writer task is the closest
  shape, but it is the *write* direction. The id→oneshot map is new; CONTEXT leaves its channel
  and map types to Claude's discretion.
- **Credentials panel (D-21)** — the crash `CentralPanel` is the only non-strip egui surface, so
  layout is genuinely new. What is *not* discretionary is the `UiAction` round-trip.

---

## Sequencing Notes for the Planner

Two hard constraints from the code, on top of CONTEXT's stated dependency chain (5→6→7):

1. **`app.rs` merge friction.** Six of nine work items touch `crates/talaria-shell/src/app.rs`
   (1425 lines). The line numbers in this document are **pre-change**; any plan that lands after
   another `app.rs` plan must re-locate by symbol, not by line. CONTEXT rules splitting the file
   out of scope. Roughly disjoint regions: D-01 at `:838-849` + call sites `:915`/`:949`;
   D-04 at `:890-907` + `:968`; D-13 at `:365-376` + `:1282-1410`; D-17 at `:1058-1067` + `:1250-1269`.
   D-13 and D-04 both touch `process_pending_evals` (`:196-291`) — sequence them, do not parallelise.
2. **`socket_path()` change (D-16) ripples into Python.** Three Rust call sites plus
   `tests/e2e/harness.py:21` and the `SOCK = ...` constant repeated in most `*_test.py` files.
   The green-baseline plan should run *before* D-16 so a socket-path break is unambiguous.

`wrap_script` (`app.rs:1088-1115`) is explicitly off-limits this phase (CONTEXT fragility warning);
D-04/D-05 are solved by in-flight tracking outside it.

## Metadata

**Analog search scope:** `crates/talaria-shell/src/`, `crates/talaria-mcp/src/`,
`crates/talaria-protocol/src/`, `tests/e2e/`, repo root (for `.github/`)
**Files read:** 13 source files + 4 planning documents
**Pattern extraction date:** 2026-08-15

# Phase 3: Table-Stakes Browsing - Pattern Map

**Mapped:** 2026-08-20
**Files analyzed:** 12 new/modified files
**Analogs found:** 12 / 12

## File Classification

| New/Modified File | Role | Data Flow | Closest Analog | Match Quality |
|---|---|---|---|---|
| `crates/talaria-shell/src/history.rs` | model/store | file-I/O (append-only) | `crates/talaria-shell/src/vault.rs` | role-match (data flow differs: append-log vs whole-array) |
| `crates/talaria-shell/src/bookmarks.rs` | model/store | CRUD (whole-array rewrite) | `crates/talaria-shell/src/vault.rs` | exact |
| `crates/talaria-shell/src/downloads.rs` | model/store | CRUD (append-on-event) | `crates/talaria-shell/src/vault.rs` | exact |
| `crates/talaria-shell/src/settings.rs` | model/store (single-object) | CRUD | `crates/talaria-shell/src/vault.rs` | role-match (single struct vs `Vec`) |
| `crates/talaria-shell/src/app.rs` (modified: `Shared`, `AppEvent`, `resolve_location`, `download`, `notify_load_status_changed`) | controller/event-loop | event-driven + request-response | itself, prior state (`pending_loads`, `AppEvent`, `Waker`) | exact (extend in place) |
| `crates/talaria-shell/src/gui.rs` (modified: `ChromePanel`, `UiAction`, four `render_*` fns, toolbar buttons) | component | request-response (UiAction round trip) | itself, credentials panel (`credentials_open`, `SetCredentialsPanel`, lines 25-52, 149-160, 225-245, 497-654) | exact |
| `tests/e2e/history_test.py` | test (e2e) | file-I/O / restart-survival | `tests/e2e/vault_test.py` | exact |
| `tests/e2e/bookmarks_test.py` | test (e2e) | CRUD / restart-survival | `tests/e2e/vault_test.py` | exact |
| `tests/e2e/downloads_list_test.py` | test (e2e) | event-driven / restart-survival | `tests/e2e/vault_test.py` (harness/home pattern) + `tests/e2e/download_bounds_test.py` (download mechanics, not read this pass — same suite family) | role-match |
| `tests/e2e/run_all.py` (modified) | test registry / config | batch | itself (existing `for name in (...)` standalone-suite loop, lines 59-64) | exact |
| `crates/talaria-shell/src/history.rs` / `bookmarks.rs` / `downloads.rs` / `settings.rs` unit test blocks | test (unit) | CRUD | `crates/talaria-shell/src/vault.rs` `#[cfg(test)] mod tests` (lines 512-647) | exact |
| `crates/talaria-mcp/src/tools.rs` (verification only — grep, not modified) | n/a | n/a | n/a | n/a (negative check: confirm no `downloads_open`/history/bookmarks tool exists) |

## Pattern Assignments

### `crates/talaria-shell/src/history.rs` (model/store, file-I/O append-only)

**Analog:** `crates/talaria-shell/src/vault.rs` — **follow it except where noted.**

**Module doc-comment pattern** (vault.rs:1-13): every store module opens with a `//!` header stating role, the data-at-rest posture, and what depends on it — mirror this for history, stating explicitly it is plaintext (not a secret) and Me-only.

**`config_dir()` helper** (`crates/talaria-shell/src/vault.rs:79-83`):
```rust
fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("talaria")
}
```
RESEARCH.md recommends extracting this to a shared helper (or duplicating it) — `history.rs` needs its own copy or a shared one; do not import vault's private fn.

**Divergence from vault.rs (deliberate, per RESEARCH.md Pitfalls 1 & 2):**
- vault.rs's `save()` (lines 414-428) does a **whole-array** `fs::write(&self.path, data)` on every mutation — **do not copy this for history.** History is append-only (`history.jsonl`, one JSON object per line via a single `write`/`OpenOptions::append` call), and every write must go through **write-to-temp + `fs::rename`** rather than vault.rs's direct `fs::write`, per RESEARCH.md Pitfall 2 (`fs::write` is not atomic; vault.rs already carries this gap, history must not repeat it because it's written far more often).
- `load()` degrade-never-abort shape to copy verbatim: `fs::read(&path).ok().and_then(|data| serde_json::from_slice(&data).ok()).unwrap_or_default()` (this exact idiom appears in RESEARCH.md's Bookmarks example, itself lifted from vault's `load()`).

**Deferred capture queue** — see Pattern below (shared with app.rs).

**Test density to match:** `crates/talaria-shell/src/vault.rs:512-647`, `#[cfg(test)] mod tests` with one focused test per behavior (`upsert_appends_a_new_entry`, `delete_of_an_absent_pair_changes_nothing`, etc.) — RESEARCH.md explicitly calls out matching this density (12+ tests), not the near-zero coverage elsewhere in the codebase.

---

### `crates/talaria-shell/src/bookmarks.rs` (model/store, CRUD whole-array)

**Analog:** `crates/talaria-shell/src/vault.rs` — this one **is** the direct pattern, encryption stripped.

**`upsert` pattern** (`crates/talaria-shell/src/vault.rs:440-452`):
```rust
pub fn upsert(&mut self, entry: CredentialEntry) {
    let host = entry_host(&entry.url);
    let existing = host.and_then(|host| {
        self.entries.iter().position(|candidate| {
            candidate.username == entry.username
                && entry_host(&candidate.url).is_some_and(|candidate| candidate == host)
        })
    });
    match existing {
        Some(index) => self.entries[index] = entry,
        None => self.entries.push(entry),
    }
    self.save();
}
```
Bookmarks' `upsert`/`remove` (already drafted in RESEARCH.md's Pattern 1 example, `is_bookmarked`/`upsert`/`remove`/`save`) follows this exact shape with `url` as the identity key instead of `(host, username)`.

**`delete` pattern** (`crates/talaria-shell/src/vault.rs:457-470`): `retain` + `len()` diff to report whether anything actually changed, only calling `save()` when `removed` is true — copy verbatim for `Bookmarks::remove`.

**`save()` — apply the same atomic-write upgrade as history** (temp+rename), per RESEARCH.md's recommendation to apply it uniformly even though bookmarks is low-churn; do not blindly copy vault.rs's plain `fs::write` (`vault.rs:414-428`).

---

### `crates/talaria-shell/src/downloads.rs` (model/store, CRUD append-on-event)

**Analog:** `crates/talaria-shell/src/vault.rs`, same shape as bookmarks (whole-array rewrite, RESEARCH.md explicitly places downloads in the "low-churn, vault-style is fine" bucket, not the append-log bucket). Records **every** completed download regardless of `TabOwner` (RESEARCH.md Pitfall 5 — deliberate asymmetry vs. history's Me-only filter). The append point is driven from `AppEvent::DownloadCompleted`, not from a UI action — see the `app.rs` pattern below for the exact call site.

The stored path field must be the **actual written path** returned by `create_unique` (`crates/talaria-shell/src/app.rs:1695`, called at `app.rs:1759` inside `download()`), never a re-derived path from the original requested filename — `create_unique` may have uniquified it (`report (1).pdf`). This is a direct security requirement from RESEARCH.md's Security Domain table (path-traversal mitigation).

---

### `crates/talaria-shell/src/settings.rs` (single-object store, CRUD)

**Analog:** `crates/talaria-shell/src/vault.rs`'s `load()`/`save()` shape, but holding one `Settings` struct instead of `Vec<T>`. First config-file-format file in the project (per CLAUDE.md, this crosses a stated boundary — flag it, don't treat it as routine). `Default for SearchEngine` must reproduce today's hardcoded DuckDuckGo behavior byte-for-byte (RESEARCH.md Code Examples section, `resolve_location`), so a missing `config.json` is a zero-regression case exactly like vault's "no plaintext file, no encrypted file → empty vault" path.

Malformed `url_template` (missing the literal `{query}` placeholder) must degrade to `SearchEngine::default()` rather than panic — mirrors vault's "malformed encrypted blob → treated as absent" degrade-never-abort rule.

---

### `crates/talaria-shell/src/app.rs` (modified)

**Analog:** itself — extend the existing idioms, do not invent parallel ones.

**Deferred pending-queue idiom to copy for history capture** (`crates/talaria-shell/src/app.rs:1882-1917`, `notify_load_status_changed`):
```rust
fn notify_load_status_changed(&self, webview: WebView, status: servo::LoadStatus) {
    let tab_id = match self.tabs.try_borrow() {
        Ok(tabs) => tabs.find_by_webview(&webview),
        Err(_) => {
            log::error!("load-status mark lost: tab table busy");
            None
        },
    };
    if let Some(tab_id) = tab_id {
        match self.pending_loads.try_borrow_mut() {
            Ok(mut pending) => { /* mutate queue */ },
            Err(_) => log::error!("load-status mark lost for tab {tab_id}: pending_loads busy"),
        }
    }
    self.window.request_redraw();
}
```
Add a sibling `pending_history_writes: RefCell<Vec<HistoryWrite>>` field on `Shared` (declared beside `pending_captures`/`pending_loads` at `crates/talaria-shell/src/app.rs:88,93`), pushed to inside this same callback using `try_borrow_mut`/`log::error!` — never `borrow_mut()` directly (re-entrancy constraint). Drain it from the event loop (same call sites `process_pending_captures()` is called from: `app.rs:761,769,803`), filtering `TabOwner::Me` only and reading `webview.url()`/`webview.page_title()` fresh at drain time, not inside the callback.

**`AppEvent` enum to extend** (`crates/talaria-shell/src/app.rs:41-49`):
```rust
pub enum AppEvent {
    Wake,
    SessionStarted { session_id: u64, client: String, events: ... },
    SessionEnded { session_id: u64 },
    Agent(AgentRequest),
}
```
Add `DownloadCompleted { session_owner: TabOwner, path: String, filename: String, url: String, bytes: u64 }` as a fifth variant, handled in the existing `user_event()` match (RESEARCH.md Pattern 3 gives the exact arm to add).

**New cross-thread proxy field** — `Shared` currently holds no `EventLoopProxy<AppEvent>` of its own; only `Waker` (`crates/talaria-shell/src/app.rs:1992`, `pub struct Waker(pub EventLoopProxy<AppEvent>)`) and the control thread's independently created one exist. Populate a new `event_proxy: EventLoopProxy<AppEvent>` field on `Shared` in `resumed()` (`app.rs:692`, where `waker` is already in scope) by cloning `waker.0`, and thread it into `download()`'s `std::thread::spawn` closure so the background thread can call `proxy.send_event(...)` after finishing — the same mechanism `control.rs` already uses cross-thread (see Shared Patterns below), just a new holder.

**`resolve_location` signature change** (`crates/talaria-shell/src/app.rs:1215-1232`, currently):
```rust
pub fn resolve_location(input: &str) -> Url {
    let input = input.trim();
    if let Ok(url) = Url::parse(input) {
        if !url.scheme().is_empty() && url.host().is_some()
            || url.scheme() == "about"
            || url.scheme() == "file"
        { return url; }
    }
    if !input.contains(' ') && input.contains('.') {
        if let Ok(url) = Url::parse(&format!("https://{input}")) { return url; }
    }
    let query: String = url::form_urlencoded::byte_serialize(input.as_bytes()).collect();
    Url::parse(&format!("https://duckduckgo.com/?q={query}")).expect("static url")
}
```
Grows a `&SearchEngine` parameter; every call site passes `&state.settings.borrow().search_engine`. **Do not touch `parse_agent_url`** — CLAUDE.md/SECURITY.md treat the two paths as deliberately separate trust roots; this is a hard constraint, not a style choice.

**`chrome_replaces_page` to generalize** (`crates/talaria-shell/src/app.rs:818-821`):
```rust
let chrome_replaces_page = |state: &Shared| {
    GUI.with_borrow(|gui| gui.as_ref().is_some_and(|gui| gui.credentials_open()))
        || state.tabs.borrow().displayed().is_some_and(|tab| tab.crashed)
};
```
Change `gui.credentials_open()` to `gui.panel_open()` (`panel != ChromePanel::None`) — the one-line generalization RESEARCH.md's Pattern 4 specifies.

---

### `crates/talaria-shell/src/gui.rs` (modified)

**Analog:** itself — the credentials panel is the load-bearing precedent for all four new panels.

**`UiAction` enum to extend** (`crates/talaria-shell/src/gui.rs:25-52`, current tail):
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
    ReloadCrashed(u64),
    SaveCredential(CredentialEntry),
    DeleteCredential { host: String, username: String },
    SetCredentialsPanel(bool),
    DismissVaultNotice,
}
```
Add: `SetPanel(ChromePanel)` (replaces `SetCredentialsPanel(bool)` — a breaking rename, update the one call site at `gui.rs:239`), `ToggleBookmark(String, String)` (url, title), `OpenDownload(String)` (path), `RemoveDownload(String)`, `RemoveBookmark(String)`, `ClearHistory`, `SaveSearchEngine(SearchEngine)`, `DismissDownloadError`.

**Toolbar button pattern to copy per new button** (`crates/talaria-shell/src/gui.rs:225-239`):
```rust
if ui
    .add(
        egui::Button::new(egui_phosphor::regular::KEY)
            .selected(credentials_open),
    )
    .on_hover_text("Credentials (Ctrl+K)")
    .clicked()
{
    actions.push(UiAction::SetCredentialsPanel(!credentials_open));
}
```
Repeat for `CLOCK_COUNTER_CLOCKWISE`/History, `STAR`/Bookmark-toggle, `BOOKMARKS`/Bookmarks-list, `DOWNLOAD`/Downloads, `GEAR`/Settings — icons confirmed present in `egui-phosphor` 0.12 per UI-SPEC.md. Each button computes `if gui.panel() == target { ChromePanel::None } else { target }` before pushing `UiAction::SetPanel`.

**Panel open/close state-reset pattern** (`crates/talaria-shell/src/gui.rs:149-160`):
```rust
pub fn credentials_open(&self) -> bool {
    self.credentials_open
}

pub fn set_credentials_panel(&mut self, open: bool) {
    self.credentials_open = open;
    self.revealed_entry = None;
}
```
Generalizes to `pub fn panel(&self) -> ChromePanel` / `pub fn set_panel(&mut self, panel: ChromePanel)`, resetting panel-local view state (`revealed_entry`, and the new `confirm_clear_history: bool` UI-SPEC.md specifies) exactly the same way on every panel switch.

**Panel body render pattern** (`crates/talaria-shell/src/gui.rs:497-501`, the `credentials_open` gate around the panel body) — each of the four new panels gets its own `render_history`/`render_bookmarks`/`render_downloads`/`render_settings` fn using `egui::CentralPanel::default()` with no custom `Frame`/margin (per UI-SPEC.md's explicit instruction to avoid inventing bespoke margins), `egui::ScrollArea::vertical()` wrapping the list body, and the exact `"Nothing saved yet."`-style empty-state string family already used by the credentials panel (UI-SPEC.md Copywriting Contract references this construction directly).

**Vault one-shot notice / dismiss pattern to copy for the download-open error** (`crates/talaria-shell/src/gui.rs:509-540`, referenced in UI-SPEC.md as the source for the "Couldn't open {filename} — {os error}" + Dismiss row) — read this range directly when implementing; not excerpted here since UI-SPEC.md already names the exact source lines.

---

### `tests/e2e/history_test.py`, `tests/e2e/bookmarks_test.py`, `tests/e2e/downloads_list_test.py`

**Analog:** `tests/e2e/vault_test.py` — restart-survival + isolated-home mechanism.

**`make_home`/`start` pattern** (`tests/e2e/vault_test.py:58-76`, confirmed present):
```python
def make_home(prefix):
    home = tempfile.mkdtemp(prefix=prefix)
    ...
def start(home, config, log=subprocess.DEVNULL):
    return harness.start_shell(..., HOME=home, XDG_CONFIG_HOME=config)
```
Each new suite: start shell → drive navigation/bookmark/download via `TALARIA_TEST_HOOKS=1` control-socket commands → `harness.stop(shell, ...)` → `start()` again with the **same** `home`/`config` → assert the entry survived. `history_test.py` additionally asserts an Agent-owned tab's navigation does **not** appear (BROWSE-01's negative case).

**Docstring convention** (`tests/e2e/vault_test.py:1-33`): open with a `"""docstring"""` stating what the suite pins down and what preconditions it assumes — match this for all three new suites, per project convention (CLAUDE.md/RESEARCH.md both call this out).

---

### `tests/e2e/run_all.py` (modified)

**Analog:** itself — the existing standalone-suite registration loop (`tests/e2e/run_all.py:59-64`):
```python
for name in ("keyboard_nav_test", "takeover_test", "download_bounds_test", "vault_test",
             "vault_ui_test", "vault_nobus_test"):
    run(name, [])
```
Add `"history_test", "bookmarks_test", "downloads_list_test"` to this tuple — each starts its own shell with its own temp `HOME`, matching the existing entries' pattern; do not add them to the Phase-1 socket-driven block above (`run_all.py:44-52`), which shares one shell.

---

## Shared Patterns

### Store module shape (load/save/degrade-never-abort)
**Source:** `crates/talaria-shell/src/vault.rs:79-83` (`config_dir`), `:283` (`load`), `:414-428` (`save`), `:440-470` (`upsert`/`delete`)
**Apply to:** `history.rs`, `bookmarks.rs`, `downloads.rs`, `settings.rs`
**Deviation required:** replace `save()`'s direct `fs::write(&self.path, data)` with write-to-temp (`<path>.tmp`, same directory) + `fs::rename` for **all four** new stores (RESEARCH.md recommends applying this uniformly, not just to history, since it costs nothing extra). Do not carry vault.rs's non-atomic write forward into new code even though vault.rs itself still has it.

### Deferred pending-queue idiom (Servo delegate callback → main-thread write)
**Source:** `crates/talaria-shell/src/app.rs:88,93` (field declarations), `:1882-1917` (`notify_load_status_changed`), `:761,769,803` (drain call sites)
**Apply to:** history capture (`pending_history_writes`)
**Rule:** callbacks only `try_borrow_mut().push(...)` + `log::error!` on failure + `request_redraw()`; all `fs`/store work happens in the drain fn called from the event loop, never inside the callback.

### `UiAction` round trip (no direct `Shared` mutation from egui closures)
**Source:** `crates/talaria-shell/src/gui.rs:25-52` (`UiAction` variants), `:225-239` (button → action push), `apply_ui_actions` in `crates/talaria-shell/src/app.rs`
**Apply to:** every new toolbar button and panel action (`SetPanel`, `ToggleBookmark`, `OpenDownload`, `RemoveDownload`, `RemoveBookmark`, `ClearHistory`, `SaveSearchEngine`)

### `EventLoopProxy<AppEvent>` cross-thread notification
**Source:** `crates/talaria-shell/src/app.rs:1992-1995` (`Waker`), `crates/talaria-shell/src/control.rs:205,249,294` (existing `send_event` cross-thread precedent, not re-read this pass but named directly by RESEARCH.md)
**Apply to:** `download()`'s background thread → `AppEvent::DownloadCompleted` (new third proxy holder on `Shared`)

### Human-only chrome surface (no MCP reachability)
**Source:** `crates/talaria-shell/src/gui.rs:225-231` (comment: "The credentials surface is human-only... no control-socket command reaches it")
**Apply to:** all four new panels and the "open download" action — verify at review time with `grep -c "downloads_open\|open_download" crates/talaria-mcp/src/tools.rs` returning `0`, per RESEARCH.md's Security Domain table.

### Empty-state copy voice
**Source:** UI-SPEC.md Copywriting Contract, `"Nothing saved yet."` (credentials panel precedent)
**Apply to:** `"Nothing visited yet."` / `"Nothing bookmarked yet."` / `"Nothing downloaded yet."`

## No Analog Found

None — every file in scope has a strong analog in this codebase (`vault.rs` for stores, the credentials panel for chrome, `vault_test.py` for e2e, `run_all.py`'s own registration loop). This phase is explicitly additive on top of already-proven mechanisms per RESEARCH.md's own framing.

## Metadata

**Analog search scope:** `crates/talaria-shell/src/` (vault.rs, gui.rs, app.rs), `tests/e2e/` (vault_test.py, run_all.py)
**Files scanned:** 5 source files (vault.rs 647 lines, gui.rs 729 lines, app.rs 2003 lines, vault_test.py 217 lines, run_all.py 72 lines) — targeted non-overlapping reads via grep-located line ranges, no full-file reads beyond vault.rs's header
**Pattern extraction date:** 2026-08-20

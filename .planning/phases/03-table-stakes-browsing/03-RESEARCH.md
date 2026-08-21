# Phase 3: Table-Stakes Browsing - Research

**Researched:** 2026-08-20
**Domain:** Local persistence + egui chrome UI for history/bookmarks/downloads/search-config in a Servo/winit/egui native browser shell
**Confidence:** HIGH (grounded directly in the read codebase for architecture/patterns; MEDIUM on a few Servo-delegate-timing questions flagged below)

## User Constraints

No `CONTEXT.md` exists for this phase — `/gsd-discuss-phase` was not run. There are no locked
decisions to copy verbatim. This research therefore surfaces the real options for every open
question the phase description raised and makes an explicit recommendation for each, so the
planner has something to lock in without a discuss-phase pass. Anywhere this research makes a
judgment call instead of reporting a locked user decision, it is marked as a **Recommendation**,
not a constraint — the planner (or a follow-up `/gsd-discuss-phase`) can override it.

## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| BROWSE-01 | User has local browsing history (URL, title, timestamp) | Storage shape (`history.jsonl`), capture points (`notify_url_changed` + `notify_load_status_changed`), owner filter (Me-only), restart-survival test pattern (§Testing, §Code Examples) |
| BROWSE-02 | User can bookmark and revisit pages | Storage shape (`bookmarks.json`, vault.rs-style whole-array CRUD), toolbar star + panel UI pattern (§Architecture Patterns) |
| BROWSE-03 | Address bar searches a **configurable** default engine, not hardcoded DuckDuckGo | Exact hardcoded location found (`resolve_location`, `crates/talaria-shell/src/app.rs:1231`), config file shape, minimal settings surface (§Question 4) |
| BROWSE-04 | Completed downloads appear in a list and can be opened | Existing `download()` implementation read in full; found it runs on a bare `std::thread::spawn` with no path back to the main thread — new `AppEvent` plumbing required; `xdg-open` risk analysis (§Question 5, §Security Domain) |

## Project Constraints (from CLAUDE.md)

These are binding, not discretionary — verify every plan task against them:

- **No `anyhow`/`thiserror`.** New code returns `Result<T, String>` / `Option` / builds on
  `Outcome::Error { message }`, matching the rest of the shell.
- **No `unwrap()`** in `talaria-shell` code paths. `expect()` is reserved for genuine startup
  invariants only (crypto provider, window creation) — none of this phase's work qualifies.
- **Degrade, never abort, on user-data problems** — the house pattern is `vault.rs`: a malformed
  or unreadable store logs a warning and starts empty/falls back, it never panics or blocks
  startup.
- **Servo/winit/egui are main-thread only.** Delegate callbacks (`notify_url_changed`,
  `notify_load_status_changed`, etc.) may only set flags/push to a `RefCell<Vec<_>>` pending queue
  and call `request_redraw()`. All actual persistence work happens from the event loop, never
  inside a callback.
- **`Gui::` methods return `Vec<UiAction>`**; the GUI never mutates `Shared` directly. Any new
  panel (history/bookmarks/downloads/settings) follows the exact shape `credentials_open` /
  `SetCredentialsPanel` already established.
- **egui/egui-winit/egui_glow are pinned at 0.34.3** to match Servo's own workspace — do not
  propose a version bump for this phase.
- **Verification is `cargo build --release`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, `python3 tests/e2e/run_all.py`.** New Rust unit tests belong beside the store
  modules (mirror `vault.rs`'s `#[cfg(test)] mod tests`); new e2e suites are
  `<feature>_test.py`, registered in `tests/e2e/run_all.py`.
- **`PROJECT.md` planning-artifact policy**: a `PLAN.md` up front is required only for work
  touching the security/protocol surface (`talaria-protocol/`, `control.rs`, `vault.rs`, the MCP
  tool surface, `parse_agent_url`). None of BROWSE-01/02/03 touch that surface (recommended design
  below adds zero new wire types). BROWSE-04 touches it only if the planner chooses to add an
  `AppEvent` variant (shell-internal enum, not `talaria_protocol`) — confirmed below this does
  **not** require a protocol change, so all four plans can go through the lighter SUMMARY.md path
  unless a plan specifically decides to expose something to MCP (not recommended — see §7).

## Summary

Phase 3 is four independent, additive features layered on an already-complete architecture:
per-tab `WebViewDelegate` callbacks feed deferred `RefCell<Vec<_>>` queues that the event loop
drains, `Gui::update` returns `UiAction`s that `apply_ui_actions` applies after egui's borrows
drop, and `vault.rs` is the established template for "small encrypted-or-plain JSON store under
`dirs::config_dir()/talaria/`, loaded once at startup, saved synchronously on every mutation,
degrades rather than aborts." None of the four BROWSE requirements need a new dependency, a new
wire-protocol type, or a new MCP tool. All four are human-only UI surfaces backed by plain
(unencrypted — none of this is a secret) JSON files living beside `vault.enc`.

The one piece of real new plumbing is BROWSE-04: `download()` currently runs on a bare
`std::thread::spawn` with **no path back to the main thread** at all — it replies to the agent (or
the human, via `OpenForUser`) over a `oneshot` channel and nothing else. Recording a completed
download into a listable store requires adding an `EventLoopProxy<AppEvent>` clone to `Shared`
(one does not exist there today, only inside `Waker` and the control thread) and a new
`AppEvent::DownloadCompleted` variant so the background thread can hand the result back to the
main loop, where the write is main-thread-safe. This is new architecture, not reuse of an existing
hook — call it out explicitly in the 03-04 plan.

The other capture point worth flagging: `notify_page_title_changed` is currently a **complete
no-op** (redraw only) — nothing on `Tab` or `Shared` captures title text from it. History needs a
reliable (URL, title, timestamp) triple, and the only place a title is actually read today is
`tab_info()` (`webview.page_title()`, called synchronously from the event loop, never from inside
a callback). The recommended capture design (§Question 2) reads `webview.url()` and
`webview.page_title()` together at `notify_load_status_changed(Complete)`, deferred through a new
pending queue in the same idiom as `pending_loads`/`pending_captures`, filtered to
`TabOwner::Me` tabs only.

**Primary recommendation:** four plain JSON/JSONL files under `dirs::config_dir()/talaria/`
(`history.jsonl`, `bookmarks.json`, `downloads.json`, `config.json`), each owned by a small store
module mirroring `vault.rs`'s load/save/CRUD shape (minus encryption — none of this data is a
secret); one generalized `ChromePanel` enum on `Gui` replacing the single `credentials_open: bool`
so history/bookmarks/downloads/settings each get a `CentralPanel`-takeover view using the exact
pattern already proven for the credentials panel; and no new MCP tools or protocol variants this
phase — agent access to browsing history/bookmarks/downloads is a deliberate v2 question, not a
v1 requirement.

## Architectural Responsibility Map

This app is a native desktop binary, not a multi-tier web stack — the table below adapts the
concept to this codebase's actual layers (main-thread UI/engine loop, background I/O threads, and
local file storage) rather than browser/server/CDN tiers, which don't apply here.

| Capability | Primary Tier | Secondary Tier | Rationale |
|------------|-------------|----------------|-----------|
| History capture (URL/title/timestamp) | Shell main thread (delegate-deferred pending queue) | Local storage (`history.jsonl`) | Servo delegate callbacks are main-thread-only and may not do I/O; the actual write happens once the deferred queue is drained on the event loop |
| History list UI | Shell main thread (egui `CentralPanel`) | — | egui only renders from the main thread |
| Bookmark toggle + list UI | Shell main thread (`UiAction` round trip) | Local storage (`bookmarks.json`) | user-initiated only; no delegate callback involved |
| Search-engine config | Local storage (`config.json`) | Shell main thread (`resolve_location`, read at navigation time) | config is read synchronously wherever the address-bar heuristic runs today |
| Download execution (network fetch) | Background thread (`std::thread::spawn` inside `download()`) | Shell main thread (via new `AppEvent::DownloadCompleted`) | network I/O must stay off the main loop (existing constraint); the completion record must be written from the main thread, which today has **no channel back** from that thread — new plumbing |
| Downloads list UI + "open" action | Shell main thread (egui panel, `std::process::Command::new("xdg-open")`) | — | must stay human-only; must never become an MCP-reachable exec primitive (§Security Domain) |
| Agent (MCP) surface | Deliberately excluded this phase | — | no BROWSE-0x requirement asks for agent-visible history/bookmarks/downloads; adding one now would expand the hardened Phase-2 agent surface without a stated need |

## Standard Stack

### Core

No new external crates are required. Reuse what `Cargo.toml`'s `[workspace.dependencies]` already
pins:

| Library | Version (pinned in `Cargo.toml`) | Purpose here | Why standard (for this codebase) |
|---------|-----------------------------------|---------------|-----------------------------------|
| `serde` / `serde_json` | 1 | Serialize/deserialize the four new store types | Already the only serialization crate in the workspace; `vault.rs`, `talaria-protocol` both use it |
| `dirs` | 5 | Resolve `dirs::config_dir()` | Already used by `vault.rs` (`config_dir()` helper) and `app.rs` (Servo profile dir) for exactly this purpose |
| `url` | 2.5 | Parse/validate URLs for history/bookmark entries, extend `resolve_location` | Already the only URL crate in the workspace |
| `egui` / `egui-winit` / `egui_glow` | 0.34.3 (pinned) | Render the four new panels | Already the chrome framework; version-locked to Servo's workspace, do not touch |
| `egui-phosphor` | 0.12 | Icons for new toolbar buttons (history/bookmark/download/settings) | Already in use for `X`, `ROBOT`, `PLUGS` icons in `gui.rs` |
| `winit` (`EventLoopProxy`) | 0.30.13 | New cross-thread notification for download completion | Already the mechanism `control::spawn` and `Waker` use to reach the main loop from another thread |
| `std::process::Command` | stdlib | Launch `xdg-open` for "open download" | No crate needed; matches the project's minimal-dependency posture |
| `std::time::SystemTime` | stdlib | Timestamps as unix-epoch milliseconds | Avoids adding `chrono`/`time` for a single duration calc; no formatting/timezone crate is in the workspace today |

### Supporting

None needed — this phase is additive UI + local file I/O on top of already-vendored capability.

### Alternatives Considered

| Instead of | Could use | Tradeoff |
|------------|-----------|----------|
| Plain `serde_json` files under `dirs::config_dir()/talaria/` | `rusqlite` (0.40.2 on crates.io `[VERIFIED: crates.io registry via cargo search]`) | A real embedded DB gives indexed search and cheap partial writes, but is a brand-new dependency class for a project whose stated posture is minimal deps (`ureq` for one blocking client, no ORM, no async DB driver anywhere). At personal-daily-driver scale (thousands to low tens-of-thousands of history rows), a capped/pruned JSON(L) file's write cost stays sub-millisecond; SQLite only starts paying for itself past that scale or if full-text search over history becomes a real requirement (v2). Not recommended for v1. |
| Plain JSON files | `sled` (`1.0.0-alpha.124` on crates.io `[VERIFIED: crates.io registry via cargo search]`) | Still alpha after years of development; wrong dependency-maturity risk profile for a project that already accepted Servo itself as the one immature dependency it needed. Do not add a second one. |
| `history.jsonl` append-log | Whole-array `history.json` rewritten on every navigation (the literal `vault.rs` pattern) | Vault entries are few (dozens); history entries accumulate every navigation, so a full-file JSON re-serialize per navigation degrades as the file grows and — because `fs::write` is not atomic (`vault.rs` has this same latent gap; see §Common Pitfalls) — a rewrite interrupted mid-write can corrupt the *entire* history file, whereas a truncated append only loses the last line. Recommend the append-log for history specifically; bookmarks/downloads/config stay low-churn enough that the vault-style whole-array pattern is fine and simpler. |
| `std::process::Command::new("xdg-open")` | the `open` crate (`5.4.1` on crates.io `[VERIFIED: crates.io registry via cargo search]`) | `open` is cross-platform (handles macOS `open`, Windows `ShellExecute` too), which matters once Phase 6 (macOS/Windows) lands. For this Linux-baseline v1 phase, `std::process::Command` + `xdg-open` needs no new dependency and matches the "Linux is the de-facto baseline" constraint. Flag as a candidate to swap for the `open` crate in Phase 6 rather than adding it now for platforms that don't build yet. |

**Installation:** none — no `Cargo.toml` changes required for the recommended design.

**Version verification:** `cargo search` was run directly against the live crates.io registry
(`[VERIFIED: crates.io registry via cargo search]`) for the two alternatives evaluated
(`rusqlite`, `sled`) and the one alternative for downloads (`open`), confirming current versions
as of 2026-08-20. None are recommended for adoption this phase, so no `Cargo.lock` risk (see
`Cargo.toml`'s documented `primeorder` pin fragility) is introduced.

## Package Legitimacy Audit

**No new external packages are introduced by this phase's recommended design.** All storage,
UI, and download-opening work reuses crates already pinned in `[workspace.dependencies]` plus
Rust stdlib (`std::process::Command`, `std::time::SystemTime`). The three alternative crates
evaluated above (`rusqlite`, `sled`, `open`) were registry-checked for version currency only,
as due diligence for the "don't hand-roll, but also don't reach for a dependency casually"
discussion — none are recommended for installation.

**Packages removed due to [SLOP] verdict:** none — none were proposed for installation.
**Packages flagged as suspicious [SUS]:** none.

## Architecture Patterns

### System Architecture Diagram

```
                     ┌─────────────────────────────────────────────┐
                     │              winit event loop                │
                     │            (main thread only)                 │
                     └───────────────┬────────────────┬─────────────┘
                                      │                │
        Servo WebViewDelegate        │                │  user_event(AppEvent)
        callbacks (main thread,      │                │  ── Wake / SessionStarted /
        may ONLY set flags/queue) ───┘                │     SessionEnded / Agent(..)
              │                                        │     + NEW: DownloadCompleted
   notify_url_changed(url) ──┐                          │
   notify_load_status_       │  push to NEW              │
     changed(Complete) ──────┤  pending_history_writes    │
                              │  (RefCell<Vec<_>>,         │
                              │   same idiom as             │
                              │   pending_loads/captures)    │
                              ▼                              │
                     drained each loop tick,                 │
                     on the event loop (NOT the callback):   │
                     read webview.url()+.page_title() fresh, │
                     filter TabOwner::Me only,                │
                     filter url != about:blank,                │
                     write ──────────────────► history.jsonl   │
                                                (append 1 line)  │
                                                                  │
   UiAction round trip (existing pattern) ─────────────────────┐ │
   toolbar star click → UiAction::ToggleBookmark(url,title)    │ │
   apply_ui_actions() on event loop → Bookmarks::upsert/delete │ │
   → write ───────────────────────────► bookmarks.json ────────┘ │
   (whole-array rewrite, vault.rs pattern)                        │
                                                                    │
   Command::Download { url, filename }                             │
     (from MCP agent OR OpenForUser/human) ──► std::thread::spawn ─┤
     runs download() off the main thread,                          │
     replies to caller via oneshot (unchanged),                    │
     THEN sends AppEvent::DownloadCompleted                        │
     over a NEW EventLoopProxy<AppEvent> field on Shared ──────────┘
     (Shared has none today — Waker's proxy is Servo-only)
   → handled in user_event(), on the main thread:
     write ──────────────────► downloads.json (whole-array rewrite)

   Address bar: user types non-URL text
   → UiAction::Go(text) → apply_ui_actions()
   → resolve_location(text, &shared.settings.borrow().search_engine)
   → reads config.json-backed Settings (loaded once at startup,
     mirrors Vault::load(); NOT re-read per keystroke)

   Chrome UI (egui, main thread only):
   Gui { panel: ChromePanel::{None,Credentials,History,Bookmarks,
                              Downloads,Settings} }  ← generalizes
   today's `credentials_open: bool`
   toolbar buttons (new, next to existing credentials button) →
   UiAction::SetPanel(variant) → CentralPanel takeover, same
   `chrome_replaces_page` gate the credentials panel already uses
   for mouse-event routing
```

### Recommended Project Structure

```
crates/talaria-shell/src/
├── app.rs          # unchanged shape; gains: pending_history_writes queue,
│                    # a `settings: RefCell<Settings>` field, an
│                    # `event_proxy: EventLoopProxy<AppEvent>` field,
│                    # AppEvent::DownloadCompleted variant + handling,
│                    # notify_load_status_changed grows a history-capture arm,
│                    # resolve_location signature grows a &SearchEngine param
├── gui.rs           # ChromePanel enum replaces credentials_open: bool;
│                    # four new render_* fns beside the existing credentials
│                    # panel render code; UiAction grows SetPanel/ToggleBookmark/
│                    # OpenDownload/SaveSearchEngine variants
├── tabs.rs          # unchanged (TabOwner::Me filter is read, not modified)
├── vault.rs         # unchanged — the template, not touched
├── history.rs        # NEW: History store (JSONL append-log, load/append/prune)
├── bookmarks.rs       # NEW: Bookmarks store (whole-array JSON, vault.rs-shaped CRUD)
├── downloads.rs        # NEW: Downloads store (whole-array JSON, append-on-complete)
├── settings.rs          # NEW: Settings store (single-object JSON, search engine config)
└── keyutils.rs      # unchanged
```

### Pattern 1: Store module mirrors `vault.rs`, minus encryption

**What:** Each new store (`History`, `Bookmarks`, `Downloads`, `Settings`) is a plain struct
holding an in-memory `Vec<T>` (or single struct, for `Settings`) plus its file `PathBuf`, with
`load()` (called once at `Shared` construction, alongside `Vault::load()`), a mutation method that
persists synchronously, and unit tests in a `#[cfg(test)] mod tests` block — the exact shape
`vault.rs` already establishes, just without `ChaCha20Poly1305`/`keyring` (none of this data is a
secret; it doesn't belong in the encrypted vault).

**When to use:** Every one of the four new stores.

**Example (bookmarks, whole-array pattern — directly modeled on `vault.rs`'s `upsert`/`save`):**
```rust
// Source: pattern read from crates/talaria-shell/src/vault.rs:414-455 (Vault::save, Vault::upsert)
use std::fs;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
    pub created_at_ms: u64,
}

pub struct Bookmarks {
    path: PathBuf,
    entries: Vec<Bookmark>,
}

fn config_dir() -> PathBuf {
    // Same helper vault.rs defines privately; extract to a shared `paths.rs`
    // (or duplicate the four lines — vault.rs's own comment doesn't mind
    // either, but a shared helper avoids drift across five files).
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("talaria")
}

impl Bookmarks {
    pub fn load() -> Self {
        let path = config_dir().join("bookmarks.json");
        let entries = fs::read(&path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        Self { path, entries }
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.entries.iter().any(|b| b.url == url)
    }

    pub fn upsert(&mut self, url: String, title: String, created_at_ms: u64) {
        if !self.is_bookmarked(&url) {
            self.entries.push(Bookmark { url, title, created_at_ms });
            self.save();
        }
    }

    pub fn remove(&mut self, url: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|b| b.url != url);
        let changed = self.entries.len() != before;
        if changed {
            self.save();
        }
        changed
    }

    fn save(&mut self) {
        let Ok(plain) = serde_json::to_vec(&self.entries) else { return };
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // See "Atomic writes" pitfall below: prefer write-to-temp + rename
        // over vault.rs's plain fs::write for anything touched this often.
        if let Err(error) = fs::write(&self.path, plain) {
            log::warn!("could not write bookmarks: {error}");
        }
    }
}
```

### Pattern 2: Deferred capture queue for history (delegate-safe)

**What:** `notify_load_status_changed` already exists and already defers work through
`pending_loads` in exactly this shape. History capture adds a sibling queue rather than doing
anything new architecturally.

**When to use:** BROWSE-01's capture point.

**Example:**
```rust
// Source: pattern read from crates/talaria-shell/src/app.rs:1882-1917
// (notify_load_status_changed) and app.rs:101-143 (pending queue doc comment)

// On Shared:
pub pending_history_writes: RefCell<Vec<HistoryWrite>>,

pub struct HistoryWrite {
    pub tab_id: u64,
}

// Inside notify_load_status_changed, in the existing Complete arm, after the
// existing pending_loads bookkeeping — same try_borrow_mut / log::error!
// degrade-not-panic idiom already used there:
if status == servo::LoadStatus::Complete {
    match self.pending_history_writes.try_borrow_mut() {
        Ok(mut pending) => pending.push(HistoryWrite { tab_id }),
        Err(_) => log::error!("history write lost for tab {tab_id}: queue busy"),
    }
}

// Drained from the event loop (alongside process_pending_captures() /
// process_pending_evals(), called from user_event and window_event's
// RedrawRequested arm — same call sites those two already use):
pub fn process_pending_history_writes(self: &Rc<Self>) {
    let writes: Vec<HistoryWrite> = self.pending_history_writes.borrow_mut().drain(..).collect();
    if writes.is_empty() {
        return;
    }
    let tabs = self.tabs.borrow();
    let mut history = self.history.borrow_mut();
    for write in writes {
        let Some(tab) = tabs.get(write.tab_id) else { continue };
        if !matches!(tab.owner, TabOwner::Me) {
            continue; // agent-driven navigation is not the human's history
        }
        let Some(url) = tab.webview.url() else { continue };
        if url.as_str() == "about:blank" {
            continue;
        }
        let title = tab.webview.page_title().unwrap_or_default();
        history.append(url.to_string(), title);
    }
}
```

### Pattern 3: Cross-thread completion notice via `EventLoopProxy` (new for this codebase)

**What:** `download()` runs on `std::thread::spawn`, which cannot touch `Rc<RefCell<..>>` state on
`Shared` (not `Send`). The existing cross-thread pattern for "something happened off the main
thread, the main loop needs to know" is `EventLoopProxy<AppEvent>::send_event`, used today by
`control::spawn`/`handle_connection` (`crates/talaria-shell/src/control.rs:205,249,294`). `Shared`
itself does not currently hold a proxy — only `Waker` (Servo's own wake mechanism) and the control
thread's independently-created one do. This phase needs a **third** proxy holder.

**When to use:** BROWSE-04's completion → persistence path.

**Example:**
```rust
// Source: pattern read from crates/talaria-shell/src/app.rs:717-720 (waker
// already available in resumed()) and crates/talaria-shell/src/control.rs:78,205
// (existing EventLoopProxy::send_event cross-thread precedent)

// New field on Shared, populated in resumed() where `waker` is already in scope:
pub event_proxy: EventLoopProxy<AppEvent>,
// ... in the Shared { ... } literal at app.rs:735:
event_proxy: waker.0.clone(),

// New AppEvent variant:
pub enum AppEvent {
    // ...existing...
    DownloadCompleted {
        session_owner: TabOwner, // who triggered it: TabOwner::Me or the Agent
        path: String,
        filename: String,
        url: String,
        bytes: u64,
    },
}

// In Command::Download's handler, clone the proxy into the spawned thread
// alongside the existing url/filename/reply capture:
let proxy = state.event_proxy.clone();
let owner = /* resolve from session_id/client, same as other commands do */;
std::thread::spawn(move || {
    let outcome = download(&url, &filename, &reply);
    if let Outcome::Ok { result: ResultPayload::Download { path, bytes } } = &outcome {
        let _ = proxy.send_event(AppEvent::DownloadCompleted {
            session_owner: owner,
            path: path.clone(),
            filename: filename.clone(),
            url: url.clone(),
            bytes: *bytes,
        });
    }
    let _ = reply.send(outcome);
});

// In user_event()'s match arm (app.rs:771-788), alongside the existing four:
AppEvent::DownloadCompleted { path, filename, url, bytes, .. } => {
    state.downloads.borrow_mut().append(path, filename, url, bytes, now_ms());
    state.window.request_redraw();
},
```

### Pattern 4: Generalized chrome panel (extends the proven credentials-panel idiom)

**What:** The credentials panel already proves the exact mechanism needed: a boolean-ish view
state on `Gui` (never on `Shared`), a `SetCredentialsPanel(bool)` `UiAction`, an
`egui::CentralPanel::default().show(ctx, ..)` takeover, and a `chrome_replaces_page` check in
`app.rs` (`crates/talaria-shell/src/app.rs:818-821`) that redirects mouse input away from the
hidden webview while a panel is open. Generalize the boolean to an enum so four more panels reuse
the identical mechanism instead of inventing four more booleans and four more
`chrome_replaces_page` conditions.

**When to use:** All three list-view UI surfaces (history, bookmarks, downloads) plus the search
settings surface (§Question 4).

**Example:**
```rust
// Source: pattern read from crates/talaria-shell/src/gui.rs:53-71 (Gui fields),
// crates/talaria-shell/src/gui.rs:159-163 (set_credentials_panel), and
// crates/talaria-shell/src/app.rs:817-821 (chrome_replaces_page)

pub enum ChromePanel {
    None,
    Credentials,
    History,
    Bookmarks,
    Downloads,
    Settings,
}

// Replaces `credentials_open: bool` on Gui with `panel: ChromePanel`.
// credentials_open() becomes matches!(self.panel, ChromePanel::Credentials).
// UiAction::SetCredentialsPanel(bool) becomes UiAction::SetPanel(ChromePanel).

// chrome_replaces_page in app.rs generalizes from:
//   gui.as_ref().is_some_and(|gui| gui.credentials_open())
// to:
//   gui.as_ref().is_some_and(|gui| gui.panel_open())   // panel != ChromePanel::None
```

### Anti-Patterns to Avoid

- **Writing to a store from inside a `WebViewDelegate` callback.** Every capture point in this
  phase (`notify_url_changed`, `notify_load_status_changed`) runs while Servo holds internal
  borrows. Only push to a `RefCell<Vec<_>>` pending queue there; do the actual `fs::write` from the
  event loop, exactly like `pending_loads`/`pending_captures`/`pending_tab_work` already do.
- **Writing to `Shared` state from the `download()` background thread directly.** It captures no
  `Rc<RefCell<_>>` today by design (`Rc` isn't `Send`) — keep it that way, and cross back to the
  main thread via `AppEvent`, not by adding `Arc<Mutex<_>>` wrapping to `Shared`'s existing
  `RefCell` fields (that would be a much larger, unnecessary architecture change).
  - See BROWSE-04 — `EventLoopProxy` cross-thread notification is new plumbing, not an existing hook.
- **Collapsing `resolve_location` (human path) and `parse_agent_url` (agent path) into one
  function** while wiring in the configurable search engine. `SECURITY.md` states explicitly:
  *"the two paths are deliberately different code... A change that collapses them into one path is
  a change to the trust model, not a refactor."* Adding a search-engine parameter to
  `resolve_location` must not touch `parse_agent_url` at all.
- **Exposing an MCP tool for history/bookmarks/downloads without a stated requirement.** None of
  BROWSE-01..04 ask for agent access; Phase 2 spent its whole scope narrowing the agent surface,
  not widening it. See §Agent Surface below.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Embedded query-able datastore | A custom binary index format for history/bookmarks | Nothing new — plain `serde_json` files, capped/pruned | At this project's actual scale (single-user daily driver, thousands of rows) a custom format is pure risk for zero benefit; `rusqlite` is the honest fallback *if* scale ever demands it, not a hand-rolled index |
| Cross-thread notification primitive | A new channel/mutex-wrapped state for download completion | `EventLoopProxy<AppEvent>`, already the project's established mechanism (`control.rs`, `Waker`) | One more `AppEvent` variant is strictly less new surface than a second synchronization primitive |
| File-type → application resolution for "open download" | Parsing MIME types / extension tables to decide how to launch a file | `xdg-open` (already installed at `/usr/bin/xdg-open`, version 1.1.3, verified on this dev machine) | This is exactly what the XDG desktop-entry spec and `xdg-utils` exist to solve; reinventing it is both more code and less correct than any browser's own "open downloaded file" |
| Atomic file writes | Ad hoc `fs::write` + hope, or a hand-rolled journal | write-to-temp-file + `fs::rename` (same directory, so it's an atomic rename on POSIX) | `vault.rs` itself does *not* do this today (`fs::write(&self.path, data)` directly) — flagged as a real, if pre-existing, gap; don't repeat it for a higher-churn store (history) |

**Key insight:** every "don't hand-roll" item in this phase resolves to "use a mechanism this
codebase (or its OS) already has," not "add a dependency." The project's own stated dependency
aversion is itself the guidance to follow here.

## Common Pitfalls

### Pitfall 1: Full-file JSON rewrite on every navigation degrades with history size

**What goes wrong:** If history is stored as a `Vec<HistoryEntry>` serialized whole (the literal
`vault.rs` pattern) and rewritten on every `Complete` load, the per-navigation write cost grows
linearly with total history size — fine at first, slower every month.

**Why it happens:** `vault.rs`'s pattern was designed for a store with dozens of entries
(credentials), not one that grows on every page visit.

**How to avoid:** Use an append-only `history.jsonl` (one JSON object per line, appended with a
single `write` syscall) instead of a whole-array rewrite. Compact/prune (drop entries past a cap —
recommend a configurable default like the last 5,000 entries or 90 days, mirroring
`TALARIA_MAX_DOWNLOAD_BYTES`'s env-var-tunable-with-sane-default pattern) once at startup, not on
every write.

**Warning signs:** A stopwatch on `notify_load_status_changed`'s Complete arm growing measurably
slower after weeks of real use; a `history.jsonl`/`history.json` file that keeps growing with no
cap.

### Pitfall 2: `fs::write` is not atomic — a crash mid-write can corrupt the whole store

**What goes wrong:** `vault.rs:save()` (`crates/talaria-shell/src/vault.rs:414-428`) calls
`fs::write(&self.path, data)` directly. `fs::write` opens, truncates, and writes in place; a
process killed (power loss, OOM-killer, `kill -9`) mid-write can leave a truncated or zero-length
file. For the vault this is a low-frequency risk (writes happen only on credential save/delete).
For history — written on every completed navigation — the same code path is exercised orders of
magnitude more often, raising the odds of hitting exactly this window over the store's lifetime.

**Why it happens:** No temp-file-plus-rename step exists anywhere in the current codebase's
persistence code.

**How to avoid:** For any store this phase adds that's mutated frequently (history in particular),
write to `<path>.tmp` in the same directory and `fs::rename` over the target — `rename(2)` is
atomic on the same filesystem, so a reader (including this process's own restart) never observes a
partially-written file. Low-churn stores (bookmarks, downloads, settings) are lower risk but cost
nothing extra to protect the same way — recommend applying it uniformly rather than special-casing
which stores get it.

**Warning signs:** A `history.jsonl`/`.json` file that fails to parse after an unclean shutdown
(power loss during a test run, `kill -9` in a script) — should be reproducible in an e2e suite by
killing the shell mid-write with `SIGKILL`.

### Pitfall 3: `notify_page_title_changed` fires independently of `notify_load_status_changed`

**What goes wrong:** A page's `<title>` can update after `Complete` fires (a slow `<title>` tag
late in the document, or a single-page app changing `document.title` after its own initial load).
If history capture only reads `webview.page_title()` once, at the moment `Complete` fires, it can
record an empty or stale title for pages whose real title arrives slightly later.

**Why it happens:** These are two independent delegate callbacks in Servo's `WebViewDelegate`
trait; there's no guaranteed ordering documented in this codebase between them, and neither
callback in `app.rs` currently threads title state anywhere.

**How to avoid (recommendation, flagged `[ASSUMED]` — not verified against Servo's own source this
session):** Read `page_title()` at `Complete` as the primary capture. Optionally, also mark
`notify_page_title_changed` as "the most recent history entry for this tab's URL may need its
title refreshed" and let the drain loop patch the last entry in-memory (and, for the JSONL append
log, either accept the imprecision or re-append a corrected line — a small non-issue at this
data's scale, since duplicate-with-corrected-title lines don't break anything downstream). Do not
block correctness of BROWSE-01 on getting this exactly right — an occasional missing/stale title
in an early build is a cosmetic gap, not a functional one, and is explicitly flagged here so the
plan doesn't over-invest in solving it up front.

**Warning signs:** History entries showing a URL as the title (the existing UI fallback pattern
already seen in `gui.rs`'s tab-strip label: `page_title().filter(|t| !t.is_empty()).unwrap_or_else(||
tab.location.clone())`) more often than expected.

### Pitfall 4: SPA / `pushState` navigations may not produce a fresh `Complete` cycle

**What goes wrong:** A single-page app that changes its URL via `history.pushState` without a full
document reload may or may not re-fire `notify_load_status_changed`/`notify_url_changed` in this
version of Servo — this session did not verify Servo 0.4.0's exact behavior here.

**Why it happens:** Unverified engine behavior; flagged `[ASSUMED]`.

**How to avoid:** Treat `notify_url_changed` (which does fire independently, per `apply_url_change`
already reading it) as the authoritative signal for "the URL changed," and gate the *history write*
specifically on `Complete` firing at least once for that tab. If SPA route changes turn out not to
re-fire `Complete`, in-SPA navigation simply won't generate new history rows for now — an
acceptable v1 gap (real browsers get this from `history.pushState` hooks Servo doesn't expose to
embedders yet, as far as this session found). Log this as an explicit Open Question rather than
guessing at Servo internals.

**Warning signs:** A demo SPA (e.g. a hash-routed test page) producing only one history entry
despite multiple in-app navigations.

### Pitfall 5: Downloads triggered by an agent still land on the human's disk

**What goes wrong:** `Command::Download` is reachable both via the MCP `download` tool (agent) and
via any future human-initiated path. If the downloads *store* only records human-triggered
downloads, an agent-triggered file silently appears on disk with zero visibility in the list the
human is meant to check — a worse security posture than recording everything.

**Why it happens:** It's tempting to mirror the history decision ("agent tabs don't pollute human
history") without noticing the two features have different threat models: history is about *what
the human browsed*, downloads are about *what landed on the human's filesystem*, which matters
regardless of who asked for it.

**How to avoid:** Recommend recording **every** completed download in the downloads list,
regardless of `TabOwner`/session — this is the opposite recommendation from history's Me-only
filter, and that asymmetry is deliberate (see §Question 5 and §Security Domain for the full
reasoning).

**Warning signs:** A downloaded file with no corresponding entry in `downloads.json` after an agent
session — should never happen under the recommended design; write an e2e assertion for it.

## Code Examples

### `resolve_location` gains a configurable search engine (BROWSE-03)

```rust
// Source: exact current hardcoded behavior read at
// crates/talaria-shell/src/app.rs:1206-1231 (resolve_location)

// CURRENT (hardcoded):
// let query: String = url::form_urlencoded::byte_serialize(input.as_bytes()).collect();
// Url::parse(&format!("https://duckduckgo.com/?q={query}")).expect("static url")

// RECOMMENDED — signature grows a &SearchEngine parameter; every call site
// (UiAction::Go in apply_ui_actions, Command::OpenForUser) passes
// &state.settings.borrow().search_engine:
pub struct SearchEngine {
    pub name: String,
    /// Must contain the literal substring "{query}" exactly once.
    pub url_template: String,
}

impl Default for SearchEngine {
    fn default() -> Self {
        // Byte-for-byte the current hardcoded behavior — a fresh install
        // (no config.json yet) behaves identically to today.
        Self {
            name: "DuckDuckGo".into(),
            url_template: "https://duckduckgo.com/?q={query}".into(),
        }
    }
}

pub fn resolve_location(input: &str, engine: &SearchEngine) -> Url {
    let input = input.trim();
    if let Ok(url) = Url::parse(input) {
        if !url.scheme().is_empty() && url.host().is_some()
            || url.scheme() == "about"
            || url.scheme() == "file"
        {
            return url;
        }
    }
    if !input.contains(' ') && input.contains('.') {
        if let Ok(url) = Url::parse(&format!("https://{input}")) {
            return url;
        }
    }
    let query: String = url::form_urlencoded::byte_serialize(input.as_bytes()).collect();
    let resolved = engine.url_template.replacen("{query}", &query, 1);
    Url::parse(&resolved).unwrap_or_else(|_| {
        // A user-edited template that doesn't parse degrades to the default
        // rather than panicking — same "degrade, never abort" house rule
        // vault.rs follows for malformed user data.
        Url::parse(&format!(
            "https://duckduckgo.com/?q={query}"
        )).expect("static url")
    })
}
```

### Config file shape (`config.json`, BROWSE-03's storage half)

```json
{
  "search_engine": {
    "name": "DuckDuckGo",
    "url_template": "https://duckduckgo.com/?q={query}"
  }
}
```

This is the **first** config-file-format file in the project (per `CLAUDE.md`: "no `.env` files, no
config file format... all configuration is environment variables plus CLI argv" was true before
this phase). It lives at `dirs::config_dir()/talaria/config.json`, sibling to `vault.enc`. A
missing file is not an error — `Settings::load()` returns `Settings::default()`, identical to
today's hardcoded behavior, so this is a zero-regression addition.

### Restart-survival e2e pattern (BROWSE-01 success criterion 1) already exists in this repo

```python
# Source: pattern read verbatim from tests/e2e/vault_test.py:73-96 (start/stop_shell)
# and its "restart with the same home directory" section (comment at line 16-19).
# This is the exact mechanism to reuse for a history_test.py / restart assertion —
# no new harness capability is needed, only a new suite file.

def make_home(prefix):
    home = tempfile.mkdtemp(prefix=prefix)
    config = os.path.join(home, ".config")
    os.makedirs(os.path.join(config, "talaria"))
    return home, config

def start(home, config):
    return harness.start_shell("about:blank", rust_log="warn",
                                HOME=home, XDG_CONFIG_HOME=config)

# first run: navigate somewhere, let it record history, stop_shell()
# second run: start() again with the SAME home/config, query history,
# assert the entry from the first run is still present.
```

## State of the Art

Not much churn is relevant here — this is a young, actively-developed project (Phase 2 closed
2026-08-17), not a case of catching up to an ecosystem shift. The one relevant "old vs current"
note:

| Old Approach | Current Approach | When Changed | Impact |
|--------------|-------------------|---------------|--------|
| Address bar always searches DuckDuckGo | Address bar searches a configurable engine | This phase (BROWSE-03) | The one behavior change users will notice; default stays DuckDuckGo so nobody's experience shifts silently |

**Deprecated/outdated:** nothing to deprecate — this phase only adds capability.

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|----------------|
| A1 | `notify_page_title_changed` can fire after `notify_load_status_changed(Complete)`, so a title read only at `Complete` may occasionally be empty/stale | Pitfall 3 | Low — cosmetic only (falls back to URL-as-title, an existing UI pattern elsewhere in `gui.rs`); does not block BROWSE-01's literal success criterion (URL, title, timestamp all present, title may occasionally lag) |
| A2 | A `pushState`/hash-route SPA navigation may not re-fire `Complete` in Servo 0.4.0, so in-SPA navigations might not generate new history rows | Pitfall 4 | Low-medium — under-counts history for SPA-heavy sites; verify during 03-01's implementation with a small local test page before committing to the capture design, or accept the gap explicitly and note it in the plan's Known Limitations |
| A3 | `egui-phosphor` 0.12 has icon constants for history/bookmark/download/settings glyphs (only `X`, `ROBOT`, `PLUGS` were confirmed in use in this codebase) | Architecture Patterns / UI | Low — worst case, pick different available icon names or fall back to text labels; does not affect functional correctness |
| A4 | A 5,000-entry / 90-day default cap for history is a reasonable default | Pitfall 1 | Low — purely a UX tuning knob; can be changed post-hoc since it's read at load time, not baked into the file format |

**If this table is empty:** N/A — see entries above; none block planning, all are flagged for
verification or explicit acceptance during execution.

## Open Questions (RESOLVED)

> All three were resolved during planning (2026-08-20) — see the plans named per question.
> The original analysis is left intact below; only the resolutions are added.

1. **Does Servo 0.4.0's `WebViewDelegate` re-fire `notify_load_status_changed`/`notify_url_changed`
   on an in-page (`pushState`/hash) navigation?**
   - What we know: The trait exposes exactly the seven methods grepped in `app.rs` (`request_navigation`,
     `request_create_new`, `notify_new_frame_ready`, `notify_load_status_changed`,
     `notify_page_title_changed`, `notify_url_changed`, `notify_crashed`, `notify_closed`) — no
     dedicated "history push" callback exists in what this codebase currently implements against.
   - What's unclear: Whether `notify_url_changed` alone (without a `Complete` cycle) is Servo's
     signal for a `pushState` change, in which case history's `Complete`-gated capture would miss
     it entirely.
   - Recommendation: Spend 30 minutes in the first plan (03-01) driving a local test page with
     `history.pushState` under the existing e2e harness and observing which callbacks fire, before
     finalizing the capture-point implementation. Cheap to verify empirically; not cheap to guess
     wrong and rediscover post-ship.
   - **RESOLVED:** taken as recommended. `03-01-PLAN.md` Task 3 drives a real `history.pushState`
     under the e2e harness and records which callbacks fire, deliberately as an observation with
     no hard assert, so the capture design is finalized against measurement rather than a guess.

2. **What is the right default history cap/retention?**
   - What we know: `TALARIA_MAX_DOWNLOAD_BYTES` establishes the project's existing pattern for a
     env-var-tunable, sane-default numeric limit.
   - What's unclear: No stated product requirement for retention length; "Should Have" priority
     suggests this isn't worth over-engineering.
   - Recommendation: A conservative default (5,000 entries) with an env var override
     (`TALARIA_HISTORY_MAX_ENTRIES`, mirroring the download-cap naming convention) is enough; defer
     time-based retention/settings-UI exposure of this knob unless a later phase asks for it.
   - **RESOLVED:** taken as recommended. `03-01-PLAN.md` fixes the default at 5,000 entries with a
     `TALARIA_HISTORY_MAX_ENTRIES` override, mirroring `TALARIA_MAX_DOWNLOAD_BYTES`.

3. **Should the search-engine config be editable only via the file, or also via an in-app settings
   panel?**
   - What we know: Three of the four BROWSE features (history, bookmarks, downloads) get list-view
     panels either way; the marginal cost of a fourth minimal panel (a text field + a few preset
     buttons) is low once the `ChromePanel` generalization (Pattern 4) exists.
   - What's unclear: BROWSE-03's literal success criterion only requires the engine be
     *configurable and used* — it does not require an in-app editor.
   - Recommendation: Ship both — `config.json` as the source of truth (satisfies "minimal correct
     thing" literally) plus a small settings panel that reads/writes the same file (satisfies "real
     daily-driver browser" without requiring users to know the file exists). This is the one area
     genuinely open to discretion; the planner or a `/gsd-discuss-phase` pass could reasonably
     descope the UI half to "file-only for v1" if time is tight — flagging it explicitly rather
     than deciding it unilaterally.
   - **RESOLVED:** ship both, as recommended. `03-UI-SPEC.md` locked the settings panel and
     `03-03-PLAN.md` implements `config.json` as the source of truth plus a panel that reads and
     writes it. The descope-to-file-only option was considered and declined, not skipped.

## Environment Availability

| Dependency | Required By | Available | Version | Fallback |
|------------|--------------|------------|---------|-----------|
| `xdg-open` (xdg-utils) | BROWSE-04 "open a completed download" | ✓ (verified on this dev machine) | 1.1.3 | None chosen — if absent at runtime, `std::process::Command::new("xdg-open").spawn()` fails with `NotFound`; the "open" UI action should surface that as a chrome-level error message (not a panic) rather than assume presence. Given Linux is the stated de-facto baseline and `xdg-utils` ships by default on essentially every desktop Linux distro, this is a low-risk assumption, but the failure path still needs a message, not an `unwrap()`. |
| Rust toolchain | All plans | ✓ | 1.97.1 (README requires ≥1.88) | — |
| `cargo` / crates.io registry reachability | Confirming any dependency additions (none needed) | ✓ | 1.97.1 | — |

**Missing dependencies with no fallback:** none — the recommended design adds zero required
external dependencies.

**Missing dependencies with fallback:** `xdg-open` absence degrades to a chrome-level error
message on the "open" action; everything else in this phase has no external dependency at all.

## Validation Architecture

### Test Framework

| Property | Value |
|----------|-------|
| Framework | Python stdlib `subprocess`/`socket` e2e harness (`tests/e2e/harness.py`) + Rust `#[cfg(test)]` unit tests (`cargo test`) — no third-party test framework in either language, per project convention |
| Config file | none — `tests/e2e/run_all.py` is itself the runner/registry |
| Quick run command | `cargo test -p talaria-shell` (new store unit tests) |
| Full suite command | `cargo build --release && cargo clippy --all-targets -- -D warnings && cargo test && python3 tests/e2e/run_all.py` |

### Phase Requirements → Test Map

| Req ID | Behavior | Test Type | Automated Command | File Exists? |
|--------|----------|-----------|---------------------|--------------|
| BROWSE-01 | A completed navigation on a **Me** tab records (url, title, timestamp) | unit | `cargo test -p talaria-shell history::tests` | ❌ Wave 0 — new `history.rs` module |
| BROWSE-01 | Recorded history **survives a restart** (the literal success criterion) | e2e | `python3 tests/e2e/history_test.py` | ❌ Wave 0 — new suite, reuse `vault_test.py`'s `make_home`/`start`/`stop_shell` pattern verbatim |
| BROWSE-01 | An **Agent**-owned tab's navigation does NOT appear in history | e2e | same `history_test.py`, additional assertion | ❌ Wave 0 |
| BROWSE-02 | Bookmarking a page adds it to the list; it's still there after restart | e2e | `python3 tests/e2e/bookmarks_test.py` | ❌ Wave 0 |
| BROWSE-02 | Bookmarks CRUD (`upsert`/`remove`/`is_bookmarked`) | unit | `cargo test -p talaria-shell bookmarks::tests` | ❌ Wave 0 |
| BROWSE-03 | Typing a non-URL with a **custom** configured engine produces that engine's search URL, not DuckDuckGo | unit | `cargo test -p talaria-shell resolve_location_tests` (extend existing coverage near `resolve_location`, currently untested per this session's grep of `#[test]` occurrences in `app.rs`) | ❌ Wave 0 — `resolve_location` has no unit tests today; this phase should add its first ones alongside the signature change |
| BROWSE-03 | A malformed/edited `config.json` degrades to the default engine, never panics | unit | same test module | ❌ Wave 0 |
| BROWSE-04 | A completed `download` (agent- or human-triggered) appears in the downloads list | e2e | `python3 tests/e2e/downloads_list_test.py` (new — distinct from the existing `download_bounds_test.py`, which only covers the cap/overwrite behavior, not listing) | ❌ Wave 0 |
| BROWSE-04 | "Open" launches `xdg-open` on the recorded path, never on arbitrary agent-supplied input | e2e or manual-only | manual-only, justified: driving a real file-open through `xdotool`/Xvfb and asserting an external app launched is high-effort for low signal; instead assert (a) the UI action carries exactly the stored path (unit-testable) and (b) no MCP tool exists that can trigger it (a `grep` over `tools.rs`, cheap and durable) | ❌ Wave 0 for the unit half; the MCP-absence check can be a one-line `grep -c downloads_open crates/talaria-mcp/src/tools.rs` assertion of `0` wired into CI rather than a Python suite |

### Sampling Rate

- **Per task commit:** `cargo build --release && cargo test -p talaria-shell`
- **Per wave merge:** full suite (`cargo clippy --all-targets -- -D warnings`, `cargo test`,
  `python3 tests/e2e/run_all.py` with the four new suites registered)
- **Phase gate:** full suite green before `/gsd-verify-work`, matching Phase 2's own closing bar

### Wave 0 Gaps

- [ ] `tests/e2e/history_test.py` — covers BROWSE-01 (capture, Me-only filter, restart survival);
      model directly on `tests/e2e/vault_test.py`'s home/restart pattern
- [ ] `tests/e2e/bookmarks_test.py` — covers BROWSE-02
- [ ] `tests/e2e/downloads_list_test.py` — covers BROWSE-04's listing half (distinct from the
      existing `download_bounds_test.py`, which stays as-is)
- [ ] Unit tests inside `history.rs`, `bookmarks.rs`, `downloads.rs`, `settings.rs` — none of these
      files exist yet; each needs its own `#[cfg(test)] mod tests` from the start, per this
      project's actual unit-test convention (`vault.rs` carries 12+ tests; the new stores should
      match that density, not the near-zero coverage elsewhere the codebase's own `TEST-04` v2 item
      flags as a known gap)
- [ ] Unit tests for `resolve_location`'s search-engine parameter — **currently zero** unit tests
      exist for `resolve_location` at all (confirmed by grepping `#[test]` occurrences near it in
      `app.rs`); this phase is the first to touch its signature, so it's also the right time to
      add coverage
- [ ] Register all new suites in `tests/e2e/run_all.py`'s Phase-2 standalone-shell block (alongside
      `keyboard_nav`, `takeover`, `download_bounds`, `vault*`), since each starts its own shell with
      its own temp `HOME`

## Security Domain

### Applicable ASVS Categories

| ASVS Category | Applies | Standard Control |
|----------------|---------|---------------------|
| V2 Authentication | No | This phase adds no new authentication surface; control-socket peer auth (SEC-01) is unchanged and out of scope here |
| V3 Session Management | No | No session concept introduced |
| V4 Access Control | **Yes** | The new "open downloaded file" action must remain human-only chrome, never reachable via `Command`/MCP tool — see below |
| V5 Input Validation | **Yes** | Filenames/URLs already validated at the existing `download()` boundary (`filename.contains('/') || filename.contains("..")` rejected, `crates/talaria-shell/src/app.rs:1461`); the new stores must not weaken that — history/bookmark URLs are stored as opaque strings (never interpreted as filesystem paths), and `config.json`'s `url_template` is validated for the literal `{query}` placeholder and falls back to the default on a malformed value rather than being blindly `Url::parse`d and trusted |
| V6 Cryptography | No | None of this phase's data is a secret (unlike the vault); no encryption is being added or removed |

### Known Threat Patterns for this stack

| Pattern | STRIDE | Standard Mitigation |
|---------|--------|-----------------------|
| Downloads-list "open" action becoming an agent-reachable arbitrary-exec primitive | Elevation of Privilege | **Explicitly flagged, as the phase description requested.** `Command::Download` is already agent-reachable via the MCP `download` tool (`MCP-07`, existing, unchanged). If a future plan adds a `downloads_open`-style MCP tool or wires the "open" `UiAction` to anything the control socket can trigger, an agent could cause an arbitrary local file (subject to the existing filename `/`/`..` rejection, so it's confined to the downloads directory) to be launched with its OS-registered default handler — including a `.desktop` file, which some Linux desktop environments treat specially. **Recommendation: do not add an MCP tool for opening downloads this phase.** Keep the "open" action reachable only from the downloads panel's own egui button, which only `apply_ui_actions` (main-thread, UI-originated `UiAction`s only) can trigger — the same boundary that already keeps `SetCredentialsPanel` and `SaveCredential` human-only. Verify at review time with a one-line `grep -c "downloads_open\|open_download" crates/talaria-mcp/src/tools.rs` returning `0`. |
| Filename/path traversal via a crafted download filename reaching `xdg-open` | Tampering | Already mitigated at the existing `Command::Download` boundary — `filename.contains('/') \|\| filename.contains("..")` is rejected before any file is created (`app.rs:1461`). The downloads-store record should carry the **actual written path** (`create_unique`'s return value, which the uniquifying logic may have renamed to `report (1).pdf`), never a re-derived path from the original filename string, so "open" can never target a path this process didn't itself create. |
| A malformed/attacker-influenceable `config.json` `url_template` producing an unexpected outbound request (e.g. a template with no `{query}` placeholder silently appending nothing, or a `javascript:`/`file:` scheme in the template) | Tampering / Information Disclosure | `resolve_location` already only reaches the human-trust-root path (never `parse_agent_url`'s allowlist), so — per `SECURITY.md`'s explicit trust model — this is treated the same as any other manually-typed address-bar input the human trust root can already do. The one new risk specific to this feature: a `config.json` a user hand-edits (or that a compromised sync/backup tool corrupts) with a broken template should degrade to the default engine rather than producing a malformed/`None` URL that panics or silently no-ops — implemented in the recommended `resolve_location` example above via `unwrap_or_else` fallback to the DuckDuckGo default, matching the vault's "degrade, never abort" rule. |
| History/bookmarks files leaking browsing activity if backed up or synced without the user's awareness | Information Disclosure | Out of scope for this phase specifically — `PROJECT.md`'s stated position is that vault/session state are "deliberately plain local files that work with existing backup tooling," and the same applies to history/bookmarks (neither is more sensitive than the credential vault, which is already unencrypted-adjacent in its own documented degradation path). No new mitigation needed beyond what already exists for `vault.json`'s plaintext-fallback case; just don't claim these files are protected when they aren't. |

## Sources

### Primary (HIGH confidence)

- Direct source read: `crates/talaria-shell/src/app.rs` (2003 lines, read in full across multiple
  passes — `Shared` struct, `AppEvent`, `WebViewDelegate` impl, `resolve_location`,
  `execute_agent_command`, `download()`, `apply_ui_actions`, `handle_browser_shortcut`, startup
  sequence in `resumed()`)
- Direct source read: `crates/talaria-shell/src/vault.rs` (647 lines, read the load/save/upsert/
  delete/verify_written functions and module doc comment in full) — the template this research
  bases every new store on
- Direct source read: `crates/talaria-shell/src/gui.rs` (relevant sections — `UiAction`, `Gui`
  struct, credentials-panel toggle mechanism, toolbar rendering, `CentralPanel` usage)
- Direct source read: `crates/talaria-shell/src/tabs.rs` (`Tab`, `TabOwner`, `TabManager` — full
  struct definitions)
- Direct source read: `crates/talaria-shell/src/control.rs` (grep + targeted reads — `EventLoopProxy`
  cross-thread pattern, `spawn`/`serve`/`handle_connection`)
- Direct source read: `crates/talaria-protocol/src/lib.rs` (full file — `Command`, `Event`,
  `ResultPayload`, `ClientMessage`/`ServerMessage`)
- Direct source read: `crates/talaria-mcp/src/tools.rs` (full file — confirms no history/bookmarks/
  downloads-list MCP tool exists today, and the `tool_box!` mechanism a new tool would need)
- Direct source read: `tests/e2e/vault_test.py`, `tests/e2e/harness.py`, `tests/e2e/run_all.py` (the
  restart-survival and suite-registration patterns)
- `Cargo.toml` (`[workspace.dependencies]`, read directly for pinned versions)
- `.planning/REQUIREMENTS.md`, `.planning/STATE.md`, `.planning/ROADMAP.md`, `.planning/PROJECT.md`,
  `.planning/phases/02-harden-the-agent-surface/deferred-items.md`, `SECURITY.md`,
  `.planning/config.json` (all read directly)
- `[VERIFIED: crates.io registry via cargo search]` — `rusqlite` 0.40.2, `sled` 1.0.0-alpha.124,
  `open` 5.4.1 (ran `cargo search` directly against the live registry, 2026-08-20)
- Local environment probe: `command -v xdg-open` / `xdg-open --version` on this dev machine
  (`/usr/bin/xdg-open`, 1.1.3), `rustc --version` (1.97.1), `cargo --version` (1.97.1)

### Secondary (MEDIUM confidence)

- None used — this research relied entirely on direct codebase inspection and registry queries
  rather than external documentation, since the phase's technical domain (local file persistence,
  egui panels, cross-thread notification) is entirely internal to conventions this codebase has
  already established, not a third-party library's API surface.

### Tertiary (LOW confidence — flagged `[ASSUMED]`, see Assumptions Log)

- Servo 0.4.0's exact `WebViewDelegate` callback firing behavior on `pushState`/hash navigations
  (A2) and the relative timing of `notify_page_title_changed` vs. `notify_load_status_changed`
  (A1) — general browser-engine-implementation knowledge, not verified against Servo's own source
  in this session (Servo's source is not vendored in this repository; `servo = "0.4.0"` is a
  crates.io dependency, and inspecting its internals was out of scope for a codebase-grounded
  research pass). Both are called out explicitly as Open Questions for the executing plan to
  verify empirically against the real engine before finalizing the capture-point implementation.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — zero new dependencies, every recommendation traces to code already in
  this repository
- Architecture: HIGH — every pattern (deferred queues, `UiAction` round trip, `EventLoopProxy`
  cross-thread notification, `vault.rs`-shaped store modules) is read directly from working code
  in this codebase, not inferred from general Rust/Servo knowledge
- Pitfalls: MEDIUM-HIGH — the atomic-write and write-amplification pitfalls are grounded directly
  in reading `vault.rs`'s actual `save()` implementation; the two Servo-callback-timing pitfalls
  (A1/A2) are flagged `[ASSUMED]` and routed to Open Questions rather than presented as verified

**Research date:** 2026-08-20
**Valid until:** 30 days (stable, actively-developed internal architecture; re-verify against
`app.rs`/`vault.rs` if either changes materially before this phase is planned)

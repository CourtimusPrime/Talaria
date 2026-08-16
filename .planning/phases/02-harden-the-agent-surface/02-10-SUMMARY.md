---
phase: 02-harden-the-agent-surface
plan: 10
subsystem: shell-chrome
tags: [ui, egui, credentials, vault, security, e2e]
status: complete
requires:
  - "Vault::upsert / delete / entries / import_notice / key_downgraded / clear_notices (plan 02-09)"
  - "crates/talaria-shell/src/gui.rs UiAction round trip"
  - "crates/talaria-shell/src/app.rs apply_ui_actions, GUI thread-local, handle_browser_shortcut"
  - "tests/e2e/harness.py (start_xvfb, start_shell with extra environment, x_env, SOCK)"
provides:
  - "UiAction::SaveCredential / DeleteCredential / SetCredentialsPanel / DismissVaultNotice"
  - "Credentials panel: add form (masked password), stored list with per-row reveal and delete"
  - "One-shot vault notice surface (plaintext import outcome, keychain downgrade)"
  - "Toolbar autofill suggestion, domain-matched, with clipboard copy controls"
  - "Ctrl+K and a toolbar key button as the only two ways into the panel"
  - "Gui::credentials_open / Gui::set_credentials_panel"
  - "tests/e2e/vault_ui_test.py — input-driven proof of the capture path"
affects:
  - "crates/talaria-shell/src/gui.rs"
  - "crates/talaria-shell/src/app.rs"
  - "crates/talaria-shell/src/vault.rs (seven #[allow(dead_code)] removed)"
  - "tests/e2e/run_all.py (phase-2 tuple, now 14 suites)"
tech-stack:
  added: []
  patterns:
    - "View state that the egui closure must see travels through the frame as locals and is written back after, because the closure cannot capture `self`"
    - "Even a view-only toggle (panel open/closed) goes through the UI-intent round trip, so no reader is taught that some mutations are legal inline"
    - "A chrome surface that replaces the page must also claim pointer input below the toolbar, or its buttons are unreachable"
    - "Secrets are masked by default and revealed one at a time; a reveal never outlives the list it indexes into"
    - "e2e input is driven from a focus point the application itself sets, so assertions do not depend on pixel layout"
key-files:
  created:
    - "tests/e2e/vault_ui_test.py"
  modified:
    - "crates/talaria-shell/src/gui.rs"
    - "crates/talaria-shell/src/app.rs"
    - "crates/talaria-shell/src/vault.rs"
    - "tests/e2e/run_all.py"
decisions:
  - "The panel's open state is an intent, not an inline field write — a single sanctioned exception is how the anti-pattern returns"
  - "A typed bare hostname is completed to https before it becomes an entry, otherwise it can never be matched or deleted again"
  - "Only one stored password is revealed at a time, and a delete clears the reveal, because the index it holds is about to renumber"
  - "The pointer-routing fix covers the crashed-tab page too — the same bug made its Reload button unclickable"
  - "The e2e suite drives the panel from the focus point the panel sets, not from row coordinates, because a row's y depends on whether this machine has a keychain"
  - "CRED-02 and CRED-03 both marked complete; the CRED-02 flagged assumption (suggest, not inject) stands as planned"
metrics:
  duration: "50m"
  completed: 2026-08-16
  tasks: 3
  files_changed: 5
  commits: 3
---

# Phase 02 Plan 10: Credentials Panel and Toolbar Autofill Suggestion Summary

Gave the vault a user. A credential typed into a panel inside Talaria now persists to the
encrypted store and comes back out of the same `cookies_read` an agent uses, and a stored entry
for the host the human is looking at is offered back in the toolbar — as an offer, never as a
write into the page.

## What Was Built

**Task 1 — the panel and its intents** (`128d3ff`)

Four `UiAction` variants, and nothing in the chrome writes shell state directly:

- `SaveCredential(CredentialEntry)` — the add form's output.
- `DeleteCredential { host, username }` — a **bare host**, which is what `Vault::delete` keys on
  (02-09's contract note). The panel parses it out of the row's URL.
- `SetCredentialsPanel(bool)` — even the panel's own open/closed state. A field on `Gui` could
  have been flipped inline from the closure and nothing would have broken this week; a single
  sanctioned exception is how "the GUI never mutates shared state directly" stops being true.
- `DismissVaultNotice` — clears 02-09's two one-shot notices.

The closure cannot see `self` (its `context` field is borrowed for the whole frame), so the
panel's view state travels in and out as locals and is written back after `run` returns — the
shape `select_location` already used. `revealed_entry`, the three draft strings and the
focus-request flag all ride that path.

The panel is a `CentralPanel` that replaces the page, in the crashed-tab page's shape, so the
framebuffer blit is skipped entirely while it is up. It holds:

- **The notices**, at the top. The import notice says which of the two outcomes the user got —
  removed, or *"a readable plaintext copy of these passwords is still on disk there"*. The
  keychain notice says the key now sits in an owner-only file beside the encrypted vault and what
  that costs. This is the user-facing surface 02-09 added those fields for.
- **An add form**: site, username, password, the last masked with `.password(true)` as it is
  typed. Save is enabled only when all three are non-empty, and Enter in the password field is
  equivalent, following the location bar's lost-focus-then-Enter idiom. The drafts clear on save.
- **A stored list**: host, username, a masked password, a per-row reveal toggle and a delete. Only
  one row can be revealed at a time and a reveal never survives the panel closing — a list that
  shows every password at a glance is precisely the shoulder-surfing surface the encryption was
  meant to avoid, and the reasoning sits in a comment at the control.

Two entry points, both human: a key-glyph toolbar button and **Ctrl+K**. No command, tool or
protocol variant was added; `git diff --stat crates/talaria-protocol/src/lib.rs` reports no change
across this whole plan.

All seven of 02-09's scoped `#[allow(dead_code)]` attributes came off, and every item they named
now has a caller (verified individually — see Requirements below).

**Task 2 — the toolbar suggestion** (`7b53946`)

The suggestion appears only when the displayed tab's host has at least one matching vault entry.
The host comes from the webview's URL, falling back to the tab's location string before the first
navigation; matching is delegated to `Vault::matching` rather than reimplemented, so it stays
exact-or-subdomain and a lookalike host cannot borrow a login by substring (T-02-10-06). The
control names the matched username — or the count when several match — so the user sees which
login they are about to take. Clicking it opens a popup with copy controls for the username and
the password.

It reads the vault under a **shared** borrow inside the closure, the same shape the tab strip
already uses for sessions. A clipboard write is not shell state, so it needs no intent; no other
inline mutation lives there.

Nothing is written into the page. No script is evaluated, no form field is located, no DOM node is
touched — `grep -c evaluate_javascript crates/talaria-shell/src/gui.rs` is 0. Injection would
collide with an agent's own `evaluate` on the same form *and* would place the user's password
inside a document every script on that page can read. That reasoning is a comment at the control,
because "just fill the form, it's friendlier" is the change most likely to be made by someone who
has not read this.

**Task 3 — the proof** (`d4627eb`)

`tests/e2e/vault_ui_test.py`, standalone with its own Xvfb and an isolated `HOME` +
`XDG_CONFIG_HOME` (02-09's lesson: `dirs::config_dir()` resolves `XDG_CONFIG_HOME` first, so
`HOME` alone does not isolate). Four assertions, all over the control socket:

- the panel opens from the toolbar and the shell is still serving the socket with it up (a
  screenshot answers — a liveness check, not a pixel comparison, since the panel replaces the
  page);
- a site, username and password typed into the three fields and saved are returned by
  `cookies_read` for that host, with the password intact;
- a fresh shell on the same home still returns it — the save was durable, not in-memory;
- deleting the row through the panel empties `cookies_read`.

The password is `secrets.token_hex(16)` per run, so nothing credential-shaped is committed.

## Negative Control

Both controls were run by isolated revert at release, and checked for *where* they fail rather
than merely that they do.

| Control | Predicted | Observed |
|---|---|---|
| `Vault::upsert` without its `self.save()` — the credential exists only in memory | SAVED passes, RESTART fails | SAVED passed, then `AssertionError: []` at the restart read |
| `UiAction::DeleteCredential` made a no-op — the panel's delete button does nothing | everything passes, DELETED fails | passed through RESTART, then failed at exactly `"deleting the row in the panel left the credential in the vault"` |

The first is the interesting one: it proves the restart assertion is not decorative — the save
assertion alone cannot tell an encrypted write from an in-memory list. The second proves the
delete assertion is reached and discriminating; the *passing* run already proves the keyboard
route lands on the delete button rather than the reveal toggle, since only the delete button
emits `DeleteCredential`.

Both files were restored with `git checkout --` and `git diff --stat` verified clean, then rebuilt
before Task 3 was committed.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] A click below the toolbar never reached the chrome**

- **Found during:** Task 1
- **Issue:** `window_event` forwarded every `MouseInput` below the toolbar straight to the page
  (`if !over_toolbar(&state)`), and the fallback arm that hands events to egui was therefore never
  reached. The credentials panel's buttons would have been unclickable — and so, already, was the
  **crashed tab's Reload button**, the only other surface that replaces the page. A pre-existing
  bug, but squarely in the path of this plan.
- **Fix:** A `chrome_replaces_page` guard beside `over_toolbar`, true while the credentials panel
  is open or the displayed tab has crashed. Both the `MouseInput` and `MouseWheel` arms now fall
  through to egui in those two states. It cannot affect ordinary browsing: in both states there is
  no page being painted to receive the click.
- **Files modified:** `crates/talaria-shell/src/app.rs`
- **Commit:** `128d3ff`
- **Verified:** a mouse click on the reveal toggle and on the delete button, at coordinates, both
  work (throwaway probe, not committed).

**2. [Rule 2 - Missing critical functionality] A typed bare hostname would have been unmatchable and undeletable**

- **Found during:** Task 1
- **Issue:** `Vault::upsert` documents that an entry whose URL has no parseable host is appended
  rather than replacing anything — deliberately, so a bare hostname is not discarded. But
  `matching` and `delete` both key on `entry_host`, which is `None` for such an entry. A user
  typing `example.com` into the site field would have got an entry that no domain match could ever
  surface and no delete could ever remove.
- **Fix:** `credential_url` in `gui.rs` completes a site that does not parse into a host to
  `https://…` before it becomes an entry, so every entry the panel creates is addressable by the
  same key the vault matches on. The reasoning is on the function.
- **Files modified:** `crates/talaria-shell/src/gui.rs`
- **Commit:** `128d3ff`

**3. [Rule 2 - Missing critical functionality] A stale reveal index after a delete**

- **Found during:** Task 1
- **Issue:** `revealed_entry` is an index into `vault.entries()`. Deleting a row renumbers
  everything below it, so a reveal left on would have uncovered a *different* entry's password on
  the next frame — an unasked-for disclosure, and exactly the class T-02-10-03 exists to prevent.
- **Fix:** the delete control clears the reveal before emitting its intent, and
  `set_credentials_panel` clears it on every open and close.
- **Files modified:** `crates/talaria-shell/src/gui.rs`
- **Commit:** `128d3ff`

### Deliberate Choices Worth Recording

- **The e2e suite drives the panel by keyboard, not by row coordinates.** The plan sanctioned
  this explicitly ("reduce the coordinate dependence instead — and keep every socket-side
  assertion exactly as written"), and every assertion is unchanged. The reason is concrete: a
  panel row's vertical position depends on whether a one-shot vault notice is showing, which
  depends on whether *the machine running the test* has a usable OS keychain. On this machine
  under Xvfb the keychain is unavailable and the notice shows; on a developer's desktop it may
  not. That is not something a committed test may depend on. Only the toolbar button's position is
  hardcoded (the toolbar's layout is fixed, and `takeover_test.py` already depends on it the same
  way), and both constants carry a comment naming what would move them.
- **The keyboard route inside the panel is a real feature, not test scaffolding.**
  `set_credentials_panel(true)` puts the caret in the site field, so the whole panel is reachable
  with Tab and Enter from a fixed starting point — by a user with no mouse as much as by the
  suite. The mouse route was verified separately and works.
- **`revealed_entry` is `Option<usize>`, not the `bool` the plan named.** A per-row toggle needs
  to know *which* row, and one-at-a-time is the stricter reading of the same requirement. Panel
  layout is Claude's discretion per 02-CONTEXT.

## Requirements

- **CRED-03 — closed.** "A user can save a credential from within Talaria." A user genuinely can:
  `tests/e2e/vault_ui_test.py` types one into the chrome and reads it back out of `cookies_read`,
  across a restart. 02-09 shipped the storage half and deliberately left this In Progress; this is
  the capture half.
- **CRED-02 — closed.** "Stored credentials are suggested for autofill by domain match in the
  shell UI." Delivered as the plan's flagged assumption reads it: a chrome-side offer naming the
  matching username, with copy controls — not page-DOM injection. That reading is unchanged from
  the plan and D-23; see Flagged Assumption below.

### The seven `#[allow(dead_code)]` attributes

`grep -c "allow(dead_code)" crates/talaria-shell/src/vault.rs` is **0**. None remain. Each named
item was individually confirmed to have a caller:

| Item | Consumer |
|---|---|
| `ImportNotice` (and its `path` / `source_removed` fields) | `gui.rs` notice surface (`notice.path`, `notice.source_removed`) |
| `Vault::upsert` | `app.rs` `UiAction::SaveCredential` arm |
| `Vault::delete` | `app.rs` `UiAction::DeleteCredential` arm |
| `Vault::entries` | `gui.rs` stored list |
| `Vault::import_notice` | `gui.rs` notice surface |
| `Vault::key_downgraded` | `gui.rs` notice surface |
| `Vault::clear_notices` | `app.rs` `UiAction::DismissVaultNotice` arm |

(02-09's summary said six; the file carried seven — the struct-level one on `ImportNotice` was not
counted there. All seven are gone.)

## Flagged Assumption (carried forward, unchanged)

The plan carried one unresolved probe row for CRED-02, and it is resolved as the plan proposed,
not silently. CRED-02's "suggested for autofill by domain match in the shell UI" is delivered as a
**chrome-side offer the user acts on**, and explicitly not as populating the page's form fields.
Actually filling the fields needs form-field detection plus a page-scripting path; it would
collide with an agent's own `evaluate` on the same page, and it would place the password inside a
document any script on that page can read. It is not delivered here, and delivering it would need
its own decision about the collision with agent scripting. Recorded in `deferred-items.md`.

## Threat Flags

| Flag | File | Description |
|------|------|-------------|
| threat_flag: input-routing | `crates/talaria-shell/src/app.rs` | New surface at the chrome↔page trust boundary: pointer events below the toolbar are now routed to egui rather than the webview in two states (credentials panel open, displayed tab crashed). Both are states in which no page is being painted, so no page loses input it would otherwise have received — but the routing predicate is new and is what keeps a page from receiving clicks aimed at the credentials panel. |

## Verification

| Check | Result |
|-------|--------|
| `cargo build --release` | exit 0, zero warnings |
| `cargo clippy --all-targets -- -D warnings` | the 7 known pre-existing lints only (`tools.rs:97,132`; `app.rs:991×2, 1016×2`; `tabs.rs:93`) — no new lint, none in `gui.rs` |
| `cargo test` | 9 passed, 0 failed (7 vault + 2 protocol) |
| `python3 tests/e2e/vault_ui_test.py` | exit 0, prints `VAULT UI CHECKS PASSED` |
| `python3 -c "import ast; ast.parse(...)"` | exit 0 |
| `python3 tests/e2e/run_all.py` (isolated, `:97`) | **14/14 PASS, exit 0** — 13/13 baseline plus the new suite, no regression |
| `git diff --stat crates/talaria-protocol/src/lib.rs` | no change — no agent-reachable surface added |
| Negative controls | both fail at the assertion they were aimed at, after the earlier ones pass |

Source criteria: `SaveCredential` in `gui.rs` 2, `UiAction::SaveCredential` in `app.rs` 1,
`UiAction::DeleteCredential` in `app.rs` 1, `vault.borrow_mut` in `gui.rs` **0**, `vault.borrow_mut`
in `app.rs` 3, `vault.borrow()` in `gui.rs` 2, `password(true)` in `gui.rs` 1, `matching(` in
`gui.rs` 1, `evaluate_javascript` in `gui.rs` 0, `unwrap()` in `gui.rs` 0, MPL header intact on
both Rust files, `vault_ui_test` in `run_all.py` 1, `cookies_read` in the new suite 6,
`allow(dead_code)` in `vault.rs` 0.

Behaviour confirmed visually under Xvfb (throwaway probes, not committed): the panel renders with
the keychain notice, the drafts clear on save, the stored row masks its password and reveals it on
the toggle; navigating to a host with a stored entry raises a toolbar suggestion naming `alice`
with a copy popup, and a host with no stored entry raises none.

## Known Stubs

None.

One residual gap, not introduced here and not reachable through the panel: an entry whose URL has
no parseable host — which only a hand-written `vault.json` can now produce, since `credential_url`
completes anything the panel creates — is still invisible to `Vault::matching` and immovable by
`Vault::delete`. Recorded in `deferred-items.md` rather than fixed, because it means changing
02-09's verified key semantics.

## Self-Check: PASSED

- `crates/talaria-shell/src/gui.rs` — FOUND
- `crates/talaria-shell/src/app.rs` — FOUND
- `crates/talaria-shell/src/vault.rs` — FOUND
- `tests/e2e/vault_ui_test.py` — FOUND
- `tests/e2e/run_all.py` — FOUND
- Commit `128d3ff` — FOUND
- Commit `7b53946` — FOUND
- Commit `d4627eb` — FOUND

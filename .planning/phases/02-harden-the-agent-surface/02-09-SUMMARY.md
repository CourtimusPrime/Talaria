---
phase: 02-harden-the-agent-surface
plan: 09
subsystem: credential-vault
tags: [security, vault, encryption, data-at-rest, e2e]
status: complete
requires:
  - "talaria_protocol::CredentialEntry"
  - "crates/talaria-shell/src/vault.rs (ChaCha20-Poly1305 at-rest encryption, keychain/key-file fallback)"
  - "tests/e2e/harness.py (start_xvfb, start_shell with extra environment, SOCK)"
provides:
  - "Vault::upsert(entry) — host+username keyed write, persists immediately"
  - "Vault::delete(host, username) -> bool — persists only on an actual removal"
  - "Vault::entries() -> &[CredentialEntry] — borrowed listing for a UI"
  - "Vault::import_notice() / Vault::key_downgraded() / Vault::clear_notices() — one-shot notices for the chrome"
  - "vault::ImportNotice { path, source_removed } — which of the two import outcomes the user got"
  - "Verified-then-remove plaintext import: the source goes only after the encrypted copy re-reads, decrypts and matches"
  - "tests/e2e/vault_test.py — import, cleanup, restart, late-plaintext, and the data-loss guard"
affects:
  - "crates/talaria-shell/src/vault.rs"
  - "tests/e2e/run_all.py (phase-2 tuple, now 13 suites)"
tech-stack:
  added: []
  patterns:
    - "Write-through accessors: upsert and delete call save() themselves, so a caller cannot forget to persist"
    - "Verify-then-mutate: a destructive cleanup is gated on re-reading what was just written, not on the write call having returned"
    - "One-shot notice fields on the owning type, read and cleared by the chrome, for degradations a log line cannot communicate"
    - "#[allow(dead_code)] with a named downstream consumer, for the storage half of a two-plan pair in a binary crate"
key-files:
  created:
    - "tests/e2e/vault_test.py"
  modified:
    - "crates/talaria-shell/src/vault.rs"
    - "tests/e2e/run_all.py"
decisions:
  - "Key on the URL host plus username rather than the whole URL, matching how `matching()` already normalises"
  - "An entry whose URL has no parseable host is appended rather than discarded — a bare hostname typed into a panel is a real case"
  - "Removal is gated on a re-read/decrypt/entry-count match, because save() returns early on failure and so 'it ran' is not evidence"
  - "A malformed plaintext file is not counted as an import, so it can never become a removal candidate"
  - "Notices live on Vault itself, so app.rs needed no change at all"
  - "Marked SEC-02 complete; CRED-03 stays In Progress — this is only its storage half"
metrics:
  duration: "33m"
  completed: 2026-08-16
  tasks: 3
  files_changed: 3
  commits: 3
---

# Phase 02 Plan 09: Vault Write Path and Verified Plaintext Import Summary

Gave the credential vault a host-and-username keyed write path, and made the plaintext import
actually finish — the source is removed only after the encrypted copy is re-read and decrypted,
and when it cannot be, the plaintext survives at mode 0600 with an error naming it.

## What Was Built

**Task 1 — the write path** (`ce6bc10`)

Three methods on `Vault`, plus the host normalisation they share with `matching`:

- `upsert(entry)` — replaces the first entry with the same URL host *and* username, otherwise
  appends; then saves. The key is the host, not the whole URL, so re-saving the same login from
  `/login` and from `/session/new` updates one entry rather than accumulating two.
- `delete(host, username) -> bool` — reports whether anything went, and saves only when
  something did.
- `entries() -> &[CredentialEntry]` — borrowed, so a UI can list without cloning the vector.
  Documented as the counterpart to `matching`, which clones because its result crosses the
  control socket.

`entry_host` and `normalise_domain` were factored out of `matching`, so upsert, delete and
matching now all agree on what a host is by construction rather than by three copies of the same
expression.

Six new unit tests (7 in the module, up from 1) built the way the existing one is — a struct
literal with `cipher: None`, so `save()` returns early and nothing touches disk or the keychain.
They cover append, replace-on-same-key, per-username split, path independence, and both delete
outcomes.

**Task 2 — verified import and the notices** (`fe8261a`)

`Vault::load` now records *whether* the entries came from the plaintext branch, then
`finish_import` does the cleanup:

1. `verify_written()` re-reads the file `save()` just wrote, decrypts it with the same cipher,
   deserialises it, and compares the entry count against what is in memory.
2. Only on that confirmation is `vault.json` removed.
3. On a failed verification *or* a failed removal: no removal, an error at `log::error!` naming
   the path that still holds plaintext credentials, and `restrict_to_owner` chmods it 0600 as a
   partial mitigation.
4. Either way an `ImportNotice { path, source_removed }` is set, so the chrome can tell the user
   which of the two outcomes they got.

The ordering is the whole safety argument and it is written down at the removal: `save()` returns
early on every failure, so "it ran" is not evidence anything landed. It is also why two shell
processes cannot half-migrate — the source disappears only after a verified write, and a process
that finds no plaintext file imports nothing, so the second one through takes the decrypt branch
and leaves the first one's work alone.

A malformed plaintext file is no longer counted as an import. Previously `unwrap_or_default()`
turned an unparseable file into an empty entry list; that list would now have "verified" against
an empty encrypted file and authorised deleting the user's hand-written file. It is parsed
explicitly, and a parse failure warns and leaves the file exactly where it is.

`load_or_create_key` returns a `KeyOutcome { key, downgraded }`. All four keychain fallback
branches set `downgraded` — a malformed keychain value, a failed store, an unavailable keychain,
and a failed keyring init — not just the one that fires most often. The key-file
`set_permissions` result is checked instead of discarded with `let _ =`; a failure logs at error
level naming the key path and sets the downgrade.

`crates/talaria-shell/src/app.rs` needed **no change**. The plan asked this to be confirmed
rather than assumed: the notices live on `Vault`, `Shared::vault` is already a `RefCell<Vault>`,
and `state.vault.borrow()` at the credentials-read command is already on the event loop. Vault
access remains main-thread-only with no borrow held across a call into Servo.

**Task 3 — the e2e proof** (`541af2c`)

`tests/e2e/vault_test.py`, standalone with its own Xvfb and an isolated `HOME` +
`XDG_CONFIG_HOME` (the vault, the key file and the engine profile all live under the config
directory). Four scenarios:

- **Import**: a plaintext file with two entries is imported, `cookies_read` returns the first
  entry with its username and password over the control socket, the plaintext file is gone, and
  the `TALARIA1`-headed file that replaced it contains neither password as a cleartext byte
  sequence.
- **Restart**: a fresh shell on the same home still returns both entries.
- **Late plaintext**: a `vault.json` written *after* the vault exists is neither imported nor
  deleted. Asserted explicitly, with a comment saying it is deliberate — the import arm runs only
  when the encrypted file is absent, and a file the shell will not read is also a file it has no
  business deleting.
- **Unverifiable write**: `vault.enc` is pre-created as a *directory*, so the encrypted write
  cannot land and re-reading it cannot succeed. The plaintext source must survive, be mode 0600,
  and be named in the log.

Passwords are generated per run with `secrets.token_hex(16)` rather than hardcoded: nothing
credential-shaped is committed, and a random token cannot coincidentally appear inside the
ciphertext the "no cleartext" scan searches.

## Negative Control

The fourth scenario was validated by isolated revert, and checked for *why* it fails rather than
merely that it does. Replacing the gate with `if true` — the naive "just delete the source"
implementation — was built at release and run: scenarios one through three passed identically
(verification succeeds in those, so nothing short-circuits), and the suite then failed at exactly
`"an unverifiable encrypted write still destroyed the plaintext source"`. That is the data-loss
guard, which is the assertion the control was aimed at. `vault.rs` was restored byte-identically
(`git diff --stat` clean) and rebuilt before Task 3 was committed.

The "plaintext removed" assertion is self-evidently discriminating — it is an existence check on
a file the suite itself wrote, which the pre-plan code left in place — so it was not separately
reverted.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] `dead_code` on the storage half of a two-plan pair**

- **Found during:** Task 1, then again in Task 2
- **Issue:** `talaria-shell` is a binary crate, so a `pub` method with no in-crate caller is
  unreachable from the crate root and rustc's `dead_code` lint fires. `upsert`, `delete`,
  `entries`, the three notice accessors and `ImportNotice`'s fields have no caller until plan
  02-10 adds the chrome — so both `cargo build --release` and
  `cargo clippy --all-targets -- -D warnings` gained a new warning the plan's own verify gate
  forbids.
- **Fix:** A scoped `#[allow(dead_code)]` on each item, each carrying a comment naming plan 02-10
  as the consumer and stating why the lint fires. Not a blanket module-level allow — each
  suppression is individually justified and individually removable when 02-10 lands.
- **Files modified:** `crates/talaria-shell/src/vault.rs`
- **Commits:** `ce6bc10`, `fe8261a`

**2. [Rule 2 - Missing critical verification] A fourth e2e scenario for the failed-verification path**

- **Found during:** Task 3
- **Issue:** The plan's Task 3 action listed three scenarios (import, restart, second import), but
  Task 2's acceptance criteria and threat T-02-09-04 both assert behaviour on the
  *failed*-verification path — the plaintext surviving, the chmod, and the error naming it. Nothing
  in the listed scenarios exercises it, so the highest-severity mitigation in the plan would have
  shipped unproven, and the verify-then-remove ordering would have been indistinguishable from an
  unconditional remove.
- **Fix:** Added the unverifiable-write scenario (pre-create `vault.enc` as a directory, capture
  the shell log, assert survival + mode 0600 + the error naming the path). This is also the
  scenario the negative control above is aimed at.
- **Files modified:** `tests/e2e/vault_test.py`
- **Commit:** `541af2c`

**3. [Rule 2 - Missing critical functionality] A malformed plaintext file is no longer an "import"**

- **Found during:** Task 2
- **Issue:** The pre-existing import arm used `serde_json::from_slice(&plain).unwrap_or_default()`.
  Under the new verified-then-remove logic that would have produced `imported = true` with an
  empty entry list, verified an empty encrypted file against it successfully, and deleted the
  user's unparseable hand-written credential file — the exact data-loss class the plan exists to
  prevent, arriving through the door the plan was opening.
- **Fix:** Parse explicitly; a parse failure warns and leaves the file alone, and never sets
  `imported`. Preserves the module's degrade-never-abort rule (T-02-09-06).
- **Files modified:** `crates/talaria-shell/src/vault.rs`
- **Commit:** `fe8261a`

### Deliberate Non-changes

- **`crates/talaria-shell/src/app.rs` is untouched.** The plan said to confirm and leave it alone
  if the chrome can reach the notices through the existing `RefCell<Vault>` field. It can.
  `git diff --stat crates/talaria-shell/src/app.rs` reports no change.
- **`XDG_CONFIG_HOME` is set alongside `HOME` in the e2e suite.** The plan said to point `HOME` at
  a temporary directory. `dirs::config_dir()` resolves `$XDG_CONFIG_HOME` *before* `$HOME/.config`,
  so `HOME` alone does not isolate a developer who has `XDG_CONFIG_HOME` set. Both are set, which
  is what `tests/e2e/download_bounds_test.py` already does for the same reason.

## Requirements

- **SEC-02 — closed.** "Importing credentials removes the plaintext source file rather than
  leaving it on disk." Marked complete; the e2e suite pins both the removal and the guard that
  makes it safe.
- **CRED-03 — deliberately left In Progress.** The requirement is "a user can save a credential
  from within Talaria." A user still cannot: there is no UI. This plan is CRED-03's storage half
  only; plan 02-10 adds the credentials panel that calls `upsert`, and that is what closes it.
  `requirements.mark-complete` was run with `SEC-02` alone.

## Contract for Plan 02-10

The names 02-10's Task 1 needs to bind against:

```rust
pub fn upsert(&mut self, entry: CredentialEntry)           // saves itself
pub fn delete(&mut self, host: &str, username: &str) -> bool
pub fn entries(&self) -> &[CredentialEntry]
pub fn import_notice(&self) -> Option<&ImportNotice>       // vault::ImportNotice
pub fn key_downgraded(&self) -> bool
pub fn clear_notices(&mut self)                            // clears both
pub fn matching(&self, domain: &str) -> Vec<CredentialEntry>   // unchanged
```

`ImportNotice` is `pub struct { pub path: PathBuf, pub source_removed: bool }` — `source_removed`
is what the notice text must reflect, since `false` means a readable plaintext copy is still on
disk. `clear_notices` is a single call clearing both notices, which matches 02-10's single
dismiss intent.

Two things 02-10 should expect:

- **`delete` takes a bare host, not a URL.** The panel's delete control must pass
  `entry_host`-shaped input — in practice the host parsed out of the row's URL.
- **The six `#[allow(dead_code)]` attributes should come off** as 02-10 wires each item up. If any
  survives after 02-10, that item has no caller and the plan missed something.

## Verification

| Check | Result |
|-------|--------|
| `cargo test -p talaria-shell` | 7 passed, 0 failed (was 1) |
| `cargo build --release` | exit 0, zero warnings |
| `cargo clippy --all-targets -- -D warnings` | the 7 known pre-existing lints only (`tools.rs:97,132`; `app.rs:968×2, 993×2`; `tabs.rs:93`) — no new lint, none in `vault.rs` |
| `python3 tests/e2e/vault_test.py` | exit 0, prints `VAULT CHECKS PASSED` |
| `python3 tests/e2e/run_all.py` (isolated, `:97`) | **13/13 PASS, exit 0** — 12/12 baseline plus the new suite, no regression |
| Negative control | fails at the data-loss guard, after the earlier scenarios pass |

Source criteria: `pub fn upsert` 1, `pub fn delete` 1, `pub fn entries` 1, `#[test]` 7,
`remove_file` 1, `set_permissions` 2, `log::error!` 4, `falling back to key file` 4 each with a
`downgraded = true`, `let _ = file.set_permissions` 0, `grep -c vault_test tests/e2e/run_all.py` 1.

## Known Stubs

None. Every method added here is exercised either by a unit test or by the e2e suite; the six
`#[allow(dead_code)]` attributes mark items awaiting their 02-10 caller, not unimplemented
behaviour.

## Self-Check: PASSED

- `crates/talaria-shell/src/vault.rs` — FOUND
- `tests/e2e/vault_test.py` — FOUND
- `tests/e2e/run_all.py` — FOUND
- Commit `ce6bc10` — FOUND
- Commit `fe8261a` — FOUND
- Commit `541af2c` — FOUND

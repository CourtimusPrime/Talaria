---
phase: 3
slug: table-stakes-browsing
# status lifecycle: draft (seeded by plan-phase) → validated (set by validate-phase §6)
status: draft
nyquist_compliant: false
wave_0_complete: false
created: 2026-08-20
---

# Phase 3 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.
> Derived from `03-RESEARCH.md` § Validation Architecture.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | Python stdlib e2e harness (`tests/e2e/harness.py`) + Rust `#[cfg(test)]` unit tests. No third-party test framework in either language, per project convention. |
| **Config file** | none — `tests/e2e/run_all.py` is itself the runner and the registry |
| **Quick run command** | `cargo test -p talaria-shell` |
| **Full suite command** | `cargo build --release && cargo clippy --all-targets -- -D warnings && cargo test && python3 tests/e2e/run_all.py` |
| **Estimated runtime** | ~2 min quick (release build dominates); ~7 min full (e2e suite is ~4.5 min today, four new suites land on top) |

---

## Sampling Rate

- **After every task commit:** `cargo build --release && cargo test -p talaria-shell`
- **After every plan wave:** full suite — `cargo clippy --all-targets -- -D warnings`, `cargo test`, `python3 tests/e2e/run_all.py` with the new suites registered
- **Before `/gsd-verify-work`:** full suite green, matching Phase 2's closing bar
- **Max feedback latency:** ~120 seconds (quick run)

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Threat Ref | Secure Behavior | Test Type | Automated Command | File Exists | Status |
|---------|------|------|-------------|------------|-----------------|-----------|-------------------|-------------|--------|
| 03-01-xx | 01 | 1 | BROWSE-01 | — | N/A | unit | `cargo test -p talaria-shell history::tests` | ❌ W0 | ⬜ pending |
| 03-01-xx | 01 | 1 | BROWSE-01 | — | Agent-owned tabs must not write to the human's history | e2e | `python3 tests/e2e/history_test.py` | ❌ W0 | ⬜ pending |
| 03-01-xx | 01 | 1 | BROWSE-01 | — | History survives a restart (literal success criterion) | e2e | `python3 tests/e2e/history_test.py` | ❌ W0 | ⬜ pending |
| 03-02-xx | 02 | 2 | BROWSE-02 | — | N/A | unit | `cargo test -p talaria-shell bookmarks::tests` | ❌ W0 | ⬜ pending |
| 03-02-xx | 02 | 2 | BROWSE-02 | — | Bookmarks survive a restart | e2e | `python3 tests/e2e/bookmarks_test.py` | ❌ W0 | ⬜ pending |
| 03-03-xx | 03 | 2 | BROWSE-03 | — | A malformed `config.json` degrades to the default engine, never panics | unit | `cargo test -p talaria-shell resolve_location` | ❌ W0 | ⬜ pending |
| 03-03-xx | 03 | 2 | BROWSE-03 | — | A configured engine is used instead of the hardcoded DuckDuckGo | unit | `cargo test -p talaria-shell resolve_location` | ❌ W0 | ⬜ pending |
| 03-04-xx | 04 | 3 | BROWSE-04 | T-03-01 | "Open" launches only on a path the shell itself recorded — never on agent-supplied input | unit | `cargo test -p talaria-shell downloads::tests` | ❌ W0 | ⬜ pending |
| 03-04-xx | 04 | 3 | BROWSE-04 | T-03-01 | No MCP tool can trigger the opener | source assertion | `grep -c 'downloads_open\|downloads_list' crates/talaria-mcp/src/tools.rs` returns `0` | ✅ (file exists) | ⬜ pending |
| 03-04-xx | 04 | 3 | BROWSE-04 | — | A completed download appears in the list | e2e | `python3 tests/e2e/downloads_list_test.py` | ❌ W0 | ⬜ pending |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

*Task IDs are placeholders (`xx`) until the planner assigns them; the plan-checker should reconcile
this table against the task IDs it finds in the PLAN.md files.*

---

## Wave 0 Requirements

- [ ] `tests/e2e/history_test.py` — BROWSE-01 (capture, Me-only filter, restart survival). Model on
      `tests/e2e/vault_test.py`, which already has the exact temp-`HOME` + restart pattern.
- [ ] `tests/e2e/bookmarks_test.py` — BROWSE-02
- [ ] `tests/e2e/downloads_list_test.py` — BROWSE-04's listing half. Distinct from the existing
      `download_bounds_test.py`, which covers only the byte cap and uniquifying create and stays as-is.
- [ ] `#[cfg(test)] mod tests` in each new store module (`history.rs`, `bookmarks.rs`, `downloads.rs`,
      `settings.rs`) from the start, at `vault.rs`'s test density rather than the sparser coverage elsewhere.
- [ ] First unit tests for `resolve_location` — it has **zero** today, and this phase is the first to
      change its signature.
- [ ] Register every new suite in `tests/e2e/run_all.py`'s standalone-shell block (alongside
      `keyboard_nav`, `takeover`, `download_bounds`, `vault`, `vault_ui`, `vault_nobus`), because each
      starts its own shell with its own temp `HOME`.

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| The downloads-list "Open" actually launches an external application | BROWSE-04 | Driving a real file-open through `xdotool`/Xvfb and asserting a *foreign* process launched is high effort for low signal. The two halves that carry the risk are automated instead: the UI action carries exactly the stored path (unit), and no MCP tool can reach the opener (source assertion). | Launch the shell, download a file, open the downloads panel, click Open, confirm the system handler launches with that file. |
| Visual layout of the three new list panels inside the existing chrome | BROWSE-01/02/04 | egui layout quality is a human judgement; there is no snapshot-testing infrastructure in this repo and adding one is out of phase scope. | Launch the shell, open each panel, confirm it does not overlap the toolbar or tab strip and that the Me/Agents toggle still reads correctly. |

---

## Validation Sign-Off

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 120s
- [ ] `nyquist_compliant: true` set in frontmatter

**Approval:** pending

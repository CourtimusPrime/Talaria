---
phase: 5
slug: distributed-mode
# status lifecycle: draft (seeded by plan-phase) → validated (set by validate-phase §6)
status: validated
nyquist_compliant: true
wave_0_complete: false
created: 2026-08-21
reconciled: 2026-08-21  # against 05-01..05-11-PLAN.md; plan-checker APPROVED at revision 3
scope_note: "Trimmed to DIST-01/02. The DIST-03 and DIST-04 rows in 05-RESEARCH.md's map belong to Phase 5.1 (decision D-05-04) and are carried forward there, not dropped."
---

# Phase 5 — Validation Strategy

> Per-phase validation contract for feedback sampling during execution.
> Derived from `05-RESEARCH.md` § Validation Architecture, trimmed to this phase's scope.

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework (Rust)** | Built-in `#[cfg(test)]` + `cargo test`. No test-framework crate; no `tempfile` — tests build unique temp paths by hand (`permissions.rs`, `vault.rs`, `settings.rs`). |
| **Framework (e2e)** | Python 3 standard library only. No pytest, no third-party deps. |
| **Config file** | None for Rust. `tests/e2e/run_all.py` is the runner; `tests/e2e/harness.py` the shared launcher. |
| **Quick run command** | `cargo test -p talaria-shell` |
| **Full suite command** | `cargo build --release --locked && cargo clippy --all-targets -- -D warnings && cargo test && python3 tests/e2e/run_all.py` |
| **Static gate** | `cargo clippy --all-targets -- -D warnings` |
| **Baseline entering this phase** | e2e **22/22**, `cargo test` **296** |
| **Target leaving this phase** | e2e **24/24** (two new suites: `remote_view_test`, `remote_latency_test`), `cargo test` well above 296 |

---

## Sampling Rate

- **After every task commit:** `cargo test -p talaria-shell` plus `cargo clippy --all-targets -- -D warnings`
- **After every plan wave:** `cargo build --release --locked`, then the affected e2e suite standalone
- **Before `/gsd-verify-work`:** `python3 tests/e2e/run_all.py` fully green plus `cargo test` green
- **Max feedback latency:** ~120 seconds (quick run)

---

## Per-Task Verification Map

| Requirement | Behaviour | Threat Ref | Test Type | Automated Command | File Exists | Status |
|-------------|-----------|------------|-----------|-------------------|-------------|--------|
| — | Tile diff finds exactly the changed tiles, including at frame edges and on a 1-pixel change | — | unit | `cargo test -p talaria-shell view::` | ❌ W0 | ⬜ |
| — | `Compression::Fast` + `Sub` frame encode stays under a stated budget for a synthetic 1280×800 frame | — | unit (bench-shaped) | `cargo test -p talaria-shell view::` | ❌ W0 | ⬜ |
| — | The wire envelope round-trips: every channel tag, every input kind, every frame-header field | — | unit | `cargo test -p talaria-protocol wire::` | ❌ W0 | ⬜ |
| — | **Malformed wire input is refused, not defaulted**: bad tag, truncated header, out-of-range coordinates, non-monotonic `seq`, unknown key name | high | unit | `cargo test -p talaria-shell remote_input::` | ❌ W0 | ⬜ |
| DIST-01 / SC 1 | The `/view` route exists, is **absent when remote access is off**, and 401s unauthenticated | high | e2e | `python3 tests/e2e/remote_view_test.py` | ❌ W0 | ⬜ |
| DIST-01 | A WebSocket carrying an `Origin` header is refused (CSWSH), and one with a wrong `Host` is refused | high | e2e | same suite | ❌ W0 | ⬜ |
| DIST-01 / SC 1 | An authorized client attaches to an agent tab and receives a keyframe | — | e2e | same suite | ❌ W0 | ⬜ |
| DIST-02 / SC 1 | A remote click navigates the agent's tab — asserted on the tab's URL over the **control socket**, so the assertion does not depend on the frame path under test | — | e2e | same suite | ❌ W0 | ⬜ |
| DIST-02 | A remote click **lands at the coordinate aimed at** — assert the *specific* link hit, not merely that navigation occurred (regression for the stale-frame coordinate trap Phase 4 hit) | — | e2e | same suite | ❌ W0 | ⬜ |
| DIST-02 / D-05-02 | **Remote input aimed at a Me tab is refused** — server-side, not merely un-shown | high | e2e + unit | same suite + `cargo test` | ❌ W0 | ⬜ |
| DIST-02 | **Remote input cannot reach the chrome** — a click at a `chrome_rects` toolbar coordinate opens no panel | high | e2e | same suite (uses `TALARIA_TEST_HOOKS=1`) | ❌ W0 | ⬜ |
| DIST-02 / SC 2 | Passive cadence is ~200–500 ms and rises to ~30–60 ms on takeover; frame timestamps prove the transition | — | e2e | `python3 tests/e2e/remote_latency_test.py` | ❌ W0 | ⬜ |
| DIST-02 / SC 2 / D-05-06 | Under a 13 Mbit/s + 40 ms shim the client **degrades down the ladder and reports**, rather than stalling or refusing takeover | — | e2e | same suite, via the link shim | ❌ W0 | ⬜ |
| SC 3 / D-05-01 | **Local mode is provably unaffected** — the existing single-process path is untouched and its e2e suites pass unmodified | — | e2e + source assertion | `takeover_test.py`, `panel_click_test.py` unmodified and green; `git diff --stat` shows no change to the local render path | ✅ | ⬜ |
| — | Revoking a viewer closes its WebSocket, asserted **from the client end** (not "a new request now fails") | high | e2e | `tests/e2e/revocation_test.py` (extend) | ✅ extend | ⬜ |
| — | Everything Phase 4 asserts still holds with the view route mounted | — | e2e | `http_transport_test.py`, `oauth_flow_test.py` **unmodified** | ✅ | ⬜ |
| — | `Cargo.lock` still pins `primeorder 0.14.0-rc.14`; the only added package is `tokio-tungstenite` | — | source assertion | `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'` is 1; lock diff reviewed | ✅ | ⬜ |
| — | Tailscale Serve mapping for the existing `:8443` service is **not disturbed** | — | source assertion | the plan names a distinct port; `tailscale serve status` unchanged for `127.0.0.1:5678` | ✅ | ⬜ |
| — | `CHANGELOG.md` records the phase under DIST-01 and DIST-02 (constraint C-9) | — | source assertion | `grep -c 'DIST-0[12]' CHANGELOG.md` is at least 2 | ✅ | ⬜ |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

*Task IDs are assigned by the planner; the plan-checker should reconcile this table against them.*

---

## Required Spikes

Both from `05-CONTEXT.md`. Phase 4's precedent is that a spike runs in an early wave and its answer
unblocks the plans that depend on it — `04-02`'s ran before the authorization-server plans were
executed and carried a falsifiable verdict enforced by `grep -qE '^A1: (CONFIRMED|REFUTED)'`.

1. **GL readback cost.** `read_to_image` at a 30 ms cadence is unmeasured and is the phase's largest
   unknown. Under Xvfb's llvmpipe it may be far worse than on real hardware — which matters, because
   the e2e harness *is* Xvfb. The frame-pump plan must not be written before this returns a number,
   and the spike must report the Xvfb figure and the real-hardware figure separately if they differ.
2. **WebSocket through Tailscale Serve.** Whether Serve proxies an upgrade cleanly, and what it does
   to the advertised-URL problem D-05-03 creates.

Both must carry a falsifiable verdict and leave no source behind.

---

## Wave 0 Requirements

- [ ] `tests/e2e/remote_view_test.py` — DIST-01, the input-authorisation assertions, SC 1
- [ ] `tests/e2e/remote_latency_test.py` — DIST-02, SC 2, including the degrade ladder under a link shim
- [ ] A link shim (stdlib only) able to impose ~13 Mbit/s and ~40 ms, for the degrade assertion
- [ ] `#[cfg(test)] mod tests` in each new module (`view`, `remote_input`, the wire types) at `vault.rs` density
- [ ] Register both new suites in `tests/e2e/run_all.py`'s standalone-shell block
- [ ] Extend `tests/e2e/revocation_test.py` for viewer-WebSocket closure

---

## Manual-Only Verifications

| Behaviour | Requirement | Why Manual | Test Instructions |
|-----------|-------------|------------|-------------------|
| Real two-machine operation over the tailnet | DIST-01 | The e2e harness runs on one host under Xvfb. A loopback or namespace stand-in proves the protocol and the authorisation, not that Tailscale Serve, MagicDNS and the real link behave. `minipc` and `courts-macbook-air` are both on this tailnet if a real run is wanted. | From the client machine, connect to the served URL, attach to an agent tab, click a link, confirm the tab navigates and the frame updates. |
| Perceived responsiveness during takeover | DIST-02 | The suite can assert frame cadence and timestamps; whether takeover *feels* immediate is human judgement, and it is the phase's actual product claim. | Take over an agent tab from the client machine on a direct path; scroll and click; confirm it does not feel like operating a remote desktop. |

*Phase 3 and Phase 4 both closed their visual items by rendering under Xvfb and reviewing frames
rather than leaving them open. The first row here cannot be closed that way — it genuinely needs a
second machine — so it should be stated as a real manual step, not quietly passed.*

---

## Validation Sign-Off

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 120s
- [ ] `nyquist_compliant: true` set in frontmatter

**Approval:** pending

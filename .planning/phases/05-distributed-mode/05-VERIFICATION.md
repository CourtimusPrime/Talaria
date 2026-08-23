---
phase: 05-distributed-mode
verified: 2026-08-24T06:20:00Z
status: human_needed
score: 1/3 must-haves verified
behavior_unverified: 2
overrides_applied: 0
behavior_unverified_items:
  - truth: "SC 1 — a `talaria-client` on a second machine, connected via Tailscale, can list, watch and drive the server's agent tabs"
    test: "Run `scripts/two-machine-check.sh` with the MacBook Air online. Bring up `scripts/tailscale-serve.sh up`, build `talaria-client` on the Mac, point it at the https:// tailnet origin, and confirm it lists the ThinkPad's agent tabs, receives frames, and lands a click."
    expected: "The client lists agent tabs, renders frames, and a click navigates the agent's tab — over a Serve-proxied `wss://` connection with the tailnet Host, TLS terminated by tailscaled."
    why_human: "Every automated assertion runs both ends on one host over loopback under Xvfb. Loopback exercises the protocol, the authorisation and the input path but never Tailscale Serve, TLS termination, the tailnet `Host` header on a real request, or link behaviour. A second machine cannot be simulated the way Phases 3 and 4 closed their visual items under Xvfb."
  - truth: "SC 2 — remote takeover latency stays within ~30–60 ms on a direct WireGuard path"
    test: "During the same two-machine run, with `tailscale status` recorded as reporting a **direct** path (not relayed, not unknown), take over an agent tab and read the client's own `reading.input_to_photon_ms`."
    expected: "The reported input-to-photon estimate sits in the ~30–60 ms band, and the client stays at rung 0 (`link.full`)."
    why_human: "Loopback hides transmission, which is the only variable this criterion turns on. The recorded 31 ms driven median is a same-host number the phase's own suite explicitly refuses to offer as evidence for this target. The build machine additionally has no hardware GL at all — every X display is an Xvfb on llvmpipe — so even the readback figure underlying it is software-rendered."
human_verification:
  - test: "Two-machine run over Tailscale — `scripts/two-machine-check.sh`, MacBook Air client against this ThinkPad, path type recorded"
    expected: "Agent tabs listed, frames rendered, a click lands; `tailscale status` says direct; the client's reported input-to-photon figure written down in milliseconds"
    why_human: "Closes SC 1's transport half and SC 2's direct-path half at once. Neither is producible on one host."
  - test: "Takeover 'feel' impression, recorded separately from the millisecond figure"
    expected: "A human states whether takeover felt immediate — the product claim, distinct from the measurement"
    why_human: "The script asks for the number and the impression separately by design; a run recording only a rung and a feeling has discarded the one number SC 2 rests on."
  - test: "Local single-process takeover latency, re-measured once on this machine"
    expected: "Within Phase 1's recorded band — a confirmation, not a discovery"
    why_human: "SC 3's second clause names latency, and no suite in the repo asserts a local takeover latency number (`takeover_test.py` never did). The structural evidence is strong (render path byte-identical, local suites byte-identical and green, the only local input change is a cost-free function extraction), so this is a low-priority confirmation rather than an open question."
deferred: []
---

# Phase 5: Distributed Mode Verification Report

**Phase Goal:** A remote client on another machine can watch and take over an agent's tab over
Tailscale, hitting the active-takeover latency target on a direct link.
**Verified:** 2026-08-24T06:20:00Z
**Status:** human_needed
**Re-verification:** No — initial verification

---

## Verdict

**The phase built the thing. It did not run the thing on two machines, and it says so.**

Every artifact this phase promised exists, is substantive, is wired, and carries real data. Every
one of the thirty-eight prohibitions across eleven plans holds under direct check. Zero debt
markers. Zero `unwrap()` on any shell path. The MCP tool surface is byte-identical. `gui.rs` is
byte-identical across the entire phase — a genuine 0-byte diff, not a small one.

What is *not* achieved is the part of the goal that lives in the words "on another machine" and "on
a direct link". Both halves of that are unmeasured, and — this is the phase's strongest quality —
**the phase never claims otherwise**. `remote_latency_test.py`'s own docstring refuses the SC 2
claim. 05-10's coverage item D14 carries an empty `verification:` list and `human_judgment: true`.
05-11's D8 does the same. `deferred-items.md` closes with a section titled "What the automated suite
does not — and cannot — prove" that states the condition for revisiting: *"Until somebody runs the
script on a direct path and writes the millisecond figure down, that target is designed for and not
contradicted, rather than measured across two machines."*

That is the correct classification, arrived at by the executor without being forced to. I found no
place where a SUMMARY over-claims. The one discrepancy I found runs the *other* way: the brief given
to me said the shimmed client "walked to rung 3", while `05-10-SUMMARY.md` (lines 169 and 431)
records **rung 2**. The artifact is the more conservative of the two; I take the artifact.

Status is `human_needed`, not `gaps_found`. Nothing is broken, missing, stubbed or unwired. Two
observable truths are present-and-wired but behaviourally unproven, and only a second machine can
prove them.

---

## Goal Achievement

### Observable Truths (ROADMAP Success Criteria)

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | A `talaria-client` on a second machine, connected via Tailscale, can list, watch and drive the server's **agent** tabs — Me tabs refused server-side, not merely hidden (D-05-02) | ⚠️ PRESENT_BEHAVIOR_UNVERIFIED | Refusal half fully verified (1a–1c below); transport half never exercised (1d) |
| 2 | Remote takeover latency stays within ~30–60 ms **on a direct WireGuard path**, and on a relayed path the client degrades and tells the user rather than silently missing the target or refusing takeover (D-05-06) | ⚠️ PRESENT_BEHAVIOR_UNVERIFIED | Degrade-and-report half fully verified (2b–2c below); the direct-path number never measured (2a) |
| 3 | Local mode is provably unaffected — the existing single-process path keeps Phase 1's measured latency | ✓ VERIFIED | `gui.rs` 0-byte diff across the phase; five named local suites 0-byte diff and green; the one local input change read and confirmed cost-free |

**Score:** 1/3 truths verified (2 present, behaviour-unverified)

That headline reads harsher than the phase deserves, because SC 1 and SC 2 are each compound. Split
into their halves, which is what the report below does, the picture is **5 of 6 sub-criteria proven**
— and the one that is not is the same missing run in both rows.

### Success Criteria, split — which half is evidence and which is design

| # | Sub-criterion | Status | Evidence |
|---|--------------|--------|----------|
| 1a | Me tabs refused **server-side**, structurally, not merely hidden | ✓ EVIDENCE | `TabManager::agent_tab` (`tabs.rs:355`) is `find(|t| t.id == id && t.owner.is_agent())` — a lookup, not a check. `ViewTabs::agent_viewport` and `hold_for_view` both route through it, so a human-owned tab is *unrepresentable* rather than refused downstream. |
| 1b | A refused attach does not distinguish a human-owned tab from a nonexistent one | ✓ EVIDENCE | `ServerView::Refused` (`wire.rs:777`) is a fieldless unit variant. `view.rs:1147` — "Every refusal below is the same refusal, with no field to differ in". `remote_view_test.py:787–800` asserts the two encodings byte-for-byte. |
| 1c | A real `talaria-client` lists, watches and drives agent tabs | ✓ EVIDENCE | `remote_view_test.py` drives an actual client binary: `CLIENT LISTED`, `CLIENT ATTACHED`, `CLIENT SHOWING`, `CLIENT AIMED LOWER`/`UPPER`, `CLIENT TYPED`. Coordinate landing asserted on the *specific* element, and the resulting navigation read back over the control socket — not over the frame path under test. |
| 1d | …**on a second machine, connected via Tailscale** | ⚠️ DESIGN | Both ends run on one host under Xvfb. Tailscale Serve, TLS termination, the tailnet `Host` on a real proxied request, and link behaviour are untested by `run_all.py`. 05-01's A2 spike did confirm Serve proxies a WebSocket upgrade and forwards the tailnet `Host` — but with a throwaway example, since deleted, not with Talaria. |
| 2a | ~30–60 ms **on a direct WireGuard path** | ⚠️ DESIGN | Never measured. Loopback driven median is 31 ms, and the suite refuses to offer it: its assertions are a *ratio* (≤ half the passive median) plus a loose 150 ms ceiling, with the comment saying why it is deliberately not 30. |
| 2b | On a constrained link the client degrades and **reports** | ✓ EVIDENCE | Through `link_shim.py` at 13 Mbit/s + 40 ms: walked to **rung 2**, drew `link.degraded`, drew no `link.full`, reported an input-to-photon estimate of **113 ms**. Rung table is integer-ms (`RUNG_LADDER`, `wire.rs:371`); hysteresis proven by `rate::tests::a_long_run_of_samples_exactly_at_the_edge_moves_the_rung_in_neither_direction` — **run independently, passed**. |
| 2c | It **never refuses** takeover because the link is slow | ✓ EVIDENCE | No refusal path exists in `rate.rs` (grep for refuse/block/disable-takeover returns nothing). `STILL REACHED`: a real click through the shim navigated the agent's tab, asserted over the control socket. |
| 3a | The local render path is byte-identical | ✓ EVIDENCE | `git diff c573a34..HEAD -- crates/talaria-shell/src/gui.rs` → **0 bytes**. |
| 3b | The local suites are unmodified and green | ✓ EVIDENCE | `takeover_test.py`, `panel_click_test.py`, `keyboard_nav_test.py`, `http_transport_test.py`, `oauth_flow_test.py` — **0 bytes diff each**; `run_all.py` 24/24, `failed: none`. |
| 3c | The SUMMARY says the true thing rather than overclaiming | ✓ EVIDENCE | 05-11 key-decision: *"Success Criterion 3 is written as behaviour-preserving change under unmodified suites, NOT as an untouched shell. The input forwarders and the visibility synchronisation did change, and a real bug was fixed mid-phase. Claiming an empty diff for files that changed would be false in a way that survives review."* Confirmed against the diff. |

---

## Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `crates/talaria-protocol/src/wire.rs` | Envelope, frame header, input + view messages | ✓ VERIFIED | 1,297 lines. Structural refusal lives here; `Command` appears **0 times** — the view channel structurally cannot carry one. |
| `crates/talaria-protocol/src/local.rs` | Unix helpers moved out of crate root | ✓ VERIFIED | `socket_path()` defined here; **no root re-export**; both call sites name `talaria_protocol::local::`. |
| `crates/talaria-shell/src/view.rs` | View sessions, agent-only lookup, frame pump, cap | ✓ VERIFIED | 2,855 lines. `DEFAULT_MAX_ATTACH = 8` with env override. Names `encode_screenshot` 0×, pending-capture queue 0×, `set_active`/`set_mode`/focus 0×. |
| `crates/talaria-shell/src/remote_input.rs` | Four structural refusals, chrome unreachable | ✓ VERIFIED | 491 lines. `webview_point` 0×, `toolbar_height` 0×, `displayed()` 0×, chrome/`UiAction`/shortcut 0× (the single match is a doc line *naming what it must not reach*). |
| `crates/talaria-shell/src/http.rs` `GET /view` | Own bearer check, not the SDK chain | ✓ VERIFIED | `view_upgrade` calls `route.verified_client(&headers)` directly before `on_upgrade`; one refusal body; `VIEW_PATH` explicitly excluded from the stream-tracking shapes and from the discovery allowlist. |
| `crates/talaria-client/` (6 modules) | Thin client, no engine | ✓ VERIFIED | 3,553 lines. `servo` absent from the manifest **and from `cargo tree`**. Data flows end to end: `net` → `present::decode` → `chrome` render; `input` → `InputMessage` → `net` send; `rate::Report` → `chrome` `link.*` labels. |
| `scripts/tailscale-serve.sh` | Port-scoped, Funnel-refusing | ✓ VERIFIED | `bash -n` clean; **executed** `funnel` argument → refusal message, **exit 1**. |
| `scripts/two-machine-check.sh` | Observes path type, exits non-zero on missing prereq | ✓ VERIFIED | `bash -n` clean, executable; **executed** → printed `PATH: UNKNOWN` (peer offline 46m), named the missing Serve mapping, **true exit 1**. Correctly refuses to read as a pass. |
| `tests/e2e/remote_view_test.py` | 47 named assertions | ✓ VERIFIED | 1,582 lines, registered in `run_all.py`. |
| `tests/e2e/remote_latency_test.py` + `link_shim.py` | Cadence transition + degrade under shim | ✓ VERIFIED | 765 + 238 lines, registered in `run_all.py`. |
| `SECURITY.md` | Fifth party, CSWSH-by-construction, Funnel refused | ✓ VERIFIED | five-parties/remote-human 6 hits; CSWSH 2; origin allowlist 1; Funnel 2; superseded no-TLS bullet **gone** (0 hits); `tailscale-user` named 2×. |
| `CHANGELOG.md` | Phase entry under DIST-01/02 | ✓ VERIFIED | DIST-01 ×3, DIST-02 ×4, "stayed the same" ×1. |

No artifact is missing, stubbed, orphaned or hollow.

---

## Key Link Verification

| From | To | Via | Status | Details |
|------|-----|-----|--------|---------|
| `talaria-client/net.rs` | server `GET /view` | `wss://` + `Authorization: Bearer` | ✓ WIRED | `view_endpoint()` derives `/view` from the base URL; token read from env or owner-only file, never argv, never a log. |
| `net.rs` | `present.rs` | `FrameHeader::from_bytes` → `decode` → `ColorImage` | ✓ WIRED | Real PNG decode with strict RGBA8 gate; keyframe/delta `decide()`. |
| `input.rs` | `net.rs` | `InputMessage` + per-connection `seq` | ✓ WIRED | Inverse fit transform; offset applied at exactly one place; out-of-surface yields `None` (four edge tests). |
| `rate.rs` | `chrome.rs` | `Report` → `link.full`/`link.degraded`/`link.relayed`/`link.measurement` | ✓ WIRED | Plus `reading.rung`, `reading.rung_changes`, `reading.input_to_photon_ms` for the harness. |
| `http.rs` `view_upgrade` | `view.rs` `ViewRoute::run` | `upgrade.on_upgrade` after bearer check | ✓ WIRED | |
| `view.rs` `Handled::Input` | `remote_input::apply` | `app.rs:851` | ✓ WIRED | The **only** caller. Deliberately hands the message back rather than holding a webview, so `remote_input` stays the single wire→engine path. |
| `remote_input::apply` | Servo webview | `app::deliver_mouse_move` / `_button` / `_wheel` | ✓ WIRED | Point passes through untouched — no toolbar subtraction, no cursor-cache write. |
| MCP tool surface | view channel | — | ✓ CORRECTLY ABSENT | `tools.rs` **0-byte diff** across the phase; `ViewSessions`/`ViewRoute`/`view::` unreachable from `talaria-mcp` or `talaria-protocol` (0 hits). |

---

## Data-Flow Trace (Level 4)

| Artifact | Data Variable | Source | Produces Real Data | Status |
|----------|--------------|--------|-------------------|--------|
| `chrome.rs` tab list | agent tabs | `net::Update` ← server `ViewTabs::agent_snapshot` ← `TabManager::agent_tabs()` | Yes — real tab table, insertion order preserved | ✓ FLOWING |
| `present.rs` surface | `ColorImage` | `png` decode of server tile-diff payload | Yes — `Compression::Fast` + `FilterType::Sub` on real Servo readback | ✓ FLOWING |
| `chrome.rs` link banner | `rate::Report` | `RateController` fed by measured round trips | Yes — 113 ms observed under the shim | ✓ FLOWING |
| server webview input | `InputEvent::*` | `remote_input::apply` ← wire `InputMessage` | Yes — asserted by specific-element hit over the control socket | ✓ FLOWING |

---

## Behavioural Spot-Checks (run by this verifier)

| Behaviour | Command | Result | Status |
|-----------|---------|--------|--------|
| A screenshot must not re-hide a tab a viewer holds (the mid-phase bug) | `cargo test -p talaria-shell a_capture_must_not_re_hide_a_tab_a_viewer_is_holding` | 1 passed | ✓ PASS |
| A hold never steals focus from the displayed tab | `cargo test -p talaria-shell a_hold_never_takes_focus_from_the_displayed_tab` | 1 passed | ✓ PASS |
| The ladder cannot oscillate on an edge-parked link | `cargo test -p talaria-client a_long_run_of_samples_exactly_at_the_edge_moves_the_rung_in_neither_direction` | 1 passed | ✓ PASS |
| Client links no web engine | `cargo tree -p talaria-client \| grep -c servo` | 0 | ✓ PASS |
| ~30–60 ms on a direct WireGuard path | — | no second machine reachable (`courts-macbook-air` offline, last seen 46m) | ? SKIP → human |

## Probe Execution

| Probe | Command | Result | Status |
|-------|---------|--------|--------|
| `scripts/two-machine-check.sh` | `bash scripts/two-machine-check.sh` | Named the missing Serve mapping, found the peer, printed `PATH: UNKNOWN`, refused to record a result. **Exit 1.** | ✓ PASS (behaves exactly as specified) |
| `scripts/tailscale-serve.sh` Funnel refusal | `bash scripts/tailscale-serve.sh funnel` | Refusal naming the vault and the logged-in sessions. **Exit 1.** | ✓ PASS |
| Syntax gate | `bash -n` on both scripts | clean | ✓ PASS |

---

## Prohibitions Disposition (38 across 11 plans)

All independently checked. **All hold.**

| Prohibition (source) | Check | Result |
|---|---|---|
| `BIND_HOST` never widened (04, 11) | `http.rs:133 const BIND_HOST: &str = "127.0.0.1"` — unchanged; `settings.rs:296` refuses any non-loopback `bind`, remote access stays **off**; 5 refusal tests present | ✓ HOLDS |
| Talaria handles no certificate (04, 11) | `grep -rniE 'certificate\|\.pem\|cert_path\|ssl_cert\|private key' crates/talaria-shell/src/` → **0** | ✓ HOLDS |
| Client never relaxes verification (07) | `grep -rniE 'danger\|no_verify\|accept_invalid\|insecure_skip\|dangerous' crates/talaria-client/src/` → **0**; certificate vocabulary survives in 3 files as explanation | ✓ HOLDS |
| Local render path unchanged (08, 11) | `gui.rs` 0-byte diff across `c573a34..HEAD` | ✓ HOLDS |
| Remote input never writes `webview_point` (06) | 0 hits | ✓ HOLDS |
| Remote input never applies the toolbar offset (06) | 0 hits | ✓ HOLDS |
| Remote input never resolves the displayed tab (06) | 0 hits | ✓ HOLDS |
| Remote input reaches no chrome (06) | 0 code hits (1 doc line naming the prohibition) | ✓ HOLDS |
| No MCP tool / control-socket command reaches the view channel (05, 06) | `tools.rs` 0-byte diff; view symbols unreachable from mcp/protocol → **0** | ✓ HOLDS |
| `Command` gains no input variant (03) | Enum unchanged: 11 variants, none input-shaped | ✓ HOLDS |
| The view channel carries no `Command` (03) | `grep -c Command wire.rs` → **0** | ✓ HOLDS |
| Unix helpers not re-exported from crate root (03) | No root re-export; both call sites name `local::` | ✓ HOLDS |
| `encode_screenshot` untouched, uncalled from the frame path (02, 08) | 0-byte diff; call sites still **1**; named 0× in `view.rs` | ✓ HOLDS |
| Frame pump uses no pending-capture queue/deadline (08) | 0 hits in `view.rs` | ✓ HOLDS |
| A viewer never changes the displayed tab, view mode or focus (08) | `set_active`/`set_mode`/focus → **0** in `view.rs`; `a_hold_never_takes_focus_from_the_displayed_tab` passes | ✓ HOLDS |
| Refusal carries no reason (05) | `ServerView::Refused` is a fieldless unit variant; byte-for-byte e2e assertion | ✓ HOLDS |
| View path never in the discovery allowlist (05) | `is_public_discovery` gate; `VIEW_PATH` explicitly excluded and named in the routing test | ✓ HOLDS |
| Advertised identity never derived from a request header (04) | `header::HOST` read at **exactly 1 place** (`refuse_page_originated`), used only for an allowlist comparison, assigned nowhere | ✓ HOLDS |
| No resize request on the wire (09) | `resize` in `wire.rs` → 1 hit, a doc comment about keyframes; 0 in `net.rs` | ✓ HOLDS |
| Out-of-surface pointer not sent, not clamped (09) | Four named edge tests present (`one_point_outside_the_{left,right,top,bottom}_edge_yields_nothing`) | ✓ HOLDS |
| Client never refuses takeover for a slow link (10) | No refusal path in `rate.rs`; `STILL REACHED` e2e | ✓ HOLDS |
| No latency claim from a loopback measurement (10) | Suite docstring refuses it explicitly; assertions are ratios/transitions | ✓ HOLDS |
| Ladder cannot oscillate (10) | Named edge test **run, passed** | ✓ HOLDS |
| Cadence arithmetic in whole ms (10) | `RUNG_LADDER` intervals are `u32` literals 30/60/60/120/250 | ✓ HOLDS |
| Serve mappings undisturbed; Funnel never invoked (01, 04, 11) | `tailscale serve status`: `/`→8086, `:8443`→`127.0.0.1:5678`, `:9446`, `:9447` all present; **no phase-5 mapping left behind**; `funnel` appears only in the refusal and the threat model | ✓ HOLDS |
| Lockfile pins `primeorder 0.14.0-rc.14` (01, 07, 11) | `grep -A1` → `version = "0.14.0-rc.14"` | ✓ HOLDS |
| Spike sources do not survive (01, 02) | No `examples/` dir; no spike source tracked | ✓ HOLDS |
| No new codec adopted (D-05-05) | `png::Compression::Fast` + `FilterType::Sub`; no webp/jpeg/zstd/lz4/av1/vp8/vp9 anywhere in any manifest | ✓ HOLDS |

---

## Locked Decisions (6/6 held)

| Decision | Held | Evidence |
|---|---|---|
| **D-05-01** Thin client | ✓ | `talaria-client` links no engine — absent from manifest *and* `cargo tree`. Builds without Servo's native prerequisites. |
| **D-05-02** Agent tabs only, refused server-side | ✓ | `agent_tab()` filter is a lookup; `agent_viewport`/`hold_for_view` both route through it; reasonless refusal proven byte-for-byte. |
| **D-05-03** Serve terminates TLS, `BIND_HOST` constant | ✓ | Zero certificate vocabulary in the shell; `BIND_HOST` unchanged at one place; `bind` config fails closed. |
| **D-05-04** 5 / 5.1 split | ✓ | DIST-03/04 carried into Phase 5.1 in ROADMAP and in `deferred-items.md`; not silently dropped. |
| **D-05-05** png `Fast` on tile diffs, no new codec | ✓ | `Compression::Fast` + `Sub`; no codec dependency added. 05-02 additionally **refuted** A8 honestly (synthetic frames bracket real Servo on time but not bytes) and 05-03 corrected the decision's own false premise about `encode_screenshot`. |
| **D-05-06** Degrade and report, never refuse | ✓ | Ladder walked to rung 2 under the shim, `link.degraded` drawn, 113 ms reported, **click still landed**. No refusal path exists. |

---

## Requirements Coverage

| Requirement | Description | Status | Evidence |
|---|---|---|---|
| DIST-01 | Client and server can run on separate machines connected via Tailscale | ? NEEDS HUMAN | The client/server split, the transport, the authorisation and the Serve script all exist and work over loopback. The words *"on separate machines"* are exactly the untested clause. |
| DIST-02 | A human can watch and take over an agent's tab when the server is remote, with adaptive frame polling (~200–500 ms passive, ~30–60 ms during takeover) | ✓ SATISFIED (with caveat) | The adaptive polling itself **is** measured: passive 251 ms → driven 31 ms → released 251 ms, plus the boundary case. This requirement's own text does not say "direct WireGuard path"; SC 2 does. The caveat is that the takeover is remote-over-loopback. |

**Ledger note (WARNING):** `.planning/REQUIREMENTS.md:171–172` marks both DIST-01 and DIST-02
**Complete**. For DIST-02 that is defensible on the measured numbers. For DIST-01 it is ahead of the
evidence — the criterion's distinguishing clause is the one clause never exercised. This is a
documentation state, not a code defect, and it is the human's call whether to leave it pending the
two-machine run or annotate it. Flagged rather than failed.

No orphaned requirements: REQUIREMENTS.md maps only DIST-01/02 to Phase 5, and both are claimed by
plans.

---

## Anti-Patterns Found

Scanned all 28 source/script/test files changed in the phase (`c573a34..HEAD`).

| File | Line | Pattern | Severity | Impact |
|------|------|---------|----------|--------|
| — | — | `TBD` / `FIXME` / `XXX` | — | **0 found** |
| — | — | `TODO` / `HACK` / `PLACEHOLDER` | — | **0 found** |
| — | — | `unimplemented!` / `todo!(` / "not yet implemented" / "coming soon" | — | **0 found** |
| — | — | `unwrap()` on a shell path | — | **0** in `app.rs`, `view.rs`, `remote_input.rs`, `http.rs` (non-test) |

Nothing to report. The project's stated convention (`expect()` only for genuine invariants, no
`unwrap()` in shell code, agent-facing failures as values) is upheld throughout the new code.

---

## Human Verification Required

### 1. The two-machine run — closes SC 1's transport half and SC 2's direct-path half at once

**Test:** Bring `courts-macbook-air` online (it was offline, last seen 46m ago, when this
verification ran). On this ThinkPad: `scripts/tailscale-serve.sh up`, and set `config.json`'s
`remote_access.enabled`, `port`, and `advertised_url`. On the Mac:
`cargo build --release -p talaria-client` (seconds — no engine), point it at the `https://` tailnet
origin, approve it once from the ThinkPad's own chrome. Then run
`scripts/two-machine-check.sh` and follow its numbered steps.

**Expected:** The client lists the ThinkPad's agent tabs, renders frames, and a click lands.
`tailscale status` reports **direct** for the peer. The client's `reading.input_to_photon_ms` sits
in the ~30–60 ms band with `link.full` drawn.

**Why human:** Phases 3 and 4 closed their visual items by rendering the chrome under Xvfb and
reviewing frames. That technique cannot reach here — a second machine is not something one host can
fake, and loopback removes precisely the variable SC 2 measures. The script deliberately does not
drive the Mac: automating a human judgement would produce a green result for a question only a human
can answer.

**Note on the readback figure:** even the underlying 1.19–1.56 ms `read_to_image` number is
software-rendered. There is no hardware GL on this machine at all — every X display is an Xvfb on
llvmpipe, and reaching either GPU would need a Wayland compositor (none installed) or an Xorg
holding DRM master (`Xwrapper.config` restricts that to `allowed_users=console`). Under software
rendering the framebuffer is already in system memory, so readback is close to a `memcpy` and
`paint()` carries the cost; **on a GPU that inverts**. The figure is plausibly optimistic and is
correctly labelled as such in `deferred-items.md`.

### 2. The impression, recorded separately from the number

**Test:** During the same run, state whether takeover **felt** immediate.

**Expected:** A recorded human impression, written down apart from the millisecond figure.

**Why human:** The script asks for both and labels them as different kinds of thing, so neither can
quietly stand in for the other. A run that recorded only a rung and a feeling would have discarded
the one number SC 2 rests on.

### 3. Local takeover latency, re-measured once (low priority)

**Test:** Run a local takeover on this machine and note the latency.

**Expected:** Within Phase 1's recorded band.

**Why human:** SC 3's second clause names latency, and no suite in the repo asserts a local takeover
latency number — `takeover_test.py` never did, before or after this phase. The structural evidence
is strong enough that I marked SC 3 VERIFIED rather than downgrading it: the render path is
byte-identical, the local suites are byte-identical and green, and I read the one local input-path
change directly — the extracted `deliver_mouse_move`/`_button`/`_wheel` perform the same
`viewport_of(webview).contains(point)` test that was previously inline, adding at worst a function
call to a path already doing engine work. This item is a confirmation, not an open question.

---

## Honest Gaps — classification audit

The six gaps named in the brief, judged against the artifacts:

| # | Gap | Recorded classification | Verdict |
|---|-----|------------------------|---------|
| 1 | The two-machine run is manual | Manual-only, `05-VALIDATION.md` D15 → `scripts/two-machine-check.sh` | ✓ **Correct.** The script exists, runs, observes the path type rather than assuming it, and exits 1 when it could not have proved anything — verified by execution. |
| 2 | SC 2 unmeasured on a direct path | `human_judgment: true`, empty `verification:` (05-10 D14, 05-11 D8); refused in the suite docstring; revisit condition written into `deferred-items.md` | ✓ **Correct, and unusually well done.** Three independent places refuse the claim. This is the honest classification, not a hedge. |
| 3 | First pairing requires someone at the server machine | Open product decision, accepted for this phase; RFC 8628 and a pairing code both noted as strictly additive | ✓ **Correct.** And the client's first-run copy says so *plainly* in the UI (`chrome.rs:529`): "Getting a token needs someone at the server machine." A gap the product tells the user about is a documented limitation, not a hidden one. |
| 4 | A remote human at a login wall has no address bar | By design — the view channel carries no `Command` | ✓ **Correct, and structurally enforced.** `grep -c Command wire.rs` → 0. It is not a missing feature that could regress; it is unrepresentable. Correctly paired with #3 in `05-CONTEXT.md` as one question. |
| 5 | A viewer sees every agent's tabs, not only its own client's | Decided (T-05-10), documented at the code site with its reasoning and its bound (who holds a token, plus a revoke) | ✓ **Correct.** Decided rather than defaulted, and the decision is written where the next reader will hit it. |
| 6 | The Xvfb readback figure is plausibly optimistic | "must not be presented as evidence about SC 2 on real hardware" — T-05-12-A's mitigation working as intended | ✓ **Correct, and self-aware.** It also names the number that *would* break the model: a hardware `read_to_image` **slower** than the software figure. That is a falsifiable statement, which is the right shape for a deferred item. |

**All six are correctly classified.** I found no gap that was under-stated, and none that was
recorded as closed while open.

---

## What I Checked That the SUMMARYs Could Not Have Faked

- `gui.rs` byte-identity is a real `git diff` over the real commit range, not a claim.
- The three behaviour-dependent invariants were run as named tests in this session, not read about.
- Both scripts were **executed**, and their exit codes taken from `$?` directly rather than through
  a pipe (the naive `| tail` reading gives a misleading `0`).
- `tailscale serve status` was read live: all four pre-existing mappings intact, no phase-5 mapping
  orphaned.
- The mid-phase bug fix `2dd72b5` was read in full. It is a genuine silent-failure bug — a routine
  agent `screenshot` re-hid a tab a viewer was holding, so frames kept arriving from the offscreen
  buffer while clicks stopped landing and the hold count still read 1. The fix routes through
  `visibility_of`, the single source of truth, rather than re-deriving the rule at the call site, and
  the regression test pins the predicate including the two-viewers and viewer-on-the-displayed-tab
  cases. This corroborates SC 3's honesty claim rather than undermining it.

---

## Gaps Summary

There are no gaps in the engineering sense: nothing missing, stubbed, unwired, or hollow, and no
prohibition violated. What is outstanding is a **measurement that requires a second machine**, and
it blocks the same two clauses in SC 1 and SC 2.

The phase's own posture on this is the right one and should be preserved verbatim when the run
happens: *"designed for and not contradicted"* is the current standing of the ~30–60 ms target, and
it should not be upgraded to "met" by anyone citing the 31 ms loopback number. The number that
closes it is the client's own `reading.input_to_photon_ms`, taken while `tailscale status` says
**direct**, written down.

Phase 5.1 does not address either item (it is reconnect and persistence), and Phase 6's performance
criterion is about local startup and screenshot capture on macOS/Windows, not remote takeover
latency. **Nothing is deferred to a later phase** — these belong to Phase 5 and stay here.

---

_Verified: 2026-08-24T06:20:00Z_
_Verifier: Claude (gsd-verifier)_

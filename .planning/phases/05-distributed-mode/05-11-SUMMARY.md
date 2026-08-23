---
phase: 05-distributed-mode
plan: 11
subsystem: infra
tags: [security, threat-model, documentation, tailscale, websocket, cswsh, changelog]

# Dependency graph
requires:
  - phase: 05-distributed-mode
    plan: 01
    provides: "the corrected `AxumServerOptions` note, the observed Serve-proxied header set including `tailscale-user-login`, the first-pairing question stated as open, and the measurement that the WebSocket dependency costs one package"
  - phase: 05-distributed-mode
    plan: 03
    provides: "the corrected `lib.rs` and `control.rs` module headers — the source the generated architecture section derives from, and therefore the reason the correction sticks"
  - phase: 05-distributed-mode
    plan: 04
    provides: "`scripts/tailscale-serve.sh` — its prerequisite checks, port scoping and Funnel refusal, which the two-machine script composes with rather than repeats; and the advertised identity that discharges Phase 4's TLS obligation"
  - phase: 05-distributed-mode
    plan: 05
    provides: "what `GET /view` actually enforces — its own direct bearer check, the agent-only lookup, the reasonless refusal, the socket a revoke closes — so the published document describes the code rather than the plan"
  - phase: 05-distributed-mode
    plan: 06
    provides: "what the input path actually reaches (a webview and nothing else) and the per-connection sequence rule"
  - phase: 05-distributed-mode
    plan: 09
    provides: "`05-VALIDATION.md`'s coverage item D15 — the manual-only two-machine row — and the client's reported input-to-photon estimate, which is what the script asks a human to write down"
  - phase: 05-distributed-mode
    plan: 10
    provides: "the rung ladder and the direct-or-relayed path lookup the client reports, which is why the script's path-type reading and the client's agree"
  - phase: 04-authenticated-remote-transport-v2
    provides: "the TLS obligation recorded in its deferred register, and the changelog register (user-visible change plus a what-stayed-the-same paragraph) this phase's entry is written in"
provides:
  - "`SECURITY.md` describing five parties rather than four, with the remote human placed between the human at the keyboard and the connected agent and characterised in both directions"
  - "The CSWSH-by-construction property written down as a property to keep, with its consequence (a browser-based viewer is structurally impossible) and the exact shape of its deletion (a diff adding an origin allowlist)"
  - "Tailscale Funnel refused by name in the published threat model, with its reason — a named non-option rather than an unconsidered one"
  - "The superseded `There is no TLS, and there is no non-loopback bind` bullet replaced, not annotated: the bind is permanent, encryption is the overlay daemon's, and this browser handles no certificate"
  - "The overlay daemon's injected `tailscale-user-*` headers named as a non-boundary"
  - "`CHANGELOG.md`'s phase-level entry naming DIST-01 and DIST-02, plus the what-stayed-the-same paragraph"
  - "The `talaria-protocol`-is-the-wire claim retired from `.claude/CLAUDE.md` and `.planning/PROJECT.md`, with a note that the architecture section is generated from the module headers and that those are the source of truth"
  - "`scripts/two-machine-check.sh` — prerequisite-checking, path-type-observing, exits non-zero when it could not have proved anything"
  - "Four closing `deferred-items.md` entries, including what the automated suite cannot prove and why"
  - "Success Criterion 3's evidence, collected rather than asserted, including the honest account of the two things that did change"
affects: [phase-05.1, phase-06, phase-07]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "A published control for a future person's decision: when the threat is that somebody later deletes a layer without knowing what it was for, the only control that reaches them is a written one placed where they will read it — so the property, its consequence AND the shape of its deletion all go in the document"
    - "A correction in a generated document carries a note naming its source, or the next regeneration silently restores the error"
    - "The evidence and the impression are asked for separately, and labelled as different kinds of thing, so neither can quietly stand in for the other"

key-files:
  created:
    - "scripts/two-machine-check.sh"
  modified:
    - "SECURITY.md"
    - "CHANGELOG.md"
    - ".claude/CLAUDE.md"
    - ".planning/PROJECT.md"
    - ".planning/phases/05-distributed-mode/deferred-items.md"

key-decisions:
  - "The certificate-absence gate is scoped to `crates/talaria-shell/src/` and deliberately NOT extended to `crates/talaria-client/src/` — 05-07 makes it a truth that the client verifies the server's certificate against the platform trust roots and names `ConnectionState::Untrusted` for the failure. Scrubbing that vocabulary to satisfy a server-side grep would delete the explanation of a security-critical behaviour. The client is asserted in the opposite direction instead: no `danger`/`no_verify`/`accept_invalid`/`insecure_skip` anywhere in it."
  - "Success Criterion 3 is written as behaviour-preserving change under unmodified suites, NOT as an untouched shell. The input forwarders and the visibility synchronisation did change, and a real bug was fixed mid-phase. Claiming an empty diff for files that changed would be false in a way that survives review."
  - "The two-machine script does not drive the second machine. A script that automated a human judgement would produce a green result for a question only a human can answer."
  - "The script asks for the millisecond figure AND the impression, separately and labelled. A run recording only a rung and a feeling has discarded the one number SC 2 rests on."

patterns-established:
  - "Recursive predicate greps (`! grep -rqi … dir/`) rather than counting greps against a directory — a `grep -c` without `-r` against a directory errors, exits 2 and prints nothing, so an 'is 0' reading passes on a command that searched nothing"
  - "A superseded claim in a published document is replaced, never left beside its replacement: a document with two contradictory statements is worse than one with the wrong statement, because a reader believes whichever they found first"

requirements-completed: [DIST-01, DIST-02]

coverage:
  - id: D1
    description: "The published threat model names five parties, including the remote human — more trusted than an agent, less verifiable than the human at the keyboard, and the first party that sends raw input rather than tool calls"
    requirement: "DIST-02"
    verification:
      - kind: other
        ref: "grep -ci 'five parties\\|fifth party\\|remote human' SECURITY.md → 6 (≥2 required)"
        status: pass
    human_judgment: false
  - id: D2
    description: "CSWSH is recorded as closed by construction, with the consequence that a browser-based viewer is structurally impossible and the shape of the diff that would delete it"
    requirement: "DIST-02"
    verification:
      - kind: other
        ref: "grep -ci 'cross-site websocket\\|CSWSH' SECURITY.md → 2; grep -ci 'origin allowlist' SECURITY.md → 1"
        status: pass
    human_judgment: false
  - id: D3
    description: "The Tailscale Funnel variant is refused by name in the published threat model, with its reason"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "grep -ci 'funnel' SECURITY.md → 2; the only other occurrence under scripts/ is the refusal in tailscale-serve.sh"
        status: pass
    human_judgment: false
  - id: D4
    description: "The superseded no-TLS/no-non-loopback bullet is replaced: the bind is permanently loopback, encryption is terminated by the overlay daemon, and this browser handles no certificate"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "grep -ci 'There is no TLS, and there is no non-loopback bind' SECURITY.md → 0; ! grep -rqi 'certificate|.pem|cert_path|ssl_cert|private key' crates/talaria-shell/src/ → clean"
        status: pass
    human_judgment: false
  - id: D5
    description: "The changelog records what shipped under DIST-01 and DIST-02, with the listener off by default, the agent-tabs-only scope, and a what-stayed-the-same paragraph"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "grep -c 'DIST-01' CHANGELOG.md → 3; 'DIST-02' → 4; 'off by default' → 2; 'agent tabs' → 1; 'stayed the same' → 1"
        status: pass
    human_judgment: false
  - id: D6
    description: "The talaria-protocol-is-the-wire claim is retired from both the generated instructions and PROJECT.md, with a source-of-truth note that survives regeneration"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "! grep -qi 'intended distributed-mode wire' .claude/CLAUDE.md .planning/PROJECT.md → no match; grep -ci 'shared vocabulary' .claude/CLAUDE.md → 4; generated/derived-from note present"
        status: pass
    human_judgment: false
  - id: D7
    description: "The one verification this harness cannot perform is a runnable script that observes and prints the path type and exits non-zero on a missing prerequisite"
    requirement: "DIST-02"
    verification:
      - kind: other
        ref: "test -x scripts/two-machine-check.sh && bash -n scripts/two-machine-check.sh → 0; exercised against an offline peer (unknown path, exit 1) and against minipc (direct path, full steps)"
        status: pass
    human_judgment: false
  - id: D8
    description: "The real two-machine run itself — a direct-path takeover from a second host, with the client's input-to-photon estimate in milliseconds written down"
    verification: []
    human_judgment: true
    rationale: "Two machines are not something a suite on one machine can produce. The automated suite runs both ends on one host under Xvfb, so loopback hides transmission — the only variable SC 2 is about. TLS termination, the tailnet Host and real link behaviour are exercised only by a human running scripts/two-machine-check.sh, and 'did takeover feel immediate' is a judgement no assertion replaces."
  - id: D9
    description: "Success Criterion 3's evidence collected: the local render path byte-identical across the phase, five local suites unmodified and passing, and an honest account of the two things that did change"
    requirement: "DIST-02"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/run_all.py → 24/24, failed: none"
        status: pass
      - kind: other
        ref: "git diff --stat <phase-base>..HEAD -- crates/talaria-shell/src/gui.rs → empty; same for takeover/panel_click/keyboard_nav/http_transport/oauth_flow suites"
        status: pass
    human_judgment: false

# Metrics
duration: 40min
completed: 2026-08-23
status: complete
---

# Phase 5 Plan 11: Phase Close Summary

**The published threat model now describes five parties rather than four, the property that makes a browser-based viewer structurally impossible is written down along with the shape of the diff that would delete it, three now-wrong claims are corrected at their source, and the one verification this harness cannot perform is an executable that reports the path type it observed.**

## Performance

- **Duration:** ~40 min
- **Started:** 2026-08-23T19:57:00Z
- **Completed:** 2026-08-23T20:36:00Z
- **Tasks:** 3 of 3
- **Files modified:** 5 modified, 1 created

## Accomplishments

- **`SECURITY.md` governs the system that now exists.** The trust model went from four parties to five. The remote human sits between the human at the keyboard and the connected agent, characterised in both directions — more trusted than an agent because they are the trust root at a distance and the document already forbids quietly restricting the human path; less verifiable than the human at the keyboard because their identity is a bearer token rather than physical presence. And the sentence that matters most: this is the first party in this browser's history that sends **raw input** rather than tool calls, which is a different kind of reach because there is no list of what it can do.

- **The fourth party's size is now set outside this repository, and the document says so.** "Anything that can open a connection to the loopback listener" was a sentence about one machine; under overlay-network exposure it becomes "any device in the tailnet, plus whatever its access rules admit" — governed by a web console, not by this source tree. What did *not* change is recorded beside it: between the daemon and the browser the traffic is still plain HTTP on loopback, every local account can still reach the port, and the token is still the entire local boundary.

- **The CSWSH-by-construction property is on the record as a property to keep.** `refuse_page_originated` is the outermost layer and refuses *any* request carrying an `Origin` header. WebSockets have no browser-side origin enforcement and no preflight, so a browser can never connect — which is why `talaria-client` is a native binary as a **consequence** of that layer rather than as a preference. The document names the exact shape of its deletion (a diff replacing the blanket refusal with an origin allowlist, "so a small web viewer can connect"), so a reader who meets one recognises it as removing a stated guarantee rather than as a convenience.

- **Two further named non-options.** Tailscale Funnel is refused by name with its reason — this process holds a credential vault and a logged-in browsing session, and the two commands differ by a few characters. And the daemon's injected `tailscale-user-login` / `-name` / `-profile-pic` headers are named as a **non-boundary**: spoofable by any local process that can reach the loopback port, so the token is checked first and they are at most a second check.

- **The superseded transport bullet is replaced, not annotated.** "There is no TLS, and there is no non-loopback bind" is gone. What stands in its place: the bind is `127.0.0.1` permanently rather than provisionally, because reach comes from putting a daemon in front of the listener rather than from widening it; every published OAuth URL uses the secure scheme once an identity is advertised, which is OAuth 2.1 §1.5 met rather than excepted; and this browser issues, loads and renews **no certificate at all** — no expiry timer, no resolver re-reading files, no day-ninety-one outage in the process that also holds the vault.

- **`CHANGELOG.md` carries the phase-level entry** naming DIST-01 and DIST-02 in the user-visible register Phase 3 and Phase 4 used, ending with the paragraph a reader most wants — what stayed the same: local browsing unchanged, the chrome module byte-identical, the agent tool surface gained nothing (so a remote click is not something an agent can ask for), the screenshot tool unchanged, the listener still off by default and still loopback-only, and the control socket's peer-credential check intact.

- **Three claims corrected at the place that produces them.** `.claude/CLAUDE.md`'s architecture section no longer calls `talaria-protocol` the distributed mode's wire; it is the shared vocabulary three named transports carry. The section now opens with a note that it is **generated from the module headers** and that those are the source of truth — so the next regeneration reproduces 05-03's correction rather than restoring the error. `.planning/PROJECT.md`'s transport constraint, its OAuth and distributed-mode checklist items, two decision rows and the crate count were corrected against what the file actually said rather than against what a plan assumed it said.

- **`scripts/two-machine-check.sh`** turns the one manual verification into a runnable step. It checks the prerequisites it can and defers the proxy mapping to `scripts/tailscale-serve.sh`; **reads the path type from the daemon's own status and prints it prominently**, with a different claim licensed for direct, relayed and unknown; prints the numbered human steps including that the first authorisation happens in the server machine's own chrome; asks afterwards for four things and says which is which; and exits non-zero when a prerequisite is missing.

## Task Commits

1. **Task 1: Rewrite the published threat model for the party this phase adds** — `87bacd4` (docs)
2. **Task 2: The changelog entry, and the three claims that are now wrong** — `634e3e1` (docs)
3. **Task 3: The real two-machine run as a script, and SC 3's evidence collected** — `939af4a` (docs)

## Files Created/Modified

- `SECURITY.md` — five parties; the fourth party's widened set; the view channel in "what is enforced today"; three new subsections (the native-viewer property, the Funnel refusal, what a remote viewer is and is not) plus the injected-headers non-boundary; the replaced transport bullet; both known limitations widened by one sentence
- `CHANGELOG.md` — the phase-level DIST-01/DIST-02 entry with its what-stayed-the-same paragraph, placed above the per-plan entries earlier plans in this phase already wrote
- `.claude/CLAUDE.md` — the retired wire claim, the three transports named one line each, the generated-from-module-headers note, the component table row, the crate count and the language line
- `.planning/PROJECT.md` — the transport constraint rewritten, two checklist items marked shipped, two decision rows resolved, the crate count
- `.planning/phases/05-distributed-mode/deferred-items.md` — four closing entries appended (19 sections total)
- `scripts/two-machine-check.sh` — **new**, executable

## Success Criterion 3: the evidence, collected

The claim is *local mode is provably unaffected*. Here is the checkable version, in four parts, and the fourth is the one that needs saying carefully.

**1. The local render path is byte-identical across the entire phase.**

```
git diff --stat c573a34..HEAD -- crates/talaria-shell/src/gui.rs
```

produces nothing. `gui.rs` appears in no plan's `files_modified` and was touched by none of them.

**2. The local end-to-end suites pass unmodified.** The same diff over `takeover_test.py`, `panel_click_test.py`, `keyboard_nav_test.py`, `http_transport_test.py` and `oauth_flow_test.py` is likewise empty, and all five pass: takeover 28s, panel clicks 86s, keyboard navigation 32s, HTTP transport 45s, OAuth flow 104s. Full suite **24/24, `failed: none`**.

**3. The supporting invariants are unchanged.**

| Check | Result |
|---|---|
| `grep -c 'const BIND_HOST' crates/talaria-shell/src/http.rs` | 1, `= "127.0.0.1"` |
| `cargo test -p talaria-shell --locked settings::` | 51 passed, 0 failed |
| `grep -c 'encode_screenshot' crates/talaria-shell/src/app.rs` | 2 — the definition and its one call site |
| `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'` | 1 |
| phase lockfile diff | 45 insertions, **two** new `[[package]]` blocks: `talaria-client` (the workspace member) and `tokio-tungstenite 0.29.0` |
| `! grep -rqi 'certificate\|\.pem\|cert_path\|ssl_cert\|private key' crates/talaria-shell/src/` | clean — the server handles no certificate |
| `! grep -rqi 'danger\|no_verify\|accept_invalid\|insecure_skip' crates/talaria-client/src/` | clean — the client never relaxes verification |
| `tailscale serve status` before/after | byte-identical (`f3df1875…`); all four pre-existing mappings including `:8443 → 127.0.0.1:5678` untouched |

The lockfile diff matches the phase's two recorded measurements exactly: 05-01's WebSocket package (`tokio-tungstenite`, the one new package), and 05-07's client transport-security set (`rustls`, `rustls-native-certs`, `tokio-rustls` were already resolved — **zero** new packages, which is the number 05-01 measured in advance).

**4. And the honest part: two things did change, behaviour-preservingly.**

Success Criterion 3 is proven by **behaviour-preserving change under unmodified suites**, not by an empty diff on the shell. Claiming otherwise would be false in exactly the way that survives review, and this phase already corrected one inherited note for the same reason.

- **The three pointer forwarders were split.** They now take a webview and a point *already relative to that tab's viewport*; the local window's toolbar-height subtraction moved out to the local call site. A remote client draws no server toolbar, so a shared subtraction would have put a silent toolbar-height error on every remote click that nothing would report. The local caller keeps the offset and the cursor cache; a remote caller supplies its own coordinates. Behaviour preservation is proven by `takeover_test.py`, `panel_click_test.py` and `keyboard_nav_test.py` passing unmodified — the suites that would notice a click landing one toolbar height off.
- **The tab visibility synchronisation learned about holds.** `sync_visibility` now consults a per-tab hold count so a tab a viewer is watching stays shown through a tab-set change the local human caused. The tab the human is displaying still always wins, and a held tab is shown but deliberately not focused.
- **A real bug in that area was found and fixed mid-phase** (`2dd72b5`): a background `screenshot` capture used to re-hide a webview without consulting the hold count, so a viewer's clicks silently stopped landing while its frames carried on arriving. It was found because the register recorded it rather than because a suite caught it, which is itself worth noting — the regression test now lives next to the fix.

## Decisions Made

1. **The certificate gate is server-scoped on purpose.** `! grep -rqi 'certificate|…' crates/talaria-shell/src/` asserts the *server* handles no certificate. `crates/talaria-client/src/` is a deliberate exception and was left alone: 05-07 makes it a truth that the client verifies the server's certificate against the platform trust roots and names `ConnectionState::Untrusted` for the failure. Scrubbing that vocabulary to pass a server-side gate would delete the explanation of a security-critical behaviour. The client is asserted in the opposite direction instead.

2. **Recursive predicates, not counting greps.** These checks are `! grep -rq … dir/`, not `grep -c … dir/`. A counting grep against a directory without `-r` errors, exits 2 and prints nothing — so an "is 0" reading passes on a command that searched nothing. Three revisions of plan review went into that shape.

3. **The script does not drive the second machine.** The questions left after the automated suite are "did takeover feel immediate" and "did the click land where I aimed". Automating a human judgement would produce a green result for a question only a human can answer.

4. **The evidence and the impression are asked for separately.** The client already computes the input-to-photon estimate from the frame header's `last_applied_input` echo; asking only for a rung and a feeling would discard the one number SC 2's headline claim rests on. Both are requested, each labelled as what it is.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 — Factual error in the plan] The plan asserted no earlier plan in this phase touched `CHANGELOG.md`; several had**

- **Found during:** Task 2
- **Issue:** The plan's `<interfaces>` said to write the phase's changelog entry "once, here — no earlier plan in this phase touches the file, matching how Phase 4 handled it." In fact 05-01, 05-08, 05-09 and 05-10 had all appended entries: `grep -n 'DIST-02' CHANGELOG.md` already returned two hits before this plan ran. Following the instruction literally would have produced either a duplicate account of the same work or a phase-level entry that contradicted the per-plan ones sitting below it.
- **Fix:** Wrote the phase-level entry at the **top** of `### Added` (the section is newest-first), covering DIST-01's headline, DIST-02's headline, and the what-stayed-the-same paragraph the existing entries lacked — and explicitly framed as "this is the phase's headline, and the entries below are its parts". The existing entries were left intact: they describe what shipped, in the right register, and deleting them to satisfy a plan's assumption would have destroyed accurate work.
- **Files modified:** `CHANGELOG.md`
- **Verification:** `grep -c 'DIST-01' → 3`, `'DIST-02' → 4`, `'off by default' → 2`, `'agent tabs' → 1`, `'stayed the same' → 1`; `cargo build --release --locked` exits 0
- **Committed in:** `634e3e1`

**2. [Rule 3 — Acceptance criterion with a false-positive trap] The Funnel-uniqueness grep matches an unrelated English word**

- **Found during:** Task 1
- **Issue:** The criterion `grep -rci 'funnel' scripts/ crates/ | grep -v ':0$' | grep -vc 'tailscale-serve.sh'` is specified as 0. It is 1, because `crates/talaria-shell/src/control.rs:220` contains `// funnel through one channel so a dedicated writer task can interleave` — the ordinary English verb, in a comment about channel multiplexing, written long before Tailscale Funnel was a consideration in this repository.
- **Fix:** **Nothing changed in `control.rs`.** The criterion's *intent* — that the Tailscale Funnel variant is invoked nowhere and appears only in refusals — holds: the only product-sense occurrences under `scripts/` and `crates/` are the refusal in `scripts/tailscale-serve.sh`. Editing a correct, unrelated comment to satisfy a grep would be a change made for a checker rather than for a reader, and it touches shell source this plan is otherwise not permitted to modify.
- **Files modified:** none
- **Verification:** `grep -ni 'funnel' crates/talaria-shell/src/control.rs` shows the single line and its context; `grep -rli 'funnel' scripts/ crates/` returns exactly `scripts/tailscale-serve.sh` and that one file.
- **Committed in:** n/a — recorded here rather than actioned

**3. [Rule 3 — Degenerate criterion] `git merge-base HEAD main` is `HEAD` on this branch**

- **Found during:** Task 3
- **Issue:** Task 3's diff criteria are written as `git diff --stat $(git merge-base HEAD main)..HEAD -- …`. This phase executed **on `main`**, so the merge base is `HEAD` and the diff is empty for every path — the criterion would pass without checking anything, which is precisely the failure mode the plan's own prohibitions warn about.
- **Fix:** Used the phase's real commit range instead: `c573a34..HEAD`, where `c573a34` is the last commit before 05-01's first execution commit (`82044eb`). Both diffs are genuinely empty against that range, so the criterion is met in substance rather than by construction.
- **Files modified:** none
- **Verification:** `git diff --stat c573a34..HEAD -- crates/talaria-shell/src/gui.rs` empty; same for the five named suites; `git diff --stat c573a34..HEAD -- crates/talaria-shell/src/` shows the ten files that *did* change, which is the control proving the range is not vacuous.
- **Committed in:** n/a — a verification-method correction, recorded here

---

**Total deviations:** 3 (1 auto-fixed content change, 2 verification-method corrections recorded rather than actioned)
**Impact on plan:** None on scope. All three are the plan's own assumptions meeting the tree as it actually is. No source file was modified by this plan.

## Issues Encountered

**The e2e suite outran a foreground tool timeout twice.** `run_all.py` takes roughly 14 minutes end to end on this machine (`oauth_flow_test` 104s, `remote_view_test` 112s, `panel_click_test` 86s, `remote_latency_test` 76s, `revocation_test` 70s are the long ones). Resolved by running it detached with `python3 -u` and polling for a completion marker — the `-u` matters, because a buffered `python3` whose parent is killed loses the entire log. Final run: **24/24, `failed: none`, exit 0**, on display `:94` per the phase's display-hygiene rule (`:98` is the Actions runner's).

## Verification

| Check | Result |
|---|---|
| `cargo build --release --locked` | exit 0 |
| `cargo clippy --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --locked` | **536 passed**, 0 failed, 9 ignored (90 + 2 + 35 + 409) |
| `python3 tests/e2e/run_all.py` | **24/24 PASS**, `failed: none`, exit 0 |
| `test -x scripts/two-machine-check.sh` / `bash -n` | both exit 0 |
| script against an offline peer | PATH: UNKNOWN, exit 1 — a run that could not have proved anything does not read as a pass |
| script against `minipc` | PATH: DIRECT, full steps and record-these-four printed |
| `tailscale serve status` | unchanged, four pre-existing mappings intact |
| primeorder pin | 1 |

Against the phase's entering baselines: **296 → 536 unit tests** and **22 → 24 e2e suites**.

## Next Phase Readiness

**Phase 5 is closed.** DIST-01 and DIST-02 are complete; DIST-03 and DIST-04 are Phase 5.1's, cross-referenced forward in the deferred register with the two design facts 5.1 will need (the frame sequence is per *attachment*, so a reconnect starts from no history and a keyframe is the only correct resync; and `link_shim.py`'s kill knob already exists for exactly this).

**Open, and stated rather than hidden:**

1. **Success Criterion 2 is designed for and not contradicted, rather than measured across two machines.** Everything automated ran both ends on one host under Xvfb. `scripts/two-machine-check.sh` is the step that closes it; until somebody runs it on a direct path and writes the millisecond figure down, the target should not be cited as confirmed. The tailnet carries real hosts for it — `courts-macbook-air` (observed direct when online) and `minipc` (direct now) — against `thinkpad` at `100.118.105.121`.
2. **First pairing still requires someone at the server machine**, and this is the item a developer will hit soonest. The phase shipped against accepting it; the client's first-run copy says so plainly. `05-CONTEXT.md` records it and the "a remote human at a login wall has no address bar" question as **one** question — *what can a remote human do that requires being physically at the server machine?* — and answering them together will be cheaper than apart. Either alternative (RFC 8628, or a pairing code) is a new plan, not an adjustment.
3. **A viewer sees every agent's tabs**, not only its own client's. A decision, not a default: the human is the trust root and a remote human is the trust root at a distance. Revisit only for a deployment where agents belong to parties that should not see each other's work — which is a different product.

## Self-Check: PASSED

- `scripts/two-machine-check.sh` — FOUND, executable
- `.planning/phases/05-distributed-mode/05-11-SUMMARY.md` — FOUND
- `87bacd4` — FOUND in `git log`
- `634e3e1` — FOUND in `git log`
- `939af4a` — FOUND in `git log`

---
*Phase: 05-distributed-mode*
*Completed: 2026-08-23*

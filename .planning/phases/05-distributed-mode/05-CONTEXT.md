---
phase: 05
slug: distributed-mode
source: orchestrator decision gate (2026-08-21)
---

# Phase 5 Context — locked decisions

`/gsd-discuss-phase` was not run. `05-RESEARCH.md` surfaced five decisions it judged product calls
rather than research findings. Four were put to the user and answered; the fifth is decided here on
the research's own measurements. They are locked.

## D-05-01 — A thin `talaria-client` binary; the server is unchanged

**Decision:** a new client binary renders frames and sends input. `talaria-shell` keeps its local
GUI and remains the product it is today. Two front ends, one engine.

**Why:** the alternative — making the shell headless so the GUI is always a client — is not a
refactor. Servo's offscreen buffer and egui's chrome share one `glow` context, so a headless shell
means rewriting the render path that Phase 1's measured latency depends on, to serve a v2 feature.
The core value is that a human can take over *instantly*; spending local-mode latency to buy remote
mode is the wrong trade.

**Consequence:** local mode must be provably unaffected. The planner should treat "the existing
local path is untouched" as a verifiable claim, not an assumption.

## D-05-02 — The remote client sees and drives **agent tabs only**

**Decision:** Me tabs stay local to the server machine. They are not viewable or drivable remotely.

**Why:** it keeps the remote input channel from ever reaching the human's own logged-in sessions.
The credential vault's entire benefit is that an agent drives a real, already-logged-in session
under supervision; putting the human's own tabs behind the same network channel would hand a single
compromised channel everything the vault protects.

**Consequence:** Success Criterion 1's "browse and drive agent tabs" is read literally. The client's
tab list is filtered server-side, not client-side — a client that asks for a Me tab is refused, not
merely un-shown. Make that a verifiable assertion.

## D-05-03 — Tailscale Serve terminates TLS; `BIND_HOST` stays a constant

**Decision:** Talaria keeps listening on loopback. Tailscale Serve fronts it with a real
certificate. Talaria does not bind a tailnet address, does not handle certificates, and does not
implement renewal.

**Why:** Phase 4 made the bind host a constant so a wider bind *cannot be expressed*, and that
guarantee is worth more than the convenience of self-binding. Serve preserves it exactly while
still satisfying the obligation `04-.../deferred-items.md` recorded, and removes cert rotation from
a long-running process entirely.

**Consequences the planner must carry:**
- **The advertised base URL becomes distinct from the bound address.** `DnsRebindProtector` is
  seeded with `127.0.0.1:8779` and the OAuth `issuer` is `format!("http://{bound}")`. Both derive
  from the bind today and both must learn the advertised URL. That is the real work of this
  decision, and it touches Phase 4's security-critical code.
- Both Tailscale prerequisites are **confirmed on this tailnet** (checked 2026-08-21, not assumed):
  MagicDNS is on with suffix `tailcd3cc6.ts.net`, and HTTPS Certificates are enabled —
  `tailscale status --json` reports `CertDomains: ["thinkpad.tailcd3cc6.ts.net"]`. So
  `tailscale cert` *could* issue, and Serve can front a real certificate. Neither blocks planning.
- `04-.../deferred-items.md` names `AxumServerOptions`' `enable_ssl`/`ssl_cert_path` as the TLS
  route. That is **wrong** — `http.rs` takes the BYO-server path with its own `TcpListener`, so
  those fields are not on Talaria's mount path at all. Correct that note.

## D-05-04 — Split into Phase 5 and Phase 5.1

**Decision:** Phase 5 delivers **DIST-01 and DIST-02** — the client/server split, the wire, the
frame pipeline, and the latency target. **Phase 5.1** delivers **DIST-03 and DIST-04** — reconnect
and resync, and the session manifest with restore-on-restart. Update `ROADMAP.md` accordingly.

**Why:** four plans was too few for an architectural split plus a frame pipeline plus reconnect plus
persistence; Phase 4 needed eight for less. Splitting ships something verifiable sooner and keeps
each phase reviewable.

## D-05-05 — Frame encoding reuses `png` at `Compression::Fast` on tile diffs; no new codec

**Decided here, on `05-RESEARCH.md`'s measurements** rather than referred, because the numbers
settle it.

The current `encode_screenshot` path cannot be the frame path: `Compression::Default` measured
**21 ms** on a page-like 1280×800 frame and **175 ms** on a photo-like one, against a 30 ms budget.
The same crate at `Compression::Fast` + `FilterType::Sub` over 64×64 tile diffs measured **0.007 ms
/ 3.9 KB** for a caret, **0.034 ms / 15 KB** for typing, and **2.16 ms / 522 KB** for a keyframe,
with a **0.14 ms** whole-frame dirty-tile scan.

`jpeg-encoder` was cleared on provenance but is **not** adopted: it is lossy on text and only helps
the scroll case.

**Dependency cost, verified by running Phase 4's procedure** (edit manifests, `cargo metadata`,
restore; `md5sum` identical before and after): `axum`'s `ws` feature adds exactly
**`tokio-tungstenite 0.29.0`** — `tungstenite 0.29.0` is already in the lock via `servo-net` — and
`primeorder` stayed at `0.14.0-rc.14`.

## D-05-06 — On a relayed link, degrade and report; never refuse takeover

**Decided here.** SC 2's ~30–60 ms cannot be met over a DERP-relayed Tailscale path and no codec
choice changes that: a 522 KB keyframe is ~321 ms on the 13 Mbit/s a relayed link has actually
measured on this user's hardware. A direct WireGuard path on the same LAN measured 342–447 Mbit/s.

The product must therefore **degrade the frame rate and tell the user**, rather than silently
missing the target or refusing to hand over control. A success criterion that quietly fails on a
relayed link is worse than one that reports. The planner should make "the client can tell the user
the link cannot meet the takeover target" a testable behaviour, and should word SC 2 so it is
conditioned on a direct path.

## Measured facts about the actual deployment (2026-08-21)

Checked on the machine this will run on, rather than assumed. Three of these change planning.

- **Tailscale Serve is already in use here.** `tailscale serve status` reports
  `https://thinkpad.tailcd3cc6.ts.net:8443` proxying to `http://127.0.0.1:5678` for an unrelated
  service. This is good news twice over: it proves Serve works on this tailnet, and it means
  **Phase 5 cannot assume port 8443 is free**. The plan must pick a distinct Serve port and must not
  disturb the existing mapping — clobbering a running service to stand up a browser feature would be
  a bad trade the user did not ask for.
- **The likely client machine has a direct path, not a relayed one.** `courts-macbook-air` shows
  `active; direct 192.168.1.176:41641`. So SC 2's ~30–60 ms target is achievable on the setup this
  will actually be used on. The DERP degradation path in D-05-06 is still required — the same Mac
  has previously fallen back to DERP at ~13 Mbit/s when tethered through a phone's symmetric NAT —
  but it is the exception case, not the expected one.
- The server is `thinkpad` at `100.118.105.121`; the tailnet also carries `minipc` and an offline
  `iphone`. A two-machine e2e has a real second host available if one is ever wanted, though the
  harness question in `05-RESEARCH.md` (loopback stand-in vs network namespace vs a second Xvfb)
  stands on its own merits.

## Required spikes — before the plans that depend on them are written

`05-RESEARCH.md` names two, and both follow Phase 4's precedent where `04-02`'s spike ran in wave 1
and its answer unblocked the authorization-server plans.

1. **GL readback cost.** `read_to_image` at a 30 ms cadence is **unmeasured** and is the phase's
   largest unknown; under Xvfb's llvmpipe it may be far worse than on real hardware. The frame-pump
   plan must not be written before this returns a number. Note the benchmark above used *synthetic*
   frames, not a Servo readback — real pages should bracket between its two cases, but re-measure.
2. **WebSocket through Tailscale Serve.** Whether Serve proxies a WebSocket upgrade cleanly, and
   what it does to the advertised-URL problem in D-05-03.

## A structural property worth writing down

`build_router` applies `refuse_page_originated` as the **outermost** layer over every route.
WebSockets have no browser-side origin enforcement, so cross-site WebSocket hijacking is closed by
construction here — and because a browser always sends `Origin`, a browser-based viewer is
structurally impossible. `05-RESEARCH.md` recommends recording that in `SECURITY.md` before someone
relaxes the layer without realising what it was load-bearing for. Do that.

## Still open — for the planner or the UI phase

- **Does first pairing require someone physically at the server machine?** OAuth consent is a
  server-chrome panel today, so as things stand, yes. Whether to add a pairing-code flow is a
  product call the planner should surface rather than silently answer.
- Whether the human client authenticates through Phase 4's OAuth or is a distinct trust class.
  `05-RESEARCH.md` leans toward reuse; confirm during planning.
- `PROJECT.md` calls `talaria-protocol` "the intended distributed-mode wire". Research judged it the
  **vocabulary, not the wire** — `TabInfo`/`Command`/`Outcome`/`Event` should go on the wire
  unchanged, but four of its ~11 public items are a Unix socket path and a `getuid` shim, `Hello`'s
  client string is self-asserted and safe only because `SO_PEERCRED` ran first, and `Screenshot`
  is pull/text/whole-frame/+33%. Update PROJECT.md's claim as part of this phase.

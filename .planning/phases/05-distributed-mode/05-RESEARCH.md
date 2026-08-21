# Phase 5: Distributed Mode *(v2)* — Research

**Researched:** 2026-08-21
**Domain:** Remote framebuffer + remote input over a private overlay network, held to a ~30–60 ms interaction budget, on top of a single-process Servo/winit/egui application
**Confidence:** HIGH on the codec/transport budget (measured this session), HIGH on the lockfile cost (resolved this session), HIGH on the code facts (read this session), MEDIUM on Tailscale operational behaviour (docs, not exercised on this tailnet), LOW on the GL readback cost at 30 ms cadence (not measured — named as a required spike)

---

## Summary

Talaria is one process. `talaria-shell` owns winit, egui, Servo, the tab table and every `RefCell` on `Shared`, all on one main thread. "Client and server on separate machines" is therefore not a feature to add to that process — it is a decision about where to cut it. This research recommends **not cutting it at all**: keep `talaria-shell` exactly as it is, including its local GUI and its measured local-mode latency, and add a **second, thin client binary** that renders frames and sends input. Two front ends, one engine. The alternative — making the shell headless and running the GUI always as a client — would restructure the shipped v1, put a serialize/encode/decode/present pipeline in the middle of the local path that Phase 1 measured at 4–36 ms, and require untangling a compositing arrangement in which Servo's offscreen buffers and egui's chrome share *one GL context* (`gui.rs:2513`, `render_to_parent_callback` into `LayerId::background()`). That is the single hardest refactor available in this codebase and it buys uniformity, not capability.

The latency target is the constraint that decides the codec, and it was measured rather than reasoned about. **The existing screenshot path cannot be the frame path.** `encode_screenshot` (`app.rs:2494`) uses `png::Compression::Default`, which this session measured at **21 ms for a page-like 1280×800 frame and 175 ms for a photo-like one** — the second is 3–6× the entire takeover budget before a byte leaves the machine. The same `png` crate already in `Cargo.toml`, switched to `Compression::Fast` + `FilterType::Sub` and applied to **64×64 tile diffs**, measured **0.007 ms / 3.9 KB for a caret-sized change, 0.034 ms / 15 KB for typing, 2.16 ms / 522 KB for a full keyframe**, with the whole-frame dirty-tile scan costing **0.14 ms**. That fits the budget with an order of magnitude to spare on interactive deltas, is lossless (which matters for text), and **needs no new codec dependency at all**.

Two things about the network are worth stating flatly. Tailscale is not one network: a direct WireGuard path on this user's own LAN measured 342–447 Mbit/s, and the same Mac tethered to a phone fell to a DERP-relayed path at ~13 Mbit/s. A full-frame scroll at 30 ms is 139 Mbit/s of PNG — fine direct, ten times over budget relayed. **Success Criterion 2 cannot be met unconditionally on a relayed link and the product must say so rather than degrade silently.** And a tailnet address is not loopback: Phase 4 shipped without TLS *because* it stayed on loopback, which is OAuth 2.1 §1.5's stated exception, and `04-.../deferred-items.md` hands this phase the obligation in as many words. The cheapest conformant answer found is **Tailscale Serve**, which terminates TLS with a Tailscale-managed certificate and proxies to `http://127.0.0.1:PORT` — the one proxy target it supports, and exactly what Talaria already binds. That preserves `BIND_HOST` as a constant and every test in `settings.rs` that refuses a `bind` key. It is not free: it forces the browser to advertise an identity (`issuer`, RFC 8707 `resource`, `DnsRebindProtector` allowlist) that is no longer the address it bound, and that identity must come from configuration rather than from a request's `Host` header.

**Primary recommendation:** a new thin `talaria-client` binary talking one multiplexed **binary WebSocket** to a route on the **existing Phase 4 axum router** (measured lockfile cost: one package, `tokio-tungstenite 0.29.0`), carrying tile-diffed `Compression::Fast` PNG frames and a hand-mapped input envelope, scoped to **agent-owned tabs only**, exposed over the tailnet via **Tailscale Serve** rather than a widened bind — and split across **more than four plans**, because this phase contains a transport, a codec, a new front end, an input channel, a resilience story and a persistence store, and Phase 4 needed eight plans for one of those.

---

## Architectural Responsibility Map

| Capability | Primary Tier | Secondary Tier | Rationale |
|------------|-------------|----------------|-----------|
| Frame capture (`paint` + `read_to_image`) | Server main thread (winit loop) | — | GL context and `WebView` are main-thread only (`app.rs:399 capture_now`); no other tier can touch them |
| Frame encode (tile diff + PNG) | Server worker thread | Server main thread (fallback) | `servo::RgbaImage` is `image::RgbaImage` = `ImageBuffer<_, Vec<u8>>`, which is `Send`; moving the encode off-thread takes 2.16 ms off a 30 ms main-thread budget |
| Frame transport (WebSocket) | Server HTTP listener thread (`http.rs`) | — | Already a long-lived off-thread actor with an `EventLoopProxy`; a fourth actor would duplicate its Origin/Host/token/TLS story |
| Input decode + validation | Server HTTP listener thread | — | Untrusted wire input must be range-checked before it becomes an `AppEvent` |
| Input delivery to the engine | Server main thread | — | `WebView::notify_input_event` (`servo-0.4.0/webview.rs:619`) hit-tests through `servo.paint()`; main-thread only |
| Frame decode + present | Client process | — | The client is the only tier with the remote human's display |
| Tab list / control / events | Server main thread → WebSocket | — | Reuses the `talaria_protocol` vocabulary that already exists |
| Session manifest persistence | Server main thread | — | Same convention as `Agents::save` and the four Phase 3 stores: whole-document `.tmp` + `rename`, `0600` |
| TLS termination | `tailscaled` (Tailscale Serve) | Server listener (rustls, fallback) | Serve is the only option that leaves `BIND_HOST` a constant |
| Authorization of a remote human | Server `oauth.rs` + `agents.rs` | — | A second credential system is a second thing to get wrong |

---

## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| DIST-01 | Client and server on separate machines connected via Tailscale — *Must Have* | § Architectural Split (Option A), § Exposure and the Certificate Story, § Package Legitimacy Audit |
| DIST-02 | Human watches and takes over an agent's tab with the server remote, adaptive polling ~200–500 ms passive / ~30–60 ms takeover — *Must Have* | § The Latency Budget, Measured, § Frame Transport and Codec, § Input |
| DIST-03 | A dropped connection does not kill in-progress agent work; the client resyncs on reconnect — *Must Have* | § Reconnect and Resync — what is actually at risk |
| DIST-04 | Server persists a session manifest and offers to restore tabs after a crash/restart — *Should Have* | § Session Manifest |

---

## Project Constraints (from CLAUDE.md)

Extracted from `./.claude/CLAUDE.md` and `~/.claude/CLAUDE.md`. The planner must verify each plan against these; several are directly at odds with the obvious way to build this phase.

**Dependency and build**
- `Cargo.lock` is load-bearing. `primeorder` is pinned at `0.14.0-rc.14`; a fresh resolution takes `0.14.0` final and breaks `p256/p384/p521 0.14.0-rc.14`. Never `cargo update`, never `cargo add`. Phase 4's procedure — edit the manifest, run `cargo metadata`, inspect the lock diff, restore on rejection — is inherited. *(This research used exactly that procedure; see § Package Legitimacy Audit.)*
- All dependency versions are declared once in `[workspace.dependencies]`; member crates use `{ workspace = true }`.
- `egui`/`egui-winit`/`egui_glow` are pinned at `0.34.3` to match Servo's own workspace, because the `glow` context type is shared. `glow` 0.17.
- CI builds `--locked`.

**Rust style**
- **`talaria-shell` has no `serde` derive.** `serde` is in `[workspace.dependencies]` with `features = ["derive"]` but is *not* a dependency of `talaria-shell`; the shell hand-maps everything through `serde_json::Value` (`settings.rs` `to_json`/`from_json`, `agents.rs`). Any new wire type used by the shell must follow that. `talaria-protocol` *does* derive.
- No `anyhow`, no `thiserror`. `main` returns `Result<(), Box<dyn Error>>`; everything else uses `std::io::Result`, `Option`, or `Outcome::Error { message }`.
- **No `unwrap()` in shell code paths.** `expect()` only for genuine startup invariants.
- Degrade, never abort, on user-data problems (`vault.rs` is the model).
- Match on `io::ErrorKind` to separate expected absence from real failure.
- Trailing comma after a block match arm (Servo convention). ~100-column wrap. Inline format captures (`{error}`), never positional.
- Spell words out: `error` not `e`, `message` not `msg`. Exception: the Python e2e suites are deliberately terse.
- `//!` module header on every Rust file; `///` on public items; design decisions cite `SPEC.md` by name inline.
- Lookups return `Option<&T>`; no-op-capable mutations return `bool`; iteration returns `impl Iterator`, not an allocated `Vec`.
- The GUI never mutates shared state directly — `Gui::update` returns `Vec<UiAction>` applied by `apply_ui_actions` after egui's borrows drop.

**Threading and re-entrancy**
- Servo, winit, egui and every `WebView` call are **main-thread only**.
- Servo invokes delegate callbacks while holding internal borrows. Callbacks may only set flags and `request_redraw()`; painting, visibility changes and follow-up `evaluate_javascript` calls must be deferred to a pending queue.
- `Shared`'s `RefCell`s are borrowed from both callbacks and the loop, so callbacks use `try_borrow`/`try_borrow_mut` and skip rather than panic.
- Cross-thread transfer is `EventLoopProxy::send_event(AppEvent)` in, `oneshot`/`mpsc` out. There are three off-thread actors today (control thread, per-download thread, HTTP listener). A frame pump would be a fourth and the only latency-critical one.
- Rendering invariant: every tab owns an `OffscreenRenderingContext`, so capturing a background tab cannot disturb the displayed one; `paint()` without a fresh frame is a no-op, which is why `sync_visibility()` must run on every tab-set change.
- Timeout ordering: socket command timeout (30 s default) > `promise_wait()` (timeout − 2 s) > `LOAD_WAIT` (20 s) > background-capture deadline (1.5 s). Keep this ordering when adding waits.

**Testing and process**
- e2e is Python 3 **stdlib only** — no pytest, no third-party deps. Suites are `<feature>_test.py`, runners `run_*.py`, shared code `harness.py`. Module constants SCREAMING_CASE. Private helpers `_leading_underscore`.
- Verification: `cargo build --release --locked`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (296 passing), `python3 tests/e2e/run_all.py` (22/22).
- **Log all changes in `CHANGELOG.md`** (global rule; the file exists).
- **Do not write code inline into documentation files** — create referenced executable scripts instead (global rule; bites the two-machine test procedure, which must be a script under `tests/e2e/`, not a README recipe).
- A PLAN.md up front is **required** for work touching the security or protocol surface — which is nearly all of this phase (`crates/talaria-protocol/`, `control.rs`, `vault.rs`, the MCP tool surface, `parse_agent_url`, anything carrying a threat register).
- Local dev servers bind ports **40000–40999** and are reached over the tailnet at `http://100.118.105.121:PORT`, never `localhost` (global rule; relevant to any manual two-machine verification from the MacBook Air).

---

## Is `talaria-protocol` the distributed-mode wire PROJECT.md claims it is?

**Assessed directly. The claim is half true, and the half that is false is the half this phase needs.** [VERIFIED: `crates/talaria-protocol/src/lib.rs`, read 2026-08-21]

The claim appears in three places and in the module's own doc comment: *"Clients (the `talaria-mcp` stdio proxy today, the distributed-mode client later) speak newline-delimited JSON"* (`lib.rs:4-5`), and `control.rs:1` calls itself *"the in-process precursor to the distributed protocol"*.

**What has held.** The vocabulary is genuinely the right one and has not drifted. `TabInfo` carries exactly what a remote tab list needs (`tab_id`, `url`, `title`, `owner`, `focused`, `crashed`, `loading`). `Command` covers open/close/focus/navigate/evaluate/screenshot. `Event` carries `TabCrashed`, `TabClosed`, `TabOpened`. The request/reply correlation by `id`, out-of-order replies, and unsolicited events on the same stream are all already designed for and already exercised. Phases 2–4 grew this file rather than working around it. **These types should be the control/tabs/event channels of the distributed wire, unchanged.**

**What has not held, and it is structural, not cosmetic:**

1. **The transport is soldered to a Unix domain socket.** `socket_dir`, `socket_path`, `ensure_socket_dir`, `current_uid` and a raw `extern "C" getuid` shim all live in this crate. Four of its ~11 public items are about a local filesystem path and a local UID — none of which mean anything to a peer on another machine. The crate is not transport-agnostic; it is a Unix-socket protocol that happens to serialize cleanly.

2. **Identity is a self-asserted string.** `ClientMessage::Hello { client: String }` is the entire identity on this wire, and it is safe today *only* because `peer_uid_ok` (`control.rs:161`) has already established that the peer runs as you. `SO_PEERCRED` has no TCP equivalent — `http.rs`'s module header says so at length. **A remote peer cannot be admitted by this handshake.** Phase 4 solved this on the HTTP side and did not touch this crate: `AgentRequest` gained nothing, `next_session_id()` was *exported from `control.rs`* so `http.rs` could share the counter, and verified identity lives in `AuthInfo` in the SDK's request extensions, not in `ClientMessage`.

3. **There is no frame channel and no input channel.** `Command::Screenshot` returns `ResultPayload::Screenshot { png_base64, width, height }` — a **base64 string** in a **newline-delimited JSON** envelope, request/reply, one frame per round trip. At 33 frames/second that is a 522 KB PNG inflated to ~696 KB of base64 inside a JSON string, 33 times a second, on a line-oriented reader. It is a screenshot *tool*, correctly shaped for what it is, and it is the wrong shape for a frame stream in every dimension: pull instead of push, text instead of binary, whole-frame instead of delta, and +33% for nothing. There is no `Command` or `Event` variant for input at all.

4. **The one variant that already knows this is `ChromeRects`, and it knows it in the opposite direction.** Its doc comment refuses to expose chrome geometry to agents *because* it would let something "aim synthetic input at the credentials button". The crate has already reasoned once about synthetic input reaching the human's own surface, and concluded it must not. That reasoning is the direct ancestor of this phase's largest new threat.

**Recommendation.** Keep `talaria-protocol` as the shared vocabulary — `TabInfo`, `Command`, `Outcome`, `Event` go on the wire unchanged. Add a **new module** in the same crate (`talaria_protocol::wire` or similar) for the multiplexed envelope, the frame message and the input message, and move `socket_dir`/`socket_path`/`ensure_socket_dir`/`current_uid` **behind `#[cfg(unix)]`** or into a `local` submodule so that the crate stops claiming a Unix socket is inherent to it. Do **not** extend `Command` with input variants: `Command` is the *agent* tool vocabulary, dispatched by `talaria_mcp::dispatch`, and putting human input into it would make remote clicks reachable from the MCP tool surface. That is the same mistake `ChromeRects` was written to avoid. **Update PROJECT.md's claim** — it should say the crate is the shared vocabulary, not the wire.

---

## Runtime State Inventory

Not a rename/refactor phase — greenfield transport work on an existing process. Recorded anyway because Phase 5 *creates* runtime state that a later phase will have to inventory.

| Category | Items Found | Action Required |
|----------|-------------|------------------|
| Stored data | None consumed. **Created:** the DIST-04 session manifest (new file in the `dirs` config dir alongside `config.json`, `agents.json`, history/bookmarks/downloads) | New store, `0600`, `.tmp` + `rename` |
| Live service config | **`tailscale serve` configuration is per-node state held by `tailscaled`, not in this repo.** If Serve is the exposure mechanism, `tailscale serve --bg 8779` (or equivalent) is a machine-level side effect nothing in git records | Document the exact command as an executable script under `tests/e2e/` or `scripts/`, per the no-inline-code rule; add a teardown |
| OS-registered state | None — Talaria is launched by hand; no systemd unit, no launchd plist, no Task Scheduler entry exists | None — verified by absence of any unit/plist in the tree |
| Secrets/env vars | Consumed: `TALARIA_COMMAND_TIMEOUT_SECS`, `TALARIA_TEST_HOOKS`, `TALARIA_E2E_DISPLAY`, `XDG_RUNTIME_DIR`, `RUST_LOG`, `TALARIA_HISTORY_MAX_ENTRIES`, `TALARIA_MAX_DOWNLOAD_BYTES`. **Created:** any cert/key path if the rustls fallback is taken. No `.env` files exist and none should be added | If a cert path becomes configurable, it is a filesystem path from `config.json` — treat it as hostile input the way `settings.rs` treats every other key |
| Build artifacts | A second binary target (`talaria-client`) changes `cargo build --release` output and the e2e harness's `BINARY` constant assumption | `harness.py` gains a second binary path constant |

---

## Standard Stack

### Core — everything below is already resolved in `Cargo.lock`

| Library | Version in lock | Purpose | Why standard |
|---------|-----------------|---------|--------------|
| `axum` | 0.8.9 | The WebSocket route (`axum::extract::ws`) on the router `http.rs` already builds | [VERIFIED: `Cargo.lock`] Already present via `rust-mcp-axum`; `http.rs` already uses `rust_mcp_axum::axum::*`. Adding it as a direct dep unifies to the **same** crate node — confirmed by the resolution run below, which produced no second `axum` |
| `tokio-tungstenite` | **0.29.0 (new)** | What `axum/ws` pulls in | [VERIFIED: `axum-0.8.9/Cargo.toml:264` requires `tokio-tungstenite 0.29.0`; `tungstenite 0.29.0` is *already* in the lock via `servo-net` → `async-tungstenite`]. This is the **only** package the entire WebSocket + codec change adds |
| `png` | 0.17 | Frame codec — `Compression::Fast` + `FilterType::Sub` on tile diffs | [VERIFIED: already a direct dependency of `talaria-shell`]. Measured this session; see § The Latency Budget |
| `tokio` | 1.53.1 | Already a shell dependency with `rt-multi-thread`, `net`, `io-util`, `sync`, `macros`, `time` | Unchanged |
| `serde_json` | 1 | Hand-mapped control/tabs/event payloads inside the binary envelope | The shell's only serialization tool (no derive) |
| `talaria-protocol` | path | `TabInfo`/`Command`/`Outcome`/`Event` on the wire; new `wire` module for the envelope | § above |

### Supporting — resolved, needed only if the named branch is taken

| Library | Version in lock | Purpose | When to use |
|---------|-----------------|---------|-------------|
| `axum-server` | 0.8.0 | TLS acceptor, `tls-rustls` feature | **Only** if the rustls-in-process branch is taken instead of Tailscale Serve |
| `rustls` | 0.23.43 | TLS. The shell already installs `aws_lc_rs::default_provider()` first thing in `main.rs` | Same branch |
| `tokio-rustls` | 0.26.4 | What `axum-server/tls-rustls` uses | Same branch |
| `rustls-pki-types` | 1.15.1 | **Has a `pem` module** (`src/pem.rs`, `PemObject` trait) — parses the PEM `tailscale cert` writes, so `rustls-pemfile` is **not** needed | Same branch. Verify the `std` feature is unified on before relying on it |
| `image` | 0.25.10 | `servo::RgbaImage` *is* `image::RgbaImage`; a direct dep would give `image::codecs::jpeg::JpegEncoder` | Only if a lossy codec is later negotiated. Adding it resolved to the **same** node — no lockfile churn |

### Alternatives considered

| Instead of | Could use | Tradeoff |
|------------|-----------|----------|
| `png` Fast + tile diff | `jpeg-encoder` 0.6/0.7 with `simd` | **Measured: 6.2 ms / 200 KB full frame vs PNG-Fast 2.16 ms / 522 KB.** Better on the wire (54 vs 139 Mbit/s at 30 ms), worse on CPU, and **lossy on text** — chroma subsampling on antialiased body text is the one artefact a browser viewer cannot afford. A new dependency (clean: 7.28 M downloads, 2021, `github.com/vstroebel/jpeg-encoder`). Recommend as a **negotiated second codec in a later plan**, only if measurement shows scroll-heavy content failing on a real link — not up front |
| `png` Fast + tile diff | `image`'s own `JpegEncoder` (already resolved) | **Measured: 19–20 ms** for the same frame — three times slower than `jpeg-encoder`, two thirds of the budget. Zero new dependencies, but it is not fast enough to be interesting |
| Any still-image codec | A video codec (H.264/VP8/AV1) | `rav1e` is in the lock but only as `ravif`'s still-image AVIF encoder — orders of magnitude too slow for realtime. There is no H.264/VP8 encoder anywhere in this tree, and adding one means a C dependency and a patent conversation. **Reject.** The measured PNG delta numbers make it unnecessary |
| WebSocket on the Phase 4 router | A second, dedicated listener | Would duplicate `refuse_page_originated`, `DnsRebindProtector`, `AuthMiddleware`, the `Connections` fd-tracking and the whole TLS story. **Reject** — see § Is a multiplexed WebSocket right? |
| WebSocket | Raw TCP with a length-prefixed framing | Cheaper on the wire, but loses the Origin/Host middleware that `http.rs` applies as the **outermost** axum layer, loses TLS via Serve, loses browser-side compatibility forever, and gains nothing measurable at these payload sizes. **Reject** |
| WebSocket | QUIC/WebTransport | No QUIC stack in the lock; adding one is a large dependency for a link that is *already* a UDP tunnel with its own congestion control. **Reject** |
| Tailscale Serve | Widen `BIND_HOST` + `tailscale cert` + rustls | Reopens the structural invariant Phase 4 spent a decision on. See § Exposure. Keep as the documented fallback |
| Tailscale Serve | Tailscale **Funnel** | Funnel exposes the service **to the public internet**. This process holds a credential vault and a logged-in browsing session. **Reject outright, and say so in `SECURITY.md`** so nobody reaches for it as "the same thing but easier" |

**Installation:**

```bash
# Workspace Cargo.toml — [workspace.dependencies]
# axum = { version = "0.8.9", features = ["ws"] }
#
# crates/talaria-shell/Cargo.toml — [dependencies]
# axum = { workspace = true }
#
# Then, per the inherited Phase 4 procedure — NOT cargo add, NOT cargo update:
cargo metadata --format-version 1 > /dev/null
git diff --stat Cargo.lock          # must show exactly one added package
cargo build --release --locked
```

**Version verification** [VERIFIED: `cargo metadata` run against edited manifests in this working tree on 2026-08-21, lockfile restored byte-identical afterwards — `md5sum` before and after both `354ddad3f4e45c6aef8a58d39af1b30d`]:

```
Locking 1 package to latest compatible version
  Adding tokio-tungstenite v0.29.0
```

Lockfile diff: one new `[[package]]` block, three feature-edge additions (`base64`, `sha1 0.10.7`, `tokio-tungstenite` under `axum`). **`primeorder` stayed at `0.14.0-rc.14`.** No version of any existing package changed. Adding `image = { version = "0.25", features = ["jpeg"] }` in the same run added **zero** packages — it unified onto Servo's existing `image 0.25.10` node.

---

## Package Legitimacy Audit

| Package | Registry | Age | Downloads | Source repo | Verdict | Disposition |
|---------|----------|-----|-----------|-------------|---------|-------------|
| `axum` 0.8.9 | crates.io | 2021– | ecosystem-standard | github.com/tokio-rs/axum | OK | Approved — already in the lock via `rust-mcp-axum`, already used by name in `http.rs` |
| `tokio-tungstenite` 0.29.0 | crates.io | 2018– | ecosystem-standard | github.com/snapview/tokio-tungstenite | OK | Approved — the only new package. Its sibling `tungstenite 0.29.0` is *already* resolved in this tree through `servo-net`, so Servo itself already trusts this codebase |
| `png` 0.17 | crates.io | — | — | github.com/image-rs/image-png | OK | Already a direct dependency; no change |
| `image` 0.25.10 | crates.io | — | — | github.com/image-rs/image | OK | Already in the lock via Servo. Only needed if a lossy codec is later added |
| `jpeg-encoder` 0.7.1 | crates.io | created 2021-04-29, latest 2026-07-27 | 7,282,570 total / 1,158,637 recent [VERIFIED: crates.io API, 2026-08-21] | github.com/vstroebel/jpeg-encoder | OK | **Not adopted in this phase.** Recorded as a cleared option for a later negotiated-codec plan |
| `axum-server` 0.8.0, `tokio-rustls` 0.26.4, `rustls` 0.23.43, `rustls-pki-types` 1.15.1 | crates.io | — | — | rustls project / programatik | OK | Already in the lock. Needed only on the rustls-in-process TLS branch |

**Packages removed due to [SLOP] verdict:** none.
**Packages flagged as suspicious [SUS]:** none.

Every package this phase needs is already resolved in a lockfile this project has built and shipped against, except `tokio-tungstenite`, whose sibling crate the same lockfile already carries. No package in this phase was discovered by WebSearch or from training data; all were found by reading `Cargo.lock` and the vendored manifests in `~/.cargo/registry/src/`. Postinstall scripts are not a concept in Cargo; the equivalent risk (`build.rs`) is unchanged for every package here.

---

## The Architectural Split — the decision the phase hangs on

### What the existing `Gui`/`Shared` split actually permits

Read directly, not inferred. [VERIFIED: `app.rs`, `gui.rs`, `tabs.rs`, `control.rs`, `http.rs`, 2026-08-21]

**What helps.** `Gui::update` returns `Vec<UiAction>` and mutates nothing, applied afterwards by `apply_ui_actions` (`app.rs:1455`). Every tab owns its own `OffscreenRenderingContext` (`tabs.rs:47`), so painting or capturing one tab never disturbs another's pixels — the property that makes a per-tab frame pump possible at all. `AppEvent` (`app.rs:49`) is already the single inbound channel from off-thread work, and `http.rs` is already a long-lived off-thread actor using it. `AgentRequest`/`Outcome` already round-trip over two different transports through `talaria_mcp::CommandSink`. `TabManager` already separates ownership (`TabOwner`) from display (`displayed()`).

**What does not.** `Shared` is `Rc`-based with `Cell`/`RefCell` fields and is explicitly not `Send` — `app.rs:128`'s doc comment says a spawned thread "cannot touch any of it". `Gui` lives in a `thread_local!` `RefCell<Option<Gui>>` (`app.rs:423`). And the compositing is not separable at all: `gui.rs:2513` takes the displayed tab's `render_to_parent_callback()` and adds it as an egui `PaintCallback` into `LayerId::background()`, so **Servo's offscreen buffer and egui's chrome are drawn through the same `glow` context**, which is why the egui version is pinned to Servo's workspace. There is no seam there to cut.

**The three functions that route input all target the displayed tab, not a tab id.** `forward_mouse_move`, `forward_mouse_button` and `forward_wheel` (`app.rs:1316`, `:1342`, `:1379`) each begin `state.tabs.borrow().displayed()`. Remote takeover of a tab that is not the server's displayed tab is not expressible today. That is a small generalization, not a redesign — `notify_input_event` is a method on `WebView` — but it is a real, named change and it comes with a subtlety: each of those functions subtracts `state.toolbar_height_device()` from the y coordinate. **A remote client renders no server toolbar, so that offset must not be applied to remote coordinates.** Applying it is a silent, plausible-looking bug that would make every remote click land ~40 px high.

### Option A — a second, thin client binary (**RECOMMENDED**)

A new binary target (`talaria-client`, or `talaria --connect <url>`) that opens one WebSocket, decodes frames into a window, and sends input. `talaria-shell` is unchanged in every existing path: it keeps its window, its chrome, its local input handling, its Phase 1 latency, and gains one route on the router it already serves.

- **Local mode is provably unaffected**, because no local code path changes. Success Criterion 2's local baseline and the core value proposition are not put at risk by remote work.
- The client can be genuinely thin: winit + `egui`/`egui_glow` for a toolbar and a texture blit, or even just winit + `softbuffer`-style direct present. It does **not** link Servo, which means it builds in seconds and runs on a machine with none of Servo's build prerequisites (`cmake`, `clang`, fontconfig/freetype headers) — a real user-facing benefit on a laptop.
- Cost: two front ends to keep visually consistent. Mitigate by scoping the client's chrome to the minimum (tab list, Me/Agents indicator, connection state) and running `/gsd-ui-phase` for it.

### Option B — headless server, GUI always a client (**REJECT**)

- Puts serialize → encode → transmit → decode → present in the middle of the *local* path. Even over a Unix socket that is ≥ 2 ms of PNG plus two copies, on a budget Phase 1 measured at 4–36 ms for capture alone. The project's stated performance constraint is the core value.
- Requires separating egui from Servo's GL context — see above. The largest available refactor, for zero capability.
- Restructures the shipped product after v1 shipped. `PROJECT.md`'s Validated list would go stale in six places.
- Its only genuine advantage — one front end — is available at a fraction of the cost by sharing chrome *code* between the two binaries.

### Option C — narrow: agent tabs only (**RECOMMENDED, as the scope of Option A**)

Not an alternative to A; the right scope *inside* A. The frame/input service serves only tabs whose `TabOwner::is_agent()` is true.

- **DIST-02 only asks for this**: "watch and take over an *agent's* tab when the server is remote."
- The human's own Me tabs are where the history rows, the bookmarks and the autofilled credentials live. `SECURITY.md`'s trust model puts the human at the keyboard as the trust root and everything else below them; remoting the human's own tabs makes "the human at the keyboard" ambiguous, which is a change to the trust model rather than a feature.
- It gives the input path a **structural** filter rather than a checked one: the frame/input handler resolves a tab id through a lookup that returns `None` for a Me tab, so remoting a Me tab is unrepresentable rather than refused. That is the same shape as Phase 4's `BIND_HOST` constant, and it is what a threat-model reviewer will look for.

**Open decision for the user.** Success Criterion 1 says *"can browse **and** drive agent tabs"*. If "browse" means the remote human gets their own Me tabs materialised on the server, Option C is too narrow and the trust-model question above must be answered first. **Recommendation:** read "browse" as "the client can open and navigate tabs"; a tab the remote client opens becomes a Me tab on the server and is therefore *not* remotely viewable — which is visibly odd and is exactly why this needs to be locked before planning, not discovered in plan 3.

---

## The Latency Budget, Measured

SC 2 requires ~30–60 ms during takeover; DIST-02 names ~200–500 ms passive. Phase 1's 4–36 ms was **capture alone, on one machine**. The whole path is capture → encode → transmit → decode → present, plus input in the other direction.

### Encode, measured this session

Standalone Rust benchmark, `--release`, 1280×800 RGBA (the Xvfb screen size the harness uses), two synthetic inputs: a *page-like* frame (white background, header bar, sidebar, ~40 rows of text-shaped pixels, one gradient image block) and a *photo-like* frame (gradient plus noise — a deliberate worst case). Times are per-encode means over 20–300 iterations. [VERIFIED: run in `/tmp/frmbench`, 2026-08-21, against `png 0.17` / `image 0.25.10` / `jpeg-encoder 0.6`]

| Encoder | page: time | page: bytes | photo: time | photo: bytes |
|---------|-----------:|------------:|------------:|-------------:|
| **`png` `Compression::Default` (what `encode_screenshot` does today)** | **20.98 ms** | 67 KB | **175.40 ms** | 2.06 MB |
| `png` `Compression::Fast` + `FilterType::NoFilter` | 5.26 ms | 3.01 MB | 8.62 ms | 4.10 MB |
| **`png` `Compression::Fast` + `FilterType::Sub`** | **1.80–2.16 ms** | **522 KB** | 4.70 ms | 3.45 MB |
| `image` `JpegEncoder` q60 | 19.28 ms | 213 KB | 30.12 ms | 552 KB |
| `jpeg-encoder` 0.6 `simd` q60 | **6.23 ms** | **203 KB** | — | — |
| raw RGBA (no encode) | 0 ms | 4.10 MB | 0 ms | 4.10 MB |

**The single most important line is the first one.** The path this browser uses for `screenshot` today costs 21 ms on a page and 175 ms on a dense image, before transmission. It cannot be the frame path. Nothing about that is a criticism of `encode_screenshot` — a one-shot agent screenshot should be small and lossless, and `Compression::Default` is right for that.

### Tile diffing, measured this session

64×64 tiles, `memcmp` against the previous frame, then `png` `Compression::Fast`+`Sub` on only the changed region:

| Operation | Time | Bytes | Mbit/s at 30 ms cadence |
|-----------|-----:|------:|------------------------:|
| Dirty-tile scan, whole 1280×800 frame (260 tiles) | **0.14 ms** | — | — |
| One 64×64 tile (caret blink, hover) | **0.007 ms** | 3.9 KB | 1.0 |
| Four 64×64 tiles (typing into a field) | **0.034 ms** | 15 KB | 4.1 |
| 384×256 block (a menu or dropdown opening) | **0.258 ms** | 87 KB | 23.1 |
| Full keyframe, 1280×800 | **2.164 ms** | 522 KB | 139.3 |
| 640×400 half-res (passive) | 0.452 ms | 138 KB | 36.9 (at 30 ms) / **5.5 (at 200 ms)** |
| 2× nearest downscale cost | 0.260 ms | — | — |

**Conclusion: PNG with a tile diff is enough, and no new codec crate is needed.** The interactive cases — the ones the 30–60 ms budget exists for — cost microseconds and single-digit Mbit/s. The only expensive case is a **whole-frame change**, which in practice means scrolling, a page load, or a tab switch.

### Transmission, and why DERP is the real constraint

Tailscale is not a uniform network. When it establishes a direct connection it *is* WireGuard and performs like it; when hole-punching fails it relays through DERP. On this user's own setup [VERIFIED: `~/Desktop/claude-memory/env_mac_tailscale_relay.md`, `env_thinkpad_firewall_layers.md`], a Mac tethered to an iPhone hit symmetric NAT, fell back to DERP, and measured **~13 Mbit/s**; the same pair on the LAN with a direct path measured **342–447 Mbit/s**. Published figures agree on the shape: around 5% of connections stay relayed, with throughput dropping to roughly 35 Mbit/s and 20–50 ms of added latency versus a direct path [CITED: tailscale.com/docs/reference/connection-types; corroborating third-party benchmarks].

Against the measured payloads:

| Case | Payload | 30 ms cadence needs | Direct (≈350 Mbit/s) | DERP (13 Mbit/s, measured on this tailnet) |
|------|--------:|--------------------:|:---------------------|:-------------------------------------------|
| Caret / hover | 3.9 KB | 1.0 Mbit/s | ✅ | ✅ |
| Typing | 15 KB | 4.1 Mbit/s | ✅ | ✅ |
| Menu open | 87 KB | 23.1 Mbit/s | ✅ | ❌ (≈54 ms just to transmit) |
| **Scroll / keyframe** | 522 KB | 139.3 Mbit/s | ✅ | ❌ (**≈321 ms** to transmit one frame) |
| Passive half-res @200 ms | 138 KB | 5.5 Mbit/s | ✅ | ✅ |

Plus 20–50 ms of relayed one-way latency, which alone consumes the whole 30–60 ms round-trip budget before any bytes move.

**So: SC 2 is not achievable on a DERP-relayed link, and no codec choice available to this project changes that.** A criterion that silently fails on a relayed link is worse than one that reports.

### What the product should do when it cannot meet the target — recommendation

**Report, degrade, and never refuse.** Three parts, in this order:

1. **Measure, don't guess.** The client timestamps each input it sends and each frame it presents; the server echoes an input sequence number in the next frame envelope. That yields a real input-to-photon estimate without a clock sync. Keep an EWMA of it and of the observed goodput.
2. **Degrade along a fixed ladder, and name each rung in one place:** full-res 30 ms → full-res 60 ms → half-res 60 ms → half-res 120 ms → passive-only 500 ms. Move down on sustained budget overrun, up on sustained headroom, with hysteresis so it does not oscillate. Half-res costs 0.26 ms to produce and cuts the payload ~4×.
3. **Tell the human, in the client's chrome, in terms of the thing they care about.** Not "13 Mbit/s" — *"Relayed connection: takeover will feel slow (~180 ms)"*, with the current rung named. And surface Tailscale's own verdict: `tailscale status` reports `direct` vs `relay` per peer, and reading it is the difference between "the browser is slow" and "your network is relayed". This is the same diagnosis the user's own memory note records having to make by hand for an unrelated service.

**Do not refuse takeover.** A degraded takeover of a login wall still clears the login wall, which is the entire product. Refusing would fail the core value in exactly the situation it exists for.

### What is measurable in this repo's harness today, and what is not

**Measurable now:** encode time (a `cargo test`-level benchmark or a `TALARIA_TEST_HOOKS` timing hook), payload bytes, tile-diff hit rate, and end-to-end wall time from an input write to a frame read over a loopback WebSocket — all in Python stdlib.

**Needs new instrumentation:**
- **The GL readback cost (`OffscreenRenderingContext::read_to_image`) at 30 ms cadence is unmeasured and is the largest unknown in this phase.** It is a 4 MB `glReadPixels`, and the e2e harness runs under Xvfb on **llvmpipe software rendering**, where it will be far slower than on the user's real GPU. Phase 1's 4–36 ms envelope includes it, which bounds it loosely, but "loosely bounded once" is not "sustainable 33 times a second alongside the local chrome's own render". **This is the phase's required spike, and it should run before the frame-pump plan is written**, exactly as 04-02's `rust-mcp-axum` spike ran before the authorization-server plan.
- Servo exposes a hook that makes input latency measurable end to end without inventing anything: `WebView::notify_input_event` returns an `InputEventId`, and `WebViewDelegate::notify_input_event_handled` fires with it [VERIFIED: `servo-0.4.0/webview.rs:619`, `webview_delegate.rs:960`]. That is the honest "the page has processed your click" signal.
- Constrained-link behaviour needs a **stdlib Python TCP shim** that forwards with an injected delay and a rate cap (see § Validation Architecture). `tc netem` and network namespaces need root and the self-hosted runner is a single unprivileged machine.

---

## Frame Transport and Codec — recommendation

**Binary WebSocket frames. `png::Compression::Fast` + `FilterType::Sub`. 64×64 tile diffs. No base64. No new codec dependency.**

Concretely:

- **Envelope: binary, not text.** One-byte channel tag, then payload. `0x01` control (JSON), `0x02` tabs (JSON, `Vec<TabInfo>`), `0x03` event (JSON, `talaria_protocol::Event`), `0x04` input (JSON — small and infrequent enough that legibility wins), `0x10` frame (fixed-size binary header + raw PNG bytes). This kills the `png_base64` +33% and one whole copy, and keeps every non-frame channel debuggable by eye.
- **Frame header:** tab id, frame sequence, `full` vs `delta`, tile origin x/y, tile width/height, and the last input sequence the server had applied when it painted. That last field is what makes latency measurable and what lets the client discard a frame that predates its own most recent input.
- **Keyframe on:** first attach, resync after reconnect, tab switch, viewport resize, and whenever the dirty-tile count exceeds a threshold (a scroll is cheaper as one keyframe than as 200 tile messages).
- **Encode off the main thread.** `read_to_image` must happen on the winit thread; the resulting buffer is `image::RgbaImage` = `ImageBuffer<Rgba<u8>, Vec<u8>>`, which is `Send`. Move it to an encoder thread that owns the previous frame and does the diff + PNG there. That takes 2.16 ms per keyframe and 0.14 ms per diff off a 30 ms main-thread budget, and it is the difference between the frame pump costing ~10% of the loop and ~25%.
- **Do not use the `pending_captures` machinery.** It waits for `notify_new_frame_ready` with a 1.5 s deadline (`app.rs:2216`) — coherent for a one-shot screenshot, incoherent at 33 Hz. Servo's own documentation says an embedder may call `paint()` and present without a prior `notify_new_frame_ready` and "Servo will repaint even without first calling" it [VERIFIED: `servo-0.4.0/webview.rs:77`]. So the frame pump paints unconditionally on its own tick and lets the **tile diff** decide whether anything is worth sending. Static pages then cost one readback and 0.14 ms of `memcmp` per tick and send nothing.
- **Hold the remotely-viewed tab `show()`n for the life of the view session, not per frame.** `capture_now(hide_after: true)` does `show → wait → paint → read → hide` per call. Doing that 33 times a second is a different cost profile from doing it once. Each tab renders into its own `OffscreenRenderingContext`, so keeping a second tab shown does not disturb the locally displayed one — that invariant is already documented at `app.rs:2197-2202`. Treat it as a **lease**: acquired on attach, released on detach, released on disconnect, and released by the grace-period timer.
- **`encode_screenshot` stays exactly as it is.** The MCP `screenshot` tool's contract (`png_base64`, lossless, `Compression::Default`) is a different product with a different consumer. Sharing the encoder between them would either make screenshots huge or make frames slow.

**Servo gives no damage rectangles.** `notify_new_frame_ready(&self, _webview: WebView)` carries nothing but the webview [VERIFIED: `servo-0.4.0/webview_delegate.rs:948`]. Damage must be computed by the shell from the readback buffer. At 0.14 ms that is not a problem — but it does mean the readback is unconditional, which is why its cost is the spike.

---

## Is a multiplexed WebSocket right? — yes, and it should ride Phase 4's server

The roadmap's line was written before Phase 4 existed. Re-examined against what Phase 4 actually landed, it holds — and the reasons are stronger than they were.

**It should be one WebSocket route on the router `http.rs` already builds, not a separate listener.** [VERIFIED: `crates/talaria-shell/src/http.rs:1044-1170`]

The decisive fact is the layer ordering. `build_router` applies `refuse_page_originated` as the **outermost** axum layer, with the comment *"so it runs first and covers every route this process serves"*. It refuses any request carrying an `Origin` header, and any request whose `Host` does not name the bound address. A WebSocket route added to that router inherits both **for free**.

That matters more for WebSockets than for anything else in this codebase. **WebSockets are not subject to the same-origin policy and there is no CORS preflight** — a page on any origin can open a `ws://`/`wss://` connection to any host, and the browser will send an `Origin` header but will *not* enforce anything itself; origin enforcement is entirely the server's job [CITED: websocket.org/guides/troubleshooting/cors/; corroborated across sources]. Cross-Site WebSocket Hijacking is the standard name for what happens when a server forgets. Talaria's existing outermost layer closes it by construction, and it closes it in the strictest possible direction: a *browser* client can never connect, because a browser always sends `Origin`. **The remote viewer therefore must be a native client — which is Option A anyway, and now it is enforced by the middleware rather than merely chosen.** Write that down in `SECURITY.md`; it is a genuinely good property and a later "let's add a web viewer" would silently delete it.

Second: the lockfile cost of `axum/ws` is one package, measured (§ Package Legitimacy Audit). A separate listener would need its own TLS story, its own Host validation, its own token verification, its own `Connections`-style fd tracking for revocation, and its own graceful shutdown — every one of which `http.rs` already has and every one of which is somewhere a second implementation could disagree with the first.

Third: `http.rs` already has the shutdown, the `RemoteAccess` state machine, the `SHUTDOWN_GRACE`, the `EventLoopSink` and the `StreamRegistry` that terminates a revoked client's open streams. A WebSocket is precisely the kind of long-lived stream that registry exists for.

**One caveat the planner must carry.** `note_streaming_connection` and the `Connections` fd-tracking (`http.rs:520`, `:1256`) were written for the SDK's streams and gate on specific paths. A WebSocket route is a *different* kind of long-lived connection and must be registered with the same machinery, or **revoking a remote viewer would leave its frame stream running while the Access panel showed the row gone** — the exact failure `04-08` spent a plan closing for `/sse`. `tests/e2e/revocation_test.py` should grow a third assertion, in the same shape as the two it already makes: closure observed from the client end.

### Should a human client reuse Phase 4's OAuth?

**A human client is a different trust class from an agent — and it should still use the same credential system.**

The classes genuinely differ. An agent is semi-trusted and gets the tool surface. A remote human is the *trust root at a distance* — `SECURITY.md` says the human at the keyboard is the trust root and that hardening "must not quietly restrict the human path". A remote human is a human whose keyboard is somewhere else, and the honest reading is that they are more trusted than an agent, not less.

But building a second credential system for them would mean a second token store, a second revocation path, a second thing the Access panel has to show, and a second place to get constant-time comparison wrong. **Recommendation: reuse Phase 4's OAuth 2.1 flow, with the client record marked as a viewer rather than an agent** (`agents.rs`'s `AgentClient` gains a kind), so:

- the Access panel lists remote viewers in the same list, revocable by the same two-click gesture, with `StreamRegistry::terminate_client` already doing the right thing;
- audience binding (RFC 8707) already prevents a token minted elsewhere from being replayed at the frame channel;
- verification stays a live read of the one shared `SharedAgents` store, so a revoke lands on the next frame rather than the next restart.

**The bootstrap problem is real and needs a decision.** OAuth's consent step happens in the *server's own chrome* (`ChromePanel::Consent`, raised by `AppEvent::ConsentRequested`). In distributed mode the human is at the *client*. Approving the first remote client therefore requires someone at the server machine. Three ways out:

1. **Accept it.** First pairing requires local access to the server. Arguably correct — it is the same property as "you have to be at the machine to unlock it the first time" — and it is zero new code.
2. **Device Authorization Grant (RFC 8628).** The client displays a user code; the human enters it in the server's chrome. Standard, well-specified, and it is *still* local access to the server, just with better ergonomics. Not obviously worth its own plan.
3. **A pairing code the server displays and the client types.** Bespoke. Rejects itself on "do not hand-roll auth".

**Recommendation: (1), with the constraint written into the client's onboarding copy.** Lock it before planning — it changes the client's first-run UI, which means it changes the UI-SPEC.

**Do not make Tailscale identity headers the boundary.** Serve injects `Tailscale-User-Login` and friends into proxied requests [CITED: tailscale.com/kb/1312/serve — *"Serve traffic includes identity headers when serving traffic from your tailnet"*]. They are useful, and they are **trivially spoofable by any local process connecting directly to `127.0.0.1:8779`**, which is every account on the machine. At most they are a second check *after* the token, in a deployment the browser has been told is behind Serve. Making them primary would be a strictly worse boundary than the one Phase 4 already built. Name this as a threat in the register so nobody reaches for it as a shortcut.

---

## Input — wire shape, and what stops it becoming a remote-control primitive

### Today's path

Winit delivers `WindowEvent::CursorMoved` / `MouseInput` / `MouseWheel` / `KeyboardInput` on the main thread. `forward_mouse_move`/`forward_mouse_button`/`forward_wheel` (`app.rs:1316-1394`) translate to `servo::InputEvent` variants and call `webview.notify_input_event` on `tabs.displayed()`. Keys go through `keyutils::keyboard_event_from_winit`, which maps a `winit::KeyEvent` to a `servo::KeyboardEvent` over a hand-written `NamedKey` table. `handle_browser_shortcut` (`app.rs:1246`) intercepts chrome shortcuts *before* the page sees them.

### Recommended wire shape

Channel `0x04`, JSON payload, hand-mapped in the shell (no serde derive):

```json
{"kind":"mouse_move","tab":7,"seq":1841,"x":412.0,"y":260.0}
{"kind":"mouse_button","tab":7,"seq":1842,"x":412.0,"y":260.0,"button":"left","action":"down"}
{"kind":"wheel","tab":7,"seq":1843,"x":412.0,"y":260.0,"dx":0.0,"dy":-76.0,"mode":"line"}
{"kind":"key","tab":7,"seq":1844,"state":"down","key":"a"}
{"kind":"key","tab":7,"seq":1845,"state":"down","named":"Enter"}
```

Six rules, each of which the planner should treat as a task-level assertion:

1. **Coordinates are in the remote viewport's device pixels, origin at the *page* top-left — never the window's.** The `toolbar_height_device()` subtraction that `forward_mouse_move` performs is a *local window* concern and must not be applied to remote input. Refactor those three functions to take an explicit `&WebView` plus a viewport-relative point, and have the local path do the toolbar subtraction before calling. That makes the offset impossible to apply twice or to apply remotely.
2. **The server clamps.** `forward_mouse_move` already checks `viewport.contains(point)` and emits `MouseLeftViewport` on exit; remote input must go through the same check, on the *target tab's* size, not the displayed tab's. Untrusted coordinates from the wire are exactly the input that check exists for.
3. **Keys do not travel as `winit::KeyEvent`.** Not all of its fields are constructible, and forging one would be a lie about provenance. Add `keyutils::keyboard_event_from_wire(state, key: Option<&str>, named: Option<&str>) -> Option<KeyboardEvent>` sharing the same `NamedKey` table, returning `None` for anything unmapped — the same failure mode the winit path already has.
4. **Remote input never reaches the chrome.** It must reach `webview.notify_input_event` and nothing else: not `Gui`, not `handle_browser_shortcut`, not `apply_ui_actions`, not `ChromeRects`. `talaria-protocol`'s own `ChromeRects` doc comment already articulates why — an actor that can aim synthetic input at the chrome can aim it at the credentials button, the bookmark star and a downloads row's Open button, which hands a file to the OS's default application. That reasoning was written for agents; it applies verbatim here.
5. **Remote input only targets agent-owned tabs.** The tab lookup on the input path returns `None` for `TabOwner::Me`. Structural, per Option C.
6. **`seq` is monotonic per connection; the server drops non-increasing sequences and echoes the last applied one in the next frame header.** That gives replay resistance within a connection, ordering under coalescing, and the latency measurement in one field.

### What stops it becoming a remote-control primitive

Layered, and every layer is either already built or is a named plan:

| Layer | Status | What it stops |
|-------|--------|---------------|
| Listener **off by default** — no thread, no bound port | Built (Phase 4) | The default installation has no input channel at all |
| `refuse_page_originated`: any `Origin` header → refused, outermost layer | Built (Phase 4) | Cross-Site WebSocket Hijacking, and every browser-based client, permanently |
| `Host` must name the advertised identity | Built (Phase 4) | DNS rebinding |
| Bearer token, live store read per request, audience-bound, individually revocable | Built (Phase 4) | An unauthenticated network peer |
| **TLS (Serve or rustls)** | **Phase 5, mandatory** | A tailnet peer or on-path observer reading or injecting input. Note the tailnet is already WireGuard-encrypted — see § Exposure for why that is not sufficient on its own |
| Agent-tabs-only tab lookup | Phase 5 | Remote input reaching the human's own tabs |
| Chrome unreachable from the input path | Phase 5 | Synthetic input aimed at credentials/bookmarks/downloads |
| Revocation closes the WebSocket, tracked by `Connections` | Phase 5 (extends Phase 4's mechanism) | A revoked viewer still driving the browser |

**The one thing none of these stop, and it must be stated in `SECURITY.md`:** a legitimate, authorized remote viewer *is* a remote-control primitive, by design. That is the feature. The boundary is which clients you authorize — the same sentence `SECURITY.md` already uses about agents, now with sharper teeth, because a viewer sends real clicks into a logged-in session rather than tool calls into an allowlisted surface.

---

## Reconnect and Resync (DIST-03) — what is actually at risk

### What "in-progress agent work" is, concretely

Five queues on `Shared`, each with its own bound [VERIFIED: `app.rs:128-271`, `control.rs:290`]:

| Queue | Contents | Bound | What a dropped link does to it |
|-------|----------|-------|-------------------------------|
| `pending_evals` | promise polling, CSP re-runs | `promise_wait()` = command timeout − 2 s ≈ 28 s | Nothing. Runs to completion; the reply's `oneshot` receiver is gone, `let _ = send` |
| `pending_loads` | `tabs_open`/`navigate` replies awaiting load | `LOAD_WAIT` 20 s | Same |
| `pending_captures` | background-tab screenshots | 1.5 s | Same |
| `evaluating` | per-tab in-flight guards | own deadline, registered with `next_capture_deadline` | Self-heals |
| in-flight `AgentRequest` | one `tokio` task per request in `control.rs` | `command_timeout_secs()` 30 s | The task answers into a dead channel; harmless |

**Good news, verified: nothing here is torn down by a disconnect, and no tabs are closed.** `AppEvent::SessionEnded` does exactly two things — `sessions.borrow_mut().remove(&session_id)` and `request_redraw()` (`app.rs:1022-1025`). Tabs are untouched. So DIST-03's headline claim, "a dropped connection does not kill in-progress agent work", is **already true of the engine** and this phase does not have to make it true; it has to keep it true and prove it.

### The gap that is real: addressability, not state

`TabOwner::Agent { session_id, client }` (`tabs.rs:22`) ties a tab to a **session id**. When the link drops and the agent reconnects it gets a *new* session id from `next_session_id()`. Its old tabs are still open, still in the Agents view, still driveable by the human — and:

- `queue_event` (`app.rs:712`) addresses lifecycle events to `session_id`. `process_pending_events` finds no such session and **discards the entry**, with a comment saying that is "correct addressing, not a lost notification". Correct before reconnect existed; wrong after.
- The reconnected agent has no way to reclaim its own tabs. `tabs_list` will show them (the Agents view is not session-scoped for listing), but crash and close notifications for them go nowhere.

**Recommendation: rebind by verified client identity on reconnect.** On a reconnect that presents a valid token, walk the tab table and re-point every `TabOwner::Agent` whose client matches the reconnecting client's **verified `client_id`** to the new session id.

**And key on the verified `client_id`, never on the `client` display string.** That string is `ClientMessage::Hello { client }` on the control socket — self-asserted, and over HTTP the tab label comes from the same place. Rebinding on a display name would let one client claim another's tabs by choosing its name. On the Unix socket that is moot (same UID); over the network it is a tab-hijack primitive. Phase 4 already has the right value: `agents.rs`'s `AgentClient` id, carried through `AuthInfo` and `NoteVerifiedIdentity`. `TabOwner::Agent` needs to carry it.

### Grace period — recommendation

**Grace period: 120 s, on the frame/view lease and on tab ownership; not on anything else.**

- Below 30 s and an ordinary Tailscale path change (a laptop moving from Wi-Fi to LTE, a DERP→direct upgrade) would expire a session mid-task. Above a few minutes and a genuinely departed client holds a `show()`n webview and a frame pump indefinitely.
- 120 s is comfortably above the longest existing bound (the 30 s command timeout) so it can never expire *while* a command it owns is still running. **Keep the ordering rule:** grace (120 s) > command timeout (30 s) > `promise_wait` (28 s) > `LOAD_WAIT` (20 s) > capture (1.5 s).
- What the grace period actually holds: the frame lease (so a reconnect within it does not need a `show()`/repaint cycle), the tab-ownership rebinding window, and the session's outbound event channel. What it does **not** hold: the token (verification stays per request — a revoke during the grace period must still take effect immediately), or any queue above.
- Make it `TALARIA_REMOTE_GRACE_SECS` with the same shape as `TALARIA_COMMAND_TIMEOUT_SECS`: parse failure falls back to the default, never to zero and never to unbounded.

### Resync — recommendation

**Snapshot, not replay.** `McpAppState` is built with `event_store: None` and the comment records why: resumability "would mean deciding how long a disconnected client may replay from, which is a question this phase has not asked" (`http.rs:1078`). Phase 5 should not answer it either. On reconnect the client sends its last known tab-list version; the server replies with a full `Vec<TabInfo>`, the current view mode, and a **keyframe** for whichever tab the client re-attaches to. That is bounded, stateless on the server, and exactly what "the client resyncs on reconnect" asks for.

Input during a drop is **discarded, not queued**. Replaying a click the human made 40 seconds ago into a page that has since navigated is worse than dropping it. The `seq` echo in the frame header is what tells the client its input was lost.

---

## Session Manifest (DIST-04)

### What must persist

Minimum: for each tab — `tab_id`, `url`, `title`, owner kind, the owning client's **stable `client_id`** (not the display label), and whether it was focused in its view. Plus a `session_epoch` (a monotonic counter bumped at each clean start) and a `written_at_ms`.

Deliberately **not** persisted: scroll position, form state, cookies beyond what the Servo profile dir already holds, or anything from a Me tab beyond what history already records. A manifest that tried to restore *page* state would be a session-restore feature, which is a different product.

### Where, and in what shape

The `dirs` config dir, alongside `config.json`, `agents.json` and the four Phase 3 stores. Follow `Agents::save` exactly (`agents.rs:928-950`): serialize the whole document, stage a `.tmp` sibling, `fs::rename` into place, `0600` via `crate::permissions::write_owner_only`. Hand-mapped through `serde_json::json!` both ways, `to_json` and `from_json` adjacent, because the shell has no derive. A malformed file degrades to "no manifest", never to a panic — the `settings.rs` model.

### Write cadence — and the trap `deferred-items.md` already documented

`04-.../deferred-items.md`'s "No last used column" entry is directly on point: every store here is a **whole-document atomic rewrite**, so a value written on every event makes every event a full-file write. A manifest written on every URL change would do exactly that.

**Recommendation:** write on structural change (tab open, tab close, owner change, view switch) and **debounce URL/title changes to at most one write per second**, with the in-memory value authoritative between flushes, plus one final write on clean shutdown. Deliberately do **not** write on every `notify_url_changed` — that callback fires several times during a single navigation.

### "Offers to restore" when the client may not be connected yet

The offer is **server-side state**, not a message. Concretely: at startup, if a manifest exists whose `session_epoch` does not match a clean-shutdown marker, `Shared` holds a `RestoreOffer`. It is surfaced two ways from that one source of truth:

- in the **server's own chrome**, as a panel (the `ChromePanel` enum is already the right shape for it);
- to **any client that connects while the offer is outstanding**, on the control channel, as part of the initial snapshot.

Whoever answers first resolves it; the other surface stops showing it. It expires on a timer (recommend the same 120 s as the grace period, or on first navigation) so a browser that has been used for an hour is not still offering to restore a session from before lunch.

**The hard part is what a restored agent tab becomes.** `TabOwner::Agent { session_id }` names a session that no longer exists, and the client that owned it may have been revoked while the browser was down. Options: (a) restore as `Me` tabs; (b) restore under a distinct "orphaned" owner in the Agents view; (c) restore only if the same `client_id` re-authorizes.

**Recommendation: (b), with (c) as the upgrade path** — restore into the Agents view with the recorded `client_id` and no live session, exactly the state `04-.../deferred-items.md` already accepted for a revoked client's tabs (*"left in the Agents view owned by a client that no longer exists. The second is what happens today and it is coherent"*). Then the reconnect rebinding from DIST-03 picks them up for free if that client comes back. Reusing an already-accepted state beats inventing a fourth ownership condition. (a) is wrong because it silently moves an agent's pages into the human's own view, where they would start appearing in history.

**Never auto-restore.** Reopening pages under an owner whose token may have been revoked, without a human saying so, is the one behaviour here that could surprise someone badly.

---

## Exposure and the Certificate Story — the inherited Phase 4 obligation

`04-.../deferred-items.md` states it without ambiguity: **"Phase 5 must not enable a non-loopback bind before that lands."** Phase 4 shipped without TLS because it stayed on loopback, which is OAuth 2.1 §1.5's explicit exception:

> *"All the OAuth protocol URLs (URLs exposed by the AS, RS and Client) MUST use the `https` scheme except for loopback interface redirect URIs, which MAY use the `http` scheme."* [CITED: draft-ietf-oauth-v2-1-13 §1.5]

Note the exact scope of the exception, because it is narrower than the Phase 4 documents' shorthand: it covers **loopback interface *redirect URIs***. The general rule — every OAuth URL the AS, RS and client expose — has no locality carve-out at all. A tailnet address is not loopback under either reading, so `issuer`, `authorization_endpoint`, `token_endpoint`, `registration_endpoint`, `revocation_endpoint` and the RFC 8707 `resource` would all have to become `https` the moment the browser is reachable at a `ts.net` name. **`oauth.rs` hardcodes `http://` in all of them** (`canonical_resource` at `:143`, `issuer` at `:1163`, the four endpoint URLs at `:1341-1344`, `authorization_servers` at `:1285`). That is a concrete, greppable work item.

### Option 1 — Tailscale Serve (**RECOMMENDED**)

`tailscale serve` runs an HTTPS reverse proxy on the node: `tailscaled` terminates TLS with a Tailscale-managed certificate and forwards to a local service. **Only `http://127.0.0.1` is supported as a proxy target** [CITED: tailscale.com/kb/1242/tailscale-serve] — which is precisely and exclusively what Talaria binds. Serve is tailnet-only; **Funnel** is the public-internet variant and must not be used.

Why this is the right answer here:

- **`BIND_HOST` stays a constant.** `settings.rs`'s `LOOPBACK_BIND`, the `bind`-key refusal, and all three of its tests (`every_bind_address_other_than_the_loopback_literal_is_refused`, and the two around it) survive untouched. D-04-04's structural enforcement is not reopened — the program still cannot express a wider bind.
- **Certificate lifecycle is not Talaria's problem.** Serve provisions and renews automatically. `tailscale cert`, by contrast, writes files and *"you are responsible for renewing any certificates that you create"*, with a 90-day Let's Encrypt expiry [CITED: tailscale.com/kb/1153/enabling-https, tailscale.com/kb/1080/cli]. A long-running rustls server that built its `ServerConfig` once at startup would happily serve an expired certificate on day 91 — the classic failure, and avoiding it means a `ResolvesServerCert` that re-reads the files, which is real code nobody wants to own in a browser.
- Prerequisites are one-time and operator-side: HTTPS Certificates enabled in the tailnet's DNS settings, MagicDNS, and acknowledging that machine names appear in Certificate Transparency logs.

**Three costs, all real, none fatal:**

1. **The browser must advertise an identity it did not bind.** Under Serve the client connects to `https://<machine>.<tailnet>.ts.net/mcp`, and `DnsRebindProtector::new(Some(vec![bound]))` is seeded with `"127.0.0.1:8779"` — so the proxied request is **refused**. And the metadata documents would advertise `http://127.0.0.1:8779`, which the client would reject against the URL it actually used. This needs a new configured value — call it the **advertised base URL** — that feeds `canonical_resource`, `issuer`, the endpoint URLs and the Host allowlist. **It must come from configuration (or from `tailscale status`), never from the request's `Host` header**, or a local page could make the browser advertise an attacker-chosen issuer.
2. **`refuse_page_originated` must keep refusing `Origin`.** A proxy that added one would break every client; verify by observation, not assumption.
3. **WebSocket upgrades through Serve have a history.** Serve does proxy upgrades, but there are reported regressions where stricter upgrade handling broke WebSocket clients behind Serve HTTPS proxying [CITED: github.com/openclaw/openclaw issue 54008; community reports]. **This is the second required spike:** stand up a trivial `axum/ws` echo behind `tailscale serve` and confirm the upgrade completes and stays up through a keep-alive interval, before the frame plan is written.

Note what Serve does *not* change: between `tailscaled` and Talaria the traffic is plain HTTP on loopback, so every local account can still reach `127.0.0.1:8779`. That is exactly the Phase 4 posture, unchanged — the bearer token remains the entire local boundary.

### Option 2 — widen the bind + `tailscale cert` + rustls in process (documented fallback)

Bind the tailnet address, load the PEM `tailscale cert` writes, terminate TLS with `axum-server`'s `tls-rustls`. Everything needed is already in the lock (`axum-server 0.8.0`, `tokio-rustls 0.26.4`, `rustls 0.23.43`), the crypto provider is already installed in `main.rs`, and `rustls-pki-types 1.15.1` has a `pem` module so even PEM parsing costs nothing.

**But** it reopens the one thing Phase 4 deliberately made unrepresentable. If this branch is taken, the safe shape is *not* a bind-address string in `config.json`:

> `RemoteAccessConfig` gains an **enum**, not a string: `Exposure::Loopback` | `Exposure::Tailnet`. `Tailnet` resolves its address **at bind time from the Tailscale interface**, not from the file. The file can express "use the tailnet"; it can never express "bind 0.0.0.0". `Exposure::Tailnet` is **rejected at load** unless a certificate is configured and loadable — so "non-loopback without TLS" is unrepresentable in the same way "wildcard bind" is today. The `bind`-key refusal in `from_json` stays exactly as it is, including its test, because a hand-edited `"bind"` is still not a thing this program honours.

That shape preserves the spirit of D-04-04 (the program cannot express a wider bind than the two it supports) while admitting the one address this phase needs. It is more code than Option 1 and it owns certificate renewal.

### Option 3 — "the tunnel is the transport security" (**REJECT, with the honest reasoning**)

The tempting argument: Tailscale is WireGuard, end-to-end encrypted and authenticated between nodes, so TLS on top is encrypting an encrypted channel. It deserves a straight answer rather than a reflex.

**What is true:** on a direct path the traffic is genuinely end-to-end encrypted between the two nodes' WireGuard keys, and on a DERP-relayed path the relay sees only ciphertext. As a confidentiality argument against a network observer, it holds.

**Why it is still not sufficient, in four parts:**

1. **OAuth 2.1 §1.5 is a normative MUST on the URL scheme, not on the achieved confidentiality.** It says OAuth protocol URLs MUST use `https`, with an exception for loopback *redirect URIs*. A conformant MCP client fetching `http://machine.ts.net/.well-known/oauth-authorization-server` is entitled to refuse, and several will. This is a conformance question before it is a security question, and the project has already chosen conformance over convenience once, deliberately, in D-04-04.
2. **The tunnel authenticates *nodes*, not *processes*.** Every process on every device in the tailnet — and every other local account on the server — reaches that port. WireGuard says "this packet came from a device in your tailnet". It does not say "this came from your MacBook's Talaria client rather than from something else running on your MacBook".
3. **It is not the tailnet's decision to make.** ACLs, a shared node, or a future Funnel toggle could widen who reaches the address without anything in Talaria changing. Security that depends on an external policy staying narrow is security that can be turned off from a web console.
4. **`SECURITY.md` already made this exact argument about loopback and refused it** — *"loopback is not a security boundary"*, *"the bearer token is the entire boundary"*. Accepting "the tunnel is the boundary" would contradict the reasoning this project already published about a strictly stronger locality claim.

**Recommendation: Option 1 (Tailscale Serve), with Option 2 documented as the fallback for anyone not using Serve, and Option 3 explicitly refused in `SECURITY.md` with reason (2) as the headline** — because it is the one a reader is most likely to rediscover and act on.

---

## Architecture Patterns

### System architecture

```
   SERVER MACHINE (Linux, Servo build prerequisites)          CLIENT MACHINE
   ┌──────────────────────────────────────────────┐
   │  talaria-shell  (one process)                │
   │                                              │
   │  ┌── winit main thread ─────────────────┐    │
   │  │  Servo engine ── WebView per tab     │    │
   │  │     └─ OffscreenRenderingContext ────┼────┼──┐
   │  │  egui chrome (same glow context)     │    │  │
   │  │  Shared (Rc, RefCell) · TabManager   │    │  │ paint() +
   │  │  apply_ui_actions ← Vec<UiAction>    │    │  │ read_to_image
   │  │        ▲                    ▲        │    │  │ (main thread only)
   │  └────────┼────────────────────┼────────┘    │  │
   │           │ AppEvent           │ AppEvent    │  ▼
   │           │ (EventLoopProxy)   │        ┌────────────────────┐
   │  ┌────────┴────────┐  ┌────────┴─────┐  │ FRAME PUMP thread  │
   │  │ control thread  │  │ http thread  │  │  prev frame buffer │
   │  │ Unix socket     │  │ axum router  │  │  64x64 tile diff   │
   │  │ SO_PEERCRED     │  │  ├ /mcp      │  │  png Fast+Sub      │
   │  │ (stdio proxy)   │  │  ├ /sse      │  └─────────┬──────────┘
   │  └─────────────────┘  │  ├ /authorize│            │ tiles + keyframes
   │                       │  ├ /token    │            │
   │                       │  ├ /revoke   │            ▼
   │                       │  └ /view  ◄──┼──── one multiplexed WS
   │                       │              │            │
   │                       │ OUTERMOST LAYER:          │
   │                       │  refuse Origin + check Host
   │                       │ THEN: AuthMiddleware (bearer, audience-bound)
   │                       └──────┬───────┘            │
   └──────────────────────────────┼────────────────────┼──────────────┘
                                  │ http://127.0.0.1:8779
                          ┌───────▼────────┐
                          │  tailscaled    │  TLS termination,
                          │ tailscale serve│  Tailscale-managed cert
                          └───────┬────────┘
                                  │ wss://machine.tailnet.ts.net/view
                    ══════════════╪══════════════ WireGuard (direct)
                                  │               or DERP (relayed)
                    ══════════════╪══════════════
                                  ▼
                    ┌─────────────────────────────────┐
                    │  talaria-client (no Servo)      │
                    │   winit window + egui           │
                    │   ┌ decode PNG ─→ texture ─→ present
                    │   ├ tab list / connection state │
                    │   ├ input capture ─→ seq ─→ WS  │
                    │   └ adaptive-rate controller     │
                    │      (EWMA latency + goodput,    │
                    │       degrade ladder, report)    │
                    └─────────────────────────────────┘
```

Trace of the primary use case (remote takeover): human clicks in the client window → client stamps `seq`, sends an input envelope → WS → outermost Origin/Host layer → auth middleware → `AppEvent` on the `EventLoopProxy` → main thread resolves tab id (agent-owned only), clamps to the tab's viewport, calls `notify_input_event` → Servo hit-tests and the page reacts → next frame-pump tick paints and reads back → encoder thread diffs against the previous frame and encodes only the changed tiles → frame envelope carries the tiles plus the last applied `seq` → WS → client decodes, blits, and updates its latency EWMA from the echoed `seq`.

### Recommended structure

```
crates/
├── talaria-protocol/src/
│   ├── lib.rs           # TabInfo, Command, Outcome, Event — unchanged
│   ├── local.rs         # socket_dir/socket_path/ensure_socket_dir/current_uid, cfg(unix)
│   └── wire.rs          # NEW: multiplexed envelope, FrameHeader, InputMessage, ViewMessage
├── talaria-shell/src/
│   ├── http.rs          # + the /view WebSocket route on the existing router
│   ├── view.rs          # NEW: frame pump, tile diff, encoder thread, view leases
│   ├── remote_input.rs  # NEW: wire input -> validated servo InputEvent, agent-tabs-only
│   ├── manifest.rs      # NEW: session manifest store (.tmp + rename, 0600)
│   ├── app.rs           # forward_mouse_* generalised to take &WebView + viewport point
│   ├── keyutils.rs      # + keyboard_event_from_wire, sharing the NamedKey table
│   └── tabs.rs          # TabOwner::Agent gains the verified client_id
└── talaria-client/      # NEW binary crate — winit + egui, does NOT link servo
    └── src/{main,net,present,input,rate}.rs
```

### Pattern 1: the frame pump is a lease, not a request

**What:** a client "attaches" to a tab; the server takes a view lease (`webview.show()` on that tab's own offscreen context, register a tick), pumps frames on its own clock, and releases on detach / disconnect / grace expiry.

**When:** whenever the cadence is above roughly 2 Hz. Below that, `pending_captures` is fine.

**Why not request/reply:** `Command::Screenshot` waits for `notify_new_frame_ready` with a 1.5 s deadline and does `show → wait → paint → read → hide` per call. At 33 Hz that is 33 show/hide cycles a second and 33 dead 1.5 s deadlines, and a static page would deliver nothing until each timeout. The lease inverts it: paint unconditionally on the tick (legal — `webview.rs:77`), diff, send only what changed.

### Pattern 2: readback on the loop, encode off it

```rust
// On the winit main thread, in the frame-pump tick:
webview.paint();
let Some(image) = context.read_to_image(rect) else { return };
// servo::RgbaImage is image::RgbaImage — ImageBuffer<Rgba<u8>, Vec<u8>> — and is Send.
let _ = encode_tx.send(FrameJob { tab_id, sequence, last_applied_input, image });
```

The encoder thread owns the previous frame per tab, does the 0.14 ms diff and the 0.007–2.16 ms PNG, and hands bytes to the WebSocket writer. The main thread never spends more than the readback.

### Pattern 3: hand-mapped wire types, both directions adjacent

Follow `settings.rs`/`agents.rs`: `to_json` and `from_json` next to each other so a field added to one is visibly missing from the other; `serde_json::json!` rather than a derive; every parse failure a refusal, never a silent default. Types that live in `talaria-protocol` may derive; anything the shell defines may not.

### Anti-patterns to avoid

- **Reusing `encode_screenshot` for frames.** 21 ms/175 ms measured. It is the right encoder for the wrong job.
- **Base64 inside a JSON envelope on the frame channel.** +33% and a copy, for nothing, on a binary transport.
- **Waiting for `notify_new_frame_ready` in the pump.** A static page never produces one; the 1.5 s deadline is a screenshot-shaped answer to a frame-shaped question.
- **Doing the tile diff or the PNG encode on the winit thread.** 2.16 ms of a 30 ms budget, for work that is trivially `Send`.
- **Routing remote input through `tabs.displayed()`.** Couples the remote human's target to whatever the local human is looking at — and the local human switching tabs would silently redirect remote clicks into a different page.
- **Applying `toolbar_height_device()` to remote coordinates.** Silent ~40 px offset on every remote click.
- **Rebinding tab ownership on the `client` display string.** Self-asserted; a tab-hijack primitive over the network.
- **Trusting `Tailscale-User-*` headers as the boundary.** Spoofable by any local process reaching `127.0.0.1:8779`.
- **A bind-address string in `config.json`.** D-04-04's whole point is that a wider bind is not expressible. If exposure widens, widen it as an enum with a certificate precondition.
- **Auto-restoring the manifest.** Reopening an agent's pages without a human saying so.
- **Writing the manifest on every `notify_url_changed`.** Whole-document rewrite per callback, several per navigation — the exact cost that killed the "last used" column.

---

## Don't Hand-Roll

| Problem | Don't build | Use instead | Why |
|---------|-------------|-------------|-----|
| WebSocket framing, masking, close handshake, ping/pong | A hand-rolled RFC 6455 codec | `axum::extract::ws` (`tokio-tungstenite 0.29.0`) | One new lockfile package (measured). Masking, fragmentation, control frames and close semantics are exactly the surface that produces protocol-confusion bugs |
| TLS certificate issuance and 90-day renewal | A cert-provisioning path in a browser | `tailscale serve` (Option 1) or `tailscale cert` + `axum-server`'s `tls-rustls` (Option 2) | ACME clients are a whole product. Serve removes renewal from this repository entirely |
| Authenticating a remote client | A pairing-secret scheme | Phase 4's OAuth 2.1 + `agents.rs` token store | It is built, tested end to end, audience-bound, individually revocable, and already wired into the Access panel |
| Origin / DNS-rebinding defence on the WebSocket | A per-route header check | The existing `refuse_page_originated` **outermost** layer | Already covers every route; a per-route check is one route away from being forgotten. WebSockets have no browser-side origin enforcement, so this is the entire defence |
| Terminating a revoked viewer's live stream | A polling "is this token still valid" loop | Extend `Connections`/`StreamRegistry` (`http.rs:520`, `:681`) | 04-08 already solved this for `/sse`, including the SDK defect that made it hard. A second mechanism would diverge |
| PNG encoding | A DEFLATE or filter implementation | `png` 0.17, already a dependency, with `Compression::Fast` + `FilterType::Sub` | Measured at 0.007 ms for a tile |
| JPEG, *if it is ever needed* | A DCT/Huffman encoder | `jpeg-encoder` 0.6/0.7 with `simd` | 6.2 ms vs `image`'s 19–20 ms for the same frame |
| Atomic `0600` store writes | Another bespoke save path | `crate::permissions::write_owner_only` + the `.tmp`/`rename` shape from `Agents::save` | Six stores already do it identically; a seventh that differs is a bug waiting for a code review |
| Detecting a relayed link | Latency heuristics alone | `tailscale status` (`direct` vs `relay`) **plus** the measured EWMA | The heuristic tells you it is slow; Tailscale tells you *why*, which is what turns a complaint into a fix |

**Key insight:** almost every hard part of this phase already exists somewhere in this repository. The transport middleware, the token store, the revocation-closes-streams mechanism, the atomic store convention, the off-thread-actor pattern and the deferred-queue discipline are all built and all tested. The genuinely new code is a frame pump, a tile diff, an input validator and a client binary. Any plan that finds itself reimplementing one of the first six has taken a wrong turn.

---

## Common Pitfalls

### Pitfall 1 — reusing the screenshot encoder for frames

**What goes wrong:** the frame path is built on `capture_now`/`encode_screenshot` because they exist and produce a PNG. Local testing on a simple page looks acceptable; a real page blows the budget.
**Why:** `Compression::Default` measured 21 ms page / 175 ms photo at 1280×800. The 175 ms case is three to six times the *entire* budget with no network involved.
**Avoid:** a separate encoder for frames, `Compression::Fast` + `FilterType::Sub`, on tile diffs, off the main thread. Leave `encode_screenshot` alone.
**Warning signs:** frame rate that collapses on image-heavy pages; a main loop that stops repainting its own chrome while a remote viewer is attached.

### Pitfall 2 — measuring on loopback and declaring the budget met

**What goes wrong:** everything passes at 2 ms RTT and fails on a relayed link.
**Why:** loopback hides transmission entirely. A 522 KB keyframe is instantaneous locally and 321 ms on a 13 Mbit/s DERP path.
**Avoid:** the Python bandwidth/latency shim from § Validation Architecture, exercised at 13 Mbit/s with 40 ms of added delay — this user's own measured relayed profile — as a committed regression test.
**Warning signs:** a plan whose only latency assertion runs against `127.0.0.1`.

### Pitfall 3 — the toolbar offset applied to remote coordinates

**What goes wrong:** every remote click lands ~40 px above where the human aimed. Nothing errors. Links near the top of the page appear to work; links below a control appear to be "flaky".
**Why:** `forward_mouse_move` subtracts `toolbar_height_device()` because the *local window* has a toolbar above the page. A remote client renders no server toolbar.
**Avoid:** refactor the three forwarders to take an already-viewport-relative point; the local path does the subtraction at the call site.
**Warning signs:** an e2e assertion that a remote click hits a link *at a known y*, not just that "a click navigated".

### Pitfall 4 — a revoked viewer keeps receiving frames

**What goes wrong:** the Access panel shows the row gone; the WebSocket keeps streaming.
**Why:** verification is per-request and a WebSocket has no next request. This is threat T-7 from Phase 4, in a new shape, and 04-08 needed a plan plus a three-call `extern "C"` block to close it for `/sse`.
**Avoid:** register the WebSocket with `Connections` at accept time and bind it to the verified client, so `terminate_client` closes it. Assert closure **from the client end**, the way `revocation_test.py` already does.
**Warning signs:** a plan that says "revocation already works" without touching `http.rs`'s connection tracking.

### Pitfall 5 — the reconnected agent loses its tabs' notifications

**What goes wrong:** after a link blip, tabs are still there and still driveable, but `TabCrashed`/`TabClosed` for them silently go nowhere.
**Why:** `process_pending_events` discards an event whose `session_id` is absent — correct before reconnect existed. A reconnect mints a new session id and nothing rebinds the tabs.
**Avoid:** rebind `TabOwner::Agent` by **verified `client_id`** on reconnect.
**Warning signs:** a DIST-03 test that only asserts the tabs are still listed. Assert that a crash on a pre-drop tab reaches the reconnected client.

### Pitfall 6 — the manifest write becomes a per-callback full-file rewrite

**What goes wrong:** navigation gets measurably slower with a large tab set, for no visible reason.
**Why:** whole-document atomic rewrite × several `notify_url_changed` callbacks per navigation.
**Avoid:** debounce to ≤1 write/second for URL/title; write immediately only on structural change.
**Warning signs:** the manifest save called from a delegate callback rather than from a drain on the loop — which is also a `RefCell` re-entrancy hazard.

### Pitfall 7 — `cargo add` or `cargo update` for the WebSocket dependency

**What goes wrong:** `primeorder` resolves to `0.14.0` final and the build fails with a trait-bound error naming none of the cause.
**Why:** documented at `Cargo.toml:15-33` and `SPEC.md:98`.
**Avoid:** edit the manifest by hand; `cargo metadata`; inspect the lock diff; expect exactly one added package.
**Warning signs:** a lock diff with more than one added `[[package]]`, or any changed version line.

### Pitfall 8 — the `Host` allowlist and the advertised issuer drift apart

**What goes wrong:** behind Tailscale Serve every request 403s, or worse, the browser advertises an issuer an attacker chose.
**Why:** `DnsRebindProtector` is seeded from the *bound* address; `issuer`/`resource` are `format!("http://{bound}...")`. Serve changes what the client uses without changing what the server bound.
**Avoid:** one **advertised base URL**, configured (or read from `tailscale status`) — never from a request header — feeding the Host allowlist, `canonical_resource`, `issuer` and all four endpoint URLs from a single place, exactly as `canonical_resource`'s own doc comment argues for the resource identifier.
**Warning signs:** a second `format!("http` or `format!("https` anywhere in `oauth.rs`.

### Pitfall 9 — a browser-based viewer gets built "because WebSockets"

**What goes wrong:** someone adds a small web UI for the client. It sends `Origin`. `refuse_page_originated` refuses it. Someone then relaxes the Origin rule to make it work, and CSWSH is live against a browser holding a credential vault.
**Why:** the Origin refusal reads like an inconvenience if you do not know what it is for.
**Avoid:** state in `SECURITY.md` that the remote viewer is native **because** the Origin refusal makes a browser client impossible, and that this is a property to keep.
**Warning signs:** any diff that adds an origin allowlist.

---

## Code Examples

### Frame encode — the measured configuration

```rust
// Source: measured this session (/tmp/frmbench) against png 0.17, already a
// dependency. 1280x800 page-like frame: 2.16 ms / 522 KB full, 0.007 ms /
// 3.9 KB for one 64x64 tile. Compare Compression::Default: 21 ms / 175 ms.
fn encode_tile(pixels: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder.set_filter(png::FilterType::Sub);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(pixels).ok()?;
    }
    Some(out)
}
```

### Dirty-tile scan — 0.14 ms over a whole 1280×800 frame

```rust
// Source: measured this session. Row-slice comparison per tile; the first
// differing row short-circuits, which is why a mostly-static frame is cheap.
const TILE: usize = 64;

fn dirty_tiles(current: &[u8], previous: &[u8], width: usize, height: usize) -> Vec<(usize, usize)> {
    let mut dirty = Vec::new();
    for tile_y in (0..height).step_by(TILE) {
        for tile_x in (0..width).step_by(TILE) {
            let tile_width = (tile_x + TILE).min(width) - tile_x;
            let changed = (tile_y..(tile_y + TILE).min(height)).any(|y| {
                let start = (y * width + tile_x) * 4;
                let end = start + tile_width * 4;
                current[start..end] != previous[start..end]
            });
            if changed {
                dirty.push((tile_x, tile_y));
            }
        }
    }
    dirty
}
```

### The WebSocket route on the existing router

```rust
// Source: axum 0.8.9 (already in Cargo.lock via rust-mcp-axum); the router is
// crates/talaria-shell/src/http.rs build_router. Added inside mcp_routes(..)'s
// result, INSIDE the two existing .layer() calls so it inherits
// refuse_page_originated as its outermost layer.
use rust_mcp_axum::axum::extract::ws::{WebSocketUpgrade, WebSocket};

async fn view_upgrade(upgrade: WebSocketUpgrade, /* verified identity, sink */) -> Response {
    upgrade.on_upgrade(|socket| async move { serve_view(socket).await })
}
```

### Servo input, targeting a tab rather than the displayed one

```rust
// Source: crates/talaria-shell/src/app.rs:1316-1377, generalised. The local
// path subtracts toolbar_height_device() BEFORE calling; the remote path does
// not, because a remote client renders no server toolbar.
fn deliver_mouse_button(
    webview: &WebView,
    viewport_point: euclid::Point2D<f32, DevicePixel>,
    button: ServoMouseButton,
    action: MouseButtonAction,
) {
    let size = webview.size();
    let viewport = euclid::Rect::new(
        euclid::Point2D::zero(),
        euclid::Size2D::new(size.width, size.height),
    );
    if !viewport.contains(viewport_point) {
        return;
    }
    webview.notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
        action,
        button,
        viewport_point.into(),
    )));
}
```

### Latency instrumentation Servo already provides

```rust
// Source: servo-0.4.0/webview.rs:619 and webview_delegate.rs:960.
// notify_input_event returns an id; the delegate is told when the page has
// handled it. That is the honest input-to-handled signal, without inventing
// a timing hook.
let event_id = webview.notify_input_event(event);
// impl WebViewDelegate for Shared:
//   fn notify_input_event_handled(&self, webview: WebView, id: InputEventId, ..)
```

---

## State of the Art

| Old approach | Current approach | When changed | Impact |
|--------------|------------------|--------------|--------|
| MCP over stdio only | Streamable HTTP + OAuth 2.1 in-process, off by default, loopback-only | Phase 4, 2026-08-21 | Phase 5 has a server, a router, a token store and a revocation mechanism it does not have to build |
| `sse_support: false` means the legacy routes are not mounted | It is cosmetic; `/sse` and `/messages` are mounted and live, and the `sse` feature is enabled transitively via `rust-mcp-axum` | Corrected 2026-08-21 in `deferred-items.md` | Any "which routes exist" reasoning must read `mcp_routes`, not the flag |
| `AxumServerOptions.enable_ssl` is the TLS path | **Talaria does not use `create_axum_server`.** It takes the BYO-server path with its own `tokio::net::TcpListener`, so `AxumServerOptions` — and its `enable_ssl`/`ssl_cert_path`/`ssl_key_path` — are **not on Talaria's mount path at all** | Observed 2026-08-21 reading `http.rs:1044-1170` | **Corrects `04-.../deferred-items.md`**, which names those fields as the certificate route. TLS in-process means `axum-server`'s `tls-rustls` (or Serve, which needs neither) |
| MCP spec revision `2025-11-25` with sessions | `2026-07-28` removes sessions from the protocol layer | Spec final 2026-07-28; SDK has not landed it | Phase 5's grace-period and rebinding design is written against the session model. When the SDK migrates, DIST-03's shape changes with it — note it in this phase's own deferred items |
| Tailscale as "a fast flat network" | Direct WireGuard ≈ LAN speed; DERP-relayed ≈ 13–35 Mbit/s with 20–50 ms added | Always true; measured on this tailnet 2026-08-14 | SC 2 is conditional on connection type, and the product must say which one it has |

**Deprecated / outdated:**
- The roadmap's "adaptive screenshot polling" phrasing — *polling* is the wrong verb. A pull-per-frame model is the thing to avoid; the recommendation is a server-driven push with an adaptive rate.
- `PROJECT.md`'s "the intended distributed-mode wire" for `talaria-protocol` — half right, see the assessment above. Update at the phase transition.
- `04-.../deferred-items.md`'s TLS route via `AxumServerOptions` — superseded, see the table row above.

---

## Assumptions Log

| # | Claim | Section | Risk if wrong |
|---|-------|---------|---------------|
| A1 | `OffscreenRenderingContext::read_to_image` is fast enough to run at 30 ms cadence alongside the local chrome's own render | Latency Budget | **Highest risk in the phase.** If the readback is 15–20 ms on llvmpipe (plausible), the e2e harness can never demonstrate SC 2 and the whole capture model needs rethinking. **Named as required spike 1** |
| A2 | Tailscale Serve proxies a WebSocket upgrade to `http://127.0.0.1` and keeps it alive | Exposure | If it does not, Option 1 collapses to Option 2 and the phase inherits certificate renewal. **Named as required spike 2**; there are public reports of upgrade regressions behind Serve |
| A3 | Adding `axum = { features = ["ws"] }` compiles against the same `axum` node `rust_mcp_axum::axum` re-exports | Standard Stack | Resolution confirmed one node; *compilation* was not attempted. Low risk, but the plan should compile before it designs |
| A4 | `rustls-pki-types`'s `std` feature is unified on in this tree, so its `pem` module is available | Standard Stack | Only matters on the Option 2 fallback; adds `rustls-pemfile` if wrong |
| A5 | A remote human is at least as trusted as a connected agent, so one credential system serves both | WebSocket / OAuth | If the user disagrees, viewers need their own class and the Access panel needs a second list |
| A6 | Success Criterion 1's "browse" means the client can open/navigate tabs, not that Me tabs become remotely viewable | Architectural Split | Changes the scope of Option C, the threat model, and the client's UI. **Must be locked before planning** |
| A7 | 120 s is the right grace period | Reconnect | Too short expires on a normal path change; too long pins a `show()`n webview. Tunable, so low risk |
| A8 | The synthetic benchmark frames are representative of real Servo output at 1280×800 | Latency Budget | Real pages sit between the two synthetic cases, so the range should bracket reality — but the numbers are from synthetic input, not from a Servo readback. Re-measure inside spike 1 |
| A9 | `tailscale status`'s per-peer `direct`/`relay` is stable enough to surface in the client's chrome | Latency Budget | Cosmetic if wrong; the EWMA still carries the degrade ladder |
| A10 | Restoring agent tabs into the Agents view under a dead `client_id` is acceptable | Session Manifest | It reuses a state `deferred-items.md` already accepted, so the risk is that the user considers *that* state a bug too |

---

## Open Questions

1. **Does "browse" in Success Criterion 1 include the human's own Me tabs?**
   - Known: DIST-02 names agent tabs only; SC 1 says "browse and drive agent tabs".
   - Unclear: whether a remote human gets Me tabs on the server, and if so what that does to the trust model that makes the human at the keyboard the trust root.
   - Recommendation: **lock before planning.** Default to agent tabs only; if Me tabs are in scope, that is a separate threat conversation and probably a separate phase.

2. **Does the first pairing require someone at the server machine?**
   - Known: OAuth consent is a server-chrome panel raised by `AppEvent::ConsentRequested`.
   - Unclear: whether that is acceptable UX or whether RFC 8628 device-code is wanted.
   - Recommendation: accept it; write it into the client's first-run copy. Lock before the UI-SPEC.

3. **Tailscale Serve or a widened bind?**
   - Known: Serve keeps `BIND_HOST` a constant and removes certificate renewal; it requires an advertised-base-URL concept and its WebSocket support needs confirming.
   - Recommendation: **Serve**, gated on spike 2. Document the widened-bind enum shape as the fallback.

4. **One phase or two?** — see § Scope below.

5. **What does the client's chrome actually contain?**
   - Known: it needs a tab list, a connection/quality indicator, and a Me/Agents context.
   - Unclear: everything else. This is a new front end and it has no UI-SPEC.
   - Recommendation: run `/gsd-ui-phase` for the client before its plan is written.

6. **Does `tabs_list` over the view channel leak other clients' tabs?**
   - Known: `queue_event` is owner-scoped, but `Command::TabsList` is not obviously so.
   - Unclear: whether a remote viewer should see every agent's tabs or only its own client's.
   - Recommendation: the human is the trust root, so a *human viewer* seeing all agent tabs is correct and is the product. But it must be a deliberate decision recorded in the threat register, not a side effect of reusing `TabsList`.

---

## Environment Availability

| Dependency | Required by | Available | Version | Fallback |
|------------|------------|-----------|---------|----------|
| Rust toolchain | everything | ✓ | 1.97.1 (README requires ≥1.88) | — |
| `cargo` + populated registry | dependency landing | ✓ | `axum-0.8.9`, `tungstenite-0.29.0` already cached in `~/.cargo/registry` | — |
| Network access to crates.io | fetching `tokio-tungstenite` | ✓ | verified — the resolution run downloaded it | — |
| Xvfb | e2e | ✓ | harness creates `:99` at 1280x800x24 | — |
| `xdotool` | e2e input | ✓ | used by 22/22 suites | — |
| Python 3 | e2e | ✓ | 3.14.6 | — |
| Tailscale on the server (ThinkPad) | DIST-01 | ✓ | tailnet IP `100.118.105.121` in active use | — |
| Tailscale on a second machine | DIST-01 SC 1 | ✓ | MacBook Air, used for all browser verification | — |
| **HTTPS Certificates enabled in the tailnet admin console** | Serve / `tailscale cert` | **✗ unverified** | — | **None. This is an operator action in the Tailscale admin console and it blocks any non-loopback exposure.** Confirm before planning |
| **MagicDNS enabled** | Serve / `tailscale cert` | **✗ unverified** | — | None — same |
| A second Xvfb display | two-shell e2e | ✓ | `TALARIA_E2E_DISPLAY` already parameterises it; `vault_nobus_test.py` already rewrites `XDG_RUNTIME_DIR` per child | — |
| `tc netem` / network namespaces | constrained-link testing | **✗ needs root** | — | **Python stdlib TCP shim** — see § Validation Architecture. Recommended regardless: it works unprivileged, in CI, and is committable |
| macOS/Windows build hosts | not this phase | n/a | — | Phase 6 |

**Missing with no fallback:** the two Tailscale tailnet settings. They are one-time admin-console actions, but nothing in this phase's exposure story works without them, and neither has been confirmed on this tailnet.

**Missing with fallback:** privileged network shaping — replaced by an unprivileged shim that is strictly better for CI.

---

## Validation Architecture

### Test Framework

| Property | Value |
|----------|-------|
| Framework (Rust) | Built-in `#[cfg(test)]` + `cargo test`. No test-framework crate; no `tempfile` — store tests build unique temp paths by hand (`permissions.rs`, `vault.rs`, `settings.rs`) |
| Framework (e2e) | Python 3 standard library only. No pytest, no third-party deps |
| Config file | None for Rust. `tests/e2e/run_all.py` is the runner; `tests/e2e/harness.py` the shared launcher |
| Quick run command | `cargo test -p talaria-shell` |
| Full suite command | `cargo build --release --locked && cargo clippy --all-targets -- -D warnings && cargo test && python3 tests/e2e/run_all.py` |
| Static gate | `cargo clippy --all-targets -- -D warnings` |
| Baseline entering this phase | e2e **22/22**, `cargo test` **296** |
| Target leaving this phase | e2e **26/26** (four new suites, below), `cargo test` well above 296 |

### Phase Requirements → Test Map

| Req ID | Behaviour | Test type | Automated command | File exists? |
|--------|-----------|-----------|-------------------|--------------|
| — | Tile diff finds exactly the changed tiles, including at frame edges and on a 1-pixel change | unit | `cargo test -p talaria-shell view::` | ❌ Wave 0 |
| — | `Compression::Fast`+`Sub` frame encode stays under a stated budget for a synthetic 1280×800 frame | unit (bench-shaped) | `cargo test -p talaria-shell view::` | ❌ Wave 0 |
| — | The wire envelope round-trips: every channel tag, every input kind, every frame-header field | unit | `cargo test -p talaria-protocol wire::` | ❌ Wave 0 |
| — | Malformed wire input is refused, not defaulted: bad tag, truncated header, out-of-range coordinates, non-monotonic `seq`, unknown key name | unit | `cargo test -p talaria-shell remote_input::` | ❌ Wave 0 |
| DIST-01 | The `/view` route exists, is absent when remote access is off, and 401s unauthenticated | e2e | `python3 tests/e2e/remote_view_test.py` | ❌ Wave 0 |
| DIST-01 | A WebSocket carrying an `Origin` header is refused (CSWSH), and one with a wrong `Host` is refused | e2e | same suite | ❌ Wave 0 |
| DIST-01 | An authorized client attaches to an agent tab and receives a keyframe | e2e | same suite | ❌ Wave 0 |
| DIST-02 | A remote click navigates the agent's tab — asserted on the tab's URL over the **control socket**, so the assertion does not depend on the frame path | e2e | same suite | ❌ Wave 0 |
| DIST-02 | A remote click lands at the coordinate aimed at (regression for the toolbar-offset pitfall): assert the *specific* link hit, not merely that navigation occurred | e2e | same suite | ❌ Wave 0 |
| DIST-02 | Remote input aimed at a **Me** tab is refused | e2e + unit | same suite + `cargo test` | ❌ Wave 0 |
| DIST-02 | Remote input cannot reach the chrome — a click at a `chrome_rects` toolbar coordinate opens no panel | e2e | same suite (uses `TALARIA_TEST_HOOKS=1`) | ❌ Wave 0 |
| DIST-02 | Passive cadence is ~200–500 ms and rises to ~30–60 ms on takeover; frame timestamps prove the transition | e2e | `python3 tests/e2e/remote_latency_test.py` | ❌ Wave 0 |
| DIST-02 | Under a 13 Mbit/s + 40 ms shim the client degrades down the ladder and **reports** rather than stalling | e2e | same suite, via the link shim | ❌ Wave 0 |
| DIST-03 | Killing the link mid-`evaluate` does not close tabs and does not cancel the evaluate | e2e | `python3 tests/e2e/remote_resync_test.py` | ❌ Wave 0 |
| DIST-03 | On reconnect the client resyncs the tab list and receives a keyframe | e2e | same suite | ❌ Wave 0 |
| DIST-03 | On reconnect an agent's tabs rebind to its new session, proven by a **crash notification on a pre-drop tab reaching the reconnected client** | e2e | same suite | ❌ Wave 0 |
| DIST-03 | Tabs are released after the grace period expires with no reconnect | e2e | same suite (with `TALARIA_REMOTE_GRACE_SECS` lowered) | ❌ Wave 0 |
| DIST-04 | The manifest survives a SIGKILL and the restart offers restore rather than auto-restoring | e2e | `python3 tests/e2e/session_manifest_test.py` | ❌ Wave 0 |
| DIST-04 | Restored agent tabs land in the Agents view under the recorded `client_id` with no live session | e2e | same suite | ❌ Wave 0 |
| DIST-04 | The manifest file is `0600` and no `.tmp` sibling survives a save | unit | `cargo test -p talaria-shell manifest::` | ❌ Wave 0 |
| DIST-04 | A malformed manifest degrades to "no offer", never a panic | unit | same | ❌ Wave 0 |
| — | Revoking a viewer closes its WebSocket, asserted **from the client end** | e2e | `tests/e2e/revocation_test.py` (extended) | ✅ extend |
| — | Everything Phase 4 asserts still holds with the view route mounted | e2e | `http_transport_test.py`, `oauth_flow_test.py` unmodified | ✅ |

### Sampling Rate

- **Per task commit:** `cargo test -p talaria-shell` + `cargo clippy --all-targets -- -D warnings`
- **Per wave merge:** `cargo build --release --locked`, then the affected e2e suite standalone
- **Phase gate:** `python3 tests/e2e/run_all.py` fully green (26/26) plus `cargo test` green, before `/gsd-verify-work`
- **Max feedback latency:** ~120 s (quick run)

### The two-machine problem, and what is actually expressible here

A genuinely two-machine test is **not** expressible in this harness, and pretending otherwise would produce a suite that passes while proving nothing about the tailnet. What *is* expressible, in Python stdlib, unprivileged, in CI:

1. **Two processes, one machine.** The client is a separate binary and a separate process. Run `talaria-shell` and `talaria-client` on the same Xvfb display (or two, via `TALARIA_E2E_DISPLAY` — the harness already parameterises it, and `vault_nobus_test.py` already rewrites `XDG_RUNTIME_DIR` for a child, so a second shell instance with its own control socket is an established pattern). This proves the entire protocol, input path, resync and manifest. It does **not** prove anything about the network.

2. **A stdlib link shim** — `tests/e2e/link_shim.py`, a `socketserver`/`selectors` TCP forwarder between the client and `127.0.0.1:8779` with three knobs: added one-way delay, a token-bucket rate cap, and a `kill()` that closes both halves. Roughly 80 lines. This is what makes DIST-02's degradation and DIST-03's reconnect *testable* rather than asserted, and it can reproduce this user's own measured relayed profile (13 Mbit/s, ~40 ms) as a committed regression. It needs no root, which `tc netem` and network namespaces both do.

3. **A real two-machine check, as a documented manual step, once per phase.** ThinkPad server, MacBook Air client, over the actual tailnet, with `tailscale status` recorded to say whether the path was `direct` or `relay`. Per the project's no-inline-code rule this must be an **executable script** (`scripts/two-machine-check.sh` or a Python equivalent) that prints its own results, not a README recipe. It belongs in `VERIFICATION.md` as a human-verify item, not in `run_all.py`.

**What must not happen:** a `run_all.py` suite that spins up a client on loopback, measures 3 ms, and reports SC 2 as met. Loopback hides the only variable that matters.

### Wave 0 gaps

- [ ] `tests/e2e/link_shim.py` — delay/rate/kill forwarder, stdlib only (blocks the DIST-02 and DIST-03 suites)
- [ ] `tests/e2e/remote_view_test.py` — DIST-01, DIST-02 correctness
- [ ] `tests/e2e/remote_latency_test.py` — DIST-02 cadence and degradation, via the shim
- [ ] `tests/e2e/remote_resync_test.py` — DIST-03
- [ ] `tests/e2e/session_manifest_test.py` — DIST-04
- [ ] `harness.py` — a second binary constant and a `start_client()` helper alongside `start_shell()`
- [ ] `scripts/two-machine-check.sh` — the real tailnet run, executable, not prose
- [ ] `tests/e2e/revocation_test.py` — a third assertion for the view WebSocket
- [ ] `tests/e2e/run_all.py` — register the four new suites

---

## Threat Model Inputs

This phase puts a **human input channel on a network port**. That is a different and in places worse surface than Phase 4's agent tool channel: an agent gets an allowlisted tool vocabulary, while a viewer gets raw clicks and keystrokes into a live, logged-in session. ASVS L1, blocking on high.

### Applicable ASVS categories

| ASVS category | Applies | Standard control |
|---------------|---------|-----------------|
| V1 Encoding & Sanitization | yes | Binary envelope with explicit lengths; every string field bounded; `gui.rs::sanitize_for_display` already exists for anything rendered |
| V2 Validation & Business Logic | yes | Coordinates clamped to the *target tab's* viewport; `seq` strictly increasing; unknown key names rejected, not defaulted |
| V3 Web Frontend Security | yes | `refuse_page_originated` (outermost layer) — the CSWSH defence, and the reason a browser client is impossible |
| V4 API & Web Service | yes | One WebSocket route on the existing router; no new listener; no new middleware chain |
| V6 Authentication | yes | Phase 4 OAuth 2.1 + PKCE S256; **no second credential system**; `Tailscale-User-*` headers are not a boundary |
| V7 Session Management | yes | Grace period, reconnect rebinding by **verified `client_id`**, revocation closes the socket |
| V8 Authorization | yes | Agent-tabs-only lookup; chrome unreachable from the input path |
| V9 Self-contained Tokens | yes | Opaque tokens, SHA-256 digests, `0600`, RFC 8707 audience binding — Phase 4, unchanged |
| V11 Cryptography | yes | TLS via `tailscaled` (Serve) or `rustls` + `aws_lc_rs`. **Never hand-rolled** |
| V12 Secure Communication | yes | HTTPS on every OAuth URL once non-loopback (OAuth 2.1 §1.5) |
| V13 Configuration | yes | Off by default; `BIND_HOST` a constant; any exposure widening as an enum with a certificate precondition, never a free-text address |
| V14 Data Protection | yes | Manifest `0600`, atomic write, no page content persisted |

### Threat register

| ID | Threat | STRIDE | Severity | Mitigation |
|----|--------|--------|----------|------------|
| T-05-01 | **Cross-Site WebSocket Hijacking** — a page in any browser on any machine opens `wss://` to the view route and drives the session | Elevation of Privilege | **high** | `refuse_page_originated` as the outermost layer refuses any `Origin`; a browser always sends one. Assert in `remote_view_test.py`. Record in `SECURITY.md` that the native-only client is a *consequence* of this |
| T-05-02 | **Unauthenticated network peer reaches the input channel** | Spoofing / EoP | **high** | Listener off by default; bearer token verified per message class; audience-bound; `Host` allowlist |
| T-05-03 | **On-path read/inject of input and frames over a non-loopback bind without TLS** | Tampering / Information Disclosure | **high** | TLS via Serve or rustls. **Blocks any non-loopback exposure** — the inherited Phase 4 obligation |
| T-05-04 | **Remote input reaches the browser chrome** — synthetic clicks at the credentials button, the bookmark star, or a downloads row's Open (which hands a file to the OS default application) | Elevation of Privilege | **high** | Input path reaches `webview.notify_input_event` and nothing else. Structural, plus an e2e assertion clicking a `chrome_rects` coordinate and asserting no panel opens. Same reasoning as `Command::ChromeRects`'s own refusal |
| T-05-05 | **Remote input reaches a Me tab** — the human's own logged-in pages, history and autofill surface | EoP / Info Disclosure | **high** | Tab lookup returns `None` for `TabOwner::Me`; unrepresentable rather than refused |
| T-05-06 | **Tab hijack by client-name collision** — reconnect rebinding keyed on the self-asserted `client` display string lets one client claim another's tabs | Spoofing | **high** | Rebind on the verified `client_id` from `agents.rs` only. `TabOwner::Agent` must carry it |
| T-05-07 | **A revoked viewer keeps receiving frames and sending input** — a WebSocket has no next request to fail | EoP | **high** | Register the socket with `Connections`; `terminate_client` closes it; assert closure from the client end (the 04-08 shape) |
| T-05-08 | **Spoofed `Tailscale-User-*` identity headers** from any local process connecting directly to `127.0.0.1:8779` | Spoofing | **high** | Never treat identity headers as the boundary. Token first; headers at most a second check |
| T-05-09 | **Advertised issuer taken from a request header** — a local page induces the browser to advertise an attacker-chosen authorization server | Spoofing / Tampering | **high** | One advertised base URL from configuration (or `tailscale status`), never from `Host`; single source feeding metadata and the Host allowlist |
| T-05-10 | **Frame channel leaks another agent's page content** to a viewer scoped to one client | Information Disclosure | medium | Decide deliberately (Open Question 6). If viewers see all agent tabs, that is a documented product decision, not a default |
| T-05-11 | **Malformed wire input crashes or wedges the server** — oversized frame, bad tag, absurd coordinates, unbounded strings | Denial of Service | medium | Explicit length caps; refusal not default; the MCP-05 posture, and a fuzz-shaped unit test |
| T-05-12 | **Frame pump starves the winit loop**, making the *local* browser unusable while a viewer is attached | Denial of Service | medium | Encode off-thread; adaptive rate; a hard cap on concurrent view leases; the readback spike must quantify the loop cost |
| T-05-13 | **Session manifest leaks browsing state** — URLs and titles of both Me and agent tabs in a file | Information Disclosure | medium | `0600` via `permissions::write_owner_only`; atomic `.tmp` + rename; unit-assert the mode and that no `.tmp` survives |
| T-05-14 | **Restore-on-restart reopens pages under a revoked client** without a human deciding | EoP | medium | Never auto-restore; the offer expires; restored tabs carry no live session |
| T-05-15 | **Input replay within a connection** — a captured click replayed to repeat a destructive action | Tampering | low | Monotonic `seq`; non-increasing dropped. TLS covers the cross-connection case |
| T-05-16 | **Certificate expiry silently degrades to a broken or downgraded connection** on the rustls branch (90-day Let's Encrypt, `tailscale cert` does not auto-renew files) | DoS | medium | Prefer Serve, which renews. On the rustls branch, a `ResolvesServerCert` that re-reads, plus a warning well before expiry |
| T-05-17 | **Tailscale Funnel is enabled instead of Serve**, exposing the browser to the public internet | EoP | **high** | Never invoke Funnel; refuse it explicitly in `SECURITY.md` so it is a named non-option rather than an unconsidered one |

### Trust-model deltas for `SECURITY.md`

`SECURITY.md` currently names four parties. Phase 5 adds a fifth and changes one:

- **New party — the remote human.** More trusted than an agent (they are the trust root, at a distance), less verifiable than the human at the keyboard (identity is a bearer token, not physical presence). This is the first party that can send raw input rather than a tool call.
- **Changed party — the unauthenticated network peer.** Previously "anything that can open a TCP connection to the loopback listener". Under any tailnet exposure it becomes "any device in the tailnet, plus whatever the tailnet's ACLs admit" — a set governed by a web console outside this repository.
- **Both `Known limitations` entries get worse in the same way `SECURITY.md` already notes for the HTTP transport.** An `evaluate` reaching the filesystem, and a wedged script costing a command timeout, are both now reachable by a party whose only qualification is holding a token — and now additionally by a party that can click.

---

## Scope — is four plans right?

**No. Four is too few, and this phase is a strong candidate for a split.**

Phase 4 needed **eight** plans, across six waves, for one concern (authentication) touching roughly five files, and the plan-checker added the eighth because a single plan was carrying "metadata, registration, a five-step validation chain, a consent panel, the code store, constant-time PKCE, refresh rotation and an anti-harassment state machine — several times any peer task". The roadmap's Phase 5 split has a comparable concentration:

- **05-01 "Multiplexed WebSocket protocol"** silently contains: a `talaria-protocol` restructure, the dependency-and-lockfile landing, a WebSocket route on the Phase 4 router with revocation tracking, **and the entire TLS/exposure story** — which alone reopens a Phase 4 structural decision, needs a spike, and has an operator prerequisite nobody has confirmed.
- **05-02 "Adaptive screenshot polling"** contains: a frame pump on the latency-critical path, a tile-diff codec, an off-thread encoder, a view-lease lifecycle, the adaptive rate controller, **and a whole new client binary with no UI-SPEC.**
- Nowhere in the four is the **input path**, which carries five of the register's high-severity threats.

The roadmap already says of Phases 4–7 that "any one of them could take longer than Phases 1–3 combined did." This is the one where that is most obviously true.

### Recommended shape

**Preferred — split into two phases.** ROADMAP already supports decimal insertions.

- **Phase 5: Remote view and takeover** — DIST-01, DIST-02. Exposure + TLS, the wire, the WebSocket route, the frame pump, the input path, the client binary. Ends when a human on the MacBook Air takes over a tab on the ThinkPad and it works.
- **Phase 5.1: Distributed resilience** — DIST-03, DIST-04. Grace period, reconnect rebinding, resync, manifest, restore offer. Ends when the link can be killed and restored without loss.

The seam is clean: 5.1 depends on 5 and nothing in 5 depends on 5.1. It also means the two Must-Haves that carry the demo ship before the two that harden it.

**Acceptable alternative — one phase, ~9–10 plans, ~6 waves:**

| # | Plan | Wave | Requirement |
|---|------|------|-------------|
| 05-00 | **Spike:** GL readback cost at 30 ms cadence under Xvfb and on real hardware (A1); WebSocket upgrade through `tailscale serve` (A2). Both before anything downstream is planned | 0 | — |
| 05-01 | `talaria-protocol` restructure: `wire` module, `local` module, envelope + frame header + input message, round-trip tests. No behaviour change | 1 | — |
| 05-02 | Dependency + lockfile landing: `axum` with `ws`, one added package, `--locked` CI green, `primeorder` verified | 1 | — |
| 05-03 | **Exposure + TLS:** advertised base URL as a single source feeding metadata and the Host allowlist; Tailscale Serve (or the exposure enum + rustls fallback); `oauth.rs`'s `http://` → `https://` | 2 | DIST-01 |
| 05-04 | The `/view` WebSocket route on the existing router; auth, `Connections` registration, revocation closes it; no frames yet | 2 | DIST-01 |
| 05-05 | Frame pump: view lease, unconditional paint, readback, off-thread tile diff + PNG, keyframes | 3 | DIST-02 |
| 05-06 | Input path: `forward_*` generalised, `keyboard_event_from_wire`, validation, agent-tabs-only, chrome unreachable | 3 | DIST-02 |
| 05-07 | `talaria-client` binary — present, input capture, tab list, connection state *(needs a UI-SPEC)* | 4 | DIST-01/02 |
| 05-08 | Adaptive rate controller + degrade ladder + honest reporting, plus the link shim and the latency suite | 5 | DIST-02 |
| 05-09 | Reconnect, grace period, rebinding by verified `client_id`, resync snapshot | 5 | DIST-03 |
| 05-10 | Session manifest store + restore offer on both surfaces | 6 | DIST-04 |

**Either way, two things should happen before planning starts:** run the 05-00 spike (Phase 4 set this precedent with 04-02's `rust-mcp-axum` spike, and both assumptions here are larger), and run `/gsd-ui-phase` for the client, which is a new front end and the only part of this phase a human looks at directly.

---

## Sources

### Primary (HIGH confidence — verified by tool in this session)

- `crates/talaria-protocol/src/lib.rs` — full read; the wire-claim assessment
- `crates/talaria-shell/src/app.rs` — `Shared` fields, `capture_now`, `process_pending_*`, `forward_mouse_*`, `forward_wheel`, `encode_screenshot`, `Command::Screenshot`, `AppEvent::SessionEnded`, `queue_event`, `process_pending_events`
- `crates/talaria-shell/src/http.rs` — module header, `BIND_HOST`, `build_router`, middleware ordering, `refuse_page_originated`, `Connections`, `StreamRegistry`
- `crates/talaria-shell/src/control.rs` — full read; `peer_uid_ok`, `next_session_id`, `command_timeout_secs`, per-request task model
- `crates/talaria-shell/src/settings.rs` — `LOOPBACK_BIND`, `DEFAULT_REMOTE_PORT`, `RemoteAccessConfig`, the three bind-refusal tests
- `crates/talaria-shell/src/gui.rs` — `render_to_parent_callback` into `LayerId::background()`; `UiAction`; `ChromePanel`
- `crates/talaria-shell/src/tabs.rs` — `TabOwner`, `Tab`, per-tab `OffscreenRenderingContext`
- `crates/talaria-shell/src/keyutils.rs`, `permissions.rs`, `agents.rs` (store shape)
- `SECURITY.md`, `.planning/PROJECT.md`, `.planning/ROADMAP.md`, `.planning/REQUIREMENTS.md`, `.planning/STATE.md`, `.planning/config.json`
- `.planning/phases/04-.../deferred-items.md`, `04-CONTEXT.md`, `04-VALIDATION.md`
- `Cargo.toml`, `crates/talaria-shell/Cargo.toml`, `Cargo.lock` (reverse-dependency analysis)
- `~/.cargo/registry/src/…/servo-0.4.0/webview.rs` + `webview_delegate.rs` — `show`/`hide`/`paint`/`notify_input_event`/`notify_input_event_handled`; **no damage rectangle on `notify_new_frame_ready`**; the "repaint without `notify_new_frame_ready`" note
- `~/.cargo/registry/src/…/axum-0.8.9/Cargo.toml` — the `ws` feature requires `tokio-tungstenite 0.29.0`, `sha1 0.10`, `base64 0.22.1`
- `~/.cargo/registry/src/…/rust-mcp-axum-1.0.1/Cargo.toml` — the `ssl` feature is `axum-server/tls-rustls` + `dep:rustls`
- `~/.cargo/registry/src/…/rustls-pki-types-1.15.1/src/pem.rs` — `PemObject`
- `~/.cargo/registry/src/…/image-0.25.10/src/codecs/jpeg/encoder.rs` — `JpegEncoder` exists
- **`cargo metadata` resolution run** against edited manifests, 2026-08-21 — one added package, `primeorder` unchanged, lockfile restored byte-identical
- **Encoding benchmark**, `/tmp/frmbench`, `--release`, 2026-08-21 — every number in § The Latency Budget
- crates.io API for `jpeg-encoder` — 7,282,570 downloads, created 2021-04-29, `github.com/vstroebel/jpeg-encoder`

### Secondary (MEDIUM confidence — official documentation)

- `draft-ietf-oauth-v2-1-13` §1.5 — the HTTPS MUST and the loopback *redirect URI* exception
- tailscale.com/kb/1153/enabling-https — `tailscale cert`, DNS-01, prerequisites, 90-day expiry, "you are responsible for renewing"
- tailscale.com/kb/1080/cli — `tailscale cert` flags
- tailscale.com/kb/1242/tailscale-serve — reverse proxy, TLS termination by the daemon, **only `http://127.0.0.1` as a proxy target**, Serve vs Funnel
- tailscale.com/kb/1312/serve — identity headers on Serve traffic, HTTPS-certificates prerequisite
- tailscale.com/docs/reference/connection-types — direct vs relayed

### Tertiary (LOW confidence — community, flagged for validation)

- WebSocket/CORS: SOP and CORS do not apply to WebSocket; origin enforcement is entirely server-side (websocket.org guides, corroborated across several sources). Consistent with RFC 6455 §10.2 and with what `http.rs` already implements — treated as MEDIUM in practice
- Reported WebSocket-upgrade regressions behind Tailscale Serve HTTPS proxying (github.com/openclaw/openclaw #54008 and community reports). **This is exactly why spike 2 exists**
- DERP throughput/latency figures (~35 Mbit/s, +20–50 ms; ~5% of connections relayed) from third-party benchmarks. Corroborated by this user's own measured 13 Mbit/s relayed vs 342–447 Mbit/s direct, which is the number the recommendations are built on

---

## Metadata

**Confidence breakdown:**

| Area | Level | Reason |
|------|-------|--------|
| Codec choice and latency budget | HIGH | Measured this session, `--release`, with the numbers in the document. The one gap is the GL readback, which is named as a required spike |
| Lockfile cost | HIGH | Resolved this session with `cargo metadata`, diff inspected, lockfile restored byte-identical |
| Existing code facts | HIGH | Every claim about `app.rs`/`http.rs`/`control.rs`/`settings.rs`/`tabs.rs`/`gui.rs`/`oauth.rs` was read, not recalled |
| Servo API constraints | HIGH | Read from the vendored `servo-0.4.0` source in the local registry |
| Architectural split | HIGH | Grounded in the compositing and `Rc`/`thread_local` facts above, not in preference |
| OAuth 2.1 TLS requirement | MEDIUM-HIGH | Quoted from the current draft; the loopback exception is narrower than the Phase 4 shorthand and that difference is called out |
| Tailscale operational behaviour | MEDIUM | Official docs, not exercised on this tailnet. The two prerequisites are unconfirmed and the WebSocket-through-Serve question is an open spike |
| DERP performance | MEDIUM-HIGH | Third-party benchmarks agreeing with this user's own measured numbers on this exact tailnet |
| Threat register | MEDIUM-HIGH | Derived from `SECURITY.md`'s existing model plus the code; not reviewed by anyone who breaks these for a living, which `SECURITY.md` already says of Phase 4's work |
| Scope recommendation | MEDIUM | A judgement, argued from Phase 4's measured 8-plan outcome and the roadmap's own warning |

**Research date:** 2026-08-21
**Valid until:** 2026-09-20 for the code and lockfile facts (30 days, stable). **7 days** for the Tailscale claims — Serve/Funnel and certificate behaviour have changed repeatedly, and the WebSocket-upgrade reports are recent.

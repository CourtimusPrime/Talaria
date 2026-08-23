# Deferred items — Phase 05

Out-of-scope discoveries and decisions logged during the phase. Not fixed here.

Opened during plan 05-01, because 05-01 is the plan whose dependency landing and
Serve spike make these concrete: the spike is what turns the inherited TLS-route
note into a demonstrable error, and `05-CONTEXT.md`'s six locked decisions are
what surface the product questions this phase carries but does not answer. Later
plans in this phase append to this file.

## Correction: `AxumServerOptions` is not Talaria's TLS route

**`04-.../deferred-items.md` is wrong on this point, and the error is corrected
by name rather than quietly superseded**, because a later reader following that
note would go looking for a configuration surface that is not on this program's
mount path at all.

The Phase 4 entry, "TLS and any non-loopback bind are Phase 5's, with a named
obligation", says Phase 5 needs "`rust-mcp-axum`'s `ssl` feature (its
`AxumServerOptions` carries `enable_ssl`, `ssl_cert_path` and `ssl_key_path`,
and `validate()` rejects `enable_ssl` without both paths)".

**Why that is not available here.** `crates/talaria-shell/src/http.rs` does not
call `create_axum_server`. It takes the bring-your-own-server path: it builds its
own `tokio::net::TcpListener`, assembles the router itself, and calls
`rust_mcp_axum::axum::serve` directly. `AxumServerOptions` is therefore never
constructed in this program, and neither is any field on it. The fields are real
and they do what the Phase 4 note says — they are simply on a code path Talaria
does not take. [Observed 2026-08-21 reading `http.rs:1044-1170`;
`05-RESEARCH.md` § State of the Art records the same.]

**The two real options**, so this correction leaves a reader with somewhere to
go rather than only a deletion:

1. **Tailscale Serve** — D-05-03, and the decision this phase is built on.
   `tailscaled` terminates TLS with a Tailscale-managed certificate and proxies
   to `http://127.0.0.1:PORT`, which is the only proxy target Serve supports and
   exactly what Talaria already binds. **No certificate handling in this
   repository at all**, and no renewal in a long-running process. `05-01-SPIKE.md`
   confirms the WebSocket upgrade survives it.
2. **`axum-server`'s rustls acceptor** — `05-RESEARCH.md`'s documented Option 2,
   for anyone not using Serve. `axum-server 0.8.0`, `tokio-rustls 0.26.4` and
   `rustls 0.23.43` are already resolved in `Cargo.lock`, the crypto provider is
   already installed in `main.rs`, and `rustls-pki-types 1.15.1` has a `pem`
   module, so even PEM parsing costs nothing new. It owns certificate renewal,
   which is the cost Option 1 exists to avoid — a server that built its
   `ServerConfig` once at startup serves an expired certificate on day 91.

**What closing it takes:** nothing, in this phase — this entry *is* the closure
of the inherited error. What a *later* phase must not do is reach for
`AxumServerOptions.enable_ssl` on the strength of the Phase 4 note. If Talaria
ever moves onto `create_axum_server`, that changes, and this entry should be
revisited rather than assumed still true.

Cited: `05-RESEARCH.md` § Exposure and the Certificate Story; `05-CONTEXT.md`
D-05-03.

## Open: does first pairing require someone at the server machine?

**Recorded as open. This phase's plans are written against option 1 below, and
that is a developer decision that has not been put to the developer.**

**The facts.** OAuth consent is raised as a panel in the *server's own* chrome:
`ChromePanel::Consent`, raised by `AppEvent::ConsentRequested`. In distributed
mode the human is at the *client*. So as things stand, approving the first remote
client requires local access to the server machine — someone at the ThinkPad.

Three ways out, with their costs, from `05-RESEARCH.md` § "Should a human client
reuse Phase 4's OAuth?":

1. **Accept it.** First pairing requires local access to the server. **Zero new
   code**, and arguably the correct property rather than a limitation — it is the
   same shape as "you have to be at the machine to unlock it the first time".
2. **RFC 8628, the Device Authorization Grant.** The client displays a user code;
   the human enters it in the server's chrome. Standard, well-specified, better
   ergonomics — and it is *still* local access to the server, just less awkward.
3. **A bespoke pairing code** the server displays and the client types. This one
   rejects itself on the project's own do-not-hand-roll-auth rule.

**This phase is written against (1)**, because it is the zero-code option and
because the other two are strictly additive — nothing built here forecloses them.
`05-07`'s first-run copy says so plainly rather than leaving the user to discover
it.

**Adopting (2) is a new plan, not an adjustment.** It changes the client's
first-run surface, which means it changes the UI contract, which means it is
planned rather than patched in.

**Condition for revisiting:** the first time a developer wants to pair a second
machine without walking to the first.

**Related, and the same question wearing a second hat** (`05-CONTEXT.md` records
them together deliberately): a remote human at a login wall has no address bar.
The view channel carries no `Command` — D-05-02 read literally, so a viewer
clicks, scrolls and types into the page and cannot open, navigate, evaluate,
close or download — and remote input correctly cannot reach the chrome. Page
input clears a CAPTCHA or a login form, which is the common case. It does not
clear a flow that needs a different URL. Both questions are really *"what can a
remote human do that requires being physically at the server machine?"*, and
answering them together will be cheaper than answering them apart.

## A lossy second codec is cleared but not adopted

**D-05-05 chose `png` at `Compression::Fast` with a `Sub` filter over 64×64 tile
diffs**, measured at **0.007 ms / 3.9 KB** for a caret-sized change, **0.034 ms /
15 KB** for typing and **2.16 ms / 522 KB** for a full keyframe, with a 0.14 ms
whole-frame dirty-tile scan. That is microseconds for interactive deltas against
a 30 ms budget, it is lossless, and it needs **no new codec dependency at all**.

`jpeg-encoder` was **cleared on provenance and deliberately not adopted**:
7,282,570 total downloads, created 2021-04-29, `github.com/vstroebel/jpeg-encoder`
[VERIFIED: crates.io API, 2026-08-21]. It measured **6.2 ms / 203 KB** for a full
frame against PNG-Fast's **2.16 ms / 522 KB** — better on the wire, worse on CPU,
and **lossy on antialiased text**. Chroma subsampling on body text is the one
artefact a browser viewer cannot afford, and it is the artefact a still-image
benchmark does not show you.

**Condition for revisiting:** measurement on a real link showing scroll-heavy
content failing where **the wire, not the CPU, is the constraint**. A relayed
DERP path at ~13 Mbit/s is the case that would produce it (D-05-06). A negotiated
second codec is then a later plan with the supply-chain audit already done — which
is the point of recording this rather than dropping it.

## Phase 5.1 inherits DIST-03 and DIST-04

**Per D-05-04.** Phase 5 delivers DIST-01 and DIST-02 — the client/server split,
the wire, the frame pipeline and the latency target. Named here so this phase's
readers do not go looking for the rest of it in this directory.

**What 5.1 owns:** the reconnect grace period; tab-ownership rebinding; the
resync snapshot; the session manifest store; and the restore-on-restart offer.

**Two design facts this phase's research established that 5.1 will need**, so they
are handed over rather than rediscovered:

1. **`TabOwner::Agent` carries a session id and a display string, but not the
   verified `client_id`.** Rebinding must key on the **verified `client_id`** and
   never on the self-asserted display string. Over a local Unix socket the display
   string was safe only because `SO_PEERCRED` had already established that the
   peer runs as you; over a network there is no such precondition, and rebinding
   on a self-asserted name is a tab-hijack primitive — a second client claiming
   the first one's name inherits its tabs.
2. **`process_pending_events` discards an event addressed to a session that no
   longer exists.** That is correct today, because a session that ended is gone
   for good. It becomes wrong the moment reconnect exists, because a dropped link
   is exactly a session that will come back. 5.1 changes this; Phase 5 does not,
   and should not.

## The MCP revision `2026-07-28` migration reaches Phase 5.1's shape

**Cross-reference, not a new obligation.** `04-.../deferred-items.md` already
tracks the migration to MCP specification revision `2026-07-28`, which
`rust-mcp-sdk 1.0.1` does not implement.

The Phase 5 consequence: **the newer revision removes sessions from the protocol
layer** — the `initialize` handshake and the session-id header go away and the
transport becomes sessionless. The grace-period and rebinding design 5.1 will be
written against is a **session-model** design, keyed on the session ids
`control.rs`'s `next_session_id()` mints and `http.rs` shares. When the SDK
migrates, 5.1's shape changes with it, and the change is structural rather than a
constant bump.

**What closing it takes:** unchanged from the Phase 4 entry — waiting for the SDK
to land `2026-07-28`, then revisiting the session model rather than the version
constant. This entry exists so that whoever does it knows 5.1's reconnect design
is downstream of the same decision.

## Two concurrent e2e runs on one X display SIGKILL each other

**Found during plan 05-01, and it cost a red CI run.**

`harness.start_xvfb()` calls `pkill` for stale Xvfb on its display, and
`kill_shells_on_display` reaps shells there. Neither takes a lock. So when the
self-hosted Actions runner and a local run use the same display, whichever
starts second kills the other's shell, and the first fails with a
`BrokenPipeError` on the control socket — a failure that looks like a product
bug and is not one.

That is exactly what happened: CI's `e2e.yml` pins `TALARIA_E2E_DISPLAY: ":98"`,
and local execution briefs had been reusing `:98` because that is the value the
workflow file shows. CI went red on `takeover_test` alone with 21 other suites
passing, which is the signature.

**Immediate handling:** local runs on this machine use a display other than
`:98`. `:98` belongs to CI.

**Not fixed here, and worth fixing.** Two candidates:
- An advisory lock file per display in `harness.start_xvfb`, so the second run
  waits or fails loudly with a legible message instead of silently killing the
  first.
- Have the harness pick a free display itself when `TALARIA_E2E_DISPLAY` is
  unset, rather than defaulting to a fixed `:99`.

The first is the smaller change and turns a confusing failure into an obvious
one. Neither is Phase 5 business; both belong wherever the harness is next
touched.

## Correction: `encode_screenshot` was never running at `Compression::Default`

**Found during plan 05-02, measured rather than reasoned.** `05-RESEARCH.md`
§ "Encode, measured this session" and `05-CONTEXT.md` D-05-05 both rest on the
sentence *"`encode_screenshot` sets colour and depth and nothing else, so it
runs at `png::Compression::Default` by omission"*, and on the **20.98 ms** page
/ **175.40 ms** photo figures that follow from it.

**That premise is false for `png 0.17.16`**, which is what this tree resolves.
`png::Info::default()` sets `compression: Compression::Fast` — the comment on
the line says *"Default to `deflate::Compression::Fast` and
`filter::FilterType::Sub` to maintain backward compatible output"*
(`png-0.17.16/src/common.rs:636-638`) — and `Encoder::set_filter`'s doc says
*"The default filter is `FilterType::Sub`"* (`encoder.rs:321`).
`png::Compression::Default` is a value you have to ask for.

So the shipped screenshot encoder has been running at **`Compression::Fast` +
`FilterType::Sub`** all along, which is exactly the configuration D-05-05 chose
for the *frame* path. `05-02-SPIKE.md` encoded the same real Servo frame both
ways and got **byte-identical** output (465,662 B, 1.685 ms vs 1.847 ms), while
an explicitly-set `Compression::Default` on the same buffer produced 183,030 B
in **21.275 ms** — within 1.4 % of the research's 20.98 ms. The benchmark was
run correctly; it measured a configuration this codebase never executes.

**What it changes:**

- **Nothing about D-05-05's choice**, which stands and is cheaper than it
  looked. PNG at `Fast`+`Sub` over 64×64 tile diffs, no new codec crate.
- **The MCP `screenshot` tool is not slow and never was** — 1.7 ms for a text
  page, 5.5 ms for a photographic one. `05-RESEARCH.md`'s Pitfall 1 ("reusing
  the screenshot encoder for frames … a real page blows the budget") is not a
  live hazard in this tree. Its conclusion survives; its reason does not.
- **The frame encoder's divergence is one line, not three.** Raw bytes instead
  of base64, plus a tile rectangle instead of the whole surface. 05-08 should
  write it as its own function anyway — two encoders for two jobs — but must not
  repeat the 21 ms / 175 ms justification in a comment, because that would embed
  a false claim in the tree.

**What closing it takes:** nothing in this phase — this entry *is* the closure.
What a later reader must not do is quote `05-RESEARCH.md`'s first encode row as
a fact about Talaria. If `png` is ever upgraded past 0.17, re-check
`Info::default()` before assuming this still holds.

Cited: `05-02-SPIKE.md` § "The finding that changes 05-08"; `05-CONTEXT.md`
D-05-05.

## Manual item for `VERIFICATION.md`: no hardware GL number exists on this machine

**Found during plan 05-02.** Every X display on the build machine is an Xvfb on
llvmpipe: `:20` and `:21` (Sunshine), `:95` (this spike), `:98` (CI). Reaching
either real GPU — the Intel iGPU or the NVIDIA T1200, both with readable
`/dev/dri` nodes — would need a Wayland compositor (none installed) or an Xorg
holding DRM master, and `/etc/X11/Xwrapper.config` restricts that to
`allowed_users=console`.

So the readback figure in `05-02-SPIKE.md` is software-rendered, stated as such,
and **must not be presented as evidence about SC 2 on real hardware** — which is
T-05-12-A's mitigation working as intended. The asymmetry is the point: under
software rendering the framebuffer is already in system memory, so the readback
is close to a `memcpy` and `paint()` carries the cost; on a GPU that inverts.
The number that could still break the capture model is a hardware
`read_to_image` *slower* than the 1.19–1.56 ms measured here.

**What closing it takes:** the two-machine manual check `05-CONTEXT.md` already
anticipates, run on a machine with a compositor, carried into `VERIFICATION.md`
as SC 2's evidence.

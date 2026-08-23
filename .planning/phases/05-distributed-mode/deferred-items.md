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

## The advertised identity has no automatic discovery

**Found during plan 05-04.** The browser is *told* its advertised origin, by a
`remote_access.advertised_url` key in `config.json` that a human writes.
`scripts/tailscale-serve.sh up` prints the exact key its mapping implies, which
is the one moment both facts are on screen together — but nothing checks that
what was printed is what was written, and nothing notices if the mapping later
moves.

Reading the origin out of the daemon's own status would close that gap, and it
is **deliberately not done here**. It would make a security-critical string —
the RFC 8707 canonical resource identifier, the RFC 8414 issuer, and the host
allowlist, all of them — depend on a live subprocess whose output format is not
this project's to keep stable. A daemon upgrade that renamed a JSON key would
silently change what this browser publishes about itself, and the failure mode
of a *wrong* advertised identity is tokens that validate against one spelling
and mysteriously never work against the other.

**Condition for revisiting:** an operator surface that made the two disagree
often enough to matter — for instance a Serve mapping that moves port across
daemon restarts, or a second fronting arrangement where the origin is not
something a human chose once. Until then the configuration key is the seam, and
its single producer is the whole of threat T-05-09's mitigation.

Cited: `05-CONTEXT.md` D-05-03; `crates/talaria-shell/src/settings.rs`
`RemoteAccessConfig::advertised_url`; `05-04-PLAN.md` T-05-09.

## Transport-identity lifecycle is the daemon's, permanently — T-05-16

**Found during plan 05-04.** D-05-03 puts the Tailscale daemon in front of an
unchanged loopback listener, so the daemon provisions and renews the identity
that fronts it and this browser holds none. `grep -rqi
'certificate\|\.pem\|cert_path\|ssl_cert\|private key'` across
`crates/talaria-shell/src/` returns nothing, and that is asserted rather than
assumed.

**What that buys.** No renewal timer, no expiry check, no resolver that
re-reads files, and therefore no day-ninety-one failure. This is the specific
hazard the alternative carries: a long-running process that built its TLS
configuration once at startup would happily serve an expired identity forever,
and a browser is exactly the kind of process that stays up for months. Threat
T-05-16 is *transferred*, not mitigated — which is an honest description of
owning none of the problem.

**What it costs.** This browser cannot be exposed over a secure transport
without a Tailscale node. Anyone wanting a different fronting proxy — nginx, a
cloud load balancer, Caddy — can still put one in front of the same loopback
listener and set `advertised_url` to whatever that proxy publishes; the
advertised-identity seam is proxy-agnostic and nothing in it names Tailscale.
What they cannot do is have Talaria terminate the transport itself. That is
`05-RESEARCH.md`'s Option 2 — an exposure enum with an identity precondition
plus in-process rustls — and it is a different plan with a different threat
model, because it inherits the renewal problem this one declined.

**Condition for revisiting:** a deployment that needs Talaria to bind a
non-loopback address directly. Note that this would also reopen D-04-04: the
bind host is a module constant precisely so a wider bind is not a value this
program can carry, and Option 2 cannot be built without making it one.

Cited: `05-RESEARCH.md` § "Exposure and the Certificate Story", Option 2;
`05-CONTEXT.md` D-05-03; `04-.../deferred-items.md`'s inherited obligation.

---

## The client has no design contract — T-05-07-UI

**What was deferred.** `crates/talaria-client` ships three user-facing surfaces —
the connection state, the agent tab list with its empty state, and the first-run
pairing block — and no `UI-SPEC.md` was written for any of them.

**Why.** The interface gate did not fire for Phase 5. It looks for changes to the
existing chrome, and this client is a *new binary*: nothing in
`crates/talaria-shell/src/gui.rs` changed, so nothing tripped it. That is a gap
in the gate's reach rather than a judgement that the surfaces did not need one.

**What was done instead.** The copy follows `04-UI-SPEC.md`'s register verbatim —
plain sentences, the fact first and the next step second, no exclamation, no
reassurance, no jargon — and the whole of it is written as a table in
`crates/talaria-client/src/chrome.rs`'s own module doc comment, so it can be read
and reviewed *as copy* the way a contract's copy tables are. Every page-supplied
value goes through the same truncate-sanitise-quote treatment the server's own
lists give an untrusted claim, and every control's geometry is recorded through
one helper and emitted on the client's standard output under
`TALARIA_TEST_HOOKS=1`, so a suite can assert against a real control by name
rather than a hardcoded coordinate.

**Condition for revisiting.** A client that grows past three surfaces should get
its own contract **before** it grows a fourth. `05-09` adds a frame surface and
`05-10` adds a link-quality one; the second of those is the fourth surface, so
this is due at `05-10` rather than at some vague later date.

**Related:** the pairing constraint this client's first-run copy states is the
open product question recorded above under *"Open: does first pairing require
someone at the server machine?"*. A reader who found the copy and wants the
alternatives should read that entry.

---

## Manual item for `VERIFICATION.md`: remote keyboard input to a tab the local human is not looking at

**Raised by:** 05-08, Task 1.

An attachment holds its tab **shown and deliberately not focused** — `visibility_of` returns
`HeldForViewing`, and `sync_visibility` calls `show()` then `blur()`. That is the correct answer to
`T-05-04-E`: focus belongs to the tab the local human is driving, and a remote party that could take
it could redirect what somebody sitting at the machine is typing into.

Servo's *hit test* needs a shown webview, which is what 05-08 supplies and what
`tests/e2e/remote_view_test.py` now proves for a background tab. Servo's **keyboard focus** is a
different thing, and the shipped end-to-end typing assertions all drive a tab that is displayed —
therefore shown *and* focused — because they were written before the hold existed and were left
unchanged as 05-06's own regression evidence.

**So: whether a remote keystroke reaches a held-but-not-focused background tab is untested, not
known-broken.** It cannot be resolved by weakening the hold; if it turns out that keyboard delivery
needs per-webview focus, the answer is a Servo-side question about what `focus()` scopes to, not a
licence for an attachment to steal the local window's focus.

**What to measure:** attach to a background agent tab with the local human in the Me view, send a
`key` message naming a character, and read the field's value back over the control socket. If it
arrives, add the assertion to `remote_view_test.py`'s frame section. If it does not, this becomes a
named limitation of remote takeover rather than a bug to be papered over.

## Correction: two of 05-08's acceptance criteria were written against a false baseline

**Raised by:** 05-08. Recorded here because a later reader running the criteria verbatim will see
them fail and should know they were checked rather than skipped.

- `grep -c 'expect("' crates/talaria-shell/src/view.rs` **is 0** — it was already **16** before the
  plan ran, every occurrence inside `#[cfg(test)]`. The criterion's stated purpose is "the encoder
  degrades rather than aborting", which is a statement about shipped paths. Verified as: zero
  occurrences outside `#[cfg(test)]`. The total is now 28, all in tests.
- `grep -ci 'latency\|cadence\|ms' tests/e2e/remote_view_test.py | head -1` **is 0** — it was
  already **4**, because `ms` matches inside `**params` (three lines) and inside the word "claims"
  (one line). `grep -c` never emits more than one line, so the trailing `head -1` had no effect
  either. The count is now 3 and every match was inspected: none is a timing claim, and the frame
  section makes none of any kind.

**The general lesson for later plans:** a source criterion of the form "`grep -c X` is 0" should be
established against the pre-task baseline when it is written, and scoped to shipped code when that
is what it means. Both of these were sound in intent and unachievable as literals.

---

## Resolved by 05-09: remote keyboard input **does** reach a held-but-not-focused background tab

**Raised by:** 05-08 (above). **Answered by:** 05-09, Task 3.

The measurement the entry above asks for was made, with the real client binary rather than a
synthetic wire message: the local human is in the Me view looking at their own tab, the client is
attached to a background agent tab, and real keystrokes at the **client's** window are typed with
`xdotool type`. The field's value read back over the control socket is `hey`.

**So a held tab is typeable into while shown and blurred, and no hold had to be weakened to get
there.** `tests/e2e/remote_view_test.py`'s real-client section carries the standing assertion
(`CLIENT TYPED`). The entry above can be struck from `VERIFICATION.md`'s manual list.

What is *not* answered, and stays a manual item: whether this holds when the local human is
actively typing into a different tab at the same moment. The suite types into one window at a time,
because one X display has one keyboard focus.

---

## Bug (shell): `screenshot` on a tab a viewer is watching silently stops that viewer's clicks landing

**Raised by:** 05-09, Task 3, by a test that was written to pass and did not.

**What happens.** `Shared::capture_now` takes a `hide_after` flag, and `process_pending_captures`
passes `true` for a background tab — the tab was shown only to produce a frame, so it is hidden
again afterwards (`crates/talaria-shell/src/app.rs`, around the `webview.hide()` in `capture_now`
and the `webview.show()` that queues the capture). **That path does not consult
`Tab::held_for_view`.** So an agent's tab that a remote viewer is attached to — held shown by
05-08 precisely so Servo will answer a hit test for it — is hidden the moment anybody takes a
screenshot of it over the control socket.

**Why it is nasty rather than merely wrong.** Every visible symptom points somewhere else:

- The hold count still reads 1, so the lease looks intact.
- Frames keep arriving and their sequence keeps advancing, because `view::capture` paints and reads
  the tab's *own* offscreen context and never asks whether the webview is shown. The viewer's
  picture is live and correct throughout.
- Only input stops working, and it stops **silently**: the shell logs
  `Empty hit test result for input event, ignoring`, the wire carries no refusal, and the frame
  header's `last_applied_input` simply stops advancing.

Reproduced directly: attach a client to a background agent tab, click a link through it (navigates),
`rpc("screenshot", tab_id=...)`, click the same link again — nothing happens, and typing into the
page's field leaves it empty, while `reading.frame_seq` climbs past 30.

**Not fixed here.** 05-09 is a client plan; its acceptance criteria require
`git status --porcelain crates/talaria-shell/src/` to be empty, and this is server source. The
end-to-end suite routes around it — `tab_viewport` reads the tab's size out of the page with
`evaluate` rather than out of a screenshot, and says so at the function — so the plan's own
assertions are unaffected.

**The fix, when somebody takes it.** `hide_after` should be `tabs.view_holds(tab_id) == 0` rather
than "this tab was not displayed", or equivalently the re-hide should go through
`sync_visibility`/`visibility_of`, which already knows about holds and is the one place that
decision is supposed to live. A regression test belongs next to it: screenshot a held tab, then
assert a remote click still navigates.

**Related:** the same class of bug 05-06 hit and 05-08 closed — a hidden webview answers no hit
test. This is that bug returning through a second door.

---

*Closing entries, appended by 05-11. Everything above was opened while a plan was
running; everything below is what the phase leaves behind on purpose.*

## Still open at the phase close: the first-pairing question, restated

**Read this one first.** It is the item a developer picking Phase 5 up will hit
soonest, and it is the only open item that is a *product* decision rather than an
engineering one.

Phase 5 shipped against **accepting** it: authorising the first remote client
requires someone at the server machine, because OAuth consent is a panel in the
server's own chrome. That is stated fully in "Open: does first pairing require
someone at the server machine?" above, with the three ways out and their costs.
`talaria-client`'s first-run copy says so plainly rather than leaving the user to
discover it at the worst moment.

**Nothing built in this phase forecloses either alternative** — RFC 8628's Device
Authorization Grant and a bespoke pairing code are both strictly additive — and
**adopting either is a new plan, not an adjustment**, because both change the
client's first-run surface and therefore its UI contract.

**The second hat.** `05-CONTEXT.md` records this and the "a remote human at a
login wall has no address bar" question as **one question**: *what can a remote
human do that requires being physically at the server machine?* Today the answer
is: approve the first client, and reach any URL the page itself will not take you
to. Answering both together will be cheaper than answering them apart.

**Condition for revisiting:** the first time somebody wants to pair a second
machine without walking to the first, or the first time a remote takeover stalls
because the flow needed a different URL rather than a different click.

## A viewer sees every agent's tabs, not only its own client's

**A decision, not a default, and not an oversight.** `view::agent_snapshot` lists
every tab owned by any agent, and any authorised viewer may attach to any of
them. It is not scoped to the tabs opened by the agent that shares the viewer's
client identity.

**Why.** The human is the trust root, and a remote human is the trust root at a
distance. Talaria's whole premise is that a person can take over *whatever* an
agent got stuck on; a viewer that could only see the work of one particular agent
would be a viewer that could not clear the login wall the other agent hit. The
asymmetry that matters is the one that *is* enforced: no agent's tab list ever
includes a human-owned tab, and no viewer can name one.

**Condition for revisiting:** a deployment where agents belong to parties that
should not see each other's work. That is a different product from this one — it
implies per-agent scoping, which `PROJECT.md` lists as deliberately out of scope —
so the revisit is a product decision before it is a code one.

## Reconnect, resync and the session manifest are Phase 5.1's

Cross-referenced here so the closing register points forward rather than leaving
DIST-03 and DIST-04 findable only inside a plan nobody re-reads. The full entry is
"Phase 5.1 inherits DIST-03 and DIST-04" above.

The two design facts this phase established that 5.1 will need:

1. **The frame sequence is per attachment, not per connection**, and the
   previous-frame buffer is released on detach, on disconnect and on the tab
   closing. A reconnect therefore starts from no history at all, which makes
   "send a keyframe" the correct and only resync — but it also means a resync
   *cannot* be a delta against what the client still has on screen without a new
   agreement about what the server remembers.
2. **`tests/e2e/link_shim.py` already has a kill knob**, unused today and built
   for exactly this: 5.1 can sever a live link mid-frame without privileges and
   without a second machine.

## What the automated suite does not — and cannot — prove

`05-VALIDATION.md` lists two manual-only verifications. Phase 3 and Phase 4 closed
their visual items by rendering under the virtual display; the first of these two
**cannot** be closed that way, and that is the entry a later reader should follow
when they wonder whether the latency claim was ever confirmed on real hardware.

**Row 1 — a real two-machine run.** Everything automated in this phase runs both
ends on **one host, under Xvfb, on software rendering**. That proves the protocol,
the authorisation, the ordering rules, the ladder walk and the degrade-and-report
behaviour. It proves **nothing** about the network, because loopback hides
transmission — which is the only variable Success Criterion 2 is about. TLS
termination by the overlay daemon, the tailnet `Host` a Serve-proxied request
carries, and real link behaviour are all untested by `tests/e2e/run_all.py`.
`tests/e2e/remote_latency_test.py` deliberately makes no claim that SC 2's target
is met, and says so in its own docstring.

**The step that closes it:** `scripts/two-machine-check.sh`. It checks the
prerequisites it can, **observes and prints the path type** (direct or relayed)
rather than assuming it, prints the numbered steps a human performs, and asks for
the client's reported **input-to-photon estimate in milliseconds** as the evidence
and the "did takeover feel immediate" impression as the product claim — separately,
because they are not the same kind of thing. It exits non-zero when a prerequisite
is missing, so a run that could not have proved anything does not read as a pass.
It deliberately does not drive the second machine: a script that automated a human
judgement would produce a green result for a question only a human can answer.

**Row 2 — remote keyboard input to a tab the local human is not looking at** was
**resolved** by 05-09 and is recorded above; it is listed here only so the pair is
accounted for.

**Condition for revisiting:** whenever SC 2's ~30–60 ms target is cited as
confirmed. Until somebody runs the script on a direct path and writes the
millisecond figure down, that target is *designed for and not contradicted*,
rather than measured across two machines.

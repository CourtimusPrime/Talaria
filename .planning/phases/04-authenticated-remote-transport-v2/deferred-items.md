# Deferred items — Phase 04

Out-of-scope discoveries and decisions logged during the phase. Not fixed here.

Opened during plan 04-02, because 04-02 is the plan whose dependency choices
make all three obligations concrete: the SDK version this phase pins is what
fixes the protocol revision, and the `auth` feature is what makes the
registration-versus-CIMD question a real code path rather than a preference.
Later plans in this phase append to this file.

## Migration to MCP specification revision `2026-07-28`

**Decided in 04-CONTEXT.md D-04-01, recorded here so it is tracked work rather
than something a later reader rediscovers.**

Phase 4 targets revision `2025-11-25`, because that is what `rust-mcp-sdk 1.0.1`
implements. `1.0.1` is the newest published release (2026-07-26, re-verified
against the crates.io API during 04-02) and it shipped two days before
`2026-07-28` went final. Chasing the current revision would mean implementing
its model ahead of the SDK — forking or bypassing the crate this project
deliberately chose not to hand-roll, on the transport that guards a credential
vault. So Phase 4 ships against a revision one behind the specification,
knowingly.

**Two consequences carry forward.**

1. **`2026-07-28` removes sessions from the protocol layer.** The `initialize`
   handshake and the session-id header go away; the transport becomes
   sessionless. Phase 4 is built on the session model — 04-02's spike observed
   it directly, with `initialize` returning an `Mcp-Session-Id` and
   `AxumRuntime::sessions()` listing it, and 04-07's revocation story is written
   against `SessionStore::delete`. Any migration has to revisit that, not just
   bump a constant.
2. **The legacy HTTP-plus-event-stream transport is deprecated on a twelve-month
   offramp.** Phase 4 does not enable the SDK feature: the `sse` feature is
   absent from the `rust-mcp-sdk` line in `Cargo.toml`, with the reasoning
   recorded at that line. It does *serve* the two legacy routes anyway, because
   `mcp_routes` mounts them unconditionally and `sse_support` turns out to be
   cosmetic — see "`sse_support: false` is cosmetic in `rust-mcp-axum` 1.0.1"
   below, which is where that correction lives. Either way this phase is on the
   right side of the deprecation; the item is recorded so nobody "fixes" the
   missing feature later.

**The revision-6 change that matters most to a later reader** is authorization
hardening: the issuer parameter moves toward a MUST. Whatever implements
`2026-07-28` inherits that alongside the sessionless model.

**What closing it takes:** waiting for `rust-mcp-sdk` to land `2026-07-28`, then
revisiting `ProtocolVersion` in `crates/talaria-mcp/src/main.rs` (04-03 lifts it
into a `lib.rs` constant both transports read) and whatever the HTTP transport
pins, plus the session-model rework above. Not a version bump.

## Client ID Metadata Documents (CIMD) are not implemented

**Decided in 04-CONTEXT.md D-04-02 on security reasoning, not preference.**

Phase 4 ships Dynamic Client Registration (RFC 7591) — deprecated in the newer
revision but functional, and what current MCP clients actually do. It does not
implement CIMD.

**Why.** CIMD requires the authorization server to fetch an attacker-supplied
HTTPS URL. That is an unauthenticated server-side request forgery primitive
**inside the process that holds the credential vault**, and one that can reach
the user's entire loopback space — which on this machine includes the control
socket's own neighbourhood and whatever else the user happens to be running. The
MCP specification itself flags the risk. Talaria's listener is loopback-only
(D-04-04), which bounds who can *reach* the AS but does nothing about where the
AS can be made to reach *to*; SSRF is an outbound problem and locality does not
help.

**Operational consequence, and it is not optional.** The authorization server
metadata advertises a registration endpoint and does **not** advertise
`client_id_metadata_document_supported`. Advertising a capability that is not
implemented breaks conformant clients that would prefer it — they would select
CIMD and then fail, rather than falling back to DCR. Silence is the correct
signal here.

**Assumption A4 rides alongside this.** `04-RESEARCH.md` records that real MCP
clients still use DCR rather than requiring CIMD, and states plainly that this
was **not verified against any specific client**. If a target client turns out to
require CIMD, this trade has to be re-opened deliberately — not worked around by
turning the capability on to make a client connect.

**What closing it takes:** a later phase adopting CIMD must bring a fetch policy
with it. An allowlist of permitted issuers, or a resolver that refuses private
and loopback address space before the request leaves — with the refusal applied
after DNS resolution, not by string-matching the URL. Turning the capability on
without that is the change this entry exists to prevent.

## TLS and any non-loopback bind are Phase 5's, with a named obligation

**Decided in 04-CONTEXT.md D-04-04.**

Phase 4's HTTP listener is disabled by default and, when enabled, binds
`127.0.0.1` only.

**Why shipping without TLS is conformant here rather than merely skipped.** OAuth
2.1 requires authorization-server endpoints to be served over HTTPS, **with an
explicit loopback exception**. A loopback-only deployment is inside the
specification, not outside it. This is the sentence that matters for a later
reader: the absence of TLS in Phase 4 is a consequence of the bind address, not
an item someone ran out of time for, and it stops being conformant the moment
the bind address changes.

That said, loopback is not a security boundary — `127.0.0.1:PORT` is reachable
by every local account, unlike the `0600`-inside-`0700` Unix control socket, and
Phase 2's `SO_PEERCRED` peer-UID check has no TCP equivalent. The bearer token is
the entire boundary. That is why the listener is off by default, and it is a
reason to keep it off, not a reason to relax about TLS.

**Phase 5 inherits this as a prerequisite, not a surprise.** A tailnet bind is
not loopback. Before Phase 5 enables any non-loopback bind it needs a
certificate story: `rust-mcp-axum`'s `ssl` feature (its `AxumServerOptions`
carries `enable_ssl`, `ssl_cert_path` and `ssl_key_path`, and `validate()`
rejects `enable_ssl` without both paths) plus a real certificate — `tailscale
cert` can issue one. **Phase 5 must not enable a non-loopback bind before that
lands.**

## No "last used" column in the Access panel

**Decided in 04-UI-SPEC.md Open Question 4 and implemented in 04-08.**

The Access panel shows when a client was *authorized*, not when it was last
seen. A last-used column would be genuinely useful — "which of these is still
in use?" is the question a human asks right before revoking — and it is
excluded on cost, not on taste.

**Why.** Every store in this browser is a whole-document atomic rewrite
(`Agents::save`, and the four Phase 3 stores before it: stage a `.tmp` sibling,
`fs::rename` it into place). A last-used timestamp is written on every
authenticated request, so the column would make **every agent request a disk
write of the entire token store**. The relative authorization time costs
nothing — it is already recorded, at the one moment a human is involved — and
answers most of what the column would.

**What closing it takes:** the store becoming an append log, or accepting a
debounced write (say, at most one flush per minute, with the in-memory value
authoritative between flushes). Either makes this a cheap addition rather than
a redesign. Do not add the column against the current write path.

## A revoked client's tabs stay open

**Decided in 04-08, implemented in `apply_ui_actions`' `UiAction::RevokeClient`
arm and documented in `SECURITY.md`.**

Revoking an agent drops its registration, every token in its family, and its
open response streams. It does **not** close the tabs that agent opened.

**Why.** Those tabs are visible in the Agents view and the human can take any
of them over — that is the product's central move. Closing a person's tabs
because a credential was withdrawn destroys state they may want and cannot get
back, and it buys nothing: the agent cannot drive them any more, which is the
whole of what revoking is for. The conservative direction here is to leave
them.

**What closing it takes:** a user asking for it, and then a decision about what
the tabs *become* — reassigned to the human (`TabOwner::Me`), or left in the
Agents view owned by a client that no longer exists. The second is what happens
today and it is coherent; the first is a different product decision, not a
bug fix. Nothing else of the human's is touched by a revoke either — no history
row, no bookmark, no credential, no downloaded file — and that should stay true
whatever is decided about tabs.

## Immediate stream termination goes through the connection, not the SDK

**Discovered in 04-08 while implementing T-7's mitigation, and recorded because
the workaround is load-bearing and a later SDK upgrade could make it
unnecessary.**

`04-02-SPIKE.md`'s assumption A6 was left partially open: a live session is
addressable by client identity, but the spike never held an open stream while
deleting one, so whether the delete *terminates* that stream was unobserved.
04-08 observed it, by asserting the closure from the client end. **It does
not.** `ServerRuntime::shutdown` cancels the reader — the client-to-server
direction — and the response body is fed from the other half of a duplex the
transport's message dispatcher still owns, on a task that outlives the
session-store entry. Nothing in `rust-mcp-sdk` 1.0.1's published API drops that
half, and the fallback `04-RESEARCH.md` named (re-verifying the token on each
keep-alive tick) is inside the SDK's own keep-alive task and equally out of
reach.

**What 04-08 does instead.** The listener notes the descriptor of every
connection it accepts (`Connections`, `crates/talaria-shell/src/http.rs`), binds
it to a session when that session opens a standalone stream over it, and closes
that connection when the session is revoked. The client's read side sees the
response body end, which is the only ending a keep-alive connection can be
given. The bound is immediate. The cost is a small amount of machinery this
module would not otherwise need: an `axum` `tap_io` hook, one router layer to
learn which connection a stream is on, and a three-call `extern "C"` block for
`dup`/`shutdown`/`close` — the same shape `talaria_protocol` already uses for
`getuid`.

**What closing it takes:** the SDK gaining a way to end a stream's *write* half
— either a public terminate on `ServerRuntime`/`SessionStore`, or the
sessionless model of specification revision `2026-07-28`, which changes the
shape of this problem entirely (see the first entry in this file). If either
lands, `Connections` and its `extern "C"` block should go, and
`tests/e2e/revocation_test.py` should keep asserting exactly what it asserts
now — the closure, from the client end — so the replacement has to prove the
same thing.

## `sse_support: false` is cosmetic in `rust-mcp-axum` 1.0.1

**Measured in 04-03 and confirmed in 04-08; recorded so nobody relies on the
flag.**

`AxumServerOptions::sse_support` only affects a log line. `mcp_routes` mounts
the legacy `/sse` and `/messages` endpoints **unconditionally**, on both the
all-in-one and BYO-server mount paths. Talaria's listener therefore serves
them whether or not it wants to.

**Why this is safe today.** Both sit behind the same middleware chain as
`/mcp`: an unauthenticated request to either is refused by `AuthMiddleware`
before it reaches a handler, and one carrying an `Origin` header — or a `Host`
naming an address this listener is not bound to — is refused before that.

**Correction, 2026-08-21.** An earlier version of this entry added "the `sse`
feature is also absent from the `rust-mcp-sdk` line in `Cargo.toml`", which
read as though the routes were inert. It is absent from that line and **it is
enabled anyway, transitively through `rust-mcp-axum`**:

```
$ cargo tree -p talaria-shell -e features -i rust-mcp-sdk
├── rust-mcp-sdk feature "http"
│   ├── rust-mcp-sdk feature "sse"
│   │   └── rust-mcp-axum v1.0.1
```

It has to be: `sse_routes` calls the `#[cfg(feature = "sse")]`
`handle_sse_connection` unconditionally, so the workspace would not otherwise
compile. `/sse` and `/messages` serve real sessions and real event streams.

**Why it is still worth writing down.** The legacy HTTP-plus-event-stream
transport is deprecated in revision `2026-07-28`, and a long-lived legacy
stream is one of the shapes threat T-7 takes. Anyone reading `sse_support:
false` and concluding those routes are not mounted would be wrong.

**Closed for T-7, 2026-08-21 (CR-02).** Stream termination used to gate on
`path == MCP_PATH`, so a revoked client's `/sse` stream kept delivering while
the Access panel showed its row gone. `note_streaming_connection` now tracks
the legacy handshake too, and `tests/e2e/revocation_test.py` asserts closure on
both paths from the client end.

Doing that surfaced a **defect in `rust-mcp-sdk` 1.0.1** worth recording,
because it is the part that was not visible from the outside.
`McpHttpHandler::handle_sse_connection` reads the request's `AuthInfo`
extension *before* it composes the middleware chain that inserts it:

```rust
let (request, auth_info) = request.take::<AuthInfo>();   // always None
...
let handle = compose(&self.middlewares, final_handler);  // AuthMiddleware
```

So every `/sse` session was created with **no verified identity**, which made
it unselectable by a revoke (an unidentified session is deliberately nobody's)
*and* made its tool calls arrive as `UNVERIFIED_CLIENT` rather than as the
client that presented the token. `crates/talaria-shell/src/http.rs` works
around it with a `NoteVerifiedIdentity` middleware placed last in the chain
and one `update_auth_info` call on the session the handshake minted.

**What would retire the workaround:** the SDK taking `AuthInfo` after the
chain rather than before, or honouring `sse_support` so the routes can simply
not be mounted. Either makes `NoteVerifiedIdentity` and `RecordingIds`
deletable.

## The consent panel's arm transition moves the button row

**Discovered in 04-07, hit again in 04-08, and recorded because it is a silent
failure mode for anyone extending the e2e suites.**

`chrome_rects` reports the last frame the shell **drew**. When a control's
armed state changes what the panel renders — the consent panel's "Approve turns
on in a moment" line disappearing moves the button row up by 13 logical points
— a click scheduled against a rect read from the *previous* frame lands in the
wrong place and does nothing at all, with no error.

**The rule for anyone adding to these suites:** after anything that changes what
a panel renders, read the rect from a frame drawn *after* the change.
`harness.wait_for_rect` on the name the *new* state uses is the way to do that,
which is one more reason the two-state controls in this chrome carry two rect
names rather than one.

**What closing it takes:** a `chrome_rects` variant that blocks until the next
frame is drawn, or a `click_rect` that re-reads after a settle. Neither was
built in Phase 4 because the two-rect-name convention already makes the wait
expressible, and a hook that forces a frame is a hook that changes what it
measures.

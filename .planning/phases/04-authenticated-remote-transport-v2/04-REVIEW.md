---
phase: 04-authenticated-remote-transport-v2
reviewed: 2026-08-21T02:29:44Z
depth: deep
files_reviewed: 20
files_reviewed_list:
  - crates/talaria-mcp/Cargo.toml
  - crates/talaria-mcp/src/lib.rs
  - crates/talaria-mcp/src/main.rs
  - crates/talaria-mcp/src/socket.rs
  - crates/talaria-mcp/src/tools.rs
  - crates/talaria-shell/Cargo.toml
  - crates/talaria-shell/src/agents.rs
  - crates/talaria-shell/src/app.rs
  - crates/talaria-shell/src/control.rs
  - crates/talaria-shell/src/gui.rs
  - crates/talaria-shell/src/http.rs
  - crates/talaria-shell/src/main.rs
  - crates/talaria-shell/src/oauth.rs
  - crates/talaria-shell/src/settings.rs
  - tests/e2e/harness.py
  - tests/e2e/http_transport_test.py
  - tests/e2e/oauth_flow_test.py
  - tests/e2e/panel_click_test.py
  - tests/e2e/revocation_test.py
  - tests/e2e/run_all.py
findings:
  critical: 4
  warning: 12
  info: 4
  total: 20
status: issues_found
---

# Phase 4: Code Review Report

**Reviewed:** 2026-08-21T02:29:44Z
**Depth:** deep (cross-file, including the two vendored SDK crates the listener is built on)
**Files Reviewed:** 20
**Status:** issues_found

## Summary

The parts of this phase that were built deliberately are, on the evidence, built correctly.
I verified each of these independently in the code rather than from the summaries:

- **T-1 (structural default-off, loopback-only).** Holds. `BIND_HOST` is a module constant with one
  use site (`http.rs:110`, `:682`); `RemoteAccessConfig` has no bind field; a hand-written
  `"bind"` key in `config.json` is refused and degrades to *off*, not to a default
  (`settings.rs:217-226`). Startup starts a listener only under `if configured.enabled`
  (`app.rs:991`). There is no localhost-trust branch, no debug bypass, and no env knob that admits
  a request — `consent_timings()` (`oauth.rs:383`) is the only test-hook read in the auth path and
  it can only shorten a window, never answer one.
- **T-3 (panel refusals).** Both refusals exist and are disjoint by construction:
  `apply_ui_actions` (`app.rs:1552`) and `Gui::set_panel` (`gui.rs:597`). `raise_consent`
  (`gui.rs:629`) performs the same `reset_panel_view_state()` and sets `focus_credentials = false`,
  which is what `set_panel` would have computed. `resolve_consent` (`app.rs:1440`) re-checks the id
  and refuses both on a mismatch.
- **T-4 (name sanitisation).** `sanitize_claim` is applied on *both* rendering surfaces — the
  consent screen (`gui.rs:2278`) and the Access panel row (`gui.rs:2097`) — truncation before
  sanitisation, and the name is fenced, quoted, suffixed "(as claimed)" and never the subject of a
  sentence.
- **T-5/T-9 (PKCE and rotation).** `S256` only, with no `plain` arm at either gate
  (`oauth.rs:1539`, `:1866`). `AuthorizationCodes::take` (`oauth.rs:610`) removes-and-returns in one
  operation under the consent mutex, so single-use holds under concurrency. Verifier comparison is
  `digests_match` (`agents.rs:415`), constant-time. Redemption re-checks client, redirect URI
  (exact, no port exception) and challenge method. `rotate_refresh` (`agents.rs:733`) marks consumed
  and issues in one mutation; an honest retry with the current token is not punished.
- **T-6 (audience).** `verify` compares `audience != self.resource` byte-for-byte
  (`oauth.rs:1234`) against the single `canonical_resource` construction, and refuses with the same
  `REFUSAL` constant as everything else. Nothing in the workspace forwards a bearer token onward.
- **T-10 (no secret in a log).** Every log call in `oauth.rs` names a `client_id` or a
  `digest_prefix`; no raw token, verifier, code or `Authorization` value is interpolated anywhere,
  including on the error paths. Confirmed by reading all call sites, not by grep.
- **Oracle collapse.** `REFUSAL` and `grant_refused()` are single expressions, and every failure
  path lands on one of them. `Agents::lookup` collapses unknown/expired/consumed/wrong-kind into one
  `None`.
- **Conventions.** Zero `unwrap()`, zero `expect()`, zero `panic!`/`unreachable!` in the
  non-test code of `http.rs`, `oauth.rs` and `agents.rs`.

What survives is concentrated in three places, and all three are places where the *documented*
mitigation and the *shipped* mitigation differ:

1. **The authorization-server routes are outside the middleware chain that carries the phase's
   T-2 defences.** The SDK dispatches them through `compose(&[], ..)`. The team recorded this in
   04-05 and reasoned about it *for the metadata document* — a correct call. `/register`,
   `/authorize`, `/token` and `/revoke` were added in 04-06/04-07/04-08 onto the same chain without
   that reasoning being re-run, and it does not extend to them. See **CR-01**.
2. **The T-7 stream termination is `/mcp`-only, and `/sse` is live.** `note_streaming_connection`
   gates on `path == MCP_PATH`. `cargo tree -e features` shows the SDK's `sse` feature *is* enabled
   transitively through `rust-mcp-axum`, contradicting `deferred-items.md:215`. A revoked client's
   open `/sse` stream keeps delivering. See **CR-02**.
3. **The connection-tracking machinery has an unbounded descriptor leak and two ways to be
   defeated by an unauthenticated peer.** See **CR-03**, **WR-01**, **WR-02**.

Plus one persistent, unauthenticated, unrecoverable-from-the-UI denial of service on the
registration endpoint (**CR-04**).

The `dup`-based connection close (the phase's most unusual code) is, on the specific questions
asked: **not** vulnerable to closing the wrong connection in the ordinary case — a loopback
four-tuple is unique and `tap_io` always writes the entry before any request on that connection can
be served, so the number read in `attach` names the current connection. It **is** vulnerable to a
descriptor leak (CR-03) and to two tracking-evasion cases (WR-01, WR-02). A non-revoked client's
connection cannot be closed by mistake: `terminate_matching` matches on `client_id` and treats an
absent identity as nobody's, which the unit tests pin.

---

## Critical Issues

### CR-01: Every OAuth endpoint bypasses both the `Origin` refusal and the `Host` validation

**File:** `crates/talaria-shell/src/http.rs:883-910`, `crates/talaria-shell/src/oauth.rs:1138-1157`

**Issue:**
`RefuseOriginHeader` and `DnsRebindProtector` are passed to `McpHttpHandler::new`
(`http.rs:883-888`), which only composes them for the *transport* routes. The SDK's auth routes are
mounted separately by `mcp_routes` → `auth_routes::routes`, and dispatch through
`McpHttpHandler::handle_auth_requests`, whose body is literally:

```rust
let handle = compose(&[], final_handler);   // rust-mcp-sdk-1.0.1/src/mcp_http/mcp_http_handler.rs:169
```

So for `/register`, `/authorize`, `/authorize/status`, `/token`, `/revoke` and both metadata
documents there is **no `Origin` check and no `Host` check at all**. `http.rs:906-909` acknowledges
the empty chain but justifies it only for the metadata document ("a metadata document a client must
fetch *before* it has a token cannot itself demand one") — that argument is about *authentication*,
not about Origin or Host, and it does not transfer to the token endpoint.

There are no CORS headers anywhere in `rust-mcp-axum` 1.0.1 (verified by grep), so a plain
cross-origin page cannot *read* these responses. Two things still get through:

**Attack path A — blind CSRF, no rebinding needed.** Attacker is any web page the human visits, in
any browser on the machine, while remote access is on. A form-encoded or `text/plain` POST is a
CORS "simple request" and is delivered without a preflight:

```js
for (let i = 0; i < 32; i++)
  fetch("http://127.0.0.1:8779/register", {method:"POST", mode:"no-cors",
        headers:{"Content-Type":"text/plain"},
        body: JSON.stringify({client_name:"x", redirect_uris:["https://evil.example/cb"]})});
```

That is CR-04's wedge, delivered from a web page. A top-level navigation to
`http://127.0.0.1:8779/authorize?...` needs no CORS at all and pops the consent panel over the
human's browsing (WR-10).

**Attack path B — DNS rebinding, which is the exact threat `DnsRebindProtector` exists for.**
Attacker serves a page from `http://rebind.evil:8779/`, whose A record then rebinds to `127.0.0.1`.
The page is now *same-origin* with Talaria's authorization server as far as the browser is
concerned, so every response is readable:

1. `fetch("/register", {method:"POST", body: JSON.stringify({client_name:"Talaria Helper",
   redirect_uris:["https://evil.example/cb"]})})` → reads `client_id` back. `Host: rebind.evil:8779`
   is never checked.
2. Opens `/authorize?client_id=…&redirect_uri=https://evil.example/cb&response_type=code&
   code_challenge_method=S256&code_challenge=…&resource=http://127.0.0.1:8779/mcp`. The consent
   panel appears with the attacker's chosen name.
3. If the human approves, the code is delivered to `https://evil.example/cb` — off the machine
   entirely, because `admissible_redirect` permits any HTTPS host (WR-04).
4. The attacker's server relays the code back to the page; the page POSTs `/token` same-origin and
   **reads the access token**.

The same request against `/mcp` is refused twice over (`Origin` present → 403;
`Host: rebind.evil:8779` not in `allowed_hosts` → 403). It is precisely the endpoints that mint and
revoke credentials that have neither check.

**Fix:** move both middlewares out of the SDK's transport chain and into an axum layer applied to
the whole router, so they cover every route the process serves. `note_streaming_connection` already
demonstrates the shape.

```rust
// http.rs, in build_router — replace the `.layer(from_fn_with_state(...))` with a stack:
let router = mcp_routes(state, &mount, http_handler)
    .layer(rust_mcp_axum::axum::middleware::from_fn_with_state(
        Arc::clone(&connections),
        note_streaming_connection,
    ))
    // Runs on every route, including the auth routes the SDK dispatches on compose(&[], ..).
    .layer(rust_mcp_axum::axum::middleware::from_fn_with_state(
        bound.to_owned(),
        refuse_page_originated,
    ));

/// `Origin` present, or `Host` other than the bound address → 403, before routing.
async fn refuse_page_originated(
    State(bound): State<String>,
    request: rust_mcp_axum::axum::extract::Request,
    next: rust_mcp_axum::axum::middleware::Next,
) -> rust_mcp_axum::axum::response::Response {
    let headers = request.headers();
    let host_ok = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| host.eq_ignore_ascii_case(&bound));
    if headers.contains_key(header::ORIGIN) || !host_ok {
        log::debug!("remote access refused a page-originated or misaddressed request");
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}
```

Keep `RefuseOriginHeader` and `DnsRebindProtector` in the SDK chain as well — this is defence in
depth, not a replacement — and add an e2e assertion that `Host: evil.example` and
`Origin: http://evil.example` are both 403 **on `/token` and `/register`**, not only on `/mcp`.
`http_transport_test.py` step 5 currently only proves it for `/mcp`.

---

### CR-02: A revoked client's open `/sse` stream is never closed — T-7 is `/mcp`-only

**File:** `crates/talaria-shell/src/http.rs:953-954`, `:889-901`;
`.planning/phases/04-authenticated-remote-transport-v2/deferred-items.md:215`

**Issue:**
`note_streaming_connection` records a stream only when the path is exactly `/mcp`:

```rust
let streaming = request.method() == Method::GET && request.uri().path() == MCP_PATH;
```

`mcp_routes` mounts `/sse` and `/messages` unconditionally (confirmed in
`rust-mcp-axum-1.0.1/src/routes.rs`), and `handle_sse_connection`
(`rust-mcp-sdk-1.0.1/src/mcp_http/http_utils.rs:751`) registers the SSE session in the *same*
`session_store`, with the same `AuthInfo` and therefore the same `client_id`. So on a revoke:

- `McpSessions::sessions()` finds the SSE session and matches it on `client_id`;
- `end()` calls `self.connections.disconnect(session_id)` → `false`, because nothing ever attached
  a socket for it;
- `end()` then falls back to the internal `DELETE /mcp`, which is the mechanism 04-08 *measured as
  insufficient* (it cancels the reader half only).

Net result: **the client's event stream keeps delivering after the human pressed Revoke**, while the
Access panel row disappears. That is exactly T-7, and exactly the "Success Criterion 3 marked done
while being false" outcome `http.rs:426-427` warns about. `revocation_test.py` cannot catch it
because it drives `/mcp`.

Compounding this, `deferred-items.md:215` states "The `sse` feature is also absent from the
`rust-mcp-sdk` line in `Cargo.toml`", implying the routes are inert. It is absent from the direct
dependency line, but it is **enabled transitively**:

```
$ cargo tree -p talaria-shell -e features -i rust-mcp-sdk
├── rust-mcp-sdk feature "http"
│   ├── rust-mcp-sdk feature "sse"
│   │   └── rust-mcp-axum v1.0.1
```

(It has to be: `rust-mcp-axum`'s `sse_routes` calls the `#[cfg(feature = "sse")]`
`handle_sse_connection` unconditionally, so the workspace would not compile otherwise.)

**Fix:** take the two-line change the deferred item itself names — drop the legacy routes rather
than trying to track them. The SSE session id is minted server-side and appears only in the response
body's `endpoint` event, so there is no request header to key `attach` on; making termination work
for `/sse` is materially harder than removing it.

```rust
// http.rs, build_router — after mcp_routes(...):
use rust_mcp_axum::axum::routing::MethodRouter;
let router = mcp_routes(state, &mount, http_handler)
    // The legacy HTTP+SSE transport is deprecated, is not a transport this
    // browser advertises, and its streams are outside what a revoke can close
    // (`Connections` keys on the standalone-stream request, which /sse has no
    // equivalent of). Removing the routes is the only way SC 3 is true.
    .route("/sse", MethodRouter::new())
    .route("/messages", MethodRouter::new())
    .layer(/* ... */);
```

Then correct `deferred-items.md`'s "the `sse` feature is also absent" sentence and add an e2e case
asserting `GET /sse` with a valid token returns 405/404 rather than a stream.

---

### CR-03: `Connections::streaming` leaks one duplicated descriptor per stream, unbounded

**File:** `crates/talaria-shell/src/http.rs:326-330`, `:358-360`, `:378-384`, `:573-580`

**Issue:**
`Connections::streaming`'s doc claims it is "Bounded by the number of live streams, and pruned
against the session store every time the directory is listed." The second half is true and the first
half depends on it — but the directory is listed **only** from `terminate_matching`, whose only two
callers are a human's revoke (`StreamRegistry::terminate_client`) and the listener's own shutdown
(`serve`, `http.rs:779`). In an ordinary session neither ever runs.

Every `attach` inserts an `OwnedSocket` holding a `dup(2)`ed descriptor. Entries are keyed by MCP
session id, and an MCP client that reconnects gets a *new* session id from `UuidGenerator`, so
nothing overwrites the old entry. The `OwnedSocket` is closed only in `Drop`, which only happens on
`disconnect`, on an overwrite of the same session id, or in `retain_live`.

Consequences, both in the process that holds the credential vault, the Servo engine and the
control socket:

1. **Descriptor exhaustion.** One leaked fd per reconnecting agent stream. At the usual `RLIMIT_NOFILE`
   of 1024, roughly a thousand agent reconnects and the browser can no longer open a file, a socket
   or a tab. Nothing else in the process is holding a budget for this.
2. **Socket lifetime extension.** The kernel socket is not released while our duplicate is open, so
   every long-dead agent connection stays half-alive in the kernel table for the lifetime of the
   browser.

This needs no attacker — a well-behaved MCP client that reconnects on a dropped stream produces it.

**Fix:** prune on the path that actually runs, and cap the map defensively.

```rust
impl Connections {
    /// Bind a session's standalone stream to the connection it arrived on,
    /// releasing any descriptor the store no longer has a session for.
    fn attach(&self, session_id: &str, peer: SocketAddr, live: &[String]) {
        // ... existing lookup/duplicate ...
        if let Ok(mut streaming) = self.streaming.lock() {
            // Pruned here, on the one path that runs on every stream, rather
            // than only on a revoke — a revoke may never happen, and every
            // entry holds a descriptor open until it does.
            streaming.retain(|session_id, _| live.iter().any(|id| id == session_id));
            streaming.insert(session_id.to_owned(), socket);
        }
    }
}
```

`note_streaming_connection` already has `Arc<Connections>`; give it the `Arc<dyn SessionDirectory>`
too (or, cheaper, have `attach` take a `&dyn Fn(&str) -> bool` that asks the session store whether an
id is still live). Either way the invariant the doc comment already claims becomes true. Then correct
the comment on `Connections::streaming`, which currently asserts a bound the code does not provide.

---

### CR-04: Registration is unauthenticated, capped at 32, never expires, and unapproved clients cannot be removed from the UI

**File:** `crates/talaria-shell/src/agents.rs:650-656`, `crates/talaria-shell/src/gui.rs:2018-2029`

**Issue:**
Three properties combine into a permanent denial of service on the phase's core feature:

1. `POST /register` is unauthenticated by design and reachable by **any local account** on
   `127.0.0.1:PORT` (T-1's attacker), and by any web page via blind CSRF (CR-01, path A).
2. `Agents::register` refuses past `MAX_REGISTERED_CLIENTS = 32` rather than evicting
   (`agents.rs:650`). There is no TTL on an unapproved registration and nothing prunes them.
3. The Access panel filters to `client.authorized_at_ms.is_some()` (`gui.rs:2025`), so unapproved
   registrations render **no row and no Revoke button**.

Thirty-two POSTs — a two-line shell script or a `fetch` loop on a web page — permanently fill the
registry. From that moment no legitimate agent can ever register again. The human sees "Nothing
authorized yet" and no evidence of what happened; the only recovery is to quit the browser and hand-
edit or delete `agents.json`. The reasoning at `oauth.rs:1369-1373` ("flooding buys an attacker a
bounded amount of disk and nothing else") accounts for the *disk* and misses the *slot*.

**Fix:** make an unapproved registration expire, and make it visible.

```rust
// agents.rs — how long a registration nobody approved is kept.
//
// A registration is a step in a flow that completes in seconds. One that has
// been sitting unapproved for an hour is abandoned, and keeping it costs a slot
// a legitimate agent needs — which is the whole of a registration flood's
// leverage.
pub const UNAPPROVED_REGISTRATION_TTL_MS: u64 = 60 * 60 * 1_000;

pub fn register(
    &mut self,
    client_name: String,
    redirect_uris: Vec<String>,
    registered_at_ms: u64,
) -> Option<AgentClient> {
    // Evict only registrations nobody approved and nobody came back for. An
    // approved client is never displaced by a flood; that property is kept.
    self.clients.retain(|client| {
        client.authorized_at_ms.is_some()
            || registered_at_ms.saturating_sub(client.registered_at_ms)
                < UNAPPROVED_REGISTRATION_TTL_MS
    });
    if self.clients.len() >= MAX_REGISTERED_CLIENTS {
        // ... existing refusal ...
    }
    // ... existing minting ...
}
```

Separately, give the Access panel a line when unapproved registrations exist — "N programs have
registered and are waiting for approval" with a "Forget them" control — so the state is diagnosable
without opening a JSON file. Fixing CR-01 removes the web-page delivery route but not the local-user
one.

---

## Warnings

### WR-01: An unauthenticated peer can wipe the connection table and defeat the stream half of a revoke

**File:** `crates/talaria-shell/src/http.rs:334-345`

**Issue:** `Connections::accepted` is written from `tap_io`, i.e. at `accept(2)` time, **before any
authentication**. Entries are keyed by peer `SocketAddr` and are removed only by `attach` or by the
overflow branch, which `clear()`s the whole map. Any local account can open 4096 short-lived
connections (each gets a distinct ephemeral port, so each is a distinct key) and force the clear.
Every legitimate agent connection that had been accepted but had not yet opened its stream loses its
entry; `attach` then returns early and that stream becomes invisible to a later revoke — silently,
because `attach` logs nothing on the `None` path.

**Fix:** evict by age instead of clearing, and log at `warn` when an `attach` finds nothing.

```rust
/// Peer address → (descriptor, when it was accepted).
accepted: Mutex<HashMap<SocketAddr, (i32, std::time::Instant)>>,

fn accepted(&self, peer: SocketAddr, fd: i32) {
    let Ok(mut accepted) = self.accepted.lock() else { return };
    let now = std::time::Instant::now();
    // A waiting-room entry is consumed within milliseconds; anything older
    // than this is a connection that never opened a stream. Evicting those
    // keeps a flood of connections from displacing a live one, which
    // `clear()` did.
    accepted.retain(|_, (_, at)| now.duration_since(*at) < Duration::from_secs(30));
    if accepted.len() >= MAX_REMEMBERED_CONNECTIONS {
        log::warn!("remote access is tracking too many connections; forgetting the oldest");
        // Drop one rather than all: see above.
        if let Some(oldest) = accepted.iter().min_by_key(|(_, (_, at))| *at).map(|(k, _)| *k) {
            accepted.remove(&oldest);
        }
    }
    accepted.insert(peer, (fd, now));
}
```

And in `attach`, replace `let Some(fd) = fd else { return };` with a `log::warn!` before the return —
a stream this browser cannot close on a revoke is worth a line.

### WR-02: A second stream on the same keep-alive connection is never tracked

**File:** `crates/talaria-shell/src/http.rs:348-353`

**Issue:** `attach` consumes the waiting-room entry (`accepted.remove(&peer)`) and nothing ever puts
it back. HTTP/1.1 keep-alive means one TCP connection commonly carries a stream, then that stream
ends, then the client opens another over the same connection — an SSE-style reconnect is the normal
case, not an exotic one. The second and every subsequent stream on that connection is untracked, so
`Connections::disconnect` returns `false` for it and a revoke falls back to the session-delete path
04-08 measured as insufficient. T-7's "immediate" bound silently becomes "never" for that client.

**Fix:** leave the entry in place and let it be superseded by the next `accepted` for the same peer,
which is already how `tap_io` behaves. With WR-01's age eviction the map still stays bounded.

```rust
let fd = match self.accepted.lock() {
    // Read, not removed: one connection may carry several standalone streams
    // over its life, and the entry is superseded by the next `accepted` for
    // this peer or aged out — see `Connections::accepted`.
    Ok(accepted) => accepted.get(&peer).map(|(fd, _)| *fd),
    Err(_) => None,
};
```

### WR-03: `parse_agent_url`'s own-origin refusal is not a boundary — page-driven navigation walks around it

**File:** `crates/talaria-shell/src/app.rs:1833-1854`, `crates/talaria-shell/src/app.rs:2667-2675`

**Issue:** `parse_agent_url` is correct for what it guards — `Command::TabsOpen` and
`Command::Navigate` — and `is_listener_origin` correctly folds in the `localhost` alias (and, via
the `url` crate's WHATWG IPv4 normalisation, `127.1`, `0177.0.0.1` and `2130706433`). But
`request_navigation` allows every navigation unconditionally, so an agent bypasses the refusal in
one call:

```
evaluate(tab_id, "location.href = 'http://127.0.0.1:8779/register'")
```

or a `window.open`, or a `<meta refresh>`, or a link click. The tab is then on the listener's origin
and further `evaluate` calls run same-origin against the authorization server. The doc comment at
`app.rs:1820-1825` states the property as though it were enforced ("An agent that can `evaluate` in
a tab it owns and can point that tab at Talaria's own endpoints…") — the mitigation does not
actually stop the thing its own comment describes.

The grant still needs a human's Approve click, so this is not a privilege escalation on its own; it
is a claimed mitigation that does not hold, which matters because later work will lean on it.

**Fix:** enforce it where navigation actually happens.

```rust
fn request_navigation(&self, webview: WebView, navigation_request: servo::NavigationRequest) {
    // The one refusal this delegate makes. Talaria is not a policy layer, but
    // the listener's own origin is not policy: it is the transport the agent is
    // already speaking, and `parse_agent_url`'s refusal is worth nothing if a
    // page-driven navigation reaches the same address. Keyed on the tab's
    // owner, so the human's own tabs are unaffected.
    let agent_owned = self
        .tabs
        .borrow()
        .find_by_webview(&webview)
        .is_some_and(|tab| matches!(tab.owner, TabOwner::Agent { .. }));
    let bound = self.remote.borrow().bound_addr().map(str::to_owned);
    if agent_owned {
        if let Some(bound) = bound {
            if is_listener_origin(navigation_request.url(), &bound) {
                log::warn!("refusing an agent tab's navigation to the listener's own origin");
                return; // dropping the request is the refusal
            }
        }
    }
    navigation_request.allow();
}
```

If that is judged too invasive for a delegate callback, then soften the doc comment so it claims
only what it does: that the two *commands* refuse the origin.

### WR-04: Any HTTPS host is registrable as a redirect URI, and the consent screen only warns for loopback

**File:** `crates/talaria-shell/src/oauth.rs:842-854`, `crates/talaria-shell/src/gui.rs:2299-2315`

**Issue:** `admissible_redirect` accepts `https://` with any host. Talaria's clients are, by
construction, local programs — RFC 8252 native clients that bind an ephemeral loopback port. There
is no client in this design that legitimately needs an authorization code delivered to a remote
server, and permitting one is what turns CR-01's rebinding attack from "a page that can poke local
endpoints" into "a page that can receive a grant".

The consent screen then under-describes it. For a loopback target it says "That address is a program
on this computer, not a website." For `https://evil.example/cb` it says only
"Sends you back to: evil.example:443" — no framing at all that the grant is about to leave the
machine, which is the case a human most needs told.

**Fix:** restrict to loopback, which is what this browser's clients are, and say so.

```rust
/// Whether a redirect URI may be registered at all.
///
/// **Loopback literals only.** Every client of this browser is a local program
/// — that is what the transport is — so a redirect that leaves the machine has
/// no legitimate client and is only a way for an authorization code to reach
/// somebody who is not on this computer. RFC 8252's loopback exception is the
/// case; an arbitrary HTTPS host is not.
fn admissible_redirect(uri: &str) -> bool {
    let Ok(url) = Url::parse(uri) else { return false };
    if url.fragment().is_some() {
        return false;
    }
    url.scheme() == "http" && host_is_loopback(&url)
}
```

If a remote redirect must stay possible, invert the consent copy so the *non*-loopback case is the
one that carries a warning line ("That address is a website on the internet. Approving sends this
grant off this computer.").

### WR-05: The authorization-code lookup uses `==` where every other digest comparison in the module uses `digests_match`

**File:** `crates/talaria-shell/src/agents.rs:612`

**Issue:**
```rust
let index = self.entries.iter().position(|record| record.digest == digest)?;
```

`agents::digests_match` exists precisely so that no digest in this browser is compared with `==`,
and its doc comment (`agents.rs:400-414`) says "This is the one place in this file where the
readable version is the insecure one." `Agents::lookup`, `Agents::rotate_refresh` and
`revoke_presented_token` all honour that. `AuthorizationCodes::take` — which compares the digest of
a **secret** an attacker is trying to guess — does not.

I do not believe this is exploitable (recovering a SHA-256 digest byte-by-byte does not yield a
preimage, and the timing signal over loopback across a `Vec` scan is negligible), which is why it is
a warning and not a blocker. It is still the module's own stated invariant, broken in the one place
the invariant was written for.

**Fix:**
```rust
let index = self.entries.iter().position(|record| digests_match(&record.digest, &digest))?;
```

### WR-06: The token store grows without bound; nothing prunes expired or consumed records

**File:** `crates/talaria-shell/src/agents.rs:733-782`, `:831-864`

**Issue:** `rotate_refresh` appends two records per rotation and deliberately keeps the consumed one
as the reuse-detection window. Nothing ever removes an expired access token, an expired refresh
token, or a consumed record whose refresh TTL has long passed. `Agents::save` rewrites the whole
document on every mutation, and `Agents::lookup` linear-scans it on **every single request**. An
agent refreshing hourly for the 30-day refresh lifetime leaves ~1440 records that will never be
removed; over a year of use `agents.json` becomes tens of thousands of records that are rewritten on
every registration, authorization, rotation and revoke.

There is a correctness edge too: a consumed refresh record only needs to outlive its own expiry to
serve reuse detection. Past that point it is pure ballast.

**Fix:** prune on load and on save, against the expiry the records already carry.

```rust
/// Forget every token that can no longer decide anything.
///
/// A record is only useful while it can resolve (`lookup`) or while it can
/// still catch a replay (`rotate_refresh`). Both stop at `expires_at_ms`, so a
/// record past its expiry is ballast that every verification then scans.
fn prune_tokens(&mut self, now_ms: u64) {
    self.tokens.retain(|record| record.expires_at_ms > now_ms);
}
```

Call it from `load_from` (with the caller's clock — `Agents` has no clock, so take `now_ms` as a
parameter, as every other function here does) and at the top of `authorize`/`rotate_refresh`, which
already receive one.

### WR-07: A failed listener-thread spawn leaves the UI claiming "Starting" forever, and blocks a retry

**File:** `crates/talaria-shell/src/http.rs:650-671`, `crates/talaria-shell/src/app.rs:1414-1420`

**Issue:** `start_remote_listener` writes `RemoteAccess::Starting` and then calls `http::spawn`. If
`std::thread::Builder::spawn` fails, `spawn` logs the error and still returns a `ShutdownHandle` and
an empty `StreamRegistry` — but **no `AppEvent::RemoteListenerFailed` is ever sent**, so the state
machine is stuck in `Starting`. `RemoteAccess::is_live()` returns `true` for `Starting`, so
`start_remote_listener` becomes a permanent no-op and clicking "Turn on remote access" again does
nothing. This contradicts `http.rs:152` ("the UI never claims a listener that is not there").

**Fix:** report the failure on the same channel every other failure uses.

```rust
if let Err(error) = thread {
    log::error!("remote access thread could not be spawned: {error}");
    // Reported, not only logged: `RemoteAccess::Starting` is a state only the
    // listener's own event may leave, so a spawn that never happened would
    // otherwise leave the chrome claiming a listener forever and
    // `start_remote_listener` refusing to try again.
    let _ = reporting_proxy.send_event(AppEvent::RemoteListenerFailed {
        addr: format!("{BIND_HOST}:{port}"),
        error: error.to_string(),
    });
}
```

(Clone the proxy once before the `move` closure so one copy survives for this arm.)

### WR-08: Toggling remote access off then on inside the grace period races the port

**File:** `crates/talaria-shell/src/app.rs:1693-1701`, `crates/talaria-shell/src/http.rs:757-799`

**Issue:** `SetRemoteAccess(false)` fires the shutdown handle and immediately writes
`RemoteAccess::Off`, but the listener thread may hold the bound socket for up to
`SHUTDOWN_GRACE` (3 s) while it drains. A human who toggles off and back on inside that window hits
`start_remote_listener` with `is_live() == false`, a fresh bind is attempted on the same port, and it
fails with `EADDRINUSE` → `RemoteListenerFailed`. The Access panel then shows a bind failure for a
port nothing is wrong with. This is a plausible thing to do (a user toggling to "reset" a stuck
agent).

**Fix:** keep a "stopping" state, or retry the bind briefly before reporting failure. The smaller
change is the second:

```rust
// A bind that fails with AddrInUse is most likely this listener's own previous
// incarnation still inside its grace period, so it is retried before it is
// reported: telling the human their port is taken when it is their own browser
// letting go of it is a failure they cannot act on.
let listener = {
    let mut attempt = 0;
    loop {
        match tokio::net::TcpListener::bind(address).await {
            Ok(listener) => break Ok(listener),
            Err(error)
                if error.kind() == std::io::ErrorKind::AddrInUse
                    && attempt < 8 =>
            {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(500)).await;
            },
            Err(error) => break Err(error),
        }
    }
};
```

### WR-09: `PUT`, `PATCH`, `DELETE` and `GET` on `/register` all create a client

**File:** `crates/talaria-shell/src/oauth.rs:2133-2135`

**Issue:** The SDK's `validate_allowed_methods` permits `POST, GET, PUT, PATCH, DELETE, OPTIONS` on
`RegistrationEndpoint` (it is anticipating RFC 7592 client management). `handle_request` routes all
six to `handle_registration(request.body(), now)`, so `PUT /register` with a JSON body registers a
client, `DELETE /register` with a JSON body registers a client, and `OPTIONS /register` with a body
registers a client. Nothing here is exploitable beyond CR-04's flood, but "a DELETE that creates
something" is the kind of surprise that becomes a real bug when RFC 7592 support is added.

**Fix:** narrow to the verb this endpoint actually implements.

```rust
OauthEndpoint::RegistrationEndpoint => match request.method() {
    &Method::POST => Ok(self.handle_registration(request.body(), now)),
    // RFC 7591 defines registration as a POST. The SDK's verb table also
    // admits the RFC 7592 management verbs, which this browser does not
    // implement — answering them with a registration would make DELETE
    // create a client.
    _ => Ok(GenericBody::create_405_response(request.method(), &[Method::POST])),
},
```

### WR-10: The consent harassment caps let an unauthenticated peer hold the chrome for most of the wall clock, and denying makes it worse

**File:** `crates/talaria-shell/src/oauth.rs:271-285`, `crates/talaria-shell/src/gui.rs:2233`

**Issue:** The consent panel is an `egui::CentralPanel` that replaces the page entirely, and it has
no dismiss — only Approve and Deny. `CONSENT_LIFETIME_MS` is 120 s and `EXPIRY_COOLDOWN_MS` is 30 s,
so a peer that registers once (unauthenticated, CR-04) and then loops `/authorize` keeps the human's
page covered for 120 of every 150 seconds — 80% of the wall clock — while remote access is on. And
because `DENY_COOLDOWN_MS` is 10 s, a human who *answers* by pressing Deny gets the next panel in
10 s instead of 30 s: engaging with the attack is punished.

The reasoning at `oauth.rs:273-278` ("A denial is evidence the human *is* at the keyboard, so the
shorter of the two cooldowns is enough") is exactly backwards from the harassment perspective: it is
the *engaged* human who gets hit hardest.

**Fix:** make repetition expensive, and let a human stop it.

```rust
/// How long `/authorize` refuses after a human pressed Deny, in milliseconds.
///
/// **Escalating**, because a fixed cooldown is a rate limit an attacker simply
/// waits out — and because the human who answers is the one a fixed short
/// cooldown punishes hardest. Doubles on each consecutive non-approval and
/// resets on an approval.
const DENY_COOLDOWN_MS: u64 = 10_000;
const MAX_COOLDOWN_MS: u64 = 15 * 60 * 1_000;

// In `ConsentState`, add:
//   consecutive_refusals: u32,
// and in `settle`'s Denied|Expired arm:
consent.consecutive_refusals = consent.consecutive_refusals.saturating_add(1);
let backoff = cooldown
    .saturating_mul(1u64 << consent.consecutive_refusals.min(8))
    .min(MAX_COOLDOWN_MS);
consent.cooldown_until_ms = now_ms.saturating_add(backoff);
```

Separately, give the panel a third exit — a "Deny and stop asking" control that arms a long cooldown
for that `client_id`, or simply turns remote access off. Right now a human under this attack has no
in-product way to make it stop.

### WR-11: `pending_consent` is never cleared when the HTTP side abandons a request

**File:** `crates/talaria-shell/src/gui.rs:2206-2207`, `crates/talaria-shell/src/app.rs:1066`

**Issue:** When the parked request times out, the panel body closes itself by setting the local
`panel` (which is correctly written back at `gui.rs:2458`) — but `Shared::pending_consent` keeps the
dead `ConsentRequest`, with its `client_id`, `client_name` and `redirect_uri`, indefinitely. It is
replaced only when the next request arrives, which may be never. `Gui` is not allowed to mutate
shared state, so the panel cannot clear it itself, which is why nothing does.

The consequence is small (a retained struct, and a `pending_consent.is_some()` that is misleading to
any future reader) but the invariant "at most one request is parked, and a parked request is live"
is quietly false.

**Fix:** clear it from the main thread, where the borrow is legal. `Gui::draw` already returns
`Vec<UiAction>`; add one:

```rust
// gui.rs, in the abandoned arm:
Some(request) if request.is_abandoned() => {
    panel = ChromePanel::None;
    // The panel cannot clear `Shared::pending_consent` itself — `Gui` never
    // mutates shared state — so it says so and the caller does it.
    actions.push(UiAction::ClearAbandonedConsent);
},

// app.rs, in apply_ui_actions:
UiAction::ClearAbandonedConsent => {
    // Only when it really is abandoned: a live request must never be dropped
    // by a stale frame's action, which would read to the HTTP side as a denial.
    let abandoned = state
        .pending_consent
        .borrow()
        .as_ref()
        .is_some_and(crate::oauth::ConsentRequest::is_abandoned);
    if abandoned {
        state.pending_consent.borrow_mut().take();
    }
},
```

### WR-12: Every HTTP client shares one session id, and no `Session` is registered, so all lifecycle events are silently dropped

**File:** `crates/talaria-shell/src/http.rs:733`, `crates/talaria-shell/src/app.rs:712-740`

**Issue:** `EventLoopSink` allocates a single `next_session_id()` for the whole listener and uses it
for every remote client. Two things follow:

1. Nothing ever inserts a `Session` for that id into `Shared::sessions` — only the control socket's
   `AppEvent::SessionStarted` does. `queue_event` therefore pushes a `PendingEvent` that
   `process_pending_events` discards on the very next drain (`app.rs:736`), so **`TabCrashed` and
   `TabClosed` are never delivered to any HTTP client**. The stdio transport delivers them. The
   comment at `app.rs:732-735` reads the discard as correct addressing, which it is not here —
   there is a client, it just has no session row.
2. If a `Session` were ever registered for that id, every remote client's tabs would share one
   addressee and events for client A's tabs would be delivered to client B.

`http.rs:724-732` explains the single id as being about tab ownership, and that reasoning is sound
for ownership; it just also silently disables event delivery, which no summary records.

**Fix:** either register a per-listener session and route events through the MCP notification
channel, or — if event delivery over HTTP is out of scope for this phase — say so explicitly at
`http.rs:733` and add it to `deferred-items.md`, so the next reader does not assume parity between
the transports that the tool-surface test appears to promise.

---

## Info

### IN-01: Access panel hover text is not truncated

**File:** `crates/talaria-shell/src/gui.rs:2101`, `:2103`

`sanitize_claim(&client.client_name)` and `sanitize_claim(uri)` are used without `truncate_chars`,
unlike the row label at `:2097`. A 10 KB `client_name` produces a 10 KB tooltip. Not a lie — the
characters that could make one are replaced — but it can cover the panel.
Fix: `sanitize_claim(&truncate_chars(&client.client_name, CLAIM_LIMIT * 4))`.

### IN-02: The listener origin is browsable during `Starting` and during the shutdown drain

**File:** `crates/talaria-shell/src/http.rs:166-171`, `crates/talaria-shell/src/app.rs:1700`

`bound_addr()` returns `None` for `Starting` and for `Off`, so `parse_agent_url` permits the
listener's origin during the millisecond between the socket binding and the main thread processing
`RemoteListenerBound`, and during the up-to-3 s graceful drain after `RemoteAccess::Off` is written.
Both windows are narrow and (given WR-03) not the effective boundary anyway. Worth a sentence in the
`bound_addr` doc comment, which currently reads as though the refusal tracks the socket exactly.

### IN-03: The token-request size cap runs after the body is already buffered

**File:** `crates/talaria-shell/src/oauth.rs:239-246`, `:1761`

`MAX_TOKEN_REQUEST_BYTES` is checked on a `&str` that `axum`'s `DefaultBodyLimit` (4 MiB) has
already read and allocated. The doc comment says the cap is "small enough that this endpoint cannot
be made to parse a megabyte of form data on an unauthenticated path" — the parse is prevented, the
read and the allocation are not. Either lower the router's `max_request_body_size`
(`McpMountOptions`) or reword the comment to claim only what it does.

### IN-04: Reuse detection revokes the family but leaves the client showing as authorized

**File:** `crates/talaria-shell/src/agents.rs:745-753`, `crates/talaria-shell/src/gui.rs:2025`

`rotate_refresh`'s `ReuseDetected` branch calls `revoke_family`, which drops the tokens and leaves
`authorized_at_ms` set. The Access panel filters on that field, so the client keeps a row that says
it has access when it holds no credential at all. The human's Revoke still works, so this is
cosmetic — but it is the one row in the product whose meaning is "this agent can drive your
browser", and it is wrong in exactly the case where a token was probably stolen.

---

_Reviewed: 2026-08-21T02:29:44Z_
_Reviewer: Claude (gsd-code-reviewer)_
_Depth: deep_

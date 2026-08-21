# 04-02 Spike — does `rust-mcp-axum` route a self-hosted authorization server?

**Assumption under test:** `04-RESEARCH.md` **A1**, the phase's largest.

> `AxumServerOptions.auth` accepts a hand-written `Arc<dyn AuthProvider>` and `rust-mcp-axum` routes
> every declared `OauthEndpoint` to `handle_request` — i.e. a self-hosted AS is a supported
> integration and not merely a trait that happens to be public.

`04-CONTEXT.md` D-04-03 requires this settled *before* 04-05 and 04-06 are executed.

**Method:** a throwaway cargo example, `crates/talaria-shell/examples/auth_routing_spike.rs`,
compiled against the workspace's real resolved dependency graph (so it also exercises A8) and run in
three separate process invocations. The file was deleted at the end of plan 04-02 Task 2; only these
findings survive.

---

## Verdict

A1: CONFIRMED

Every endpoint the provider declared reached that provider's `handle_request`, with the correct
`OauthEndpoint` discriminant, on a server started by `create_axum_server` with nothing but
`AxumServerOptions { auth: Some(...), sse_support: false, .. }`. No fork, no hand-written tower
layer, no `mcp_routes()` mounting. Nothing upstream of the routing assumes a remote issuer.

04-05 and 04-06 can be executed as planned.

---

## Versions and exact sources read

| What | Version | Path read |
|------|---------|-----------|
| `rust-mcp-sdk` | `1.0.1` | `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rust-mcp-sdk-1.0.1/` |
| `rust-mcp-axum` | `1.0.1` | `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rust-mcp-axum-1.0.1/` |
| `rust-mcp-schema` | `0.10.3` | resolved transitively; `ProtocolVersion::V2025_11_25` used |

These are the **published crates.io tarballs**, not GitHub `main` — which is precisely what A2 asked
for. Both crates are still newest at `1.0.1` (published 2026-07-26); re-verified against the
crates.io API at execution time.

---

## Probe A — a provider declaring the full co-hosted AS endpoint set

Endpoints declared (the paths `04-RESEARCH.md` Option B lists). `IntrospectionEndpoint` was
deliberately **not** declared: the AS and the RS are one process, so there is nothing to introspect
across, and an endpoint declared but unimplemented is a 500 waiting.

`verify_token` rejected unconditionally. `handle_request` returned `200` with a body naming the
request's method, path and resolved `endpoint_type`, so a response proves *which* route reached it.

| Request | Status | Reached `handle_request`? | `endpoint_type` resolved to |
|---------|--------|---------------------------|------------------------------|
| `GET /authorize` | `200` | **yes** | `AuthorizationEndpoint` |
| `POST /token` | `200` | **yes** | `TokenEndpoint` |
| `POST /register` | `200` | **yes** | `RegistrationEndpoint` |
| `POST /revoke` | `200` | **yes** | `RevocationEndpoint` |
| `GET /.well-known/oauth-authorization-server` | `200` | **yes** | `AuthorizationServerMetadata` |
| `GET /.well-known/oauth-protected-resource` | `200` | **yes** | `ProtectedResourceMetadata` |
| `POST /introspect` (control — undeclared) | `404` | no | — |
| `GET /token` (wrong verb on a declared endpoint) | `200` | **yes** | `TokenEndpoint` |
| `POST /mcp` (no credential) | `401` | n/a — rejected by `AuthMiddleware` | — |

Verbatim, for the six declared endpoints:

```
  GET http://127.0.0.1:44703/authorize
    status:  200
    headers: content-type: text/plain | content-length: 85 | date: ...
    body:    REACHED handle_request method=GET path=/authorize endpoint_type=AuthorizationEndpoint
  POST http://127.0.0.1:44703/token
    status:  200
    body:    REACHED handle_request method=POST path=/token endpoint_type=TokenEndpoint
  POST http://127.0.0.1:44703/register
    status:  200
    body:    REACHED handle_request method=POST path=/register endpoint_type=RegistrationEndpoint
  POST http://127.0.0.1:44703/revoke
    status:  200
    body:    REACHED handle_request method=POST path=/revoke endpoint_type=RevocationEndpoint
  GET http://127.0.0.1:44703/.well-known/oauth-authorization-server
    status:  200
    body:    REACHED handle_request method=GET path=/.well-known/oauth-authorization-server endpoint_type=AuthorizationServerMetadata
  GET http://127.0.0.1:44703/.well-known/oauth-protected-resource
    status:  200
    body:    REACHED handle_request method=GET path=/.well-known/oauth-protected-resource endpoint_type=ProtectedResourceMetadata
```

The undeclared-path control:

```
  POST http://127.0.0.1:44703/introspect
    status:  404
    body:    The requested uri does not exist:
             uri: /introspect
```

### Does anything upstream assume a *remote* issuer?

**No.** Settled by source, with the lines:

- `rust-mcp-axum-1.0.1/src/routes/auth_routes.rs:9-16` — `routes()` folds `Router::new()` over
  `mcp_handler.oauth_endpoints()`, adding `router.route(endpoint, any(handle_auth_request))` for
  each declared *path string*. There is no issuer, no remote URL, and no `RemoteAuthProvider`
  anywhere in the fold. The keys of the provider's own `HashMap<String, OauthEndpoint>` become the
  routes, literally.
- `rust-mcp-sdk-1.0.1/src/mcp_http/mcp_http_handler.rs:138-142` — `oauth_endpoints()` is
  `self.auth.as_ref().and_then(|a| a.auth_endpoints().map(|e| e.keys().collect()))`. The provider is
  the only source of truth.
- `rust-mcp-sdk-1.0.1/src/mcp_http/mcp_http_handler.rs:144-167` — `handle_auth_requests` calls
  `auth_provider.handle_request(req, state)` **through `compose(&[], final_handler)`**: the empty
  slice is load-bearing. Auth endpoints are composed with *no* middlewares, so `AuthMiddleware` does
  not run on them and `/token` is reachable without a bearer token, which is the only way an
  authorization server can work. This is why Probe A's declared endpoints returned `200` while
  `/mcp` returned `401` in the same process.
- `rust-mcp-axum-1.0.1/src/server.rs:400-409` — `AuthMiddleware::new(auth_provider.clone())` is
  pushed into the *general* middleware chain and the same provider is handed to
  `McpHttpHandler::new`. One provider serves both roles; nothing distinguishes a local one from a
  remote one.

`RemoteAuthProvider` exists (`src/auth/auth_provider/remote_auth_provider.rs`) but is one optional
implementation of the trait, not a requirement of the routing.

### Finding that changes 04-06's work: the SDK does **not** enforce allowed methods

`GET /token` returned `200` and reached `handle_request` as `TokenEndpoint`. `AuthProvider` provides
`validate_allowed_methods` with a complete per-endpoint verb table
(`auth_provider.rs:63-99` — `TokenEndpoint` allows only `POST`/`OPTIONS`), and it builds the `405`
for you, **but nothing calls it**. `handle_auth_requests` goes straight to `handle_request`.

**04-06 must call `self.validate_allowed_methods(endpoint, request.method())` as the first thing in
its own `handle_request` and return the `405` it hands back.** Otherwise `GET /token`,
`DELETE /revoke` and friends fall through into the token-issuance path with an empty body. This is a
provider obligation the trait's shape makes look automatic and is not.

### The unauthenticated MCP-endpoint response (04-05's input)

The SDK constructs the `WWW-Authenticate` challenge itself, and it includes `resource_metadata`
whenever the provider's `protected_resource_metadata_url()` returns `Some`:

```
  POST http://127.0.0.1:44703/mcp
    status:  401
    content-type: application/json
    www-authenticate: Bearer error="invalid_token",
                      error_description="Missing access token in Authorization header",
                      resource_metadata="http://127.0.0.1:44703/.well-known/oauth-protected-resource"
    body: {"error":"invalid_token","error_description":"Missing access token in Authorization header"}
```

Built at `rust-mcp-sdk-1.0.1/src/mcp_http/middleware/auth_middleware.rs:79-87`
(`create_www_auth_value`) and `:90-139` (`error_response`). 04-05 does **not** need to write the
challenge; it needs to return the right `AuthenticationError` variant and a correct absolute
`protected_resource_metadata_url`.

Two behaviours 04-04's token store must respect, both from `AuthMiddleware::validate`
(`auth_middleware.rs:23-77`):

1. **A token with `expires_at: None` is rejected outright** — `"Token has no expiration time"`,
   `401`. An `AuthInfo` with no expiry is not "never expires", it is invalid.
2. **`verify_token` runs on every request**, before the handler, and the middleware re-checks expiry
   and scopes each time. This is what makes SC 3's "the next request fails after a revoke" work
   provided the store is not memoised behind the request boundary.

Status mapping, for 04-05: `InvalidToken`/`InactiveToken` → `401` + challenge;
`InsufficientScope` → `403` + challenge; `TokenVerificationFailed` → `403` if it carries `403`,
otherwise its own status or `400`, **with no challenge**; everything else → `400`, no challenge.

---

## Probe B — a provider declaring *no* endpoints (04-03's interim configuration)

`auth_endpoints()` returning `None`. The question was whether the server still starts and still
refuses.

```
bound: 127.0.0.1:36081 (server started: yes)
  POST http://127.0.0.1:36081/mcp
    status:  401
    www-authenticate: Bearer error="invalid_token", error_description="Missing access token in
                      Authorization header", resource_metadata="http://127.0.0.1:36081/.well-known/oauth-protected-resource"
  POST http://127.0.0.1:36081/token
    status:  404
```

**The server starts and the MCP endpoint is still refused.** `oauth_endpoints()` returns `None`,
`unwrap_or_default()` gives an empty vector, and the auth router is empty — no panic, no startup
error. 04-03 can ship a provider that declares nothing and still have a protected listener.

**One thing 04-03 must decide.** The `401` still advertises
`resource_metadata="…/.well-known/oauth-protected-resource"`, and with no endpoints declared that
path now `404`s. A conformant client following the challenge gets a dangling pointer. 04-03 should
either return `None` from `protected_resource_metadata_url()` until 04-06 declares the metadata
endpoint, or declare `ProtectedResourceMetadata` early. Advertising a document that does not exist
is worse than advertising nothing.

---

## A2: ANSWERED — the published `1.0.1` source is byte-identical to GitHub `main`

The research read `main`; A2 asked whether the published tarball differs. Diffed directly:

| File | Result |
|------|--------|
| `rust-mcp-sdk/src/auth/auth_provider.rs` | **identical** (`diff` empty) |
| `rust-mcp-axum/src/server.rs` (whole file, incl. `AxumServerOptions`) | **identical** |
| `rust-mcp-axum/src/routes/auth_routes.rs` | **identical** |

So the research's *quotations* were accurate. Two deltas nevertheless bit the spike, both in the
research's illustrative **code examples** rather than in the source it quoted:

1. **`AuthenticationError::InvalidToken` is a struct variant**, not a unit variant:
   `InvalidToken { description: &'static str }` (`src/auth/error.rs:14-15`). The research's
   `AuthProvider` seam example writes `AuthenticationError::InvalidToken` bare; that does not
   compile. Note `description` is `&'static str`, so a runtime-formatted reason cannot go in it —
   use `TokenVerificationFailed { description: String, status_code: Option<u16> }` if you need one,
   and remember that variant maps to `400`/`403`, not `401`.
2. **`OauthEndpoint` does not derive `Debug`.** It derives `Hash, Eq, PartialEq, Clone` only
   (`src/auth/metadata.rs:16`). `format!("{endpoint:?}")` fails to compile; 04-06 needs a hand-written
   match to log or name one.

Two more signature facts worth having before 04-03 and 04-07 are written:

3. `AxumRuntime::runtime_by_session()` returns `Result<Arc<ServerRuntime>, TransportServerError>`,
   **not** `Option`.
4. `http` is not a direct dependency of `talaria-shell` and must not become one just for this — the
   SDK re-exports it as `rust_mcp_sdk::mcp_http::http`, which is also the only way to be sure the
   `http` version matches the one the trait's signatures are written against.

### `AxumServerOptions` as published — the fields 04-03 will set

Defaults from `impl Default` (`server.rs:307-336`); `host` is `"127.0.0.1"` and `port` is `8080`.

| Field | Type | Default | Note for 04-03 |
|-------|------|---------|----------------|
| `host` | `String` | `"127.0.0.1"` | already loopback; C-7 still forbids ever setting a wildcard |
| `port` | `u16` | `8080` | Open Question 4 wants a non-8080 default |
| `sse_support` | `bool` | **`true`** | must be set to `false` explicitly — Pitfall 4 |
| `auth` | `Option<Arc<dyn AuthProvider>>` | `None` | the whole seam |
| `event_store` | `Option<Arc<dyn EventStore>>` | `None` | leave `None` until resumability is designed |
| `enable_json_response` | `Option<bool>` | `None` (⇒ `false`) | `false` means replies come back as an event stream |
| `dns_rebinding` | `DnsRebindingOptions` | protection on | auto-derives `allowed_hosts` from `host:port` unless the bind is a wildcard |
| `max_request_body_size` | `Option<usize>` | `None` (⇒ 4 MiB) | `413` above the cap |
| `session_store` | `Option<Arc<dyn SessionStore>>` | `None` (⇒ `InMemorySessionStore`, 10k cap) | see A6 |
| `health_endpoint` | `Option<String>` | **`None`** | health check is off unless asked for |
| `ping_interval` | `Duration` | 12 s | |
| `enable_ssl` / `ssl_cert_path` / `ssl_key_path` | `bool` / `Option<String>` ×2 | `false` / `None` | Phase 5's; `validate()` rejects `enable_ssl` without both paths |
| `session_id_generator`, `task_store`, `client_task_store`, `message_observer`, `health_handler`, `custom_*_endpoint`, `transport_options` | various `Option`s | `None`/default | not needed in Phase 4 |

`AxumServerOptions` is **not** `Clone` and has no builder; construct it with
`..Default::default()`. `create_axum_server(server_details, handler, options) -> AxumServer`, then
either `AxumServer::start()` (awaits forever) or `AxumServer::start_runtime()` (returns an
`AxumRuntime` and keeps serving). Probe C used the latter.

One live-fire caveat the spike hit: **three `new_multi_thread` runtimes in one process made server
startup nondeterministic**, because the SDK spawns a `shutdown_signal` task per server that installs
its own ctrl-c and `SIGTERM` handlers (`server.rs:579-607`). Talaria runs exactly one HTTP listener
so this does not bite 04-03, but a future test that starts several in-process should expect it. That
same shutdown task calls `state.session_store.clear()` on signal.

---

## A6: ANSWERED for identification and for the request path; PARTIALLY OPEN for an in-flight stream

> A6: Terminating a revoked client's live handles is expressible against `rust-mcp-axum`'s session
> store.

Probe C opened one *authenticated* session (`initialize` with a token `verify_token` accepted), then
asked the server's own runtime and session store about it:

```
  initialize -> status 200
    mcp-session-id: Some("2bd73818-ed5c-4cd8-984a-6d940acec101")
    body: data: {"id":1,"jsonrpc":"2.0","result":{...,"protocolVersion":"2025-11-25",...}}

  AxumRuntime::sessions() -> ["2bd73818-ed5c-4cd8-984a-6d940acec101"]
    session 2bd73818-…: auth_info client_id=Some("spike-client") token_unique_id=Some("spike-token-id")
  deleting session 2bd73818-… from the store...
    session_store.has(2bd73818-…) after delete -> false
  post-delete tools/list -> status 404
    body: {"code":-32016,"data":null,"message":"Session not found"}
```

**What is settled:**

- **A live session is addressable by client identity.** `AxumRuntime::sessions()` lists session ids;
  `runtime_by_session(id)` yields the `Arc<ServerRuntime>`; `McpServer::auth_info_cloned()` on it
  returns the `AuthInfo` that was in force when the session was created — including `client_id` and
  `token_unique_id`. The plumbing exists because `start_new_session` threads `auth_info` into
  `create_server_instance` (`src/mcp_http/http_utils.rs:399-424`). So "find every session belonging
  to client X" is a filter over `sessions()`, not a structure 04-07 has to invent.
- **Dropping it takes immediate effect on the request path.** `SessionStore::delete(&id)` removes
  the entry, `has()` goes false, and the very next request bearing that `Mcp-Session-Id` gets
  `404 {"code":-32016,"message":"Session not found"}`. `AxumServer::state()` hands out the
  `Arc<McpAppState>` that owns the store, so 04-07 can hold it.

**What is still open:** whether deleting the session also *terminates an already-open* response
stream. `InMemorySessionStore::delete` only removes the map entry
(`src/session_store/in_memory_session_store.rs:160-165`) — it does not signal, close, or abort the
`ServerRuntime`, and the stream task holds its own `Arc`, so the `Arc` outliving the map entry is
the expected case rather than a surprise. Probe C did not hold an open `GET /mcp` stream while
deleting, so this was not observed either way.

This matters less than it did when A6 was written, for two reasons: `sse_support` is `false`, so the
deprecated long-lived transport is not exposed at all; and `AuthMiddleware` re-runs `verify_token`
per request, so any *new* request on a revoked token fails regardless of session state. The residual
is narrow — a standalone `GET /mcp` stream opened before the revoke. **04-07 must verify that case
explicitly** and, if the stream survives, the fallback `04-RESEARCH.md` names (a per-request
re-check inside the stream's keepalive) is still available; the ping interval is 12 s by default.

---

## A8: CONFIRMED

`talaria-shell` gaining `rust-mcp-sdk` and `rust-mcp-axum` produced no version conflict with
Servo's tree. Demonstrated rather than argued: the spike is a `talaria-shell` cargo example, so it
linked the shell's real resolved graph — Servo, egui, winit, rustls and the new HTTP stack in one
binary — and `cargo build --release --locked` is green. The two-process alternative is not needed.

The spike also ran the exact Pattern 3 arrangement 04-03 will use: a named `std::thread::Builder`
thread with a `tokio::runtime::Builder::new_multi_thread().enable_all()` runtime built inside it,
`block_on`-ing the server. That shape works against the shell's dependency graph.

---

## Consequences for the rest of the phase

| Plan | What this changes |
|------|-------------------|
| 04-03 | Pattern 3's thread/runtime shape is verified. Ship the interim provider with `auth_endpoints() -> None` — the server starts and still refuses. Decide the `protected_resource_metadata_url` question above; set `sse_support: false` explicitly; use `rust_mcp_sdk::mcp_http::http`, not a new `http` dependency. |
| 04-04 | The token store must never issue an `AuthInfo` with `expires_at: None` — the middleware rejects it. `verify_token` is called per request; do not memoise. |
| 04-05 | Do not hand-write the `401` or the `WWW-Authenticate` challenge; the SDK builds both from the returned `AuthenticationError` and `protected_resource_metadata_url()`. Use the status mapping table above. `InvalidToken`'s `description` is `&'static str`. |
| 04-06 | Proceed as planned — the routing works. **Call `validate_allowed_methods` yourself**; nothing else will. Do not declare `IntrospectionEndpoint`. `OauthEndpoint` has no `Debug`. |
| 04-07 | Revocation on the request path is settled (`SessionStore::delete` → `404` on the next request, plus per-request `verify_token`). The in-flight-stream case is the one thing left to verify; `sse_support: false` already removes most of it. |

---

*Spike executed during plan 04-02, Task 2. Throwaway target
`crates/talaria-shell/examples/auth_routing_spike.rs` deleted in the same task.*

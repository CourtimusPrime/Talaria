# Phase 4: Authenticated Remote Transport (v2) — Research

**Researched:** 2026-08-20
**Domain:** MCP Streamable HTTP transport; OAuth 2.1 resource server + co-hosted authorization server; per-client token lifecycle and revocation; native-chrome consent UI
**Confidence:** HIGH on the specification and SDK facts (fetched from the normative spec and the crate's own source this session); MEDIUM on the architecture recommendations (grounded in this codebase, but they are judgement calls no CONTEXT.md has ratified)

> **No CONTEXT.md exists for this phase.** `/gsd-discuss-phase` was not run, so there are no locked
> decisions. Every design call below is a *recommendation with its alternative stated*, not a
> constraint. The fork in [The Architectural Fork](#the-architectural-fork) in particular is a
> product decision the user should make, not one research should quietly settle.
>
> **Tooling note:** the `gsd-tools query research-plan`, `classify-confidence` and
> `package-legitimacy` seams are not present in the installed `gsd-tools` build (`Unknown command`).
> Provider selection and confidence tiers below were assigned by hand against the documented source
> hierarchy, and the package audit was run directly against the crates.io API.

---

## Summary

Three facts, discovered this session, reshape this phase before a line is planned.

**One: the current MCP specification revision is `2026-07-28`, not `2025-11-25`.** Talaria pins
`ProtocolVersion::V2025_11_25` (`crates/talaria-mcp/src/main.rs:99`), and `rust-mcp-sdk 1.0.1` — the
newest release, published 2026-07-26, two days before the new spec went final — implements
`2025-11-25`. The `2026-07-28` revision removes sessions from the protocol layer, retires the
`initialize` handshake and the `Mcp-Session-Id` header, deprecates the legacy HTTP+SSE transport on a
twelve-month offramp, and hardens authorization through six SEPs. Phase 4 should be planned against
`2025-11-25` because that is what the SDK ships, and should say so out loud rather than discovering
it during execution.

**Two: `rust-mcp-sdk` does not give Talaria an OAuth authorization server.** `SPEC.md:106` records
the resolved decision that OAuth would be "built on the **rust-mcp-sdk** crate … rather than
hand-rolled, which already provides OAuth metadata discovery, CIMD/DCR, PKCE, and token refresh."
That list is real, and it is the **client** half. The server half is `RemoteAuthProvider`, which
*verifies tokens issued by somebody else* — Keycloak, WorkOS AuthKit, Scalekit — via JWKS or
introspection. The `OAuthProxy` that would sit closest to what the roadmap wants is documented in the
SDK's own README as "still in development, please use RemoteAuthProvider for now." The
authorization-server logic — PKCE verification, the code store, token minting, refresh rotation,
exact redirect matching, consent — is Talaria's to write. What the SDK *does* provide is the
plumbing around it: the `AuthProvider` trait is public, its `auth_endpoints()` method enumerates
`AuthorizationEndpoint`, `TokenEndpoint`, `RegistrationEndpoint`, `RevocationEndpoint`,
`IntrospectionEndpoint`, `AuthorizationServerMetadata` and `ProtectedResourceMetadata`, and
`rust-mcp-axum` routes every declared path straight to `handle_request`. So the decision is not
"SDK or hand-rolled" — it is "hand-rolled *inside* the SDK's routing and 401 machinery," which is a
much better place to be than hand-rolled from scratch, and a much worse place than the SPEC entry
implies.

**Three: the security boundary this phase creates is not the one Phase 2 built.** Phase 2's
control-socket peer-UID check (`crates/talaria-shell/src/control.rs:158`) works because a Unix domain
socket carries kernel-vouched peer credentials and lives at `0600` inside a `0700` per-UID directory.
A loopback TCP socket has neither property: `SO_PEERCRED` does not exist for TCP, and every local
account can connect to `127.0.0.1:PORT`. The token *is* the entire boundary the moment the listener
opens, and the listener sits inside a process holding an encrypted credential vault and a live
logged-in browsing session. That is the largest attack-surface increase in the project's history,
and it argues for a listener that is **off by default**, loopback-only in Phase 4, and turned on by
an explicit human action in the chrome.

**Primary recommendation:** Model Talaria as an OAuth 2.1 **Resource Server that co-hosts its own
Authorization Server** — which is what the spec permits and what the roadmap meant — host the
listener in `talaria-shell` on its own thread, issue **opaque** per-client tokens stored as SHA-256
hashes in a new plaintext-but-`0600` store, render the consent decision in **native chrome** rather
than in a page, and split the phase into six or seven plans instead of three.

---

## Architectural Responsibility Map

| Capability | Primary Tier | Secondary Tier | Rationale |
|------------|-------------|----------------|-----------|
| HTTP/Streamable-HTTP listener | Shell process (`talaria-shell`) | — | Only long-lived process; owns every store the auth layer needs; owns the chrome the consent prompt must render in |
| MCP protocol framing over HTTP | `rust-mcp-axum` + `rust-mcp-sdk` (server feature) | — | Same SDK already framing stdio; SC 1 requires an identical tool surface |
| Tool surface definition & dispatch | Shared library extracted from `talaria-mcp` | — | One definition, two transports — the only way SC 1 ("the same tool surface") can be structurally true rather than reviewed-for |
| Command execution against webviews | winit main thread (`Shared` / `App`) | — | Servo/winit/egui are main-thread only; unchanged from Phase 1 |
| Token verification (`verify_token`) | Shell, per HTTP request | — | Must read live store state so revocation lands on the next request |
| Token minting, PKCE verify, refresh rotation | Shell — new `oauth` module | — | Hand-written; SDK provides routing only |
| Token persistence | Shell — new `agents.rs` store | — | Follows the Phase 3 store pattern (atomic `.tmp`+rename, `0600`, hand-mapped JSON) |
| Consent decision (Approve/Deny) | egui chrome panel | HTML holding page on the loopback listener | The human is the trust root; the decision must be unreachable from page JS and from `evaluate` |
| Connected-agents list + revoke | egui chrome panel (fifth `ChromePanel`) | — | Reuses the Phase 3 panel mechanism verbatim |
| stdio transport | `talaria-mcp` binary, unchanged | — | Spec: STDIO implementations **SHOULD NOT** follow the authorization spec |
| Peer authentication on the Unix control socket | `control.rs` peer-UID check, unchanged | — | Still correct and still load-bearing; see [Q5](#q5-keeping-stdio-unauthenticated-without-creating-a-bypass) |

---

## Project Constraints (from CLAUDE.md and PROJECT.md)

These are directives, not preferences. Any plan that contradicts one is wrong.

| # | Constraint | Source | Consequence for this phase |
|---|-----------|--------|----------------------------|
| C-1 | `Cargo.lock` is load-bearing; a fresh resolution pulls `primeorder 0.14.0` final and breaks `p256/p384/p521 0.14.0-rc.14`. Recovery: `cargo update -p primeorder --precise 0.14.0-rc.14` | `.claude/CLAUDE.md`, `Cargo.toml:15-38`, `SPEC.md:98` | Adding an HTTP stack **will** touch the lock. Must be a named task with an explicit verification step, not a footnote |
| C-2 | `talaria-shell` depends on `serde_json` but **not** `serde` — no derive available | `crates/talaria-shell/Cargo.toml`, `settings.rs:73-80` | New on-disk types in the shell get hand-written `to_json`/`from_json`. Confirmed: `talaria-mcp` and `talaria-protocol` *do* have `serde` with `derive` |
| C-3 | No `anyhow`/`thiserror`; `main` returns `Box<dyn Error>`, everything else is `io::Result` / `Option` / `Outcome::Error { message }` | `.claude/CLAUDE.md` | The OAuth error type is a plain enum or a `String` message, not a `thiserror` derive |
| C-4 | Zero `unwrap()` in `app.rs`; `expect()` only for genuine startup invariants | `.claude/CLAUDE.md` | Token parsing, header parsing and JSON decoding in the auth path must all degrade to a 400/401, never panic |
| C-5 | Degrade, never abort, on user-data problems | `vault.rs` wholesale | A corrupt `agents.json` yields an empty token set (every agent must re-authorize) — never a failed startup, and never an *empty allow* |
| C-6 | Servo/winit/egui are main-thread only; the only routes onto the loop are `EventLoopProxy::send_event` and the control socket | `.claude/CLAUDE.md`, `app.rs:110-122` | The HTTP server is off-thread and reaches the loop by `AppEvent`, exactly as `control.rs` does |
| C-7 | Never bind to `0.0.0.0`; dev/listening services use loopback plus a `tailscale0`-scoped forward | `~/.claude/CLAUDE.md` (global) | Phase 4 binds `127.0.0.1` only. Tailnet exposure is Phase 5's problem and must bring TLS with it |
| C-8 | Do not write code inline into documentation; create referenced executable scripts | `~/.claude/CLAUDE.md` (global) | Snippets below are illustrative excerpts, not the deliverable |
| C-9 | Log all changes in `CHANGELOG.md` | `~/.claude/CLAUDE.md` (global) | `CHANGELOG.md` exists; the phase close must add AUTH-01/02/03 entries |
| C-10 | A PLAN.md up front is **required** for work touching the security or protocol surface | `PROJECT.md`, Planning artifact policy | Every plan in this phase touches it. All get a PLAN.md, no exceptions |
| C-11 | Talaria is infrastructure, not a policy layer — no per-agent permission scoping, rate limiting, or audit trails | `PROJECT.md`, `SECURITY.md` | Scopes exist because the spec needs them, but Phase 4 ships **one** scope. Do not build a permission matrix |
| C-12 | No telemetry, no cloud dependency; vault and session state are plain local files | `PROJECT.md` | Rules out delegating auth to a hosted IdP — see the fork below |
| C-13 | LSP tools before grep for codebase navigation | `~/.claude/CLAUDE.md` (global) | Execution-time guidance for the planner's tasks |

---

## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| AUTH-03 | An HTTP/SSE MCP transport exists alongside stdio — *Must Have* | [Standard Stack](#standard-stack): `rust-mcp-axum 1.0.1` + `rust-mcp-sdk` `streamable-http` feature; [Q2](#q2-transport--who-hosts-the-listener) settles shell-vs-proxy; the SDK's `AxumServerOptions` defaults to `127.0.0.1:8080` with DNS-rebinding protection on |
| AUTH-01 | OAuth 2.1 authorization server for the MCP endpoint (Authorization Code + PKCE, per-client tokens) — *Must Have* | [The Architectural Fork](#the-architectural-fork) establishes the RS+AS model the spec requires; [Q3](#q3-token-lifecycle) sets format/storage/expiry/rotation; the `AuthProvider` trait is the integration seam |
| AUTH-02 | A user can view and individually revoke a connected agent's access — *Should Have* | [Q3](#q3-token-lifecycle) (revocation lands on the next request because `verify_token` runs per-request) and [Q4](#q4-the-authorization-prompt) (fifth `ChromePanel`); the open-stream gap is called out as its own task |

---

## The Architectural Fork

The roadmap says Talaria is **"protected by its own OAuth 2.1 authorization server."** The current
spec says an MCP server is an **OAuth 2.1 resource server**. Those are not in conflict, but the
roadmap's wording hides four `MUST`s, and it names a mechanism (DCR) the spec has since deprecated.
Here is the fork drawn plainly.

### What the spec actually requires

From the normative authorization document, revision `2026-07-28`, and identically in `2025-11-25`
except where noted [CITED: modelcontextprotocol.io/specification/2026-07-28/basic/authorization]:

> "A protected *MCP server* acts as an OAuth 2.1 resource server… The *authorization server* is
> responsible for interacting with the user (if necessary) and issuing access tokens for use at the
> MCP server. The implementation details of the authorization server are beyond the scope of this
> specification. **It may be hosted with the resource server or a separate entity.**"

So co-hosting is explicitly permitted. What is *not* optional:

| Requirement | Level | Who | Note |
|---|---|---|---|
| Implement RFC 9728 Protected Resource Metadata, including `authorization_servers` | **MUST** | MCP server (RS) | The roadmap does not mention this at all |
| Advertise it via `WWW-Authenticate: Bearer resource_metadata="…"` on 401, **or** a well-known URI | **MUST** (one of) | RS | `/.well-known/oauth-protected-resource` or `…/oauth-protected-resource/mcp` |
| Validate that the token's audience is this server (RFC 8707) | **MUST** | RS | Reject anything else; **MUST NOT** accept or transit other tokens |
| Return 401 for invalid/expired, 403 for insufficient scope, 400 for malformed | **MUST** | RS | Table is normative |
| Implement OAuth 2.1 | **MUST** | AS | PKCE `S256`, exact redirect-URI matching, no implicit grant, no password grant |
| Provide RFC 8414 AS metadata **or** OIDC Discovery | **MUST** (one of) | AS | `/.well-known/oauth-authorization-server`; **MUST** include `code_challenge_methods_supported` or conformant clients refuse to proceed |
| Rotate refresh tokens for public clients | **MUST** | AS | OAuth 2.1 §4.3.1 |
| Issue short-lived access tokens | SHOULD | AS | |
| Include the `iss` parameter in authorization responses (RFC 9207) | **SHOULD** (`2026-07-28` only; expected to become MUST) | AS | Requires `authorization_response_iss_parameter_supported: true` in metadata if emitted |
| Support Client ID Metadata Documents | SHOULD | AS + client | New preferred registration mechanism |
| Support Dynamic Client Registration (RFC 7591) | **MAY**, and **deprecated** | AS + client | "retained for backwards compatibility" |
| Clearly display the redirect URI hostname during authorization | **MUST** | AS | Directly shapes the consent screen |
| Display additional warnings for `localhost`-only redirect URIs | SHOULD | AS | Ditto |
| Serve all AS endpoints over HTTPS | **MUST**, *except* loopback redirect URIs which **MAY** use `http` | AS | OAuth 2.1 §1.5. This is why Phase 4 must stay on `127.0.0.1` |
| STDIO implementations follow this spec | **SHOULD NOT** | — | SC 4 is not a compromise; it is the spec's own instruction |

### Option A — Delegate to an external authorization server (what the SDK gives you free)

Implement `RemoteAuthProvider` against Keycloak, WorkOS AuthKit or Scalekit. `rust-mcp-extra` ships
drop-in providers. Talaria writes almost no OAuth code.

**Cost:** a browser whose stated positioning is *no cloud dependency, no telemetry, local plain
files* would now require either a SaaS identity provider or a locally-run Keycloak — a JVM service
larger than Talaria itself — before an agent can connect. It directly contradicts C-12. It also
fails the product's own single-user premise: there is no organisation to model, no directory, and
nobody to authenticate *as*.

**Verdict: reject.** Worth stating in the plan so it is visibly considered and closed, because it is
the path of least engineering resistance and someone will propose it.

### Option B — Co-hosted RS + AS (recommended)

Talaria serves, from one loopback listener:

```
/mcp                                        Streamable HTTP MCP endpoint (protected)
/.well-known/oauth-protected-resource       RFC 9728  — RS role
/.well-known/oauth-authorization-server     RFC 8414  — AS role
/authorize                                  AS — parks the request, raises consent in chrome
/token                                      AS — code→token with PKCE S256, refresh rotation
/register                                   AS — RFC 7591 DCR
/revoke                                     AS — RFC 7009
```

Every one of those paths is already an `OauthEndpoint` variant the SDK's `AuthProvider` trait knows
how to route, and `rust-mcp-axum`'s `auth_routes.rs` folds each declared endpoint into the router
automatically [VERIFIED: rust-mcp-sdk source, `crates/rust-mcp-sdk/src/auth/auth_provider.rs`,
`crates/rust-mcp-axum/src/routes/auth_routes.rs`, fetched 2026-08-20].

**What this costs that the roadmap does not budget:** the RFC 9728 document, the RFC 8414 document,
the 401 challenge, audience validation, the authorization-code store with single-use semantics and
short expiry, PKCE `S256` verification, refresh-token rotation with reuse detection, exact
redirect-URI matching with the RFC 8252 loopback-port exception, and a consent surface. That is
roughly the size of Phase 3's entire four-plan scope, and it is only two of this phase's three
requirements.

**Verdict: recommended.** It is what the roadmap meant, it is spec-conformant, and it keeps Talaria
dependency-free in the way the project's positioning requires.

### Recommended correction to Success Criterion 2

Current: *"A new agent client can complete the Authorization Code + PKCE flow against Talaria's own
authorization server and receive a working token."*

Recommended: *"A new agent client, given only Talaria's MCP endpoint URL, receives a 401 carrying
`WWW-Authenticate: Bearer resource_metadata=…`, discovers Talaria's authorization server through the
protected-resource metadata document, completes Authorization Code + PKCE (S256) with the `resource`
parameter set to Talaria's canonical URI, and receives an access token that Talaria accepts on `/mcp`
and rejects on any other audience."*

The change is not pedantry. The original wording lets a plan be written that mints a token and never
implements discovery or audience binding — and discovery is the only reason a real MCP client can
find the flow at all, while audience binding is what makes the token not usable elsewhere.

### On Dynamic Client Registration

The roadmap's 04-02 names DCR. As of `2026-07-28`, DCR is deprecated in favour of Client ID Metadata
Documents [CITED: .../authorization/client-registration]. But CIMD requires the *client* to host a
JSON document at an HTTPS URL and requires the *authorization server* to fetch that
attacker-supplied URL.

**Recommendation: implement DCR and pre-registration in Phase 4; do not implement CIMD.** Three
reasons, in order of weight:

1. **CIMD makes Talaria fetch an arbitrary URL supplied by an unauthenticated caller.** The spec
   itself flags this: authorization servers "**SHOULD** consider Server-Side Request Forgery (SSRF)
   risks." Talaria's process holds the credential vault and can reach the user's whole loopback
   space. An unauthenticated SSRF primitive inside the browser process is a strictly worse trade
   than shipping a deprecated-but-functional registration mechanism.
2. Agent clients here are local CLI processes. Many have no HTTPS origin to host a document at.
3. DCR remains "functional for backward compatibility" and is what current MCP clients actually do.

Concretely: advertise `registration_endpoint` in the AS metadata; **do not** advertise
`client_id_metadata_document_supported` (advertising a capability that is not implemented breaks
conformant clients that prefer it). Record CIMD as deferred, with the SSRF reasoning, in
`deferred-items.md` — the same treatment MCP-09 and MCP-10 got.

---

## Standard Stack

### Core

| Crate | Version | Purpose | Why standard |
|-------|---------|---------|--------------|
| `rust-mcp-sdk` | `1.0.1` (existing) — add features `streamable-http`, `auth` | MCP framing over HTTP; `AuthProvider` trait; 401/`WWW-Authenticate` middleware | Already the project's MCP SDK; newest release (published **2026-07-26**), not stale [VERIFIED: crates.io API] |
| `rust-mcp-axum` | `1.0.1` | The actual HTTP server: `create_axum_server`, `AxumServerOptions`, DNS-rebinding protection, `mcp_routes()` for BYO-server mounting | The SDK's own supported Axum backend, same repo, same maintainer |
| `axum` | `0.8` (via `rust-mcp-axum`) | HTTP router | Transitive; not a direct dependency |
| `axum-server` | `0.8` (via `rust-mcp-axum`) | Listener + graceful shutdown; carries the optional `tls-rustls` path | Transitive |

**Feature deltas to `Cargo.toml`:**

```toml
# workspace.dependencies — add streamable-http and auth to the existing line
rust-mcp-sdk = { version = "1.0.1", default-features = false,
                 features = ["server", "macros", "stdio", "streamable-http", "auth"] }
rust-mcp-axum = "1.0.1"
```

Deliberately **omit** `sse`. See [Pitfall 4](#pitfall-4-a-legacy-sse-stream-outlives-its-own-revocation).

`AxumServerOptions.sse_support` defaults to `true`; set it explicitly to `false`.

### What each feature actually pulls in

[VERIFIED: crates.io `/api/v1/crates/rust-mcp-sdk/1.0.1` feature table + dependency list, fetched 2026-08-20]

| Feature | Pulls |
|---------|-------|
| `streamable-http` | `rust-mcp-transport/streamable-http`, `http`, `http-body`, `http-body-util`, `tokio-stream` |
| `sse` | same set plus `rust-mcp-transport/sse` |
| `auth` | `url`, `jsonwebtoken/aws_lc_rs` (`^10.1`), `reqwest` (`^0.12`, `rustls-tls`+`json`+`cookies`+`multipart`+`stream`), `sha2` (`^0.11`) |

Note that neither transport feature pulls hyper or axum — the SDK is framework-agnostic and only
speaks `http` types. The server comes from `rust-mcp-axum`.

### The good news about the lockfile

Most of that stack is **already in `Cargo.lock`**, dragged in by Servo [VERIFIED: `grep` against the
committed `Cargo.lock`, 934 packages]:

| Crate | Already in lock | New? |
|-------|-----------------|------|
| `hyper` | `1.11.0` | no |
| `http` | `1.5.0` (and `0.2.12`) | no |
| `http-body` / `http-body-util` | `1.1.0` | no |
| `tower` | `0.5.3` | no |
| `tokio-stream` | `0.1.19` | no |
| `rustls` / `tokio-rustls` | `0.23.43` / `0.26.4` | no |
| `aws-lc-rs` / `aws-lc-sys` | `1.18.0` / `0.44.0` | no |
| `sha2` | `0.10.9` **and `0.11.0`** | no |
| `uuid`, `futures`, `thiserror`, `url`, `base64` | present | no |
| `axum`, `axum-server` | — | **yes** |
| `jsonwebtoken` | — | **yes** |
| `reqwest` | — | **yes** |

`sha2 0.11` already being resolved is a meaningful de-risking: the `auth` feature's most
pre-release-flavoured requirement is already satisfied by the pinned tree.

### Alternatives considered

| Instead of | Could use | Tradeoff |
|------------|-----------|----------|
| `rust-mcp-axum` | `rust-mcp-actix` | Adds actix-web, an entirely new async ecosystem, alongside the tokio tree Servo already carries. No |
| `rust-mcp-axum`'s `create_axum_server` | `mcp_routes()` + `McpMountOptions` (BYO server) | Only worth it if Talaria already had an HTTP server to mount into. It does not. Revisit in Phase 5 if the WebSocket protocol wants to share a port |
| `RemoteAuthProvider` | custom `impl AuthProvider` | See the fork. Custom is the recommendation |
| Opaque tokens | JWT via `jsonwebtoken` | See [Q3](#q3-token-lifecycle). JWT buys nothing here and costs an algorithm-confusion surface |
| `reqwest` (pulled by `auth`) | `ureq 2` (already a dependency) | Not a choice — `reqwest` arrives transitively with `auth`. Talaria will carry **two** HTTP clients. Worth an explicit note in the plan; the alternative is forgoing the `auth` feature and hand-writing the 401 middleware |

---

## Package Legitimacy Audit

The `gsd-tools query package-legitimacy` seam is unavailable in the installed build, so this was run
directly against the crates.io API on 2026-08-20 [VERIFIED: crates.io API].

| Package | Registry | First published | Total dl | Recent dl | Source repo | Verdict | Disposition |
|---------|----------|-----------------|----------|-----------|-------------|---------|-------------|
| `rust-mcp-sdk` | crates.io | 2025-03-29 | 238,133 | 103,210 | github.com/rust-mcp-stack/rust-mcp-sdk | OK | Already a dependency; approved |
| `rust-mcp-axum` | crates.io | **2026-06-24** (2 months) | **600** | 600 | github.com/rust-mcp-stack/rust-mcp-sdk | **SUS** (young, low downloads) | **Approved with justification** — same repository, same crates.io owner (`hashemix`) as the SDK already in use, and the SDK's own README names it as the supported backend. The low download count reflects the crate's age, not an unrelated publisher. No `checkpoint:human-verify` needed beyond noting it |
| `axum` | crates.io | 2021-07-22 | 432M | 107M | github.com/tokio-rs/axum | OK | Transitive; approved |
| `axum-server` | crates.io | 2021-08-20 | 32.7M | 8.6M | github.com/programatik29/axum-server | OK | Transitive; approved |
| `jsonwebtoken` | crates.io | 2015-11-02 | 172M | 42.6M | github.com/Keats/jsonwebtoken | OK | Transitive via `auth`; approved |
| `reqwest` | crates.io | 2016-10-16 | 654M | 164M | github.com/seanmonstar/reqwest | OK | Transitive via `auth`; approved |

**Packages removed due to `SLOP` verdict:** none.
**Packages flagged `SUS`:** `rust-mcp-axum` — cleared on provenance (same repo/owner as an existing
verified dependency). The planner should still name it in the plan rather than letting it arrive
silently.

Version-pin notes: the SDK constrains `jsonwebtoken` to `^10.1` (newest published is `11.0.0`) and
`reqwest` to `^0.12` (newest is `0.13.4`). Resolution will therefore take the older major lines. This
is the SDK's constraint, not drift, but it means two extra major-version trees.

---

## Architecture Patterns

### System architecture diagram

```
                            ┌───────────────────────────────────────────────────┐
  ┌──────────────┐  stdio   │              talaria-mcp  (unchanged)             │
  │  local MCP   ├─────────►│  ServerHandler → tools::dispatch → ShellConnection │
  │  host (IDE)  │          └──────────────────────┬────────────────────────────┘
  └──────────────┘                                 │ newline-JSON over
                                                   │ $XDG_RUNTIME_DIR/talaria.sock
                                                   │ (0600 in a 0700 dir, peer-UID checked)
                                                   ▼
  ┌──────────────┐   HTTP    ╔══════════════════════════════════════════════════════════════╗
  │ remote MCP   │  Bearer   ║                      talaria-shell process                   ║
  │ agent client ├──────────►║                                                              ║
  └──────┬───────┘  127.0.0.1║  thread: talaria-http                thread: talaria-control ║
         │                   ║  ┌───────────────────────────┐      ┌────────────────────┐   ║
         │ opens system      ║  │  axum (rust-mcp-axum)     │      │  UnixListener      │   ║
         │ browser at        ║  │   /mcp   ──────┐          │      │   peer_uid_ok()    │   ║
         ▼ /authorize        ║  │   /.well-known/*          │      └─────────┬──────────┘   ║
  ┌──────────────┐           ║  │   /authorize /token       │                │              ║
  │ user's       ├──────────►║  │   /register  /revoke      │                │              ║
  │ browser      │           ║  └──────┬─────────┬──────────┘                │              ║
  └──────────────┘           ║         │         │                           │              ║
                             ║   AuthProvider    │  AgentRequest             │ AgentRequest ║
                             ║   ::verify_token  │                           │              ║
                             ║         │         ▼                           ▼              ║
                             ║         │   ┌───────────────────────────────────────────┐    ║
                             ║         │   │  EventLoopProxy<AppEvent>  (the ONLY way   │   ║
                             ║         │   │  off-thread work reaches the engine)       │   ║
                             ║         │   └───────────────────┬───────────────────────┘    ║
                             ║         │                       ▼                            ║
                             ║  ┌──────▼──────┐   ┌────────────────────────────────────┐    ║
                             ║  │  agents.rs  │   │      winit main thread              │   ║
                             ║  │ token store │◄──┤  Shared: tabs, sessions, vault,     │   ║
                             ║  │ (hashes,    │   │  history, bookmarks, settings,      │   ║
                             ║  │  0600)      │   │  downloads, pending queues          │   ║
                             ║  └─────────────┘   │      │                    │          │  ║
                             ║                    │      ▼                    ▼          │  ║
                             ║                    │   Servo WebViews      egui chrome    │  ║
                             ║                    │   (one per tab)       ├ Consent panel│  ║
                             ║                    │                       └ Access panel │  ║
                             ║                    └────────────────────────────────────┘    ║
                             ╚══════════════════════════════════════════════════════════════╝
```

Trace the primary use case (a remote agent calls `evaluate`): request arrives at `/mcp` with a
Bearer token → `AuthProvider::verify_token` looks the hash up in `agents.rs` → on success the SDK
hands the framed `tools/call` to the handler → the handler builds a `Command::Evaluate` and an
`AgentRequest`, and sends `AppEvent::Agent(request)` down the `EventLoopProxy` → the main loop
executes it against the webview and resolves the `oneshot` → the HTTP handler awaits that `oneshot`,
bounded by the same command timeout `control.rs` uses, and writes the response. On revocation, the
next request's `verify_token` misses and returns 401 while every other client's lookup still hits.

### Recommended module layout

```
crates/talaria-mcp/
├── src/
│   ├── lib.rs         # NEW — re-exports tools + a transport-agnostic dispatch
│   ├── main.rs        # stdio binary, now a thin consumer of lib.rs
│   ├── socket.rs      # unchanged
│   └── tools.rs       # moved under lib.rs; dispatch takes a `CommandSink` not a ShellConnection
crates/talaria-shell/src/
│   ├── http.rs        # NEW — thread spawn, AxumServerOptions, shutdown, port reporting
│   ├── oauth.rs       # NEW — impl AuthProvider; the seven endpoint handlers
│   ├── agents.rs      # NEW — token/client store, the Phase 3 store pattern
│   └── gui.rs         # ChromePanel gains Consent + Access; UiAction gains the approve/deny/revoke intents
```

### Pattern 1: One tool surface, two transports

**What:** extract `talaria-mcp`'s tool structs and `dispatch` into a library, and make `dispatch`
generic over how a `Command` reaches the shell.

**Why:** SC 1 says an HTTP client must "drive the same tool surface." Today `tools.rs` hard-codes
`&ShellConnection` (`crates/talaria-mcp/src/tools.rs:127`). If the shell grows a second copy, the two
surfaces drift and SC 1 becomes something a human has to re-verify every phase instead of something
the type system holds.

```rust
// crates/talaria-mcp/src/lib.rs  (sketch)
#[async_trait::async_trait]
pub trait CommandSink: Send + Sync {
    async fn request(&self, client: &str, command: Command) -> Result<Outcome, String>;
}

// The existing socket.rs impl — no behaviour change for stdio.
#[async_trait::async_trait]
impl CommandSink for ShellConnection {
    async fn request(&self, client: &str, command: Command) -> Result<Outcome, String> {
        ShellConnection::request(self, client, command).await
    }
}

pub async fn dispatch(
    sink: &dyn CommandSink,
    client: &str,
    tool: TalariaTools,
) -> Result<CallToolResult, CallToolError> { /* body moves verbatim from tools.rs */ }
```

The shell's implementation sends `AppEvent::Agent(AgentRequest { .. })` down its `EventLoopProxy` and
awaits the `oneshot` — the same shape `control.rs:230-260` already uses, including the
`TALARIA_COMMAND_TIMEOUT_SECS` bound.

**Anti-pattern this avoids:** the shell connecting to its own Unix control socket to talk to itself.
That would work, and it would double every round trip, re-enter the peer-UID check pointlessly, and
put a self-deadlock in reach.

### Pattern 2: The auth surface is a trait impl, not a fork of the SDK

The `AuthProvider` trait is small enough to quote in full intent:

```rust
#[async_trait]
pub trait AuthProvider: Send + Sync {
    async fn verify_token(&self, access_token: String) -> Result<AuthInfo, AuthenticationError>;
    fn required_scopes(&self) -> Option<&Vec<String>> { None }
    fn auth_endpoints(&self) -> Option<&HashMap<String, OauthEndpoint>>;
    async fn handle_request(&self, request: http::Request<&str>, state: Arc<McpAppState>)
        -> Result<http::Response<GenericBody>, McpHttpError>;
    fn endpoint_type(&self, request: &http::Request<&str>) -> Option<&OauthEndpoint> { /* path lookup */ }
    fn protected_resource_metadata_url(&self) -> Option<&str>;
    fn validate_allowed_methods(&self, endpoint: &OauthEndpoint, method: &Method)
        -> Option<http::Response<GenericBody>> { /* 405 for wrong verbs */ }
}
```

[VERIFIED: `crates/rust-mcp-sdk/src/auth/auth_provider.rs`, fetched from GitHub `main` 2026-08-20]

`AuthInfo` carries `token_unique_id`, `client_id`, `user_id`, `scopes`, `expires_at`, `audience`
(an RFC 8707 resource identifier) and a free-form `extra` map [VERIFIED: `auth/auth_info.rs`]. That
is exactly the shape a per-client token store needs to return, which is a good sign the trait was
designed for this.

`OauthEndpoint`'s variants (`AuthorizationEndpoint`, `TokenEndpoint`, `RegistrationEndpoint`,
`RevocationEndpoint`, `IntrospectionEndpoint`, `AuthorizationServerMetadata`,
`ProtectedResourceMetadata`) are what `rust-mcp-axum` folds into routes. Declare only the ones
Talaria implements — an endpoint declared but unimplemented is a 500 waiting to happen, and
`IntrospectionEndpoint` in particular is unnecessary when the AS and RS are the same process.

### Pattern 3: Off-thread listener, main-thread execution

`control::spawn` is the exact precedent (`crates/talaria-shell/src/control.rs:78-88`): a named
`std::thread::Builder`, a tokio runtime built inside it, `block_on(serve(proxy))`.

**Recommendation: a second named thread, `talaria-http`, not a second task on the control thread.**
Two reasons. The control thread's runtime is `new_current_thread`, and the control socket must never
be starved by an HTTP request — it is the path a *local* user's tooling and the single-instance
launcher both depend on. And `rust-mcp-axum` depends on `tokio` with `features = ["full"]` and
expects to own its listener; giving it `new_multi_thread` in its own thread is the least surprising
arrangement.

The thread must be spawned **conditionally**, only when the listener is enabled in settings, so a
default install pays nothing.

### Anti-patterns to avoid

- **Making the HTTP handler talk to the shell over the Unix socket.** Two hops, a self-connection,
  and a peer-UID check against yourself. Use `EventLoopProxy` directly (Pattern 1).
- **Caching the token→identity mapping at connection setup.** SC 3 requires the *next request* to
  fail after a revoke. `verify_token` runs per request by design; keep the store as the live source
  of truth and do not memoise it behind the request boundary.
- **Rendering the Approve button in the HTML served at `/authorize`.** That page is loadable in a
  Talaria tab, and an agent with `evaluate` can drive a Talaria tab. See [Q4](#q4-the-authorization-prompt).
- **Reusing the credential vault for tokens.** See [Q3](#q3-token-lifecycle).
- **`unwrap()` anywhere on the request path.** A malformed `Authorization` header is attacker input.
  `app.rs` has zero `unwrap()`; the new modules must match (C-4).
- **Binding `0.0.0.0`, or "temporarily" binding it for a test.** C-7.
- **Blindly running `cargo update`.** C-1.

---

## Don't Hand-Roll

| Problem | Don't build | Use instead | Why |
|---------|-------------|-------------|-----|
| MCP framing over HTTP, session ids, resumability, DNS-rebinding protection, body-size limits | A custom JSON-RPC-over-HTTP handler | `rust-mcp-axum::create_axum_server` | `AxumServerOptions` already carries `dns_rebinding`, `max_request_body_size` (4 MiB default), `session_store`, `event_store`, ping interval and graceful shutdown |
| The 401 challenge, `WWW-Authenticate` construction, endpoint routing, 405 handling, scope-based 403 | A tower layer of your own | `impl AuthProvider` + `AuthMiddleware` | The SDK does the mechanical half; you supply only `verify_token` and `handle_request` |
| PKCE `S256` verification | A bespoke digest comparison | `sha2` (already in the lock) + constant-time compare | The *algorithm* is three lines; the danger is a non-constant-time compare and accepting `plain`. Reject any `code_challenge_method` other than `S256` |
| Random token generation | Anything seeded from time or a PRNG | `rand::rngs::OsRng` — already used by `vault.rs` for keys and nonces | The vault's precedent is right there |
| Password/credential storage for the resource owner | A Talaria login | Nothing — there is no login | Single-user local app. The human at the keyboard *is* the resource owner and is already authenticated by having the window. Do not invent an account system |
| TLS termination | Hand-rolled rustls wiring | `rust-mcp-axum`'s `ssl` feature — **and not in Phase 4** | Phase 4 is loopback-only, where OAuth 2.1 §1.5 permits `http`. Phase 5 needs TLS; let Phase 5 own it |
| A permissions matrix | Per-tool scopes | One scope | C-11. `SECURITY.md` states outright that per-agent permission scoping is out of scope by design |

**Key insight:** almost everything mechanical in this phase has a home. What has *no* library answer
is the part specific to Talaria — the consent surface, the token store, and the revocation semantics.
That is exactly where the plan's effort should go, and it is exactly what the roadmap's three-plan
split under-budgets.

---

## Design Decisions and Recommendations

### Q1 — see [The Architectural Fork](#the-architectural-fork)

### Q2: Transport — who hosts the listener?

**Recommendation: `talaria-shell` hosts it, on a dedicated `talaria-http` thread. One binary keeps
its job, and no new binary is created.**

The reasoning, in order of weight:

1. **The shell is the only long-lived process.** `talaria-mcp` is spawned per-connection by an MCP
   host over stdio; there is no single instance of it to own a network listener, and there is no
   supervisor for one.
2. **The shell already owns every piece of state auth needs.** Tokens must persist across
   restarts alongside the four Phase 3 stores. The consent prompt must render in the chrome. The
   revoke button must live in a `ChromePanel`. Putting the listener anywhere else means shipping all
   three of those *back* across a process boundary.
3. **Spawning and supervising a child process is a bigger architectural change than adding a
   thread.** Decision 03-04 established that `UiAction::OpenDownload` (`xdg-open`) is the browser's
   *only* process spawn and is deliberately unreachable from the agent surface. A supervised
   long-lived child would be the second, and it would be started by a settings toggle.
4. **The proxy is stateless by design** (`crates/talaria-mcp/src/socket.rs` header) and should stay
   that way. Holding tokens would make it stateful and would split the security boundary in two.

**Cost, stated honestly:** `talaria-shell` grows `rust-mcp-sdk` (server + streamable-http + auth +
macros) and `rust-mcp-axum`, on top of Servo. Build time and binary size both go up in a workspace
whose `target/` is already ~42 GB. Mitigate by gating nothing at compile time (a cargo feature on
the shell would fragment CI) but by *not* spawning the thread when the listener is disabled.

**Alternative, for the record:** put the listener in a `talaria-mcp --serve` mode that proxies to the
shell over the existing Unix socket, with the shell owning the token *file* and the proxy reading it.
This keeps Servo's crate free of axum. It fails on lifecycle (who starts it, and what tells the user
it died), on the consent round trip (a new control-socket command purely to raise a UI panel), and on
revocation latency (the proxy would have to re-read the file per request anyway, at which point the
process split buys nothing). Recommend rejecting, but say so in the plan.

**One binary or two?** Two, unchanged. `talaria-mcp` remains the stdio binary. Adding an HTTP mode to
it would be the alternative above.

### Q3: Token lifecycle

**Format — opaque random, 32 bytes from `OsRng`, not a JWT.** Four reasons:

1. SC 3 requires revocation to bite on the next request. A self-contained JWT cannot be revoked
   without a denylist lookup — at which point you have paid for the JWT and still do the lookup.
2. AS and RS are the same process. There are no verification keys to distribute, which is the only
   thing JWTs are genuinely good at.
3. `AuthProvider::verify_token(String) -> AuthInfo` is a lookup hook. Opaque fits it exactly.
4. It sidesteps the entire JWT algorithm-confusion / `alg: none` / key-confusion class. `jsonwebtoken`
   arrives transitively with the `auth` feature; Talaria simply need not call it.

**Storage — a new `crates/talaria-shell/src/agents.rs`, storing only the SHA-256 of each token.**

- **Separate from the vault, not inside it.** The vault holds the *human's passwords* under a
  keychain-derived key, and it has a documented startup hang on machines with no session D-Bus
  (`Vault::load()`, tracked as a candidate v1 blocker in `STATE.md`). Coupling agent authorization to
  that failure mode means "no D-Bus" becomes "no agent can connect." Separately: `cookies_read`
  reaches the vault from the agent tool surface. Tokens must not live behind a door an agent already
  has a key to.
- **Hash-only means the file needs no encryption.** A stolen `agents.json` yields hashes, not
  tokens. That is the standard argument and it is what lets this store skip the keychain entirely.
- **Follow the Phase 3 store pattern exactly:** `permissions::create_dir_owner_only` for the
  directory, atomic write staged into a `.tmp` sibling and `fs::rename`d into place (the shape
  `bookmarks.rs` and `settings.rs` use — *not* `vault.rs`'s plain `fs::write`),
  `permissions::write_owner_only` on the staged file so `0600` is the mode that lands, hand-written
  `to_json`/`from_json` through `serde_json::Value` (C-2), unit tests at `vault.rs` density.
- **Degrade, never abort** (C-5): a corrupt file yields an empty client set — every agent must
  re-authorize. Never an empty *allow*.

**Per-client identity.** Registration (DCR) mints a `client_id` and records `client_name`,
`redirect_uris`, and the time of first authorization. This is the project's **first non-self-asserted
client identity** — `ClientMessage::Hello { client }` and the resulting `Session.client` string are
whatever the peer typed (`crates/talaria-protocol/src/lib.rs:78`, `app.rs:87`). Worth stating in the
plan, because the Agents view currently labels sessions with that self-asserted string and the new
panel must not.

**Expiry and refresh.** Access tokens: short — one hour is a defensible default and matches the
spec's SHOULD. Refresh tokens: rotated on every use (**MUST**, OAuth 2.1 §4.3.1, public clients).
Implement reuse detection — presenting an already-consumed refresh token revokes the whole token
family, which is the standard mitigation and is cheap when the store is local. Authorization codes:
single-use, ~60 second lifetime, bound to `client_id` + `code_challenge` + `redirect_uri`.

**Revocation reaching an in-flight connection.** The spec is explicit that authorization "**MUST** be
included in every HTTP request from client to server, even if they are part of the same logical
session." So for ordinary request/response traffic, revocation lands on the next request for free —
*provided* `verify_token` reads live state. That is the whole mechanism behind SC 3 and it is
almost too easy.

**The part that is not free:** a Streamable HTTP response can be a long-lived `text/event-stream`. A
revoked client's already-open stream has no next request to fail. **Revocation must therefore also
terminate that client's open streams.** Keep a `token_hash → live session handles` map and drop the
handles on revoke, then assert stream closure in the e2e suite. This deserves its own task; it is
the single most likely way SC 3 gets marked done while being false.

### Q4: The authorization prompt

**Recommendation: the HTTP endpoint serves a holding page; the Approve/Deny decision happens in
native egui chrome.**

The flow:

1. Client generates PKCE params and opens the system browser at
   `http://127.0.0.1:PORT/authorize?client_id=…&code_challenge=…&redirect_uri=http://127.0.0.1:EPHEMERAL/callback&resource=…&state=…`.
   RFC 8252 requires native apps to use an **external user-agent**, not an embedded webview, so this
   step is a real browser and cannot be collapsed into the chrome.
2. `/authorize` validates the request (client known, redirect URI matches registration modulo the
   loopback port, `code_challenge_method=S256`, `resource` is Talaria's canonical URI), parks it
   keyed by a random request id, and raises an `AppEvent` that opens `ChromePanel::Consent` in the
   Talaria window.
3. The HTTP response is a **static holding page** — "Approve this connection in the Talaria window" —
   carrying no secret and no control. It polls a status endpoint keyed by the request id.
4. The human sees, in the Talaria chrome: the client name (untrusted, and labelled as such), the
   **redirect URI hostname** (a spec **MUST**), the requested scope, and — because the redirect is
   loopback-only — the additional warning the spec SHOULDs for that case. Approve and Deny are egui
   buttons emitting `UiAction`s, applied in `apply_ui_actions` after egui's borrows drop.
5. On approve, the parked request completes: a single-use code is minted and the holding page is
   redirected to the client's loopback callback.

**Why not put the buttons in the page.** `http` is in the agent navigation allowlist
(`SECURITY.md`: `http`, `https`, `data`, `about:blank`), so an agent with `evaluate` can open
`http://127.0.0.1:PORT/authorize` in a tab it owns and script it. A page-hosted Approve button would
be clickable by the very party asking for permission. Keeping the decision in native chrome makes it
structurally unreachable from page JavaScript, from `evaluate`, and from the control socket — the
same property that kept `chrome_rects` off the MCP tool surface and that makes autofill *suggest*
rather than inject.

**Additional hardening this implies:** `parse_agent_url` should refuse the listener's own
`host:port`. An agent that can navigate to Talaria's own auth endpoints can at minimum self-register
and enumerate metadata; there is no reason to allow it. This is a small, cheap, concrete change and
it belongs in the same plan as the listener.

**Feeds UI-SPEC.** Two panels, both new: `ChromePanel::Consent` (transient, raised by an event, not
by a toolbar button) and a fifth persistent panel for AUTH-02. **Name the latter
`ChromePanel::Connections` or `ChromePanel::Access`, not `Agents`** — `ViewMode::Agents` already
exists and means something different (tabs an agent is driving). Two things called "Agents" in one
chrome is a bug waiting to be filed.

`ChromePanel` and `UiAction` are the established mechanism (`crates/talaria-shell/src/gui.rs:33`,
`:42`); this is a fifth instance of a pattern that already has four, not new machinery.

### Q5: Keeping stdio unauthenticated without creating a bypass

**Confirmed: Phase 2's peer-UID check still holds, and it is still the right answer for the Unix
socket.** `peer_uid_ok` reads `SO_PEERCRED` via `stream.peer_cred()`, fails closed on a lookup error,
and returns a closed connection with no reply (`control.rs:158-171`). The socket is `0600` inside a
`0700` per-UID directory (`talaria_protocol::ensure_socket_dir`). An attacker who can reach it is
already running as the user — at which point they can read `~/.config/talaria/` directly, including
the vault key file when the keychain is unavailable. Adding OAuth in front of the Unix socket would
protect nothing that the filesystem does not already gate.

The spec agrees: *"Implementations using an STDIO transport **SHOULD NOT** follow this
specification, and instead retrieve credentials from the environment."* SC 4 is not a concession; it
is conformance.

**What is genuinely new and must be said in `SECURITY.md`:** the TCP listener has **no equivalent**
of the peer-UID check.

- `SO_PEERCRED` does not exist for TCP sockets.
- `127.0.0.1:PORT` is reachable by **every local user account**, unlike the `0700`-directory Unix
  socket. Loopback is not a privacy boundary between local users.
- Any web page in any browser on the machine can `fetch()` the endpoint. DNS-rebinding protection
  (on by default in `AxumServerOptions`, validating `Host`/`Origin`) covers the classic rebinding
  attack, and CORS stops a page *reading* the response — but neither is a substitute for the token.

**Therefore:** the token is the entire boundary; the listener is off by default; the port binds
`127.0.0.1` only; `allowed_hosts` is set explicitly; and requests carrying any `Origin` header should
be rejected outright, because a legitimate MCP CLI client sends none and a page always does.

`SECURITY.md`'s closing paragraph currently reads: *"The control socket has no authentication beyond
the peer-UID check, and needs none while the transport is a Unix domain socket owned by one user.
That changes the moment a network transport exists."* This phase is that moment. Updating
`SECURITY.md` is a deliverable, not a courtesy.

### Q6: Testing under a stdlib-only Python harness

Everything needed is in the standard library — verified on this machine (Python 3.14.6):

| Need | Stdlib module |
|------|---------------|
| HTTP requests to the listener | `http.client` / `urllib.request` |
| The client's loopback redirect receiver | `http.server.HTTPServer` on port 0 (ephemeral) |
| PKCE `S256` | `hashlib.sha256` + `base64.urlsafe_b64encode` |
| Reading a `text/event-stream` | raw `socket` + `select` — the exact technique `mcp_client_test.py` already uses on the stdio fd, and for the same reason (a buffered reader that has timed out once refuses every later read) |
| Query/form encoding, JSON | `urllib.parse`, `json` |

**New suites:** `http_transport_test.py` (SC 1 + SC 4), `oauth_flow_test.py` (SC 2),
`revocation_test.py` (SC 3, including the open-stream case).

**Configuration, not new hooks, wherever possible.** `settings.rs` reads `config.json` at startup and
`history_test.py` / `bookmarks_test.py` already establish seeding a store file before launch. The
harness should write `config.json` with the listener enabled at a **fixed** test port before starting
the shell. That avoids a hook purely to discover an ephemeral port.

**Drive consent through the real chrome, not a bypass.** `chrome_rects` + `wait_for_rect` +
`click_rect` already exist in `harness.py` and are exercised by `panel_click_test.py` and
`vault_ui_test.py`. Emit rects named `consent.approve`, `consent.deny`, `access.revoke.N` and click
them for real. This gives an end-to-end test of the actual human decision path and — crucially —
means **no auto-approve bypass ever ships**. An `TALARIA_AUTO_APPROVE=1` env var would be a
one-variable authentication bypass in a browser holding a credential vault; do not create one.

**Hooks that are genuinely needed**, all behind `TALARIA_TEST_HOOKS=1` and refused exactly the way
`ChromeRects` is refused (`app.rs:1906`):

- Rects for the two new panels (an extension of the existing hook, not a new one).
- Possibly a command reporting the listener's bound address, if the fixed-port approach proves
  flaky under parallel CI.

`panel_click_test.py:467` already asserts `chrome_rects` is refused without the hook; the new rect
names extend that assertion rather than replacing it.

**Harness note:** `run_all.py`'s Phase-1 shared shell runs with `TALARIA_COMMAND_TIMEOUT_SECS=3`. The
OAuth suites need their own `config.json` and therefore belong in the Phase-2 standalone list
(alongside `vault_ui_test`, `history_test`, and friends), each starting its own shell.

**Rust-side unit tests** carry the parts that need no browser: PKCE verification (including rejecting
`plain` and a mismatched verifier), code single-use and expiry, refresh rotation and reuse detection,
redirect-URI exact matching with the loopback-port exception, the `agents.rs` round trip, and the
`0600` mode assertion (`permissions::mode_of` exists for exactly this, `permissions.rs:143`).

### Q7: Scope — is the three-plan split right?

**No. Recommend six or seven plans.** For calibration: Phase 3 delivered four table-stakes features
in four plans. This phase delivers an HTTP server, an OAuth 2.1 resource server, an OAuth 2.1
authorization server, a token store, two chrome panels, and a revocation mechanism — and the roadmap
itself says of Phases 4–7 that "each is its own product" and "any one of them could take longer than
Phases 1–3 combined did."

Recommended split:

| Plan | Deliverable | Closes | Wave |
|------|-------------|--------|------|
| 04-01 | Extract the tool surface into `talaria-mcp/src/lib.rs` with a `CommandSink` trait. No behaviour change; stdio proxy untouched externally | — | 1 |
| 04-02 | Dependency + lockfile landing: add `streamable-http`/`auth` features and `rust-mcp-axum`; verify `primeorder 0.14.0-rc.14` survives; CI `--locked` green. **Explicitly a task, not a footnote** (C-1) | — | 1 |
| 04-03 | HTTP/Streamable-HTTP listener in the shell, off by default, loopback-bound, `sse_support: false`; `config.json` gains the toggle; Settings-panel control; `parse_agent_url` refuses the listener's own origin | AUTH-03, SC 1, SC 4 | 2 |
| 04-04 | `agents.rs` — client registry + hashed opaque token store, atomic `0600` writes, hand-mapped JSON, unit tests at `vault.rs` density. No HTTP in this plan | — | 2 |
| 04-05 | Resource-server half: `impl AuthProvider`, RFC 9728 protected-resource metadata, 401 + `WWW-Authenticate`, audience validation. The endpoint becomes protected | part of AUTH-01 | 3 |
| 04-06 | Authorization-server half: RFC 8414 metadata, DCR, `/authorize` + the chrome consent round trip, `/token` with PKCE `S256`, refresh rotation with reuse detection | AUTH-01, SC 2 | 4 |
| 04-07 | Revocation: `/revoke` (RFC 7009), the Connections/Access panel, **and terminating a revoked client's open streams** | AUTH-02, SC 3 | 5 |

04-01 and 04-02 can share a wave; everything after 04-03 is sequential because each builds on the
previous listener state.

**Also recommend running `/gsd-discuss-phase` before planning.** The fork in Q1, the decision to skip
CIMD, the default-off listener, and the consent-in-chrome call are all product decisions that a
CONTEXT.md should lock rather than leaving to a research recommendation the planner may or may not
follow.

---

## Common Pitfalls

### Pitfall 1: Regenerating `Cargo.lock` and silently breaking the build

**What goes wrong:** adding axum, `jsonwebtoken` and `reqwest` triggers a resolution that takes
`primeorder 0.14.0` final, which no longer satisfies the trait bounds `p256/p384/p521
0.14.0-rc.14` are written against. The build fails with an E0277 that names none of this.
**Why it happens:** the pin exists only inside the committed lockfile — nothing in
`[workspace.dependencies]` expresses it (`Cargo.toml:15-38`).
**How to avoid:** add dependencies with `cargo add` (which does a minimal update), then immediately
`grep 'name = "primeorder"' -A1 Cargo.lock` and confirm `0.14.0-rc.14`. If it drifted:
`cargo update -p primeorder --precise 0.14.0-rc.14`. Verify with `cargo build --release --locked`.
**Warning signs:** a lockfile diff touching hundreds of packages instead of a dozen; any E0277
mentioning `p256`, `p384`, `p521` or elliptic-curve traits.
**Good news:** `hyper`, `http`, `tower`, `rustls`, `aws-lc-rs`, `tokio-stream`, `uuid` and **`sha2
0.11.0`** are already resolved in the lock, so the new surface is smaller than it looks — genuinely
new are `axum`, `axum-server`, `jsonwebtoken` and `reqwest`.

### Pitfall 2: Assuming loopback is a security boundary

**What goes wrong:** the listener is treated as "local, therefore safe," and the token check is
made lax or the listener is left on by default.
**Why it happens:** the Unix control socket *is* safe by locality — but only because it is a
filesystem object at `0600` in a `0700` directory with kernel-vouched peer credentials. None of that
transfers to TCP.
**How to avoid:** default off. Explicit `127.0.0.1`. Explicit `allowed_hosts`. Reject requests with
an `Origin` header. Treat the token as the sole boundary and test the unauthenticated path returns
401 with no side effects.
**Warning signs:** any code path that skips `verify_token`; any "if it's from localhost" branch.

### Pitfall 3: Implementing the AS and forgetting the RS

**What goes wrong:** tokens mint and validate, but no `/.well-known/oauth-protected-resource`
document exists and 401s carry no `WWW-Authenticate`. A real MCP client cannot *find* the
authorization server and gives up before the flow starts.
**Why it happens:** the roadmap phrases the whole requirement as "its own OAuth 2.1 authorization
server," which does not mention the resource-server obligations at all.
**How to avoid:** implement `protected_resource_metadata_url()` and the metadata document in 04-05,
*before* the AS in 04-06, so the discovery path is provably first.
**Warning signs:** a test that hard-codes the token endpoint URL instead of discovering it. The e2e
suite should start from the MCP endpoint URL and nothing else.

### Pitfall 4: A legacy SSE stream outlives its own revocation

**What goes wrong:** SC 3 is marked done because a *new* request from the revoked agent gets a 401,
while its already-open event stream keeps delivering.
**Why it happens:** `verify_token` runs per request, and an open stream has no next request.
**How to avoid:** set `sse_support: false` (the `2026-07-28` spec deprecates the legacy HTTP+SSE
transport anyway, so this also removes future work), and for Streamable HTTP's own SSE responses,
keep a `token_hash → live handles` map and drop the handles on revoke. Write the test that asserts
the stream *closes*, not merely that the next POST fails.
**Warning signs:** a revocation test that only issues a fresh request.

### Pitfall 5: Accepting a PKCE `code_challenge_method` of `plain`

**What goes wrong:** an AS that accepts `plain` accepts a "verifier" equal to the challenge, which is
no protection at all.
**Why it happens:** RFC 7636 defines both; only `S256` is safe, and OAuth 2.1 requires `S256` "when
technically capable."
**How to avoid:** advertise `"code_challenge_methods_supported": ["S256"]` and reject anything else
with `invalid_request`. Compare the digest in constant time.
**Warning signs:** a `match method { "plain" => …` arm existing at all.

### Pitfall 6: Doing work inside a Servo delegate callback, or mutating shell state from an egui closure

Both are named anti-patterns in `.claude/CLAUDE.md` and both are reachable here: the consent panel is
egui, and completing a parked authorization touches `Shared`. Use the established `UiAction` round
trip (`gui.rs` returns a `Vec<UiAction>`; `apply_ui_actions` at `app.rs:775` performs them once
egui's borrows are released), and a pending queue for anything the HTTP thread parks. `UiAction`'s
doc comments already spell out why — see `SetPanel` and `ToggleBookmark`.

### Pitfall 7: Two things named "Agents"

`ViewMode::Agents` is the tab view. A `ChromePanel::Agents` holding authorized OAuth clients would be
a different concept with the same name in the same chrome. Name it `Connections` or `Access`.

### Pitfall 8: `client_name` is attacker-controlled and it is going on a consent screen

DCR lets a caller register any `client_name`. That string is then displayed to the human at the exact
moment they are deciding whether to grant browser access. This is the phishing surface of the whole
phase. Truncate it, strip control characters and newlines, render it in a visually distinct
"as claimed by the client" region, and display the redirect URI hostname beside it (a spec **MUST**).
Precedent exists: `03-04` refused to let a download's row "lie about its own name," and the
`DownloadEntry` comment records that the requesting session's label is "a string the agent wrote."
Same class of problem, same answer.

---

## Code Examples

### Enabling the listener (illustrative shape — not the deliverable, per C-8)

```rust
// crates/talaria-shell/src/http.rs — sketch
pub fn spawn(proxy: EventLoopProxy<AppEvent>, config: &RemoteTransport, auth: Arc<TalariaAuth>) {
    if !config.enabled {
        return; // Default install pays nothing: no thread, no listener, no port.
    }
    let (host, port) = (config.bind.clone(), config.port);
    std::thread::Builder::new()
        .name("talaria-http".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("http server runtime"); // startup invariant, per C-4
            runtime.block_on(async move {
                let server = rust_mcp_axum::create_axum_server(
                    server_details(),
                    handler.to_mcp_server_handler(),
                    rust_mcp_axum::AxumServerOptions {
                        host,                       // "127.0.0.1" — never 0.0.0.0 (C-7)
                        port,
                        sse_support: false,         // legacy transport is deprecated upstream
                        auth: Some(auth),           // Arc<dyn AuthProvider>
                        event_store: None,          // resumability off until it is designed
                        ..Default::default()        // DNS-rebinding protection stays on
                    },
                );
                if let Err(error) = server.start().await {
                    log::error!("remote MCP transport failed: {error}");
                }
            });
        })
        .expect("spawn http thread");
}
```

`AxumServerOptions` defaults to `host: "127.0.0.1"`, `port: 8080`, DNS-rebinding protection enabled
(auto-deriving `allowed_hosts` from `host:port` unless the bind is a wildcard), and a 4 MiB body cap
[VERIFIED: `crates/rust-mcp-axum/src/server.rs`, fetched from GitHub `main` 2026-08-20].

### The `AuthProvider` seam

```rust
#[async_trait]
impl rust_mcp_sdk::auth::AuthProvider for TalariaAuth {
    async fn verify_token(&self, access_token: String) -> Result<AuthInfo, AuthenticationError> {
        // Live lookup, never cached past the request boundary — this is what makes SC 3 work.
        let digest = sha256_hex(&access_token);
        self.store.lookup(&digest).ok_or(AuthenticationError::InvalidToken)
        // ... plus expiry and audience checks, both returning the same opaque error.
    }

    fn auth_endpoints(&self) -> Option<&HashMap<String, OauthEndpoint>> {
        Some(&self.endpoints) // "/authorize" => AuthorizationEndpoint, "/token" => TokenEndpoint, ...
    }

    fn protected_resource_metadata_url(&self) -> Option<&str> {
        Some(&self.resource_metadata_url) // absolute; goes into WWW-Authenticate on 401
    }

    async fn handle_request(&self, request: http::Request<&str>, state: Arc<McpAppState>)
        -> Result<http::Response<GenericBody>, McpHttpError> {
        match self.endpoint_type(&request) { /* dispatch to the seven handlers */ }
    }
}
```

### The house store pattern the token store must follow

```rust
// The shape settings.rs and bookmarks.rs use — serde derive is NOT available here (C-2).
fn to_json(&self) -> serde_json::Value {
    serde_json::json!({ "client_id": self.client_id, "token_sha256": self.token_sha256, /* ... */ })
}
fn from_json(value: &serde_json::Value) -> Option<Self> {
    Some(Self {
        client_id: value.get("client_id")?.as_str()?.to_owned(),
        token_sha256: value.get("token_sha256")?.as_str()?.to_owned(),
        // ...
    })
}
```

Save via a `.tmp` sibling written with `permissions::write_owner_only`, then `fs::rename` — the mode
of the *staged* file is the mode that lands, which `permissions.rs`'s own doc comment spells out.

---

## State of the Art

| Old approach | Current approach | When changed | Impact on this phase |
|---|---|---|---|
| MCP servers are "their own auth server, no external IdP needed" (`SPEC.md:106`) | MCP server is an OAuth 2.1 **resource server**; the AS may be co-hosted but the RS role and RFC 9728 discovery are **MUST**s | 2025-06 spec revision onward | Rewrites what AUTH-01 means; see the fork |
| Dynamic Client Registration is the registration mechanism | **Client ID Metadata Documents**; DCR deprecated, "retained for backwards compatibility" | `2026-07-28` | Roadmap 04-02 names DCR; recommend keeping it *and* documenting why CIMD is deferred |
| `initialize`/`initialized` handshake, `Mcp-Session-Id` header, stateful sessions | Stateless core; per-request `_meta` protocol version and `MCP-Protocol-Version` header; `server/discover` RPC | `2026-07-28` (SEP-2575, SEP-2567) | Not adopted this phase — `rust-mcp-sdk 1.0.1` implements `2025-11-25`. Say so explicitly in the plan |
| HTTP+SSE as a first-class transport | Streamable HTTP; legacy HTTP+SSE **deprecated** with a twelve-month offramp | `2026-07-28` | Set `sse_support: false`. AUTH-03's wording ("HTTP/SSE") predates this |
| No issuer validation on the authorization response | RFC 9207 `iss` parameter, SHOULD now and expected to become MUST | `2026-07-28` (SEP-2468) | Cheap to emit; advertise `authorization_response_iss_parameter_supported: true` if emitted |
| OAuth 2.0 implicit and password grants | Removed entirely in OAuth 2.1; PKCE mandatory for all clients | OAuth 2.1 draft | Only `authorization_code` and `refresh_token` need implementing |
| `rust-mcp-sdk` provides OAuth for servers (as `SPEC.md` records) | It provides *client* OAuth and *remote-IdP token verification* for servers; `OAuthProxy` "still in development" | Verified against the crate's own README and source, 2026-08-20 | The key premise correction of this research |

**Deprecated / outdated in this repo:**

- `SPEC.md:106`'s OAuth resolution — factually wrong about what `rust-mcp-sdk` gives the server side.
  Should be amended at phase close, not silently worked around.
- `ProtocolVersion::V2025_11_25` — correct for the SDK today, one revision behind the spec. Worth a
  `deferred-items.md` entry so the drift is tracked rather than discovered.
- `SECURITY.md`'s closing paragraph — accurate today, obsolete the moment 04-03 lands.

---

## Threat Model Inputs

ASVS level 1, blocking on high severity, per `.planning/config.json`. This is the raw material for the
planner's per-plan `<threat_model>` blocks.

### What changes about the trust model

`SECURITY.md` names three parties: the human (trust root), a connected agent (semi-trusted), and web
content (untrusted). This phase adds a fourth and changes one:

- **New: an unauthenticated network peer.** Anything that can open a TCP connection to the listener,
  including every other local user account and every page in every browser on the machine. It was
  previously impossible for such a party to exist.
- **Changed: "a connected agent" is no longer synonymous with "a process running as the user."** Over
  stdio it was, by the peer-UID check. Over HTTP it is whoever holds a token.

### Threat register

| ID | Threat | STRIDE | Severity | Standard mitigation | Where |
|----|--------|--------|----------|---------------------|-------|
| T-1 | Another local user connects to `127.0.0.1:PORT` and drives the browser | Spoofing / EoP | **High** | Bearer token required on every request; listener off by default; no localhost trust branch | 04-03, 04-05 |
| T-2 | A web page in any local browser `fetch()`es the MCP endpoint | Spoofing / Tampering | **High** | DNS-rebinding protection on; explicit `allowed_hosts`; reject any request carrying `Origin` | 04-03 |
| T-3 | An agent with `evaluate` navigates a tab to Talaria's own auth endpoints and self-registers or drives consent | EoP | **High** | `parse_agent_url` refuses the listener's own origin; the Approve control lives only in native chrome | 04-03, 04-06 |
| T-4 | Malicious `client_name` on the consent screen phishes the human into approving | Spoofing | **High** | Sanitise and truncate; render as untrusted claim; display the redirect URI hostname (spec MUST) | 04-06 |
| T-5 | Authorization code intercepted on the loopback callback by another local process racing the port | Spoofing | **High** | PKCE `S256` mandatory; single-use codes with ~60s expiry; exact redirect matching with the loopback-port exception only | 04-06 |
| T-6 | A token issued for another resource is replayed at Talaria (confused deputy / passthrough) | Spoofing | **High** | Audience validation against Talaria's canonical URI; **MUST NOT** accept or transit other tokens | 04-05 |
| T-7 | A revoked agent's open event stream keeps delivering | EoP | **High** | Terminate the client's live handles on revoke; `sse_support: false`; assert stream closure in e2e | 04-07 |
| T-8 | `agents.json` readable by other local users | Information Disclosure | Medium | `0600` via `permissions::write_owner_only`; `0700` directory; store only SHA-256 hashes | 04-04 |
| T-9 | Stolen refresh token replayed after the legitimate client rotated | Spoofing | Medium | Rotation on every use plus reuse detection revoking the family | 04-06 |
| T-10 | Access token leaked into logs | Information Disclosure | Medium | Never log tokens or `Authorization` headers; log the `client_id` and the hash prefix only | 04-05 |
| T-11 | Unbounded registration — an attacker fills the client registry | DoS | Medium | Cap registered clients; require the consent step before a client can *do* anything; a registration with no approved authorization is inert | 04-06 |
| T-12 | SSRF: the AS fetches an attacker-supplied HTTPS URL (CIMD) | Tampering / EoP | **High if built** | **Do not implement CIMD in Phase 4.** Deferring it removes the primitive entirely | (deferred) |
| T-13 | HTTP thread starves or deadlocks the control thread or the main loop | DoS | Medium | Separate named thread with its own runtime; every request bounded by `TALARIA_COMMAND_TIMEOUT_SECS`; `oneshot::is_closed()` cancellation as established in 02-04 | 04-03 |
| T-14 | Panic in the auth path takes down the browser | DoS | Medium | Zero `unwrap()`; malformed headers/JSON degrade to 400/401 (C-4) | 04-05 |
| T-15 | A corrupt `agents.json` fails open, admitting every token | EoP | **High** | Degrade to an **empty client set**, never an empty allow-list check. Unit-test the corrupt-file path (C-5) | 04-04 |

### Pre-existing limitations that get worse, not better

`SECURITY.md` documents that an agent with `evaluate` can reach the local filesystem (MCP-09) and
that a wedged script costs one command timeout (MCP-10). Both are currently bounded by "you chose to
connect this agent locally." A network transport widens who can become that agent. Neither is fixed
by this phase, and `SECURITY.md`'s "what reduces the risk meanwhile" advice — *treat `evaluate` as
equivalent to filesystem read access for the user account running Talaria* — becomes materially more
important once the connection can come over a network. Say so in the updated `SECURITY.md`.

---

## Security Domain

`security_enforcement: true`, `security_asvs_level: 1`, `security_block_on: "high"`.

### Applicable ASVS categories

| ASVS category | Applies | Standard control |
|---|---|---|
| V2 Authentication | **yes** | OAuth 2.1 Authorization Code + PKCE `S256`; opaque bearer tokens; no password store (single-user, the human at the keyboard is the resource owner) |
| V3 Session Management | **yes** | Short-lived access tokens; refresh rotation with reuse detection; per-request validation; revocation terminates live handles |
| V4 Access Control | **yes** (minimal by design) | One scope. C-11 rules out a permission matrix. The access-control decision is binary: authorized client or not |
| V5 Input Validation | **yes** | Every OAuth parameter is attacker-controlled: `client_id`, `redirect_uri`, `code_challenge`, `state`, `resource`, `client_name`. Validate before use; reject rather than coerce |
| V6 Cryptography | **yes** | `OsRng` for tokens and codes (never a time-seeded PRNG); `sha2` for token hashing and PKCE; constant-time comparison. Never hand-roll |
| V7 Error handling & logging | **yes** | Opaque 401s that do not distinguish "unknown token" from "expired token"; never log token material |
| V13 API & web service | **yes** | Correct 400/401/403 semantics; `WWW-Authenticate` on 401; body-size cap; no tokens in query strings (spec **MUST NOT**) |

### Known threat patterns for this stack

| Pattern | STRIDE | Standard mitigation |
|---|---|---|
| Authorization code interception on a loopback redirect | Spoofing | PKCE `S256`, single-use short-lived codes, exact redirect matching |
| Token audience confusion / passthrough | Spoofing | RFC 8707 `resource`; validate audience; never forward a received token upstream |
| Authorization-server mix-up | Spoofing | RFC 9207 `iss` in the authorization response (SHOULD at `2026-07-28`, becoming MUST) |
| Open redirect via `redirect_uri` | Tampering | Exact match against registration; loopback port is the only permitted variance |
| DNS rebinding against a loopback service | Spoofing | `Host`/`Origin` validation — on by default in `AxumServerOptions` |
| CSRF from a local page against the MCP endpoint | Tampering | Bearer token required; reject any request with an `Origin` header |
| Clickjacking / UI redress of a consent page | Spoofing | Consent decision lives in native chrome, not in HTML |
| JWT algorithm confusion | Spoofing | Avoided structurally: opaque tokens, `jsonwebtoken` never called |
| Timing attack on token comparison | Information Disclosure | Constant-time compare of hashes |

---

## Validation Architecture

### Test framework

| Property | Value |
|----------|-------|
| Framework (Rust) | Built-in `#[cfg(test)]` + `cargo test`. No test-framework crate; `tempfile` is deliberately **not** a dependency — store tests build unique temp paths by hand (`permissions.rs` tests, `vault.rs` tests) |
| Framework (e2e) | Python 3 standard library only (3.14.6 on this machine). No pytest, no third-party deps |
| Config file | None for Rust. `tests/e2e/run_all.py` is the e2e runner; `tests/e2e/harness.py` is the shared launcher |
| Quick run command | `cargo test -p talaria-shell` |
| Full suite command | `cargo build --release && python3 tests/e2e/run_all.py` |
| Static gate | `cargo clippy --all-targets -- -D warnings` (CI runs the full `--all-targets`, no command-line allow-list — decision 02-11) |
| Current baseline | e2e 19/19, `cargo test` 117 |

### Phase requirements → test map

| Req | Behaviour | Test type | Automated command | File exists? |
|-----|-----------|-----------|-------------------|--------------|
| AUTH-03 | Streamable HTTP endpoint serves the same nine tools as stdio | e2e | `python3 tests/e2e/http_transport_test.py` | ❌ Wave 0 |
| AUTH-03 | `tools/list` over HTTP is byte-identical to `tools/list` over stdio | e2e | same suite | ❌ Wave 0 |
| AUTH-03 / SC 4 | stdio still works unauthenticated with the listener on | e2e | `python3 tests/e2e/mcp_client_test.py` (extend) | ✅ exists |
| AUTH-03 | Listener is absent by default — nothing listening on the port with no `config.json` | e2e | `python3 tests/e2e/http_transport_test.py` | ❌ Wave 0 |
| AUTH-01 | Unauthenticated `/mcp` returns 401 with `WWW-Authenticate: Bearer resource_metadata=…` | e2e | `python3 tests/e2e/oauth_flow_test.py` | ❌ Wave 0 |
| AUTH-01 | Protected-resource metadata document is fetchable and names the AS | e2e | same suite | ❌ Wave 0 |
| AUTH-01 / SC 2 | Full discovery → DCR → `/authorize` → chrome approve → `/token` → authorized `tools/call` | e2e | same suite (drives consent via `chrome_rects` + `click_rect`) | ❌ Wave 0 |
| AUTH-01 | A wrong PKCE verifier is rejected; `plain` is rejected | unit | `cargo test -p talaria-shell oauth::` | ❌ Wave 0 |
| AUTH-01 | An authorization code is single-use and expires | unit | `cargo test -p talaria-shell oauth::` | ❌ Wave 0 |
| AUTH-01 | Refresh rotation issues a new token; reuse of the old one revokes the family | unit | `cargo test -p talaria-shell oauth::` | ❌ Wave 0 |
| AUTH-01 | Redirect URI matches exactly, loopback port excepted | unit | `cargo test -p talaria-shell oauth::` | ❌ Wave 0 |
| AUTH-01 | A token whose audience is not Talaria is rejected | unit + e2e | `cargo test` + `oauth_flow_test.py` | ❌ Wave 0 |
| AUTH-02 / SC 3 | Two agents authorized; revoking one 401s its next request while the other still works | e2e | `python3 tests/e2e/revocation_test.py` | ❌ Wave 0 |
| AUTH-02 / SC 3 | A revoked agent's **open stream closes** | e2e | same suite | ❌ Wave 0 |
| AUTH-02 | The Access panel lists authorized clients and the Revoke button is clickable by rect | e2e | same suite | ❌ Wave 0 |
| AUTH-02 | Revocation survives a shell restart | e2e | same suite | ❌ Wave 0 |
| — | `agents.json` lands at `0600` | unit | `cargo test -p talaria-shell agents::` (via `permissions::mode_of`) | ❌ Wave 0 |
| — | A corrupt `agents.json` yields an empty client set, not an empty allow | unit | `cargo test -p talaria-shell agents::` | ❌ Wave 0 |
| — | `chrome_rects` still refused without `TALARIA_TEST_HOOKS=1`, including the new rect names | e2e | `python3 tests/e2e/panel_click_test.py` (extend) | ✅ exists |
| — | An agent cannot navigate a tab to the listener's own origin | e2e | `python3 tests/e2e/scheme_refusal_test.py` (extend) | ✅ exists |

### Sampling rate

- **Per task commit:** `cargo test -p talaria-shell` (fast, no browser) plus `cargo clippy
  --all-targets -- -D warnings`.
- **Per wave merge:** `cargo build --release --locked` then the affected e2e suite(s) standalone.
- **Phase gate:** `python3 tests/e2e/run_all.py` fully green (currently 19/19; this phase should take
  it to 22/22) plus `cargo test` green, before `/gsd-verify-work`.

### Wave 0 gaps

- [ ] `tests/e2e/http_transport_test.py` — covers AUTH-03, SC 1, SC 4
- [ ] `tests/e2e/oauth_flow_test.py` — covers AUTH-01, SC 2
- [ ] `tests/e2e/revocation_test.py` — covers AUTH-02, SC 3
- [ ] `tests/e2e/harness.py` — a helper that seeds `config.json` with the listener enabled at a fixed
      port before `start_shell`, and a minimal PKCE + loopback-callback helper the OAuth suites share
      (the same "two suites need it, so it lives in the harness" reasoning that put `chrome_rects`
      there)
- [ ] `tests/e2e/run_all.py` — register the three new suites in the Phase-2 standalone list
- [ ] `crates/talaria-shell/src/agents.rs` — `#[cfg(test)] mod tests` at `vault.rs` density
- [ ] `crates/talaria-shell/src/oauth.rs` — `#[cfg(test)] mod tests` for PKCE, codes, rotation,
      redirect matching, audience
- [ ] No framework install needed — both harnesses already exist

---

## Environment Availability

| Dependency | Required by | Available | Version | Fallback |
|------------|------------|-----------|---------|----------|
| Rust toolchain (≥ 1.88) | Everything | ✓ | 1.97.1 | — |
| `cargo` | Build, test | ✓ | 1.97.1 | — |
| Python 3 | e2e suites | ✓ | 3.14.6 | — |
| Python stdlib `http.server`, `http.client`, `hashlib`, `base64`, `ssl` | OAuth e2e suites | ✓ | stdlib | — |
| `Xvfb` | Headless shell for e2e | ✓ | present | — |
| `xdotool` | Synthetic input in e2e | ✓ | 3.20160805.1 | — |
| Loopback TCP, ports available | HTTP listener + tests | ✓ | — | Fixed test port; fall back to an ephemeral port plus a hook if CI collides |
| `curl` | Manual probing | ✓ | 8.20.0 | `python3 -m urllib` |
| `openssl` | Only if TLS is attempted | ✓ | 3.6.3 | Not needed — Phase 4 is loopback `http` |
| `tailscale` | **Phase 5**, not Phase 4 | ✓ | 1.102.2 | — |
| Session D-Bus / OS keychain | **Not required** by this phase | conditional | — | Deliberate: the token store avoids the keychain, which is why the known `Vault::load()` D-Bus hang does not extend to auth |
| Self-hosted CI runner | e2e gate on merge to `main` | ✓ | — | `ci.yml` fast gate still runs on every push |

**Missing dependencies with no fallback:** none.
**Missing dependencies with fallback:** none.

---

## Assumptions Log

| # | Claim | Section | Risk if wrong |
|---|-------|---------|---------------|
| A1 | `AxumServerOptions.auth` accepts a hand-written `Arc<dyn AuthProvider>` and `rust-mcp-axum` routes every declared `OauthEndpoint` to `handle_request` — i.e. a self-hosted AS is a supported integration and not merely a trait that happens to be public | The Fork, Pattern 2 | If the routing assumes a remote provider somewhere, 04-05/04-06 grow a fork of `rust-mcp-axum` or a hand-written tower layer. **The plan should verify this with a throwaway build before committing to 04-05.** Field names and the routing fold were read from `main`, not from the published `1.0.1` tarball | 
| A2 | `rust-mcp-sdk 1.0.1`'s `main` branch source matches the published `1.0.1` crate for the auth module | Pattern 2, Code Examples | Signatures shift; a mechanical fix, but it will surprise the executor. Verify against `docs.rs/rust-mcp-sdk/1.0.1` or the vendored source |
| A3 | Adding `axum`, `axum-server`, `jsonwebtoken` and `reqwest` will not disturb the `primeorder 0.14.0-rc.14` pin, given how much of the tree is already resolved | Pitfall 1 | The whole build breaks with an unrelated-looking E0277. This is why 04-02 exists as its own plan |
| A4 | Real MCP clients (Claude Code, Claude Desktop, etc.) still use DCR rather than requiring CIMD | The Fork | If a target client demands CIMD, SC 2 cannot be demonstrated against it and the SSRF trade has to be revisited. Not verified against any specific client this session |
| A5 | One hour is an appropriate access-token lifetime and 60 seconds an appropriate code lifetime | Q3 | Purely a default; both are the conventional values, neither is spec-mandated |
| A6 | Terminating a revoked client's live handles is expressible against `rust-mcp-axum`'s session store | Pitfall 4, T-7 | If not, SC 3's stream case needs a different mechanism (e.g. a per-request re-check inside the stream's keepalive). Not verified in the SDK source this session |
| A7 | The e2e harness can drive the consent panel with `chrome_rects` + `click_rect` as reliably as it drives the credentials panel | Q6 | Falls back to a `TALARIA_TEST_HOOKS`-gated approve command — acceptable but strictly worse, and it must be gated exactly as `ChromeRects` is |
| A8 | `talaria-shell` gaining `rust-mcp-sdk` + `rust-mcp-axum` does not create a version conflict with anything Servo pulls | Q2 | Would force the two-process alternative. Cheap to check with a build |

---

## Open Questions

1. **Does `rust-mcp-axum` route a self-hosted AS's endpoints as cleanly as reading the source suggests?**
   - What we know: `auth_routes.rs` folds every path from `mcp_handler.oauth_endpoints()` into
     `any(handle_auth_request)`, which calls `handle_auth_requests` on the provider. `OauthEndpoint`
     includes `AuthorizationEndpoint`, `TokenEndpoint` and `RegistrationEndpoint`, which only make
     sense for a co-hosted AS.
   - What's unclear: whether anything upstream of that assumes a remote issuer.
   - Recommendation: a 30-minute spike before 04-05 is planned. Cheapest possible de-risking of the
     phase's largest assumption.

2. **Protocol revision: hold at `2025-11-25` or chase `2026-07-28`?**
   - What we know: the SDK implements `2025-11-25` and Talaria pins it. `2026-07-28` is stateless,
     removes the handshake, and deprecates HTTP+SSE.
   - What's unclear: when `rust-mcp-sdk` will ship `2026-07-28`, and whether Phase 5's distributed
     protocol would prefer the stateless model.
   - Recommendation: hold at `2025-11-25` for Phase 4; log the drift in `deferred-items.md` so it is
     tracked rather than rediscovered.

3. **What is Phase 5's TLS story, and does it belong here?**
   - What we know: OAuth 2.1 §1.5 requires AS endpoints over HTTPS with a loopback exception only. A
     tailnet bind is not loopback. `rust-mcp-axum` has an `ssl` feature; `tailscale cert` can issue a
     real certificate.
   - Recommendation: Phase 4 is loopback-only and Phase 5 owns TLS. State this as a scope boundary so
     Phase 5 inherits a named obligation rather than a surprise.

4. **Fixed port or ephemeral?**
   - A fixed default (8080 is the SDK's) is discoverable and testable but collides. Ephemeral needs a
     way to tell the user and the tests what was bound.
   - Recommendation: a configurable port with a non-8080 default, surfaced in the Settings panel next
     to the enable toggle. The e2e harness pins it via `config.json`.

5. **Should the Connections panel show *sessions* or *authorized clients*, or both?**
   - AUTH-02 says "connected agents." A token can exist with nothing connected; a stdio session can be
     connected with no token. These are different lists.
   - Recommendation: authorized clients, with a live/idle indicator. Note in UI-SPEC. Related deferred
     item AGENT-05 (per-agent session naming) is adjacent and should not be silently absorbed.

6. **Does anything need to change in `talaria-protocol`?**
   - The consent round trip is shell-internal (`AppEvent`, not a wire command), so probably nothing.
     But if the e2e suites need to observe consent state over the control socket, a
     `TALARIA_TEST_HOOKS`-gated command joins `ChromeRects`. Decide in 04-06, not before.

---

## Sources

### Primary (HIGH confidence)

- **MCP Authorization specification, revision `2026-07-28`** — roles, RFC 9728 MUST, RFC 8707 MUST,
  RFC 9207 `iss`, PKCE, error codes, refresh rotation, stdio SHOULD NOT.
  `https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization`
- **MCP Client Registration, `2026-07-28`** — CIMD requirements, DCR deprecation, `application_type`
  and native/loopback constraints, authorization-server binding.
  `https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration`
- **MCP Authorization Security Considerations, `2026-07-28`** — token theft, communication security,
  open redirection, localhost redirect risks, confused deputy, audience validation.
  `https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/security-considerations`
- **MCP Authorization specification, revision `2025-11-25`** — the revision `rust-mcp-sdk 1.0.1`
  implements. `https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization`
- **MCP Versioning** — current revision is `2026-07-28`.
  `https://modelcontextprotocol.io/specification/versioning`
- **`rust-mcp-sdk` README (`main`)** — feature list, `RemoteAuthProvider`, `OAuthProxy` status,
  `AxumServerOptions`, security considerations, protocol version.
  `https://raw.githubusercontent.com/rust-mcp-stack/rust-mcp-sdk/main/crates/rust-mcp-sdk/README.md`
- **`rust-mcp-sdk` source, `crates/rust-mcp-sdk/src/auth/auth_provider.rs`** — the `AuthProvider`
  trait and the `OauthEndpoint` variants.
- **`rust-mcp-sdk` source, `crates/rust-mcp-sdk/src/auth/auth_info.rs`** — `AuthInfo` fields.
- **`rust-mcp-axum` source, `crates/rust-mcp-axum/src/server.rs`** — `AxumServerOptions` fields and
  defaults, `McpMountOptions` (BYO-server).
- **`rust-mcp-axum` source, `crates/rust-mcp-axum/src/routes/auth_routes.rs`** — how declared OAuth
  endpoints become routes.
- **crates.io API** — versions, publish dates, feature tables, dependency lists and download counts
  for `rust-mcp-sdk`, `rust-mcp-axum`, `rust-mcp-transport`, `axum`, `axum-server`, `jsonwebtoken`,
  `reqwest`.
- **Context7 `/rust-mcp-stack/rust-mcp-sdk`** — Axum turnkey server example, `McpAuthConfig` (client
  side), Keycloak/WorkOS/Scalekit provider options.
- **This repository** — `Cargo.toml`, `Cargo.lock`, `crates/talaria-mcp/src/{main,socket,tools}.rs`,
  `crates/talaria-shell/src/{control,vault,settings,permissions,gui,app}.rs`,
  `crates/talaria-protocol/src/lib.rs`, `tests/e2e/{harness,run_all,mcp_client_test}.py`,
  `SECURITY.md`, `SPEC.md`, `.claude/CLAUDE.md`, `.planning/{PROJECT,REQUIREMENTS,ROADMAP,STATE}.md`.

### Secondary (MEDIUM confidence)

- **RFC 8252, OAuth 2.0 for Native Apps** — loopback IP literals (`127.0.0.1`/`[::1]`) preferred over
  `localhost`; the AS **MUST** allow any port for loopback redirect URIs; native apps **MUST** use an
  external user-agent, not an embedded webview. `https://datatracker.ietf.org/doc/html/rfc8252`
- **OAuth 2.1 draft-ietf-oauth-v2-1-13** — §1.5 HTTPS with the loopback `http` exception; §4.1.1 PKCE
  mandatory for all clients; §4.3.1 refresh rotation for public clients; exact redirect matching;
  implicit and password grants removed.
  `https://datatracker.ietf.org/doc/html/draft-ietf-oauth-v2-1-13`
- **MCP blog, "The 2026-07-28 Specification"** — the six auth SEPs (2468 `iss`, 837
  `application_type`, 2352 issuer binding, DCR deprecation), statelessness, SSE deprecation offramp.
  `https://blog.modelcontextprotocol.io/posts/2026-07-28/`

### Tertiary (LOW confidence)

- General web-search summaries of the `2026-07-28` release (WorkOS, Stacktree, mcpservers.org) — used
  only to locate the primary sources above; no claim in this document rests on them alone.

---

## Metadata

**Confidence breakdown:**

| Area | Level | Reason |
|------|-------|--------|
| MCP authorization spec requirements | HIGH | Fetched from the normative spec at both revisions this session; MUST/SHOULD language quoted directly |
| `rust-mcp-sdk` features and versions | HIGH | crates.io API feature table and dependency list, plus the crate's own README and source from `main` |
| What the SDK does *not* provide (no AS) | HIGH | The README says `OAuthProxy` is "still in development"; `RemoteAuthProvider` is documented as verifying tokens from external DCR-capable providers |
| Lockfile risk assessment | HIGH | Direct `grep` of the committed `Cargo.lock`; most of the HTTP stack already resolved |
| Codebase grounding (patterns, constraints, threats) | HIGH | Every file named was read this session |
| Architecture recommendations (Q2, Q3, Q4) | MEDIUM | Well-grounded in this codebase's precedents, but judgement calls with no CONTEXT.md behind them |
| That a self-hosted AS integrates cleanly with `rust-mcp-axum` | MEDIUM | Trait and routing read from source; not compiled. See A1 and Open Question 1 |
| Scope estimate (six or seven plans) | MEDIUM | Calibrated against Phases 2 and 3; estimation, not measurement |

**Research date:** 2026-08-20
**Valid until:** 2026-09-19 (30 days) for the codebase facts; **2026-09-03 (14 days)** for the MCP
spec and `rust-mcp-sdk` facts — the spec moved one full revision in the eight months before this
phase and the SDK is one revision behind it right now. Re-verify `rust-mcp-sdk`'s newest version and
the current spec revision at the start of execution.

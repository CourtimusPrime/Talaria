---
phase: 04
slug: authenticated-remote-transport-v2
source: orchestrator decision gate (2026-08-20)
---

# Phase 4 Context — locked decisions

`/gsd-discuss-phase` was not run for this phase. `04-RESEARCH.md` surfaced four forks it judged
product decisions rather than research findings, and recommended locking them before planning.
Three were put to the user and answered; the fourth was decided on the research's own security
reasoning. They are locked here.

## D-04-01 — Target MCP spec revision `2025-11-25`

**Decision:** Phase 4 targets the revision `rust-mcp-sdk` 1.0.1 actually implements. It does **not**
chase `2026-07-28`.

**Why:** 1.0.1 is the newest release and shipped two days before `2026-07-28` went final. Chasing
the current revision means implementing the sessionless model and Streamable HTTP ahead of the SDK
— forking or bypassing the crate this project deliberately chose not to hand-roll — while the SDK
may well land it first.

**Consequences the planner must carry:**
- The legacy HTTP+SSE transport is deprecated in the newer revision. Phase 4 uses it knowingly.
- Dynamic Client Registration is deprecated in the newer revision. Phase 4 ships DCR anyway
  (see D-04-02), which is the roadmap's own plan `04-02`.
- **A migration to `2026-07-28` is now tracked work**, not an accident to discover later. Record it
  as a deferred item naming both the sessionless change and the transport deprecation, so the next
  milestone sees it.

## D-04-02 — Ship DCR (RFC 7591); do not implement CIMD

**Decision:** Dynamic Client Registration, deprecated-but-functional. Client ID Metadata Documents
are explicitly **not** implemented, and `client_id_metadata_document_supported` is **not**
advertised.

**Why:** CIMD requires the authorization server to fetch an attacker-supplied HTTPS URL. That is an
unauthenticated SSRF primitive inside the process that holds the credential vault. Decided on
`04-RESEARCH.md`'s security reasoning rather than referred, because the reasoning is not a
preference.

**Consequence:** record the CIMD deferral with this rationale, so a later phase adopting it does so
deliberately and with a fetch policy.

## D-04-03 — Expand the phase to 6–7 plans

**Decision:** The roadmap's 3-plan split is superseded. Follow `04-RESEARCH.md`'s recommended split,
and **update `ROADMAP.md`'s Phase 4 plan list to match** as part of planning.

**Why:** the 3-plan split puts metadata discovery, DCR, PKCE and token issuance in a single plan.
The two additions that carry the most weight:
- **Extract the tool surface into a shared library** (`talaria-mcp/src/lib.rs` plus a `CommandSink`
  trait) as the first plan, so Success Criterion 1's "drive the same tool surface" is **structurally
  true** rather than something a reviewer has to confirm by eye.
- **A spike on `rust-mcp-axum`** — whether it routes a self-hosted authorization server's endpoints
  as cleanly as its source suggests. `04-RESEARCH.md` flags this as assumption A1, its largest, and
  costs about 30 minutes to settle. Plan it before the authorization-server plan is planned, not
  after it is written.

## D-04-04 — Listener off by default, loopback-only

**Decision:** the HTTP listener is **disabled by default**. When enabled it binds **127.0.0.1 only**.
TLS and any non-loopback bind are deferred to Phase 5.

**Why:** Phase 2's peer-UID check (SEC-02) does not transfer — `SO_PEERCRED` has no TCP equivalent,
and `127.0.0.1:PORT` is reachable by every local account, unlike the `0600`-inside-`0700` Unix
socket. The bearer token becomes the entire boundary, on brand-new unaudited auth code, in a process
holding a credential vault.

**Consequences:**
- The user needs an explicit, discoverable way to turn it on — this feeds the UI-SPEC alongside
  AUTH-02's connected-agents panel.
- OAuth 2.1 requires HTTPS for authorization-server endpoints **with a loopback exception**. Staying
  on loopback is what makes shipping without TLS conformant rather than sloppy. Say so in the plan;
  do not let a later reader think TLS was merely skipped.
- Phase 5's tailnet bind therefore needs a certificate story. Record it as a Phase 5 input.

## Still open — for the planner or the UI phase, not blocking

- **Consent screen location.** PKCE's authorization step needs human approval. In a browser that
  *is* the product, this could be a chrome panel, a real page, or a system dialog. Route to the
  UI-SPEC.
- **Fixed vs ephemeral listener port.** Ephemeral is friendlier to tests; fixed is friendlier to
  client config. Decide in the transport plan and say why.
- **Does the AUTH-02 panel list sessions or authorized clients?** `04-RESEARCH.md` recommends
  clients, and notes the adjacency to deferred AGENT-05.
- **Success Criterion 2's wording** permits a plan that mints tokens and never implements discovery.
  `04-RESEARCH.md` proposes a rewording that names the four MUSTs the current wording hides —
  RFC 9728 protected-resource metadata, the `WWW-Authenticate` 401 challenge, RFC 8707 audience
  validation, and RFC 8414 AS metadata. Apply it when updating ROADMAP.md.

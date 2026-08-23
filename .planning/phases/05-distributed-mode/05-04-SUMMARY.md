---
phase: 05-distributed-mode
plan: 04
subsystem: infra
tags: [oauth, rfc8707, rfc8414, rfc9728, tailscale-serve, dns-rebinding, tls-termination, settings, reverse-proxy]

# Dependency graph
requires:
  - phase: 05-distributed-mode
    provides: "05-01's A2 spike — the measured fact that Serve forwards `Host: <tailnet-name>:<serve-port>`, which is why this plan is required rather than optional, and that no `Origin` is synthesised, which is why the CSWSH defence needed no change"
  - phase: 04-authenticated-remote-transport-v2
    provides: "`canonical_resource` as the sole construction site of the resource identifier, `TalariaAuth`'s build-once-from-`bound` discipline, `refuse_page_originated` as the router's outermost layer, `DnsRebindProtector`'s explicit allowlist, `RemoteAccessConfig`'s `bind`-key refusal, and D-04-04's bind-host-as-constant guarantee"
provides:
  - "`remote_access.advertised_url` — a validated, optional origin in `config.json`, absent by default, refusing rather than normalising, and failing closed onto remote access off"
  - "`oauth::AdvertisedIdentity` — one scheme-plus-authority value, resolved once where the listener knows both facts, feeding every string this browser publishes about itself"
  - "`http::admitted_hosts` — the two-name host allowlist shared by `DnsRebindProtector` and `refuse_page_originated`, deduplicated to one when nothing is advertised"
  - "The secure scheme on every published OAuth URL once an identity is advertised — OAuth 2.1 §1.5 met rather than excepted, discharging the obligation `04-.../deferred-items.md` recorded"
  - "`scripts/tailscale-serve.sh` — executable up/down, prerequisite-checking, port-scoped, Funnel-refusing, exercised against the live tailnet"
  - "Two `deferred-items.md` entries: no automatic discovery of the advertised identity, and the transport-identity lifecycle as permanently the daemon's (T-05-16)"
affects: [05-05, 05-06, 05-07, 05-09, 05-11, phase-05.1]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Two facts where there was one: the address the listener *bound* (the socket, what the human is shown, what a local client addresses) and the origin it *publishes* (configuration, what a remote client reaches). One type carries the second, and a comment at every site where they meet says which is which"
    - "A scheme is a named constant, not a literal at a `format!` site — which makes 'is any scheme hardcoded here' a question `grep` answers"
    - "Validation refuses rather than normalises, when the validated value is later compared byte for byte: a normalisation is a second spelling"
    - "A degrade that is scoped to one key *except* when the key's failure mode is publishing an identity nobody wrote, in which case the whole block goes to its default — off"
    - "Machine-level state outside git gets an executable script with an up and a down, which captures the full state before acting, refuses anything it did not create, and prints a diff on teardown"

key-files:
  created:
    - scripts/tailscale-serve.sh
  modified:
    - crates/talaria-shell/src/settings.rs
    - crates/talaria-shell/src/oauth.rs
    - crates/talaria-shell/src/http.rs
    - crates/talaria-shell/src/app.rs
    - tests/e2e/harness.py
    - .planning/phases/05-distributed-mode/deferred-items.md
    - CHANGELOG.md

key-decisions:
  - "The advertised URL is refused rather than normalised — no trailing slash, no path, no query, no fragment, no user information, no uppercase in the authority — because the value ends up in an RFC 8707 identifier compared byte for byte, and a normalisation is a second spelling of the same server"
  - "A malformed advertised URL costs the *whole* remote-access block rather than just itself: a browser running with an identity nobody wrote would publish an issuer a client is entitled to believe"
  - "`TalariaAuth`'s `bound` field was removed rather than kept: its only remaining use was as the issuer, and a second scheme-plus-host construction in that module is exactly Pitfall 8's warning sign"
  - "The scheme is a module constant (`LOOPBACK_SCHEME`) rather than a literal, which is what makes `grep -c 'format!(\"http'` == 0 a meaningful assertion rather than an accident"
  - "`http::MCP_PATH` became `pub(crate)` so `canonical_resource` appends *that* path rather than a second spelling of `/mcp` — the change removed a duplicate spelling instead of adding one"
  - "The host allowlist is deduplicated when the two names coincide, so the no-advertised-URL case produces the exact one-element list Phase 4 built"
  - "The script refuses Funnel by name with its reason in the code, and refuses any port carrying a mapping it did not create, in both directions"

patterns-established:
  - "Pattern: pin a published string against a *literal* rather than against a second call to the constructor that produced it — a tautological assertion lets the whole scheme change unnoticed"
  - "Pattern: when a script manages state shared with unrelated services, assert the untouched neighbours by capturing and diffing the full list, not by trusting the scope of the command"

requirements-completed: [DIST-01]

coverage:
  - id: D1
    description: "`remote_access.advertised_url` is a validated optional origin: accepted verbatim when well-formed, refused for every second spelling and every non-origin shape, and every refusal lands on remote access off without touching the search engine"
    requirement: "DIST-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/settings.rs#a_secure_origin_with_a_host_and_a_port_is_preserved_verbatim, #a_secure_origin_with_no_port_is_accepted, #the_insecure_scheme_belongs_to_the_loopback_literal_with_a_port_and_to_nothing_else, #an_advertised_url_carrying_a_path_query_or_fragment_is_refused, #a_bare_trailing_slash_is_refused_rather_than_normalised_away, #an_advertised_url_carrying_user_information_is_refused, #an_advertised_url_with_no_usable_host_is_refused, #an_advertised_url_that_is_not_a_string_is_refused, #a_refused_advertised_url_disables_remote_access_and_keeps_the_search_engine"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/settings.rs#an_advertised_url_round_trips_through_save_and_load, #a_saved_document_with_no_advertised_url_writes_no_key"
        status: pass
    human_judgment: false
  - id: D2
    description: "One advertised identity feeds the canonical resource, the metadata URL, the issuer, the advertised authorization server and all four endpoint URLs — never four independent constructions, and never a request header"
    requirement: "DIST-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#with_an_advertised_url_every_published_string_derives_from_it, #an_advertised_identity_splits_its_origin_into_a_scheme_and_an_authority, #the_canonical_identifier_comes_from_the_advertised_identity_and_nowhere_else"
        status: pass
      - kind: other
        ref: "grep -c 'format!(\"http' crates/talaria-shell/src/oauth.rs == 0; grep -c 'format!(\"https' == 0; grep -c 'header::HOST' crates/talaria-shell/src/http.rs == 1 (the comparison, assigned nowhere)"
        status: pass
    human_judgment: false
  - id: D3
    description: "Every OAuth URL carries the secure scheme once an identity is advertised — OAuth 2.1 §1.5 met rather than excepted (T-05-03)"
    requirement: "DIST-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#with_an_advertised_url_every_published_string_derives_from_it (asserts `starts_with(\"https://\")` on the resource, the metadata URL and all four endpoints)"
        status: pass
    human_judgment: false
  - id: D4
    description: "`refuse_page_originated` and `DnsRebindProtector` admit exactly the bound address and the advertised authority, and refuse a plausible third name — which is what makes a Serve-proxied request admissible at all"
    requirement: "DIST-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/http.rs#the_host_allowlist_admits_two_names_and_refuses_every_other, #with_no_advertised_url_the_host_allowlist_is_the_one_bound_address, #discovery_is_public_and_every_other_route_is_origin_and_host_checked"
        status: pass
    human_judgment: false
  - id: D5
    description: "A token is bound to the advertised spelling and not the bound one: byte-for-byte audience validation admits exactly one canonical identifier (T-05-09-A)"
    requirement: "DIST-01"
    verification:
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#an_audience_is_the_advertised_spelling_and_not_the_bound_one"
        status: pass
    human_judgment: false
  - id: D6
    description: "D-04-04's structural guarantee is preserved exactly: `BIND_HOST` is still a constant used at one place, no configurable bind field exists, the three bind-refusal tests pass with unchanged text, and an advertised URL is not a way to smuggle a bind in"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "grep -c 'const BIND_HOST' crates/talaria-shell/src/http.rs == 1, value unchanged; grep -c 'pub bind\\|bind: String\\|bind_host' crates/talaria-shell/src/settings.rs == 0"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/settings.rs#a_non_loopback_bind_disables_remote_access_and_keeps_the_search_engine, #every_bind_address_other_than_the_loopback_literal_is_refused, #the_loopback_literal_is_the_one_bind_address_that_is_honoured (all three unchanged), plus #an_advertised_url_does_not_make_a_wider_bind_expressible"
        status: pass
    human_judgment: false
  - id: D7
    description: "This browser handles no transport identity at all — no path, no PEM parsing, no renewal timer, no expiry check (T-05-16, transferred)"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "! grep -rqi 'certificate|\\.pem|cert_path|ssl_cert|private key' crates/talaria-shell/src/ — exit 0 (recursive and predicate-form, so a directory-without-`-r` false pass is impossible)"
        status: pass
    human_judgment: false
  - id: D8
    description: "With the key absent, nothing about Phase 4's behaviour moved — the two end-to-end suites that would notice pass unmodified"
    requirement: "DIST-01"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/http_transport_test.py exit 0 and python3 tests/e2e/oauth_flow_test.py exit 0, with `git diff --stat` on both files empty"
        status: pass
      - kind: unit
        ref: "crates/talaria-shell/src/oauth.rs#with_no_advertised_url_every_published_string_is_what_phase_four_published (asserted against literals, not constructors)"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/run_all.py exit 0, 22/22 PASS, `failed: none`"
        status: pass
    human_judgment: false
  - id: D9
    description: "Standing the proxy up and taking it down is one runnable, port-scoped, prerequisite-checking step that cannot take down a neighbouring service and cannot reach the public internet"
    requirement: "DIST-01"
    verification:
      - kind: integration
        ref: "./scripts/tailscale-serve.sh up then down against the live tailnet: `tailscale serve status` md5 f3df1875edd9a80b61d460fdb56a7ca0 before and after (the same value 05-01 recorded), all four pre-existing mappings intact including :8443 -> http://127.0.0.1:5678"
        status: pass
      - kind: integration
        ref: "TALARIA_SERVE_PORT=8443 up exits 1 naming the existing target and changes nothing; TALARIA_SERVE_PORT=8443 down exits 1 and changes nothing; up with no loopback listener exits 1; down with no mapping is a no-op"
        status: pass
      - kind: other
        ref: "test -x, bash -n, grep -qi 'funnel' (named), ! grep -qE 'tailscale[^|;&]* funnel' (never invoked), grep -c '8449' == 4, grep -ci 'D-05-03' == 1, grep -ci 'advertised' == 2"
        status: pass
    human_judgment: false

# Metrics
duration: 48min
completed: 2026-08-23
status: complete
---

# Phase 5 Plan 04: The Advertised Base URL and the Serve Script Summary

**One configured origin now produces every string this browser publishes about itself — the RFC 8707
canonical resource identifier, the RFC 8414 issuer, the four endpoint URLs, the RFC 9728 document,
the DNS-rebinding allowlist and the outermost layer's `Host` comparison — while the bind host stays a
module constant and this process handles no transport identity at all; plus an executable Serve
script that stood a mapping up and took it down without moving one byte of this node's four
unrelated live mappings.**

## Performance

- **Duration:** 48 min
- **Started:** 2026-08-23T12:11:00Z
- **Completed:** 2026-08-23T12:59:31Z
- **Tasks:** 3
- **Files modified:** 7 (1 created, 6 modified)

## Which case the spike settled, and why this plan was required

`05-01-SPIKE.md` measured a Serve-proxied request arriving with:

```
host: thinkpad.tailcd3cc6.ts.net:8449
```

**The client's own name and the Serve port — not the loopback target.** That is the first of the two
branches `<interfaces>` named, and it is the expensive one. `refuse_page_originated` compares `Host`
against the bound address and `DnsRebindProtector` is seeded from the same string, so *every
Serve-proxied request was refused before this plan*. The `http.rs` half was therefore the two-entry
allowlist, not nothing.

**The case that was not taken:** had the proxy rewritten `Host` to the loopback target, the second
allowlist entry would have been unnecessary for admission — and still correct to carry. What a proxy
rewrites is not a property this browser controls, and admitting the name this browser itself
publishes is not a widening. `admitted_hosts`' doc comment records that branch so a later reader
finds it there rather than re-deriving it.

The other half of the spike's finding needed no work at all: **Serve synthesises no `Origin`
header**, and a client-sent one passes through unchanged. The CSWSH defence that makes a
browser-based viewer structurally impossible is untouched, and `RefuseOriginHeader` still runs ahead
of everything.

## Accomplishments

### One value, not four `format!`s

`oauth::AdvertisedIdentity` is a scheme and an authority together. It is constructed **once**, in
`serve`, at the one place that knows both facts — the address `listener.local_addr()` reported and
the origin configuration named — and threaded down exactly the way `bound` already was. Its
constructor falls back to the bound address under `LOOPBACK_SCHEME` when nothing is advertised, so
the absent case reproduces Phase 4 without a branch at any use site.

`TalariaAuth`'s `bound` field is **gone**, not kept. Its only remaining use was
`"authorization_servers": [format!("http://{}", self.bound)]` — which is to say, the issuer, spelled
a second time. `05-RESEARCH.md` Pitfall 8's warning sign is literal, so that key now reads
`[self.issuer]`.

`canonical_resource`'s doc comment gained the sentence its own argument now needs: the reason it is
the sole construction site covers a *second spelling* — the address bound and the address advertised
— and a browser that let those disagree would issue tokens that validate against one and
mysteriously never work against the other.

**The scheme is a constant.** `grep -c 'format!("http'` and `grep -c 'format!("https'` in `oauth.rs`
are both `0`, in the shipping code *and* in the tests, because both would otherwise have hidden a
hardcoded scheme. `http::MCP_PATH` became `pub(crate)` so `canonical_resource` appends that constant
rather than a second literal `/mcp` — a change that removed a duplicate spelling instead of adding
one.

### The configuration key, and the guarantee it must not weaken

`RemoteAccessConfig` gained one optional field. Its doc comment does three jobs, because the
adjacency to the `bind` refusal is exactly what a later reader will misread: what it is (the origin
clients reach this browser at), what it is **not** (a bind address — nothing reads it when binding,
`BIND_HOST` is still a constant used at one place, and the hand-edited `bind` key is still refused
with all three of its tests unchanged), and why it comes from configuration and never from a request
header (T-05-09: a local page that could set the advertised issuer could choose the authorization
server this browser points a client at).

**The exact validation rules,** and the reason each exists:

| Rule | Why |
|------|-----|
| scheme is `https` | A tailnet name is not loopback under any reading of OAuth 2.1 §1.5 |
| …unless the host is the loopback literal **with a port** | The one case §1.5's exception covers, and the one a developer testing locally needs |
| authority non-empty, no `@` | Credentials in a URL are not something this browser publishes about itself |
| no path, no query, no fragment | An origin is a scheme and an authority; anything after is a second thing |
| **no bare trailing slash** | See below |
| no uppercase in the authority | A host that would be lower-cased to be understood is a second spelling |

**On the trailing slash, refused rather than normalised.** `https://host:8449/` and
`https://host:8449` name the same server and are different strings. RFC 8707 audience validation
compares byte for byte, so accepting the first by quietly deleting its slash would produce a browser
that minted tokens against one spelling and validated them against the other — issued, and then
mysteriously never working. A normalisation *is* a second spelling. That is T-05-09-A, and the rule
is asserted directly (`a_bare_trailing_slash_is_refused_rather_than_normalised_away`).

Every refusal returns `None` from `from_json`, which lands the **whole** remote-access block on its
default — off. The reason it is the whole block rather than just this key is written into the
refusal's comment: a browser running with an advertised identity nobody wrote publishes an issuer a
client is entitled to believe, which is worse than a browser with remote access off. Same direction,
same shape as the `bind` refusal (T-05-02).

**The stale prediction is corrected.** `RemoteAccessConfig`'s doc comment used to end *"Phase 5 is
where a non-loopback bind gets considered, and it must bring TLS with it."* Phase 5 considered it and
**declined**. The paragraph now says so, keeps the OAuth 2.1 sentence that made the earlier posture
conformant, and `http.rs`'s module header got the same correction: Phase 5 kept the bind where it is
and put transport security in front of it, so loopback-only is now permanent rather than
provisional.

### The allowlist that makes a proxied request admissible

`http::admitted_hosts` returns the bound address plus the advertised authority, deduplicated when
they coincide — which is the no-advertised-URL case, and therefore returns the exact one-element list
Phase 4 built. One `Arc<[String]>` feeds both `DnsRebindProtector` and `refuse_page_originated`, so
the two cannot drift.

`refuse_page_originated`'s doc comment gained the reason there are now two names (a proxy in front of
a loopback listener means the address a client addresses and the address this process bound are
legitimately different, and refusing the former would refuse every proxied request) and the property
that matters: **the set is enumerated from configuration before the first connection is accepted, is
never grown by a request, and the header it compares is never a source for it.** `grep -c
'header::HOST'` in `http.rs` is `1` — the comparison, and nothing else.

### With the key absent, nothing moved

`with_no_advertised_url_every_published_string_is_what_phase_four_published` asserts the resource,
the metadata URL, the issuer, the advertised authorization server and all four endpoint URLs against
**literals** rather than against second calls to the constructors that produced them — a tautological
assertion would let the whole scheme change unnoticed. And `http_transport_test.py` and
`oauth_flow_test.py` both pass with `git diff --stat` empty on them, which is the only honest form of
that proof.

### The script

`scripts/tailscale-serve.sh`, executable, `up` and `down` and no third mode, implementing D-05-03 by
name. Up confirms its prerequisites rather than assuming them, refuses a port already carrying a
mapping (naming the existing target), refuses to publish a mapping pointing at nothing, and finishes
by printing the resulting URL beside the exact `config.json` key it implies — the one moment both
halves of the identity are on screen together, which is the moment they have to agree. Down removes
only its own port and prints a before/after diff.

**Funnel is refused by name**, with its reason in the code rather than merely omitted, because
`05-RESEARCH.md` asked for it to be a named non-option: it publishes to the public internet and the
process behind this mapping holds an encrypted credential vault and the human's logged-in browsing
sessions, and the two commands differ by a few characters.

## Task Commits

1. **Task 1: the advertised-URL configuration key** — `e3d1b67` (feat)
2. **Task 2: one advertised identity, threaded** — `8da4fee` (feat)
3. **Task 3: the proxy as an executable script** — `c708d59` (feat)

## Files Created/Modified

- `crates/talaria-shell/src/settings.rs` — `advertised_url`, `is_valid_advertised_url`,
  `SECURE_SCHEME`/`LOOPBACK_EXCEPTION_SCHEME`, the corrected `LOOPBACK_BIND` and
  `RemoteAccessConfig` doc comments, 13 new tests (38 → 51)
- `crates/talaria-shell/src/oauth.rs` — `AdvertisedIdentity`, `LOOPBACK_SCHEME`,
  `canonical_resource`/`metadata_url`/`TalariaAuth::new` taking the identity, `bound` field removed,
  4 new tests
- `crates/talaria-shell/src/http.rs` — `admitted_hosts`, the two-entry allowlist in both layers,
  `MCP_PATH` made `pub(crate)`, `advertised` threaded through `spawn`/`serve`/`build_router`, the
  corrected module header, 2 new tests
- `crates/talaria-shell/src/app.rs` — clones where it copied, and passes the whole
  `RemoteAccessConfig` to `start_remote_listener` (see Deviations)
- `scripts/tailscale-serve.sh` — **new**, executable
- `tests/e2e/harness.py` — `write_agents`' docstring gained the advertised-audience sentence
- `.planning/phases/05-distributed-mode/deferred-items.md` — two entries (8 → 10)
- `CHANGELOG.md` — two `### Added` entries (see Deviations)

## Decisions Made

Recorded in `key-decisions` above. The two worth restating:

- **`TalariaAuth::bound` was deleted rather than kept.** The plan says to keep `bound` where it
  genuinely means the socket — but in `oauth.rs` it did not: its one remaining use was the issuer,
  spelled a second time. The socket-meaning uses live in `http.rs`, where `bound` is still the
  variable and where the log line deliberately reports what was *bound* rather than what is
  advertised, with a second line for the advertised origin only when the two differ. The comment at
  the point where the two are resolved says which is which.
- **`MCP_PATH` became `pub(crate)`** so the canonical resource appends the same constant the router
  serves. The alternative — a private `/mcp` literal in `oauth.rs` — would have been a second
  spelling of a path that is part of a byte-for-byte-compared identifier.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] `crates/talaria-shell/src/app.rs` is not in `files_modified`, and had to be**

- **Found during:** Task 1.
- **Issue:** `RemoteAccessConfig` was `#[derive(Copy)]`. Adding `Option<String>` drops `Copy`, and
  two sites in `app.rs` moved out of a `Ref<Settings>` (`let configured = state.settings.borrow()
  .remote_access;`). The build failed with three `E0507`/`E0382` errors. There is no way to add a
  string-carrying field without touching them.
- **Fix:** `.clone()` at both sites, with a comment at the first naming the field that made the type
  non-`Copy`. In Task 2 the same function gained the advertised value: `start_remote_listener` now
  takes `&RemoteAccessConfig` rather than a bare `port`, because the port and the advertised origin
  are both facts the listener needs before it binds and both come from configuration.
- **Verification:** `cargo build --release --locked`, `cargo clippy --all-targets --locked -- -D
  warnings`, `cargo test --locked` all exit 0.
- **Committed in:** `e3d1b67` and `8da4fee`.

**2. [Rule 2 - Missing critical] `CHANGELOG.md` updated**

- **Found during:** Task 3.
- **Issue:** `files_modified` does not list `CHANGELOG.md`, but the global rule is "log all changes
  in a project CHANGELOG.md", and 05-01 — the precedent this phase set — logged its work there.
- **Fix:** two `### Added` entries, in the register the existing ones use: the advertised identity
  (including that the bind did not widen, which is the thing a reader would otherwise assume it did)
  and the Serve script (including the Funnel refusal as a named non-option).
- **Committed in:** `c708d59`.

**3. [Rule 1 - Bug] One pre-existing test assertion became a tautology and was repaired**

- **Found during:** Task 2.
- **Issue:** `the_canonical_identifier_comes_from_the_bound_address_and_nowhere_else` asserted
  `canonical_resource(BOUND) == format!("http://{BOUND}/mcp")`. Mechanically rewriting the call site
  turned both halves into the same construction, which asserts nothing — the whole scheme could
  change and the test would still pass.
- **Fix:** pinned against the literal `"http://127.0.0.1:40501/mcp"` and renamed to
  `…comes_from_the_advertised_identity_and_nowhere_else`. The same discipline is applied to the two
  new zero-regression tests: literals, never constructors.
- **Committed in:** `8da4fee`.

### Corrections to the plan's stated premises

**4. There are four pre-existing Serve mappings on this node, not three.** The plan says three
(`:8443`, `:9446`, `:9447`); there is also one on the default `:443` proxying
`http://100.118.105.121:8086`, added since the 2026-08-21 research. 05-01 recorded the same
correction. Nothing about the plan changed — `8449` was still free — but the criterion "the three
pre-existing mappings intact" was evaluated as **all four intact, no fifth**, which is what it means.
The `md5` of `tailscale serve status` before and after this plan's script runs is
`f3df1875edd9a80b61d460fdb56a7ca0`, the same value 05-01 recorded.

**5. The `funnel` prohibition's second half needed care in prose, not just in code.** The predicate
`! grep -qE 'tailscale[^|;&]* funnel'` is case-sensitive and matches across a comment as readily as
across a command. The script therefore never writes lowercase `funnel` after lowercase `tailscale` in
any segment — the refusal is spelled `Funnel` in prose and the dispatch arm matches the literal
argument. Both halves verified: `grep -qi 'funnel'` finds it (3 occurrences), the predicate form
finds no invocation.

### Not done, deliberately

- **No bind-address configuration field appeared**, and `BIND_HOST` is unchanged. `grep -c 'pub
  bind\|bind: String\|bind_host'` in `settings.rs` is `0`; the three bind-refusal tests pass with
  unchanged text; and a new test asserts that an advertised URL is not a way to smuggle a wider bind
  in (`an_advertised_url_does_not_make_a_wider_bind_expressible`).
- **No `Cargo.toml` or `Cargo.lock` change.** `git diff --stat` on both is empty and
  `grep -A1 'name = "primeorder"' Cargo.lock | grep -c '0.14.0-rc.14'` is `1`.
- **The e2e suites were not edited to accommodate the change.** That was the point.

---

**Total deviations:** 3 auto-fixed (1 blocking, 1 missing-critical, 1 bug) plus 2 premise
corrections.
**Impact on plan:** No scope creep. The `app.rs` edit is unavoidable given the type change and is
the smallest form of it; the CHANGELOG entry follows this phase's own precedent; the tautology repair
strengthened an assertion the plan depends on.

## Verification

| Check | Result |
|-------|--------|
| `cargo build --release --locked` | exit 0 |
| `cargo clippy --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --locked` | exit 0, **336 passing** (310 shell + 24 protocol + 2 mcp; 9 ignored doc-tests, pre-existing) |
| `python3 tests/e2e/run_all.py` | exit 0, **22/22 PASS**, `failed: none` |
| `python3 tests/e2e/http_transport_test.py` **unmodified** | exit 0, `git diff --stat` empty |
| `python3 tests/e2e/oauth_flow_test.py` **unmodified** | exit 0, `git diff --stat` empty |
| `grep -c 'format!("http' crates/talaria-shell/src/oauth.rs` | **0** |
| `grep -c 'format!("https' crates/talaria-shell/src/oauth.rs` | **0** |
| `grep -ci 'advertised'` — settings / oauth / http | 72 / 56 / 26 |
| `grep -c 'const BIND_HOST' crates/talaria-shell/src/http.rs` | 1, value `"127.0.0.1"` unchanged |
| `grep -c 'pub const LOOPBACK_BIND' crates/talaria-shell/src/settings.rs` | 1, value unchanged |
| `grep -c 'pub bind\|bind: String\|bind_host' crates/talaria-shell/src/settings.rs` | **0** |
| `grep -ci 'Phase 5 is where a non-loopback bind gets considered'` | **0** |
| `! grep -rqi 'certificate\|\.pem\|cert_path\|ssl_cert\|private key' crates/talaria-shell/src/` | exit 0 |
| `grep -c 'PUBLIC_DISCOVERY_PATHS: \[&str; 2\]' crates/talaria-shell/src/http.rs` | 1 |
| `grep -c 'header::HOST' crates/talaria-shell/src/http.rs` | 1 |
| `! grep -q 'unwrap()' oauth.rs http.rs` | exit 0 |
| `grep -c '#\[test\]' crates/talaria-shell/src/settings.rs` | 51 (was 38; +13, criterion was ≥ +11) |
| `test -x scripts/tailscale-serve.sh` / `bash -n` | exit 0 / exit 0 |
| `grep -qi 'funnel'` / `! grep -qE 'tailscale[^\|;&]* funnel'` | named (3×) / never invoked |
| up → down → `tailscale serve status` | byte-identical, md5 `f3df1875edd9a80b61d460fdb56a7ca0` |
| `tailscale serve status \| grep -c '127.0.0.1:5678'` | 1 (the unrelated service, intact) |
| `grep -c '^## ' deferred-items.md` / `grep -c 'T-05-16'` | 10 / 2 |
| `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'` | 1 |

The e2e suite ran on display `:95` with a private `XDG_RUNTIME_DIR` — **not `:98`**, which is this
machine's self-hosted CI runner's display, where two concurrent runs SIGKILL each other
(`deferred-items.md` records this).

## Issues Encountered

None. Every failure encountered was a compile error from the deliberate type change, resolved in the
same task.

## Known Stubs

None. Every path this plan added is wired end to end: the configuration key is read at startup, the
identity is constructed in `serve`, and both layers that check a `Host` read the same allowlist.

## Threat Flags

None. This plan added no network endpoint, no route, no auth path, no file access pattern and no
schema at a trust boundary. It changed *what two existing layers admit* and *what six existing
strings say*, both from one configuration value with a single producer — which is the
`<threat_model>`'s subject rather than new surface. The one Serve mapping created during Task 3's
verification was torn down in the same task and the teardown is asserted by `md5`, not intended.

## User Setup Required

None required to build or test. To actually reach this browser over a tailnet:

1. Turn remote access on in Talaria's Access panel (or set `remote_access.enabled`).
2. `./scripts/tailscale-serve.sh up` — it prints the exact `config.json` key its mapping implies.
3. Write that `advertised_url` into `config.json` **verbatim** and restart the browser; settings are
   read once, at startup.
4. `./scripts/tailscale-serve.sh down` when finished.

## Next Phase Readiness

- **05-05** can mount its WebSocket route knowing `refuse_page_originated` admits the advertised
  authority, so a proxied upgrade reaches the router at all — and knowing (from 05-01) that no
  `Origin` is synthesised, so the CSWSH refusal needs no relaxation.
- **05-07**'s client connects to `wss://<advertised>`, which is now exactly the origin the metadata
  documents publish; there is one spelling to configure it with.
- **05-11** has `SECURITY.md` material that this plan deliberately did not write: the loopback-only
  bind is now permanent rather than provisional; transport security is terminated by a daemon this
  process does not talk to; Funnel is a named non-option; and the hop from the daemon to this browser
  is still plain HTTP on loopback, so every local account can still reach the listener directly and
  the bearer token remains the entire local boundary — the Phase 4 posture, unchanged.
- **Open, and recorded:** the advertised identity has no automatic discovery, and nothing verifies
  that the key a human wrote matches the mapping that exists. Both `deferred-items.md` entries state
  the condition for revisiting.

---
*Phase: 05-distributed-mode*
*Completed: 2026-08-23*

## Self-Check: PASSED

All listed files exist on disk; all three task commit hashes (`e3d1b67`, `8da4fee`, `c708d59`)
are present in `git log`. Every acceptance criterion in the Verification table above was executed
and its output read, not assumed.

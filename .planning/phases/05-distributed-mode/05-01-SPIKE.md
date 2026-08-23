# 05-01 Spike — does Tailscale Serve proxy a WebSocket upgrade to loopback?

**Assumption under test:** `05-RESEARCH.md` **A2**, the assumption D-05-03's entire exposure story
rests on.

> `tailscale serve` proxies a WebSocket upgrade to `http://127.0.0.1:PORT` and keeps it alive.

`05-CONTEXT.md` names this as required spike 2 and requires it settled *before* 05-04 and 05-05 are
executed. If it were refuted, Option 1 collapses to `05-RESEARCH.md`'s Option 2 — the exposure enum
with a certificate precondition plus in-process rustls — and the phase inherits certificate renewal
(threat T-05-16).

**Method:** a throwaway cargo example, `crates/talaria-shell/examples/serve_ws_spike.rs`, compiled
against the workspace's real resolved dependency graph — which is why it ran after Task 1's `axum`
`ws` landing rather than in isolation. Driven by a hand-written RFC 6455 client over Python's stdlib
`ssl` and `socket`, no third-party client, because the e2e suites are stdlib-only and
`remote_view_test.py` will carry the same decoder. Both throwaway files were deleted at the end of
plan 05-01 Task 2; only these findings survive.

---

## Verdict

A2: CONFIRMED

Tailscale Serve `1.102.2` proxies an RFC 6455 upgrade to a loopback `axum` service cleanly. The
handshake returns `101 Switching Protocols` with a **correct** `Sec-WebSocket-Accept`, text frames
echo in both directions, and the connection survives idle intervals of **30 s and 90 s** — both well
past `McpAppState`'s 12 s ping interval — then carries traffic again without a reconnect.

D-05-03 stands as written. `BIND_HOST` stays a constant, Talaria handles no certificates, and 05-05's
frame channel can be an `axum` WebSocket route on the Phase 4 router.

**But one observed fact changes 05-04's work, and it is the more valuable half of this spike:**
Serve forwards `Host: thinkpad.tailcd3cc6.ts.net:8449` — the **tailnet name and the Serve port**, not
the loopback target it proxies to. `refuse_page_originated` compares `Host` against the *bound*
address, so **as things stand every Serve-proxied request would be refused**. 05-04 must teach the
allowlist the advertised identity. That is the branch this spike existed to pick between, and it is
now picked on observation rather than on a guess.

---

## Versions and exact commands

| What | Version / value |
|------|-----------------|
| `tailscale` | `1.102.2` (commit `6cac918179d4d673bfebe2fc74f81183ddd73fea`, long version `1.102.2-t6cac91817-g6ff0ddc72`) |
| Node | `thinkpad`, `100.118.105.121`, MagicDNS name `thinkpad.tailcd3cc6.ts.net` |
| `CertDomains` | `["thinkpad.tailcd3cc6.ts.net"]` — Serve can front a real certificate |
| Throwaway listener | `127.0.0.1:40771` (the 40000–40999 range the global dev-server rule reserves) |
| Serve port | **`8449`** — deliberately distinct from the four mappings already live on this node |
| Date run | 2026-08-23 |

```
tailscale serve status                                   # baseline, captured first
tailscale serve --bg --https=8449 http://127.0.0.1:40771
#   ... probes ...
tailscale serve --https=8449 off
tailscale serve status                                   # diffed against the baseline
```

**Correction to the plan's premise, worth recording.** `05-CONTEXT.md` and `05-01-PLAN.md` both name
**three** pre-existing Serve mappings (`:8443`, `:9446`, `:9447`). At execution there were **four** —
a mapping on the default `:443` proxying `http://100.118.105.121:8086` had been added since the
2026-08-21 research. The research put a seven-day validity on every Tailscale claim and this is
exactly why. `8449` was still free, so nothing about the plan changed; the count did.

---

## Did the upgrade complete?

Yes, through the proxy, on the first attempt and on every repeat.

```
HANDSHAKE STATUS: HTTP/1.1 101 Switching Protocols
HANDSHAKE RESPONSE HEADERS:
    Connection: upgrade
    Date: Sun, 23 Aug 2026 10:47:35 GMT
    Sec-Websocket-Accept: gnSvGk8Pa/2qBMt8Vj1k6lRRj1o=
    Upgrade: websocket
Sec-WebSocket-Accept expected=gnSvGk8Pa/2qBMt8Vj1k6lRRj1o= observed=gnSvGk8Pa/2qBMt8Vj1k6lRRj1o= match=True
```

The `Sec-WebSocket-Accept` value was independently recomputed client-side as the base64 of
`SHA-1(key + GUID)` and compared, rather than merely being present. The probe's own derivation was
first validated against RFC 6455 §1.3's worked example
(`dGhlIHNhbXBsZSBub25jZQ==` → `s3pPLMBiTxaQ9kYGzzhZRbK+xOo=`) — the first run of this spike reported
`match=False` because the probe carried a mistyped GUID constant, and that was a bug in the
measurement, not a finding about Serve. It is recorded because a false `REFUTED` here would have
re-planned 05-04 onto the fallback for nothing.

## Did it survive a keep-alive interval?

Yes, at 30 s and again at 90 s. `McpAppState`'s ping interval is 12 s, so both clear it; a connection
that established and died quietly at ~20 s would have been a `REFUTED` dressed as a pass.

```
  t+  0.0s opcode=1 echo=1:hello-0
  t+  0.0s opcode=1 echo=2:hello-1
  t+  0.0s opcode=1 echo=3:hello-2
  idling 90s (longer than the 12s McpAppState ping interval) ...
  t+ 90.0s opcode=1 echo=4:after-idle-0
  t+ 90.0s opcode=1 echo=5:after-idle-1
```

The counter continues from `3` to `4` across the idle, which proves it is the *same* server-side
socket rather than a transparent reconnect: the counter lives in the `echo_frames` task's own stack.

---

## The complete header set a Serve-proxied request carries

Verbatim, echoed by the throwaway's plain `GET /` route through
`https://thinkpad.tailcd3cc6.ts.net:8449/`:

```
accept-encoding: gzip
host: thinkpad.tailcd3cc6.ts.net:8449
tailscale-headers-info: https://tailscale.com/s/serve-headers
tailscale-user-login: CourtimusPrime@github
tailscale-user-name: Court
tailscale-user-profile-pic: https://avatars.githubusercontent.com/u/190138327?v=4
user-agent: talaria-a2-spike
x-forwarded-for: 100.118.105.121
x-forwarded-host: thinkpad.tailcd3cc6.ts.net:8449
x-forwarded-proto: https
```

The same request made **directly** to `http://127.0.0.1:40771/`, for contrast:

```
connection: close
host: 127.0.0.1:40771
user-agent: talaria-a2-spike
```

### `Host` — the fact 05-04 is written against

**`host: thinkpad.tailcd3cc6.ts.net:8449`.**

Serve forwards the name and port **the client used**, not the loopback target. It does not rewrite
`Host` to `127.0.0.1:40771`. So the two branches this spike existed to choose between resolve as
follows:

- **`refuse_page_originated` would refuse every proxied request today.** Its `addressed_here` check
  is `headers.get(HOST) == bound`, and `bound` is the listener's own `local_addr()` —
  `127.0.0.1:8779`. A proxied request arrives claiming a different host and is answered with the
  403 refusal body.
- **`DnsRebindProtector::new(Some(vec![bound]))` has the same problem** from the same seed.
- So 05-04's advertised-base-URL work is **required**, not optional, and it must feed the `Host`
  allowlist as well as `canonical_resource`, `issuer` and the four endpoint URLs.

**And it must come from configuration, never from the request's `Host` header** (Pitfall 8,
threat T-05-09). The header is what an attacker controls; deriving the browser's own advertised
identity from it would let a local page make the browser advertise an issuer of the attacker's
choosing. The value is a configured advertised base URL, or one read from `tailscale status`.

Note also that the port travels with the name. An advertised identity of just
`thinkpad.tailcd3cc6.ts.net` would not match `thinkpad.tailcd3cc6.ts.net:8449`; the configured value
has to carry the Serve port.

### `Origin` — does the proxy synthesise one?

**No.** No `Origin` header appears on a proxied request that did not send one — the header set above
is the complete list, and `origin` is absent from it.

This is the answer T-05-01 needed. `refuse_page_originated` is the outermost layer over every route
and refuses *any* `Origin`, which is what closes cross-site WebSocket hijacking by construction and
what makes a browser-based viewer structurally impossible. A proxy that added one would have broken
every legitimate client. It does not.

Two supporting observations:

- A proxied request that **does** send `Origin: https://evil.example` arrives with
  `origin: https://evil.example` intact — Serve passes it through unchanged, neither stripping nor
  rewriting it. So the refusal still fires on exactly the requests it should.
- The throwaway itself accepted an upgrade carrying an `Origin`, both directly on loopback and
  through the proxy. That control matters: it establishes that the throwaway has no opinion about
  `Origin`, so nothing observed here is the throwaway's behaviour standing in for `http.rs`'s. The
  real refusal lives in `http.rs` and is unaffected.

### Identity and forwarding headers — recorded, and **not** a boundary

| Header | Observed value |
|--------|----------------|
| `tailscale-user-login` | `CourtimusPrime@github` |
| `tailscale-user-name` | `Court` |
| `tailscale-user-profile-pic` | `https://avatars.githubusercontent.com/u/190138327?v=4` |
| `tailscale-headers-info` | `https://tailscale.com/s/serve-headers` |
| `x-forwarded-for` | `100.118.105.121` |
| `x-forwarded-host` | `thinkpad.tailcd3cc6.ts.net:8449` |
| `x-forwarded-proto` | `https` |

**None of these is a security boundary, and none may become one** (threat T-05-08). Between
`tailscaled` and Talaria the traffic is plain HTTP on loopback, so any local account on this machine
can connect straight to the bound port and send whatever it likes for every one of them. They are
recorded here precisely so that a later reader who notices `tailscale-user-login` and thinks "the
identity problem is already solved" finds this sentence first. The verified boundary is Phase 4's
bearer token, which is already built.

`x-forwarded-proto: https` is the one that is genuinely useful *as information*: it is how a
Serve-proxied deployment could tell it is behind TLS. It is still spoofable, so it may inform a log
line, never an authorization decision.

Two smaller mechanical facts worth carrying forward: Serve **drops** hop-by-hop headers
(`connection: close` did not reach the service) and **adds** `accept-encoding: gzip`. Neither
affects this phase, but a later reader comparing a direct and a proxied capture will see the
difference and should not read it as tampering.

---

## The client's own TLS WebSocket dependency, measured for 05-07

Measured in Task 1 by the same inherited procedure — hand-edit the manifests, resolve with
`cargo metadata`, read the lock diff, restore — with the manifests and lockfile confirmed
byte-identical afterwards by `md5sum`. Recorded here so 05-07 has one place to read it.

The client binary connects over `wss://`, so it needs `tokio-tungstenite` as a **direct** dependency
with a TLS feature. Against the post-Task-1 lockfile:

| Feature spelling | Packages added | Notes |
|------------------|----------------|-------|
| `rustls-tls-native-roots` | **0** | `rustls 0.23.43`, `tokio-rustls 0.26.4`, `rustls-pki-types 1.15.1` and `rustls-native-certs 0.8.4` are all already resolved in this tree |
| `rustls-tls-webpki-roots` | **0** | `webpki-roots` is already resolved too (both the `0.26.11` and `1.0.9` lines) |
| `native-tls` | 7 | `native-tls`, `tokio-native-tls`, `openssl`, `openssl-sys`, `openssl-macros`, `foreign-types`, `foreign-types-shared` — and a C dependency on OpenSSL. Listed only as the contrast that makes the rustls answer obvious; **do not take this branch** |

No package changed version and none was removed on any of the three. `primeorder` stayed at
`0.14.0-rc.14` throughout.

**So 05-07's client TLS is free.** Either rustls spelling costs zero packages;
**`rustls-tls-native-roots` is the recommendation**, because Serve's certificate is a real
Let's Encrypt certificate that chains to a public root the OS trust store already carries, and using
the platform store is the behaviour a user expects from a client on their own machine.
`tokio-tungstenite`'s `connect` and `handshake` features are on by default and cost nothing either.

---

## Teardown, asserted rather than intended

- `tailscale serve --https=8449 off` ran in the same task that created the mapping.
- `tailscale serve status` was captured before and after and compared: **byte-identical**, `md5sum`
  `f3df1875edd9a80b61d460fdb56a7ca0` both times. All four pre-existing mappings survive, including
  the `:8443` → `http://127.0.0.1:5678` one belonging to an unrelated running service.
- The throwaway listener was stopped and `127.0.0.1:40771` is no longer bound.
- `crates/talaria-shell/examples/serve_ws_spike.rs` was deleted. The findings survive; the code does
  not.

---

## What this settles, and what it hands to 05-04

| Question | Answer | Consequence |
|----------|--------|-------------|
| Does Serve proxy a WebSocket upgrade to loopback? | Yes, `101` with a correct accept key | D-05-03 stands; 05-05 builds an `axum` `ws` route |
| Does it survive a keep-alive interval? | Yes, at 30 s and 90 s, same server-side socket | No reconnect workaround needed in this phase |
| What `Host` does a proxied request carry? | `thinkpad.tailcd3cc6.ts.net:8449` — the client's, with the Serve port | 05-04 **must** widen the `Host` allowlist and the rebind protector to a configured advertised identity |
| Does the proxy synthesise an `Origin`? | No, and it passes a client-sent one through unchanged | `refuse_page_originated` needs no change; the CSWSH defence is intact |
| Are the Tailscale identity headers usable as a boundary? | No — trivially spoofable from loopback | Recorded as T-05-08; the bearer token remains the boundary |
| What does the client's `wss://` dependency cost? | Zero packages on either rustls spelling | 05-07 lands it knowing the number |

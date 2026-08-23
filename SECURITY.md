# Security

Talaria is a web browser that an AI agent can drive and a human can take over
mid-session. That combination has a threat model most browsers do not, and this
document is where the honest version of it lives.

Talaria is pre-1.0 and has not been audited. Do not use it to browse anything you
would not want an agent — or a bug in one — to reach.

## Reporting a vulnerability

Report privately through GitHub's [private vulnerability
reporting](https://github.com/CourtimusPrime/Talaria/security/advisories/new)
rather than in a public issue.

Useful reports include the version or commit, the transport in use (the Unix
control socket, the stdio MCP proxy, the loopback HTTP listener, or the remote
view channel that rides on it), and whether the attacker in your scenario is a
web page, an agent connected over MCP, another local user account, an
unauthenticated network peer, or a remote human holding a bearer token.

There is no bounty. This is a personal project.

## The trust model, stated plainly

Five parties, and they are not equally trusted.

**The human at the keyboard is the trust root.** Anything a user can do in a
normal browser, they can do in Talaria: type a `file://` URL into the address bar,
open a local page, browse anywhere. Talaria does not second-guess its own user,
and hardening work must not quietly restrict the human path in the course of
restricting the agent one.

**A remote human is the trust root at a distance, and that is a real
distance.** This is the fifth party, and it arrived with the remote view
channel. It sits between the human at the keyboard and a connected agent, in
both directions. *More* trusted than an agent, because it is the same person:
the sentence above — hardening must not quietly restrict the human path —
applies to them, and a remote takeover that a security measure made unusable
would have removed the feature rather than protected it. *Less* verifiable than
the human at the keyboard, because their identity is **a bearer token rather
than physical presence**. Nobody checked that the person at the far end is the
person who approved the token; what was checked is that the connection presents
a credential this browser issued and has not revoked.

And the part that is genuinely new: **this is the first party in this browser's
history that sends raw input rather than tool calls.** An agent asks for
`navigate`, `evaluate`, `screenshot` — a fixed vocabulary that a reviewer can
enumerate and a policy can be written against. A remote human sends a click at
a coordinate, a scroll, a keystroke. Raw input into an already-logged-in session
is a different kind of reach from an allowlisted vocabulary: there is no list of
what it can do, because it can do whatever the page in that tab can be made to
do by a person operating it. That is the feature, and the section
"What a remote viewer is, plainly" below says so without hedging.

**A connected agent is semi-trusted.** It gets a real, already-logged-in
session — that is the entire point of the product — but it is not the user and
does not inherit the user's reach. Agent-initiated navigation is allowlisted to
`http`, `https`, `data`, and the exact literal `about:blank`. Anything else is
refused with an error naming the scheme.

**Web content is untrusted**, as in any browser.

**An unauthenticated network peer is hostile.** This party did not exist before
Talaria had a network transport: anything that can open a TCP connection to the
HTTP listener, which means every other user account on this machine and every
page running in every browser on it. It is not a party you connected; it is a
party that can reach a port.

**And that party's size is now set outside this repository.** "Anything that can
open a connection to the loopback listener" was a sentence about one machine.
Under the overlay-network exposure this browser is designed for — `tailscaled`
terminating TLS and proxying to the unchanged `127.0.0.1` listener — the set
widens to *any device in the tailnet, plus whatever the tailnet's access rules
admit*, and those rules live in a web console, not in this source tree. Read the
old sentence as still bounding the set and you will be wrong by however much
somebody has shared that tailnet with.

What did **not** change is the local half. Between the overlay daemon and this
browser the traffic is still plain HTTP on loopback. Every local account on this
machine can still reach the port, `SO_PEERCRED` still has no TCP equivalent, and
the bearer token is still the entire local boundary. Fronting the listener with
a daemon that speaks TLS to the network did not make the listener itself
private.

That fourth party also *changes* the third. "A connected agent" used to be a
synonym for "a process running as you" — the Unix control socket says so with a
kernel-vouched peer-UID check, and the socket's own file permissions back it up.
Over HTTP it is not a synonym for anything of the kind. A connected agent is
whoever holds a valid bearer token, and nothing about holding one implies
running as you.

Because the human is the trust root and the agent is not, the two paths are
deliberately different code. `parse_agent_url` enforces the allowlist;
`resolve_location`, which serves the address bar, does not. A change that
collapses them into one path is a change to the trust model, not a refactor.

## What is enforced today

- **The remote HTTP transport is off by default, loopback-only, and requires a
  bearer token** — and every clause of that sentence is load-bearing. It is off
  until a `config.json` says otherwise, which means no thread and no bound port
  rather than a flag consulted per request. When on it binds `127.0.0.1` and
  only `127.0.0.1`; there is no bind-address setting to get wrong. **The
  peer-credential check has no TCP equivalent** — `SO_PEERCRED` does not exist
  for an AF_INET socket — and **loopback is not a boundary between local
  accounts**: any user on this machine can connect to `127.0.0.1:PORT`. The
  bearer token is therefore the entire boundary on this transport. Tokens are
  opaque, stored only as SHA-256 digests in a `0600` file, short-lived,
  individually revocable, and bound by audience to this exact server, so a
  token minted for some other service cannot be replayed here. Verification is
  a live read of that store on every request, so a revoked token stops working
  on the next one. Host validation and an outright refusal of any request
  carrying an `Origin` header cover the case of a local web page finding the
  port — a real MCP client sends no `Origin` and a page always does — but they
  are defence in depth, **not a substitute for the token**.
- **The remote view channel is off by default, checks the token itself, and can
  only ever name an agent's tab** — and, as above, every clause of that is
  load-bearing. `GET /view` exists only when the listener does, so a default
  installation has **no input channel at all**, not a closed one. It performs
  its **own** direct call to the shared authorization provider rather than
  inheriting a check: the SDK's middleware chain covers only its own transport
  handlers, so a route merged beside them would have been origin-checked and
  host-checked and *unauthenticated*, which is the worst of the three
  combinations because it looks protected. A viewer may attach to **agent tabs
  only**, through `TabManager::agent_tab`, a lookup that cannot return a
  human-owned tab — so remoting one of your own tabs is *unrepresentable* on
  this path rather than refused somewhere downstream. Refusals carry no reason:
  "that tab belongs to the human", "there is no such tab" and "the attachment
  cap is full" are one answer, so the channel is not an enumeration oracle for
  what you have open. Input reaches a webview and nothing else — no chrome, no
  browser-shortcut handler, no panel, no active-tab state, no view mode, no
  window focus — which is why a click at the credentials control's real
  coordinates opens nothing at all. Revoking a viewer **closes its socket**, not
  merely its next message, and that closure is asserted from the client's own
  side of the wire in `tests/e2e/revocation_test.py`. The per-connection input
  sequence is strictly increasing and a non-increasing value is dropped, so a
  replay within a connection is a no-op.
- **Approval is a native control, not a web page.** Getting a token means
  getting a human's approval, and that approval happens in the browser's own
  chrome. The page an authorization request serves has no form, no button and
  no link on it, and repeats nothing the caller sent. `http` is in the agent
  navigation allowlist, so an agent holding `evaluate` can point a tab at this
  browser's own authorization endpoint; there is nothing on that page for it to
  click. Approve is disabled for a second after the prompt appears, so a click
  already in flight when an uninvited prompt arrives cannot land on it, and at
  most one prompt is on screen at a time.
- **Revocation is individual, and it closes open streams.** The Access panel
  lists every authorized agent and revokes one on a deliberate two-click
  gesture. See "What revocation guarantees" below for exactly what that does
  and does not do — the interesting part is that stopping *new* requests is not
  sufficient on its own.
- **Agent navigation scheme allowlist** — `tabs_open` and `navigate` refuse
  `file:`, `javascript:`, `blob:` and everything else outside the allowlist.
  Proven by `tests/e2e/scheme_refusal_test.py`, which also asserts the human path
  is untouched.
- **Control-socket peer authentication** — the socket verifies the connecting
  peer's UID and fails closed: a credential-lookup error rejects the connection,
  and a rejected peer gets a closed connection with no reply. The fallback socket
  lives at `talaria-$UID/talaria.sock` inside a `0700` per-UID directory, so
  another local user cannot reach it. This peer-credential check is still
  correct and still load-bearing; the HTTP listener does not replace it, and
  nothing about the token work weakened it.
- **Bounded downloads** — the `download` tool enforces a byte cap computed from
  bytes actually read and written, never from the attacker-controlled
  `Content-Length`. An unparseable `TALARIA_MAX_DOWNLOAD_BYTES` falls back to the
  2 GiB default, never to zero and never to unbounded.
- **Encrypted credential vault** — credentials are stored with
  ChaCha20-Poly1305 under a key held in the OS keychain. Importing a plaintext
  `vault.json` removes the source only after re-reading, decrypting, and
  matching the entry count of the encrypted file; a failed verification chmods
  the source to `0600` and logs an error rather than deleting it.
- **Autofill suggests, it never injects** — a matching credential is offered in
  the browser chrome with copy controls. Talaria does not write your password
  into the page's DOM, where every script on that page could read it, and where
  it would collide with an agent's own `evaluate` scripting the same form.

### Why the remote viewer is a native program, and must stay one

This is a property, not an inconvenience, and it is written down here because it
does not look like one from the outside.

`build_router` applies `refuse_page_originated` as the **outermost** layer over
every route, and it refuses any request that carries an `Origin` header at all —
not one from a disapproved origin, *any*. WebSockets are not subject to the
same-origin policy and they have no preflight, so a browser will happily open one
across origins and hand the response to the page's script; origin enforcement on
a WebSocket is entirely the server's job, and forgetting it is precisely what
cross-site WebSocket hijacking (CSWSH) is. Talaria's layer closes that class **by
construction** rather than by remembering to check, and it closes it in the
strictest available direction: a browser always sends `Origin`, so **a browser
can never connect to this listener at all**.

The consequence is that `talaria-client` is a native binary as a *result* of that
layer, not as a preference. A browser-based remote viewer for Talaria is
structurally impossible, and that is the intended state.

**What its deletion would look like**, so that a reader who meets one recognises
it: a diff that replaces the blanket refusal with an **origin allowlist**, most
likely justified as "so a small web viewer can connect". That change does not
add a feature to an unrelated defence — it removes the property above. If it is
ever made deliberately, CSWSH becomes a live threat class against a process
holding a credential vault and a logged-in session, and the reasoning for
accepting that belongs in this document beside the diff.

### The public-internet exposure variant is refused, by name

Tailscale Serve proxies a tailnet-private name to a loopback port. **Tailscale
Funnel** is the sibling command that does the same thing to the **open
internet**. They differ by a few characters on the command line.

Funnel is not an option for Talaria and is refused rather than merely omitted:
`scripts/tailscale-serve.sh` refuses an argument asking for it, and the reason is
in the refusal. The process on the other end of that mapping holds an encrypted
credential vault and a human's logged-in browsing sessions, and the entire local
boundary is a bearer token. Publishing it to the internet moves the fourth party
from "devices on a private overlay network" to "everyone", against a boundary
that was never designed for that.

A named non-option is safer than an unconsidered one. If you need reach beyond a
tailnet, that is a fronting-proxy design question, not a one-word substitution.

### What a remote viewer is, plainly

An authorised remote viewer **is a remote-control primitive**. By design. That is
the feature, and none of the layers above stop it — nor should they. The boundary
is *which clients you authorise*: the same sentence this document already uses
about agents, now with sharper teeth, because a viewer sends real clicks into a
logged-in session rather than tool calls into an allowlisted surface. Approve a
viewer only where you would hand somebody the machine.

The asymmetry is the interesting part, so state what a viewer still cannot do. It
cannot see or drive the human's own tabs — they are not in the snapshot and not
nameable by the lookup. It cannot reach the chrome, so it cannot touch the
credentials control, the bookmark star, or a downloads row's handoff to the
operating system's default application. It cannot open a tab, navigate one, close
one, evaluate script in one, or download anything: the view channel has no
message for any of those, which is an absence in the wire types rather than a
runtime check. And it cannot move, refocus or blank anything on the local
screen — attaching holds a tab *shown*, deliberately not focused, and the tab the
local human is displaying always wins.

### The overlay daemon's injected identity headers are not a boundary

A Serve-proxied request arrives carrying `tailscale-user-login`,
`tailscale-user-name` and `tailscale-user-profile-pic`, filled in by the daemon
from the tailnet's own identity. They are genuinely useful and they are also
**spoofable by any local process that can reach the loopback port**, which is
every account on this machine — the daemon is not the only thing that can send
bytes to `127.0.0.1:PORT`. The bearer token is checked first and these are at
most a second check. Treating them as authentication would hand the listener to
the weakest party in the model.

## What revocation guarantees

Revoking an agent in the Access panel is worth stating precisely, because the
obvious reading of it is slightly too strong in one place and slightly too weak
in another.

**Two things are guaranteed.** The agent's *next* request is refused, with the
same answer a token that was never issued gets — verification is a live read of
the token store on every request, so a record removed a microsecond ago is not
there to be found. And any response stream the agent already had open is
**closed**, which matters more than it sounds: a long-lived event stream has no
next request to fail, so a revocation that only stopped new requests would leave
a revoked agent still receiving from the browser while the panel showed its row
gone. Talaria closes the connection that stream is riding on, so the agent's
read side sees it end. Both are asserted end to end, from the agent's side of
the wire, in `tests/e2e/revocation_test.py`.

**One thing is not, and the honest answer is that it should not be.** A request
that had already passed verification when the revoke landed runs to completion.
Verification is per request, and a command already executing against the browser
engine is not interruptible — there is no way to stop it that does not amount to
tearing down the page it is acting on. The boundary is the verification, not the
click.

**Revocation survives a restart.** The client's registration goes with its
tokens, so the row does not reappear later without a human approving it again.

**What revocation deliberately leaves alone: the agent's tabs.** They stay open.
They are visible in the Agents view, and the human can take any of them over —
closing somebody's tabs because a credential was withdrawn would destroy state
they may want, and the agent cannot drive them any more, which is the whole of
what a revoke is for. Nothing else of the human's is touched either: no history
row, no bookmark, no stored credential, no downloaded file.

A client can also revoke its **own** token through the standard endpoint
(RFC 7009). That is a different act — a client disowning a credential rather
than a human withdrawing access — so it drops the token and its family and
leaves the registration alone. It answers success for any token, valid or not,
as the specification requires: an endpoint that said which was which would tell
an unauthenticated caller which tokens exist.

## Known limitations

These are open, they are known, and they are listed here rather than in a
tracker's "in progress" column because neither is blocked on someone finding the
time. Both are blocked on something external: one on a design conflict that has
to be resolved before code can be written, one on an upstream engine capability
that does not exist yet.

### An agent with `evaluate` can still reach the local filesystem

**Requirement:** MCP-09, partially closed.

The navigation allowlist covers the tools that take a URL. It does not cover
JavaScript that an agent has already been allowed to run. An agent that calls
`evaluate` on a tab it owns can set `location.href = 'file:///etc/passwd'`, or
call `window.open` with the same, and then read the content back out of the
resulting document. This has been reproduced end to end; it is not theoretical.

**Why it is still open.** Closing it means enforcing policy at
`WebViewDelegate::request_navigation` and `request_create_new` — the point where
the engine asks the shell whether a navigation may proceed — rather than at the
tool boundary. That hook does not distinguish *who* asked. It fires the same way
for a navigation an agent's script triggered and for one the human triggered by
clicking a link during takeover. A naive policy there would break the rule that
the human is the trust root: take over an agent's tab, click a local file link,
get refused in your own browser.

**What closing it takes:** propagating enough provenance into the navigation
decision to tell script-initiated navigation on an agent-owned tab apart from
human-initiated navigation on the same tab, then denying only the former. That is
a design change, and it needs to be designed before it is written.

**What reduces the risk meanwhile:** treat `evaluate` as equivalent to filesystem
read access for the user account running Talaria, and do not connect agents you
would not grant that.

**A network transport makes that advice matter more, not less.** The bound was
never technical — it was "you chose to connect this agent, locally, on purpose".
The HTTP listener widens who can become that agent from "a process you started"
to "anything holding a token". The token is a real boundary and it is checked on
every request, but the thing on the far side of it still gets filesystem read
access for your account. Approve agents accordingly, and revoke ones you no
longer use.

**And a remote human widens it once more, in a way no allowlist reaches.** An
authorised viewer cannot call `evaluate` — the view channel has no such
message — but it can type into the page, and a page the human is logged into is a
page whose own scripting the human can invoke. The party on the far side of this
gap is now "anything holding a token" *and* "anybody who can click".

### A wedged script still costs one command timeout

**Requirement:** MCP-10, partially closed.

A page whose JavaScript never yields — an infinite loop, a pathological
regex — will hold the tab's script thread. The *first* `evaluate` against such a
tab runs until the control-socket command timeout expires (30s by default) and
never returns a result.

What is fixed: that tab no longer poisons anything else. Per-tab in-flight
tracking means a second `evaluate` against the same tab is refused immediately
with `tab {id} busy — a previous evaluate is still running` rather than queueing
behind the first, and `screenshot`, `tabs_close`, `tabs_focus` and `tabs_list`
keep answering for that tab and every other one. Both sides of the control socket
are pipelined, so a stalled call does not serialize the connection.

**Why it is still open.** Interrupting a script that will not yield requires the
JavaScript engine to interrupt it. SpiderMonkey has a slow-script interrupt
callback; libservo does not currently expose it. This cannot be fixed in this
repository.

**What reduces the risk meanwhile:** lower `TALARIA_COMMAND_TIMEOUT_SECS` if your
agent should give up sooner, and close the tab — `tabs_close` still works.

The same widening applies here in a smaller way: an authorized remote client can
also wedge a tab and spend one command timeout doing it, and so can a remote
human who merely clicks a link to a page whose script never yields. The cost is
bounded and per-tab, and it is one more reason the set of clients you approve
should be the set you actually use.

## Out of scope, on purpose

Talaria is infrastructure, not a policy layer. Per-agent permission scoping, rate
limiting, and audit trails are deliberately not features: an agent that connects
gets the tool surface, and the security boundary is which agents you let connect.
If you need to constrain what a particular agent may do, constrain it in the
agent, or do not connect it.

The control socket has no authentication beyond the peer-UID check, and needs
none: it is a Unix domain socket at `0600` inside a `0700` directory, so an
attacker who can reach it already runs as you and can read this browser's config
directory — including its token store — directly. The stdio MCP proxy is
unauthenticated for the same reason, and deliberately: the MCP specification
itself instructs stdio implementations to take credentials from the environment
rather than to follow the authorization specification. That is conformance, not
a concession.

Authentication arrived with the network transport, because remote access is what
made it necessary. What is deliberately **not** yet true, so that nobody reads
more into this than is there:

- **There is no non-loopback bind, and there never will be.** The listener binds
  `127.0.0.1` and only `127.0.0.1`, and that is now **permanent rather than
  provisional**. The bind host is a module constant used at exactly one place;
  `config.json` cannot express an address, and a hand-edited `bind` key is
  refused out loud. Remote reach is not obtained by widening the bind — it is
  obtained by putting an overlay-network daemon in front of it, which terminates
  TLS and proxies to the loopback port that was already there.

  So **this browser issues, loads and renews no certificate at all**: no key on
  disk, no expiry timer, no resolver re-reading files, no day-ninety-one
  outage in the process that also holds your credential vault. `config.json`
  gains only `remote_access.advertised_url` — the origin clients actually reach
  this browser at — and every OAuth URL this browser publishes uses the secure
  scheme once that identity is set, which is OAuth 2.1 §1.5 **met** rather than
  excepted. (With nothing advertised, the loopback exception still applies and
  every published string is byte-identical to what shipped before.)

  What it costs: exposure requires an overlay-network node on both ends. Anyone
  wanting a different fronting proxy is on their own documented path, and it is a
  fronting proxy — not a certificate feature added to this process.
- **There is still no per-agent permission scoping**, per the section above.
  A token is binary: an agent that holds one gets the tool surface.
- **There is no audit trail.** Talaria logs what it did; it does not keep a
  record for you to review later, and the Access panel shows when an agent was
  authorized rather than what it has been doing. Revoking is the control, not
  reading back a history of what happened before you revoked.
- **The authorization code path is new and unaudited**, like the rest of this
  transport. It implements OAuth 2.1 with PKCE `S256` and refresh rotation, and
  it has unit and end-to-end tests for every way each of those is meant to
  fail — which is not the same thing as having been looked at by somebody who
  breaks these for a living.

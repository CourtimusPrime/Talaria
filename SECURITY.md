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
control socket, the stdio MCP proxy, or the loopback HTTP listener), and whether
the attacker in your scenario is a web page, an agent connected over MCP,
another local user account, or an unauthenticated network peer.

There is no bounty. This is a personal project.

## The trust model, stated plainly

Four parties, and they are not equally trusted.

**The human at the keyboard is the trust root.** Anything a user can do in a
normal browser, they can do in Talaria: type a `file://` URL into the address bar,
open a local page, browse anywhere. Talaria does not second-guess its own user,
and hardening work must not quietly restrict the human path in the course of
restricting the agent one.

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
also wedge a tab and spend one command timeout doing it. The cost is bounded and
per-tab, and it is one more reason the set of clients you approve should be the
set you actually use.

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

- **There is no TLS, and there is no non-loopback bind.** Both are the next
  milestone's. They belong together: staying on loopback is what makes shipping
  without TLS *conformant* rather than skipped — OAuth 2.1 requires HTTPS for
  authorization-server endpoints with an explicit loopback exception, and
  Talaria takes that exception rather than ignoring the requirement. A bind to
  any other address needs a certificate story first.
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

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

Useful reports include the version or commit, the transport in use (stdio today),
and whether the attacker in your scenario is a web page, an agent connected over
MCP, or another local user account.

There is no bounty. This is a personal project.

## The trust model, stated plainly

Three parties, and they are not equally trusted.

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

Because the human is the trust root and the agent is not, the two paths are
deliberately different code. `parse_agent_url` enforces the allowlist;
`resolve_location`, which serves the address bar, does not. A change that
collapses them into one path is a change to the trust model, not a refactor.

## What is enforced today

- **Agent navigation scheme allowlist** — `tabs_open` and `navigate` refuse
  `file:`, `javascript:`, `blob:` and everything else outside the allowlist.
  Proven by `tests/e2e/scheme_refusal_test.py`, which also asserts the human path
  is untouched.
- **Control-socket peer authentication** — the socket verifies the connecting
  peer's UID and fails closed: a credential-lookup error rejects the connection,
  and a rejected peer gets a closed connection with no reply. The fallback socket
  lives at `talaria-$UID/talaria.sock` inside a `0700` per-UID directory, so
  another local user cannot reach it.
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

## Out of scope, on purpose

Talaria is infrastructure, not a policy layer. Per-agent permission scoping, rate
limiting, and audit trails are deliberately not features: an agent that connects
gets the tool surface, and the security boundary is which agents you let connect.
If you need to constrain what a particular agent may do, constrain it in the
agent, or do not connect it.

The control socket has no authentication beyond the peer-UID check, and needs
none while the transport is a Unix domain socket owned by one user. That changes
the moment a network transport exists, which is why the OAuth 2.1 work is
sequenced immediately after the HTTP transport rather than before it — auth's
driver is remote access, and there is no remote access yet.

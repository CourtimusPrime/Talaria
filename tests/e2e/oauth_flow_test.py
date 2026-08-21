#!/usr/bin/env python3
"""E2E: a client that knows only the MCP endpoint URL can discover how to
authenticate, and a client that holds a token drives the same browser the stdio
proxy drives.

What this pins down:

- **Discovery, starting from nothing but the endpoint URL.** That is the whole
  point and it is why nothing here hardcodes the metadata path: a suite that
  knew where the document lived would be testing a fact a real client does not
  have. An unauthenticated POST is answered ``401`` with a ``WWW-Authenticate``
  challenge naming a ``resource_metadata`` URL, and following that URL — with
  no credential — yields the RFC 9728 document naming this server's canonical
  resource identifier and its authorization server. ``04-RESEARCH.md``'s
  Pitfall 3 is a server that mints and validates tokens while publishing none
  of this, so a conformant client gives up before the flow begins.
- **The document advertises nothing the browser does not implement.** In
  particular no client-metadata-document capability (``04-CONTEXT.md``
  D-04-02), because a conformant client that prefers CIMD would be sent down a
  road that dead-ends.
- **Success Criterion 1.** An authorized client's ``tools/list`` over HTTP is
  *identical* — same count, names, descriptions and input schemas — to
  ``tools/list`` over stdio against the same running browser. 04-01 made that
  structurally true by putting both transports on one ``talaria_mcp`` library;
  this is the assertion that says so out loud. And it is not only a list: an
  authorized ``tools/call`` opens a real tab, which the control socket then
  reports as owned by the *verified* ``client_id`` rather than a name the
  caller typed.
- **Audience binding (RFC 8707, T-6).** A token minted for a different
  resource identifier is refused, and the refusal is *byte-identical* to the
  one an entirely unknown token gets — so the endpoint is not an oracle for
  which tokens exist.

The **consent half** (added by 04-06) starts over with an empty store and no
seeded credential at all, and drives the flow a real client drives: discover,
register, authorize with PKCE S256, and get a human's approval — by a **real
pointer click on the real Approve control** in the browser chrome. It also
pins every refusal that happens *before* a human is asked, the denial and
expiry paths, both anti-harassment cooldowns, the one-request-at-a-time cap,
and a deliberately hostile display name.

The **exchange half** (added by 04-07) closes Success Criterion 2: the code a
human's click produced becomes a token pair at the token endpoint, that token
opens and closes a real tab, the refresh token rotates, and every way the
exchange is supposed to fail does — a code redeemed twice, a code redeemed with
the wrong verifier and then with the right one, a code left to go stale, a
refresh token replayed after its family rotated, and a client that registered
and was never approved. The token endpoint's URL is read from the
authorization-server metadata like every other address here; nothing in this
file spells a Talaria path a real client would not have discovered.

Two things about how that half is driven, stated plainly because both look
like bypasses and neither is.

**The seeded ``agents.json`` in the discovery half** (``harness.write_agents``)
is what a *prior* approval leaves on disk. It grants nothing a real approval
would not have granted, nothing in the shell treats a seeded record specially,
and no environment variable, hook or code path mints or accepts a token for a
test.

**The three ``TALARIA_CONSENT_*`` variables** the consent half launches with
shorten how long a human has to answer and how long a cooldown lasts. They are
gated behind ``TALARIA_TEST_HOOKS=1``, they are clamped so they can only ever
make a window *smaller* — never longer, never zero, never off — and **none of
them answers a request**. Every approval below is a real click on the real
control, which is exactly why no auto-approving environment variable exists to
be tempted by: a one-variable authentication bypass in a process holding a
credential vault is the thing worth avoiding most. What they buy is time: at
the shipped values an expiry plus the cooldown it arms is two and a half
minutes, and an assertion that slow is one somebody quietly deletes.

``04-UI-SPEC.md`` Surface 1 records the one tradeoff this suite does rely on:
with ``TALARIA_TEST_HOOKS=1`` the ``chrome_rects`` mechanism can reach
``consent.approve``. That is precisely what lets the suite exercise the real
decision path rather than a substitute for it. The gate is on the *command*,
and ``panel_click_test.py`` pins that a shell started without the hook refuses
it outright.

Standalone (starts its own Xvfb and shell), with an isolated ``HOME`` and
``XDG_CONFIG_HOME`` because it writes a ``config.json`` and an ``agents.json``
and must not touch the real ones. Nothing here reaches the network: every
request is to a loopback port this suite started."""
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness

# The seeded authorization: a client this browser "registered" and a human
# "approved" at some earlier point, plus the one token it walked away with.
CLIENT_ID = "0e0f1a2b3c4d5e6f708192a3b4c5d6e7"
CLIENT_NAME = "oauth-flow-e2e"
TOKEN = "tal_e2e_access_2f4a6c8e0b1d3f5a7c9e1b3d5f7a9c1e"
# A token for a resource server that is not this one (T-6).
FOREIGN_TOKEN = "tal_e2e_foreign_9e7c5a3f1d9b7c5a3e1f9d7b5c3a1e9f"
FOREIGN_AUDIENCE = "http://127.0.0.1:1/mcp"

MCP_BINARY = os.path.join(harness.TARGET, "release", "talaria-mcp")

tmp = tempfile.mkdtemp(prefix="talaria-oauth-flow-e2e-")
config = os.path.join(tmp, ".config")
talaria = os.path.join(config, "talaria")
os.makedirs(talaria)
SHELL_LOG = os.path.join(tmp, "shell.log")


def request(url, body=None, method="POST", **headers):
    """One HTTP round trip. An error status is an answer here, not an
    exception — most of the interesting cases in this suite are refusals."""
    outgoing = urllib.request.Request(url, data=body, method=method)
    for name, value in headers.items():
        if value is not None:
            outgoing.add_header(name.replace("_", "-"), value)
    try:
        with urllib.request.urlopen(outgoing, timeout=20) as response:
            return response.status, dict(response.headers), response.read()
    except urllib.error.HTTPError as error:
        return error.code, dict(error.headers), error.read()


def header(headers, name):
    """A header by name, case-insensitively."""
    for key, value in headers.items():
        if key.lower() == name.lower():
            return value
    return None


def sse_payload(body):
    """The JSON of a Streamable-HTTP reply.

    The listener runs with the SDK's default ``enable_json_response: false``,
    so a reply arrives as a one-event ``text/event-stream`` rather than a bare
    JSON body. Both shapes are accepted here so this suite does not break if
    that default is ever flipped."""
    text = body.decode()
    for line in text.splitlines():
        if line.startswith("data:"):
            return json.loads(line[len("data:"):].strip())
    return json.loads(text)


class HttpMcpClient:
    """The minimum real Streamable-HTTP MCP client: initialize, notify, call."""

    def __init__(self, endpoint, token):
        self.endpoint = endpoint
        self.token = token
        self.session_id = None

    def _headers(self):
        return {"Content_Type": "application/json",
                "Accept": "application/json, text/event-stream",
                "Authorization": f"Bearer {self.token}",
                "Mcp_Session_Id": self.session_id}

    def call(self, method, params=None, message_id=1):
        message = {"jsonrpc": "2.0", "id": message_id, "method": method,
                   "params": params if params is not None else {}}
        status, headers, body = request(self.endpoint, json.dumps(message).encode(),
                                        **self._headers())
        if self.session_id is None:
            self.session_id = header(headers, "Mcp-Session-Id")
        return status, sse_payload(body) if body else None

    def notify(self, method):
        message = {"jsonrpc": "2.0", "method": method, "params": {}}
        status, _headers, _body = request(self.endpoint, json.dumps(message).encode(),
                                          **self._headers())
        return status

    def initialize(self):
        status, reply = self.call("initialize", {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": CLIENT_NAME, "version": "1.0"}})
        assert status == 200, (status, reply)
        assert self.session_id, "the server issued no session id"
        assert self.notify("notifications/initialized") in (200, 202)
        return reply["result"]


def rpc(command, client="oauth-flow-e2e", **params):
    """One control-socket request on its own connection.

    Every assertion about what the *browser* did goes over this socket rather
    than over HTTP, so a bug in the transport under test cannot also be the
    thing reporting success."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(harness.SOCK)
    try:
        f = s.makefile("rw")
        f.write(json.dumps({"type": "hello", "client": client}) + "\n")
        f.flush()
        assert json.loads(f.readline())["type"] == "hello_ack"
        f.write(json.dumps({"type": "request", "id": 1, "command": command,
                            **params}) + "\n")
        f.flush()
        while True:
            m = json.loads(f.readline())
            if m.get("type") == "reply" and m.get("id") == 1:
                return m
    finally:
        s.close()


def accepts(host, port, timeout=1.0):
    s = socket.socket()
    s.settimeout(timeout)
    try:
        s.connect((host, port))
        return True
    except OSError:
        return False
    finally:
        s.close()


def wait_until_accepting(host, port, seconds=20.0):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if accepts(host, port):
            return True
        time.sleep(0.2)
    return False


def stdio_tools():
    """initialize + tools/list against a freshly spawned talaria-mcp.

    The same round trip ``mcp_client_test.py`` makes, kept here rather than
    imported so this suite stays standalone."""
    proc = subprocess.Popen([MCP_BINARY], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, text=True, bufsize=1)
    try:
        def send(message):
            proc.stdin.write(json.dumps(message) + "\n")
            proc.stdin.flush()

        def reply(wanted):
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                line = proc.stdout.readline()
                if not line:
                    break
                try:
                    m = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if m.get("id") == wanted:
                    return m
            raise AssertionError(f"talaria-mcp never answered {wanted}")

        send({"jsonrpc": "2.0", "id": 1, "method": "initialize",
              "params": {"protocolVersion": "2025-11-25", "capabilities": {},
                         "clientInfo": {"name": "oauth-flow-e2e", "version": "1.0"}}})
        assert "result" in reply(1)
        send({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})
        send({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
        return reply(2)["result"]["tools"]
    finally:
        try:
            proc.stdin.close()
            proc.wait(timeout=5)
        except (OSError, ValueError, subprocess.TimeoutExpired):
            proc.kill()
            proc.wait()


def normalise(tools):
    """A tool list in a form two transports can be compared byte for byte."""
    return json.dumps(sorted(tools, key=lambda tool: tool["name"]), sort_keys=True)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    """An opener that reports a redirect instead of following it.

    Most of the interesting answers in the consent half *are* redirects — an
    authorization code, an ``access_denied``, a cooldown refusal — and what
    they carry in ``Location`` is the assertion. Following them would replace
    that with the callback server's own bland 200."""

    def redirect_request(self, *args):
        return None


NO_REDIRECT = urllib.request.build_opener(NoRedirect)


def unfollowed(url, body=None, method="GET", **headers):
    """One round trip with redirects reported rather than followed."""
    outgoing = urllib.request.Request(url, data=body, method=method)
    for name, value in headers.items():
        if value is not None:
            outgoing.add_header(name.replace("_", "-"), value)
    try:
        with NO_REDIRECT.open(outgoing, timeout=20) as response:
            return response.status, dict(response.headers), response.read()
    except urllib.error.HTTPError as error:
        return error.code, dict(error.headers), error.read()


def query_of(url):
    """The query of a URL as a flat dict."""
    parsed = urllib.parse.urlsplit(url or "")
    return {key: values[-1] for key, values
            in urllib.parse.parse_qs(parsed.query).items()}


def find_window(env):
    """The Talaria window's X id, for driving a real pointer at it."""
    for _ in range(15):
        out = subprocess.run(["xdotool", "search", "--name", "Talaria"], env=env,
                             capture_output=True, text=True).stdout.split()
        if out:
            return out[0]
        time.sleep(1)
    raise AssertionError("Talaria window never appeared")


def stop_shell(shell):
    """Stop the shell and wait for its socket to go."""
    shell.terminate()
    for _ in range(60):
        if shell.poll() is not None:
            break
        time.sleep(0.1)
    else:
        shell.kill()
    shell.wait()
    for _ in range(60):
        if not os.path.exists(harness.SOCK):
            return
        time.sleep(0.1)
    try:
        os.remove(harness.SOCK)
    except FileNotFoundError:
        pass


xvfb = harness.start_xvfb()
log = None
tal = None
callback = None
consent_home = None
try:
    log = open(SHELL_LOG, "w")

    # --- 1. a browser with one prior authorization on disk ---------------
    port = harness.free_port()
    endpoint = f"http://127.0.0.1:{port}/mcp"
    harness.write_config(talaria, remote_access={"enabled": True, "port": port})
    # The audience is the shell's own canonical resource identifier for the
    # port it is about to bind. It is compared byte for byte, so this suite has
    # to spell it exactly as `oauth::canonical_resource` does.
    harness.write_agents(talaria, CLIENT_ID, CLIENT_NAME, TOKEN,
                         audience=endpoint,
                         extra_tokens=[(FOREIGN_TOKEN, FOREIGN_AUDIENCE)])
    tal = harness.start_shell("about:blank", log=log, rust_log="info", wait=10,
                              HOME=tmp, XDG_CONFIG_HOME=config, TALARIA_TEST_HOOKS="1")
    assert wait_until_accepting("127.0.0.1", port), f"the listener never bound {port}"
    assert rpc("tabs_list")["outcome"] == "ok", "the shell is not answering its socket"
    print(f"SEEDED: one approved client, one token, listener on 127.0.0.1:{port}")

    # --- 2. discovery step one: the challenge ----------------------------
    # Everything below starts from `endpoint` and nothing else, which is all a
    # real client is configured with.
    status, headers, unknown_body = request(
        endpoint, b'{"jsonrpc":"2.0","id":1,"method":"tools/list"}',
        Content_Type="application/json", Accept="application/json, text/event-stream")
    assert status == 401, (status, unknown_body[:200])
    challenge = header(headers, "WWW-Authenticate")
    assert challenge, ("a 401 with no WWW-Authenticate tells a client nothing about "
                       "how to authenticate", headers)
    assert challenge.lower().startswith("bearer"), challenge
    assert "resource_metadata=" in challenge, (
        "the challenge names no metadata document — this is Pitfall 3, and a "
        "client that knows only the endpoint URL cannot get any further", challenge)
    metadata_url = challenge.split('resource_metadata="', 1)[1].split('"', 1)[0]
    assert metadata_url.startswith("http://"), metadata_url
    print(f"CHALLENGE: 401 points at {metadata_url}")

    # --- 3. discovery step two: the document -----------------------------
    # Fetched with no credential, deliberately: a metadata document a client
    # must read *before* it has a token cannot itself demand one.
    status, headers, body = request(metadata_url, method="GET")
    assert status == 200, (status, body[:200])
    content_type = header(headers, "Content-Type") or ""
    assert "json" in content_type, content_type
    document = json.loads(body)
    assert document["resource"] == endpoint, (document, endpoint)
    servers = document.get("authorization_servers") or []
    assert servers, ("the document names no authorization server, so a client "
                     "still cannot find where to ask for a token", document)
    assert document.get("bearer_methods_supported") == ["header"], document
    # D-04-02: not implemented, therefore not advertised.
    assert "client_id_metadata_document_supported" not in document, document
    print(f"METADATA: resource={document['resource']} authorization_servers={servers}")

    # --- 4. an authorized client drives the browser ----------------------
    client = HttpMcpClient(endpoint, TOKEN)
    initialized = client.initialize()
    assert initialized["serverInfo"]["name"] == "talaria", initialized
    status, listed = client.call("tools/list", message_id=2)
    assert status == 200, (status, listed)
    http_tools = listed["result"]["tools"]
    print(f"AUTHORIZED: tools/list over HTTP returned {len(http_tools)} tools")

    # --- 5. SC 1: the same tool surface stdio serves ---------------------
    tools_over_stdio = stdio_tools()
    assert normalise(http_tools) == normalise(tools_over_stdio), (
        "the two transports do not serve the same tool surface",
        sorted(t["name"] for t in http_tools),
        sorted(t["name"] for t in tools_over_stdio))
    print(f"SC 1: the {len(http_tools)} tools over HTTP are identical to the "
          "ones over stdio — same names, descriptions and input schemas")

    # --- 6. an authorized tools/call actually drives the browser ---------
    before = len(rpc("tabs_list")["result"]["tabs"])
    status, opened = client.call("tools/call", {"name": "tabs_open",
                                                "arguments": {"url": "about:blank"}},
                                 message_id=3)
    assert status == 200, (status, opened)
    assert not opened["result"].get("isError"), opened
    tabs = rpc("tabs_list")["result"]["tabs"]
    assert len(tabs) == before + 1, (before, tabs)
    # The owner is the *verified* client id, not the `clientInfo.name` this
    # suite sent in `initialize`. That difference is what the token buys.
    agent_tabs = [tab for tab in tabs if tab["owner"] == CLIENT_ID]
    assert len(agent_tabs) == 1, (
        "the tab is not owned by the verified client id",
        [tab["owner"] for tab in tabs])
    assert CLIENT_NAME not in [tab["owner"] for tab in tabs], (
        "a tab is labelled with the self-asserted client name rather than the "
        "registered identifier", [tab["owner"] for tab in tabs])
    status, closed = client.call("tools/call", {"name": "tabs_close",
                                                "arguments": {"tab_id": agent_tabs[0]["tab_id"]}},
                                 message_id=4)
    assert status == 200, (status, closed)
    assert len(rpc("tabs_list")["result"]["tabs"]) == before
    print(f"TOOLS/CALL: a tab opened, owned by {CLIENT_ID}, then closed")

    # --- 7. T-6: a token for another audience, refused indistinguishably --
    status, headers, foreign_body = request(
        endpoint, b'{"jsonrpc":"2.0","id":1,"method":"tools/list"}',
        Content_Type="application/json", Accept="application/json, text/event-stream",
        Authorization=f"Bearer {FOREIGN_TOKEN}")
    assert status == 401, (status, foreign_body[:200])
    status, _headers, absent_body = request(
        endpoint, b'{"jsonrpc":"2.0","id":1,"method":"tools/list"}',
        Content_Type="application/json", Accept="application/json, text/event-stream",
        Authorization="Bearer tal_this_token_was_never_issued_by_anyone")
    assert status == 401, (status, absent_body[:200])
    assert foreign_body == absent_body, (
        "a token for another audience is distinguishable from an unknown one, "
        "which makes the endpoint an oracle for which tokens exist",
        foreign_body, absent_body)
    # And the browser is untouched by either.
    assert len(rpc("tabs_list")["result"]["tabs"]) == before
    print("AUDIENCE: a token minted for another resource is refused, byte-identically "
          "to one that was never issued")

    # --- 8. a malformed credential does not take the browser down --------
    for label, value in (("no scheme", TOKEN),
                         ("scheme only", "Bearer"),
                         ("empty token", "Bearer "),
                         ("wrong scheme", f"Basic {TOKEN}"),
                         ("oversized", "Bearer " + "A" * 200_000)):
        status, _headers, _body = request(
            endpoint, b'{"jsonrpc":"2.0","id":1,"method":"tools/list"}',
            Content_Type="application/json",
            Accept="application/json, text/event-stream", Authorization=value)
        assert status in (400, 401, 403, 413, 431), (label, status)
    assert rpc("tabs_list")["outcome"] == "ok", \
        "a malformed Authorization header took the browser down"
    print("MALFORMED: five malformed credentials each answered with a status code")

    print("OAUTH DISCOVERY CHECKS PASSED")

    # =====================================================================
    # The consent half: from *nothing* to an authorization code, through a
    # human's real click on the real control.
    # =====================================================================
    stop_shell(tal)
    tal = None

    # --- 1. a browser that has never authorized anything -----------------
    consent_home = tempfile.mkdtemp(prefix="talaria-oauth-consent-e2e-")
    consent_config = os.path.join(consent_home, ".config")
    consent_talaria = os.path.join(consent_config, "talaria")
    os.makedirs(consent_talaria)
    port = harness.free_port()
    endpoint = f"http://127.0.0.1:{port}/mcp"
    harness.write_config(consent_talaria, remote_access={"enabled": True, "port": port})
    # No `write_agents` at all. This half has to get from an empty store to an
    # authorization code, which is the whole of SC 2's discovery-through-
    # consent stretch.
    #
    # The three TALARIA_CONSENT_* values shorten the parked-request lifetime
    # and the two cooldowns. Read the module docstring for what they are and,
    # more importantly, what they are not: they are gated, they can only make
    # a window smaller, and **none of them answers a request**. Every approval
    # below is a real pointer click on the real control.
    tal = harness.start_shell(
        "about:blank", log=log, rust_log="info", wait=10,
        HOME=consent_home, XDG_CONFIG_HOME=consent_config,
        TALARIA_TEST_HOOKS="1",
        TALARIA_CONSENT_LIFETIME_MS="3000",
        TALARIA_CONSENT_DENY_COOLDOWN_MS="2000",
        TALARIA_CONSENT_EXPIRY_COOLDOWN_MS="2000")
    assert wait_until_accepting("127.0.0.1", port), f"the listener never bound {port}"
    X = harness.x_env()
    wid = find_window(X)
    callback = harness.loopback_callback()
    print(f"FRESH: no authorized client, listener on 127.0.0.1:{port}, "
          f"callback on 127.0.0.1:{callback.port}")

    # --- 2. discovery, again from the endpoint URL and nothing else ------
    status, headers, body = request(
        endpoint, b'{"jsonrpc":"2.0","id":1,"method":"tools/list"}',
        Content_Type="application/json", Accept="application/json, text/event-stream")
    assert status == 401, (status, body[:200])
    challenge_header = header(headers, "WWW-Authenticate")
    resource_metadata = challenge_header.split('resource_metadata="', 1)[1].split('"', 1)[0]
    status, _headers, body = request(resource_metadata, method="GET")
    assert status == 200, (status, body[:200])
    protected = json.loads(body)
    issuer = protected["authorization_servers"][0]
    # RFC 8414's own construction, which is what a real client performs — not
    # a Talaria path this suite happens to know.
    status, _headers, body = request(
        issuer + "/.well-known/oauth-authorization-server", method="GET")
    assert status == 200, (status, body[:200])
    server = json.loads(body)
    assert server["issuer"] == issuer, server
    assert server["code_challenge_methods_supported"] == ["S256"], (
        "a conformant client refuses to proceed without S256, and would be "
        "entitled to use `plain` if it were advertised", server)
    assert "client_id_metadata_document_supported" not in server, (
        "CIMD is not implemented (D-04-02) and must not be advertised", server)
    assert server["authorization_response_iss_parameter_supported"] is True, server
    registration_endpoint = server["registration_endpoint"]
    authorization_endpoint = server["authorization_endpoint"]
    # Read, never spelled. A suite that knew Talaria's token path would be
    # testing a fact a real client does not have.
    token_endpoint = server["token_endpoint"]
    print(f"AS METADATA: issuer={issuer} challenge_methods="
          f"{server['code_challenge_methods_supported']}")

    # --- 3. dynamic client registration ----------------------------------
    status, _headers, body = request(
        registration_endpoint,
        json.dumps({"client_name": "Example Agent",
                    "redirect_uris": [callback.url]}).encode(),
        Content_Type="application/json")
    assert status == 201, (status, body[:400])
    registration = json.loads(body)
    client_id = registration["client_id"]
    assert client_id and client_id != "Example Agent", registration
    assert registration["redirect_uris"] == [callback.url], registration
    print(f"REGISTERED: client_id={client_id} (minted by the browser, not chosen)")

    def authorize_url(**overrides):
        """A complete, valid authorization request, then whatever a caller
        wants to break about it."""
        params = {"response_type": "code", "client_id": client_id,
                  "redirect_uri": callback.url,
                  "code_challenge": overrides.pop("challenge", None) or CHALLENGE,
                  "code_challenge_method": "S256", "resource": endpoint,
                  "state": "opaque-e2e-state"}
        for name, value in overrides.items():
            if value is None:
                params.pop(name, None)
            else:
                params[name] = value
        return authorization_endpoint + "?" + urllib.parse.urlencode(params)

    def no_panel(what):
        """Nothing was raised — asserted after long enough for a panel to have
        appeared if one were going to."""
        time.sleep(0.8)
        rects, _ = harness.wait_for_rect("consent.approve", timeout=1.0, present=False)
        assert "consent.deny" not in rects, (what, sorted(rects))

    def consent_state(status_url):
        """The parked request's state, as one word, without following the
        redirect a terminal state carries."""
        _status, headers, _body = unfollowed(status_url)
        return header(headers, "X-Talaria-Consent")

    def open_authorization(url):
        """Start a flow. Returns ``(status, headers, body)`` unfollowed."""
        return unfollowed(url)

    # --- 4. every refusal that happens before a human is asked -----------
    # The verifier is kept and deliberately never presented: exchanging the
    # code is 04-07's, and this section ends at the code.
    VERIFIER, CHALLENGE = harness.pkce_pair()
    refusals = [
        ("an unknown client id", authorize_url(client_id="deadbeef" * 4), 400),
        ("a redirect URI that is not the registered one",
         authorize_url(redirect_uri="http://127.0.0.1:1/elsewhere"), 400),
        ("a challenge method other than S256",
         authorize_url(code_challenge_method="plain"), 302),
        ("a resource parameter that is not the canonical identifier",
         authorize_url(resource="http://127.0.0.1:1/mcp"), 302),
        ("a missing challenge", authorize_url(code_challenge=None), 302),
    ]
    for what, url, expected in refusals:
        status, headers, _body = open_authorization(url)
        assert status == expected, (what, status)
        if expected == 302:
            # Once the client and its redirect URI are both known good the
            # error goes to that URI, which is the only way the caller's own
            # flow learns what happened. Before that it cannot: answering an
            # unverified redirect URI would make this an open redirector.
            error = query_of(header(headers, "Location")).get("error")
            assert error in ("invalid_request", "invalid_target"), (what, error)
        no_panel(f"{what} raised a consent panel")
    assert callback.last() is None, ("a refusal reached the callback", callback.last())
    print(f"REFUSED: {len(refusals)} invalid requests, none of which troubled the human")

    # --- 5. a valid request answers with a page that can approve nothing --
    # Focused up front, and once: the arm-delay assertion below has to get a
    # real pointer onto the Approve control inside 1000 ms of the request that
    # raised it, and `xdotool windowfocus --sync` plus its settle is a third of
    # that budget spent on something this step does not need.
    harness.focus_window(wid, X)
    # Timed from *here*, because this is the request that raises the panel and
    # the arm delay below is measured from the moment it was raised.
    raised = time.monotonic()
    status, headers, page = open_authorization(authorize_url())
    assert status == 200, (status, page[:400])
    assert "text/html" in (header(headers, "Content-Type") or ""), headers
    assert header(headers, "Cache-Control") == "no-store", headers
    assert header(headers, "Content-Security-Policy") == \
        "default-src 'none'; frame-ancestors 'none'", headers
    assert header(headers, "X-Frame-Options") == "DENY", headers
    assert header(headers, "Referrer-Policy") == "no-referrer", headers
    text = page.decode()
    # No control of any kind. A page an agent can navigate to and script has
    # nothing on it to script.
    for control in ("<form", "<button", "<a ", "<input", "<script", "onclick"):
        assert control not in text.lower(), (control, text)
    # And no value the caller supplied — which is stricter than escaping, and
    # removes the injection and phishing surface rather than neutralising it.
    for supplied in ("Example Agent", callback.url, CHALLENGE, "opaque-e2e-state",
                     client_id):
        assert supplied not in text, (supplied, text)
    status_path = re.search(r'url=([^"\']+)', text)
    assert status_path, ("the holding page has no way to find out what happened", text)
    status_url = issuer + status_path.group(1)
    print("HOLDING PAGE: no form, no button, no link, and nothing the caller sent")

    # --- 6. approval, by a real click, and not one moment sooner ---------
    harness.click_rect("consent.approve", wid, X, settle=0.2, focus=False)
    elapsed = time.monotonic() - raised
    assert elapsed < 0.9, (
        "the harness took %.2fs to reach the Approve control, so the arm-delay "
        "assertion below could not run inside the 1000ms window" % elapsed)
    assert consent_state(status_url) == "pending", (
        "a click landed inside the arm delay and approved anyway — a click "
        "already in flight can grant full browser control")
    print(f"ARM DELAY: a click at {elapsed:.2f}s granted nothing")

    time.sleep(1.2)
    harness.click_rect("consent.approve", wid, X)
    assert consent_state(status_url) == "approved", "the real Approve control did nothing"
    harness.wait_for_rect("consent.approve", present=False)

    # --- 7. the code arrives at the client's own callback ----------------
    # Followed this time, so the caller's loopback receiver is what records
    # the answer — which is what a real client's flow does.
    request(status_url, method="GET")
    landed = callback.last()
    assert landed is not None, "the callback never received the redirect"
    assert landed.get("code"), landed
    assert landed.get("state") == "opaque-e2e-state", landed
    assert landed.get("iss") == issuer, ("RFC 9207: the metadata claims iss", landed)
    assert "error" not in landed, landed
    # Kept, and deliberately **not** exchanged here. The exchange half below
    # uses it for one assertion and one only: a code has a sixty-second
    # lifetime, that constant is not overridable (04-06 fixed it that way on
    # purpose — a suite that redeems immediately buys nothing by shortening
    # it), and by the time the rest of this file has run this one is well past
    # it. Ageing it in the background costs no wall-clock time; waiting out a
    # freshly minted code would cost a minute.
    stale_code = landed["code"]
    stale_minted_at = time.monotonic()
    print(f"APPROVED: a code reached {callback.url} with the original state")
    callback.clear()

    # --- 8. denial, and the cooldown a denial arms -----------------------
    status, _headers, page = open_authorization(authorize_url())
    assert status == 200, status
    denied_status = issuer + re.search(r'url=([^"\']+)', page.decode()).group(1)
    harness.click_rect("consent.deny", wid, X)
    assert consent_state(denied_status) == "denied", "the real Deny control did nothing"
    request(denied_status, method="GET")
    landed = callback.last()
    assert landed.get("error") == "access_denied", landed
    assert "code" not in landed, ("a denial minted a code", landed)
    print("DENIED: access_denied reached the callback and no code was issued")

    # Immediately again: the cooldown is in force and no panel is raised.
    status, headers, _body = open_authorization(authorize_url())
    assert status == 302, ("the denial cooldown let a request straight through", status)
    assert query_of(header(headers, "Location")).get("error") == "access_denied"
    no_panel("the denial cooldown still interrupted the human")
    print("COOLDOWN (deny): the next request was refused without raising anything")
    time.sleep(2.2)
    callback.clear()

    # --- 9. expiry, and the cooldown an expiry arms ----------------------
    # The production values are the ones in 04-UI-SPEC.md — 120s parked, 30s
    # cooldown. The override only shortens them, which is what makes this step
    # cost seconds instead of two and a half minutes.
    status, _headers, page = open_authorization(authorize_url())
    assert status == 200, status
    expiring_status = issuer + re.search(r'url=([^"\']+)', page.decode()).group(1)
    harness.wait_for_rect("consent.approve")
    # Nobody answers.
    harness.wait_for_rect("consent.approve", timeout=8.0, present=False)
    assert consent_state(expiring_status) == "expired", "the parked request never expired"
    request(expiring_status, method="GET")
    landed = callback.last()
    assert landed.get("error") == "access_denied", landed
    assert "code" not in landed, ("an unanswered request minted a code", landed)
    print("EXPIRED: the panel closed on its own and the caller was refused")

    # And the expiry cooldown — the one a denial-only cooldown would have left
    # open, letting a peer wait out a human and re-raise the instant it lapsed.
    status, headers, _body = open_authorization(authorize_url())
    assert status == 302, ("the expiry cooldown let a request straight through", status)
    assert query_of(header(headers, "Location")).get("error") == "access_denied"
    no_panel("the expiry cooldown still interrupted the human")
    print("COOLDOWN (expiry): the next request was refused without raising anything")
    time.sleep(2.2)
    callback.clear()

    # --- 10. one request on screen at a time -----------------------------
    status, _headers, page = open_authorization(authorize_url())
    assert status == 200, status
    first_status = issuer + re.search(r'url=([^"\']+)', page.decode()).group(1)
    harness.wait_for_rect("consent.approve")
    status, headers, _body = open_authorization(authorize_url())
    assert status == 302, ("a second request was parked while one was on screen", status)
    assert query_of(header(headers, "Location")).get("error") == "access_denied"
    rects, _ = harness.chrome_rects()
    assert len([name for name in rects if name.startswith("consent.")]) == 2, (
        "a second consent panel was stacked on the first", sorted(rects))
    harness.click_rect("consent.deny", wid, X)
    assert consent_state(first_status) == "denied"
    print("ONE AT A TIME: a second request was refused with no second panel")
    time.sleep(2.2)
    callback.clear()

    # --- 11. a deliberately hostile display name -------------------------
    # A newline to take a second line, a bidi override to reorder what follows
    # it, and an ASCII double quote to close the quotes the name is rendered
    # inside and speak in Talaria's own voice. None of the three may displace
    # a control.
    hostile = 'Talaria" verified by Talaria "\n\u202eApproved'
    status, _headers, body = request(
        registration_endpoint,
        json.dumps({"client_name": hostile, "redirect_uris": [callback.url]}).encode(),
        Content_Type="application/json")
    assert status == 201, (status, body[:400])
    hostile_id = json.loads(body)["client_id"]
    hostile_url = authorization_endpoint + "?" + urllib.parse.urlencode(
        {"response_type": "code", "client_id": hostile_id, "redirect_uri": callback.url,
         "code_challenge": CHALLENGE, "code_challenge_method": "S256",
         "resource": endpoint, "state": "hostile-state"})
    status, _headers, page = open_authorization(hostile_url)
    assert status == 200, status
    hostile_status = issuer + re.search(r'url=([^"\']+)', page.decode()).group(1)
    rects, _ = harness.wait_for_rect("consent.approve")
    consent_rects = {name: rect for name, rect in rects.items()
                     if name.startswith("consent.")}
    assert sorted(consent_rects) == ["consent.approve", "consent.deny"], sorted(rects)
    for name, (x, y, width, height) in consent_rects.items():
        assert width > 0 and height > 0, (name, consent_rects[name])
        assert 0 <= x and 0 <= y, ("a hostile name pushed a control off screen",
                                   name, consent_rects[name])
        assert x + width <= 1280 and y + height <= 800, (
            "a hostile name pushed a control past the window", name, consent_rects[name])
    harness.click_rect("consent.deny", wid, X)
    assert consent_state(hostile_status) == "denied"
    print("HOSTILE NAME: two controls, both on screen, both still clickable")

    # The browser is still the browser.
    assert rpc("tabs_list")["outcome"] == "ok", "the consent flow took the browser down"
    print("OAUTH CONSENT CHECKS PASSED")

    # =====================================================================
    # The exchange half: from a code to a credential, and from a credential
    # to a driven browser. Success Criterion 2, closed.
    # =====================================================================
    # The hostile registration above ended on Deny, which arms the denial
    # cooldown. Let it lapse before asking for anything else.
    time.sleep(2.2)
    callback.clear()
    # Focused once, so every approval below can click without paying for a
    # `windowfocus --sync` inside the parked request's shortened lifetime.
    harness.focus_window(wid, X)

    def register_agent(name):
        """One dynamic registration, returning the id the browser minted."""
        status, _headers, body = request(
            registration_endpoint,
            json.dumps({"client_name": name,
                        "redirect_uris": [callback.url]}).encode(),
            Content_Type="application/json")
        assert status == 201, (status, body[:400])
        return json.loads(body)["client_id"]

    def flow_url(agent_id, challenge, redirect=None, state="opaque-e2e-state"):
        """A complete, valid authorization request for one flow."""
        return authorization_endpoint + "?" + urllib.parse.urlencode(
            {"response_type": "code", "client_id": agent_id,
             "redirect_uri": redirect or callback.url,
             "code_challenge": challenge, "code_challenge_method": "S256",
             "resource": endpoint, "state": state})

    def approve(url):
        """Run one authorization to a human's real click, and return the code
        the client's own loopback callback received.

        The wait before the click clears the panel's 1000 ms arm delay, which
        section 6 above already pins as an assertion — this is only long enough
        to get past it, not a second measurement of it."""
        callback.clear()
        status, _headers, page = open_authorization(url)
        assert status == 200, (status, page[:400])
        poll = issuer + re.search(r'url=([^"\']+)', page.decode()).group(1)
        harness.wait_for_rect("consent.approve")
        # Past the arm delay, and then some. Two things happen at 1000 ms: the
        # button enables, and the "Approve turns on in a moment" line above it
        # disappears — which moves the whole button row *up* by the height of
        # that line. A click aimed at a rect read from an unarmed frame lands
        # thirteen logical points below the button and does nothing at all, so
        # this waits until the armed layout is the one being drawn rather than
        # racing the transition. Section 6 above is where the delay itself is
        # asserted; this is only a wait long enough to clear it.
        time.sleep(1.25)
        harness.click_rect("consent.approve", wid, X)
        state = consent_state(poll)
        assert state == "approved", ("the real Approve control did nothing", state)
        harness.wait_for_rect("consent.approve", present=False)
        request(poll, method="GET")
        landed = callback.last()
        assert landed is not None and landed.get("code"), landed
        assert landed.get("state") == "opaque-e2e-state", landed
        return landed["code"]

    def token_request(**params):
        """One POST to the token endpoint, form-encoded, as a client sends it."""
        status, _headers, body = request(
            token_endpoint, urllib.parse.urlencode(params).encode(),
            Content_Type="application/x-www-form-urlencoded")
        try:
            return status, json.loads(body)
        except json.JSONDecodeError:
            return status, {"raw": body[:200].decode(errors="replace")}

    def exchange(code, verifier, agent_id, redirect=None):
        return token_request(grant_type="authorization_code", code=code,
                             code_verifier=verifier, client_id=agent_id,
                             redirect_uri=redirect or callback.url)

    def refresh(token):
        return token_request(grant_type="refresh_token", refresh_token=token)

    def drives_the_browser(access_token, owner):
        """An authorized client opens a real tab, the control socket sees it
        owned by the *verified* client id, and the client closes it again.

        Asserted over the control socket rather than over HTTP, so a bug in
        the transport under test cannot also be the thing reporting success."""
        before = len(rpc("tabs_list")["result"]["tabs"])
        agent = HttpMcpClient(endpoint, access_token)
        agent.initialize()
        status, opened = agent.call("tools/call",
                                    {"name": "tabs_open",
                                     "arguments": {"url": "about:blank"}}, message_id=3)
        assert status == 200, (status, opened)
        assert not opened["result"].get("isError"), opened
        tabs = rpc("tabs_list")["result"]["tabs"]
        assert len(tabs) == before + 1, (before, tabs)
        mine = [tab for tab in tabs if tab["owner"] == owner]
        assert len(mine) == 1, ("the tab is not owned by the verified client id",
                                owner, [tab["owner"] for tab in tabs])
        status, closed = agent.call("tools/call",
                                    {"name": "tabs_close",
                                     "arguments": {"tab_id": mine[0]["tab_id"]}},
                                    message_id=4)
        assert status == 200, (status, closed)
        assert len(rpc("tabs_list")["result"]["tabs"]) == before

    # --- 12. a code becomes a credential ---------------------------------
    first_id = register_agent("Exchange Agent")
    first_verifier, first_challenge = harness.pkce_pair()
    first_code = approve(flow_url(first_id, first_challenge))
    status, granted = exchange(first_code, first_verifier, first_id)
    assert status == 200, (status, granted)
    access_token = granted.get("access_token")
    refresh_token = granted.get("refresh_token")
    assert access_token and refresh_token, granted
    assert granted.get("token_type") == "Bearer", granted
    assert isinstance(granted.get("expires_in"), int) and granted["expires_in"] > 0, granted
    assert granted.get("scope"), granted
    print(f"EXCHANGED: a code became a Bearer token expiring in "
          f"{granted['expires_in']}s, with a refresh token beside it")

    # --- 13. and exactly once --------------------------------------------
    status, replayed = exchange(first_code, first_verifier, first_id)
    assert status != 200, ("a code was redeemable twice", status, replayed)
    assert "access_token" not in replayed, replayed
    assert replayed.get("error") == "invalid_grant", replayed
    print("SINGLE USE: the same code a second time was refused and issued nothing")

    # --- 14. SC 2: the token drives the browser --------------------------
    drives_the_browser(access_token, first_id)
    print(f"SC 2: a client that knew only {endpoint} registered, was approved by a "
          f"real click, exchanged its code, and drove a real tab as {first_id}")

    # --- 15. a wrong verifier fails, and burns the code ------------------
    second_id = register_agent("Second Exchange Agent")
    second_verifier, second_challenge = harness.pkce_pair()
    second_code = approve(flow_url(second_id, second_challenge))
    wrong_verifier, _unused = harness.pkce_pair()
    status, refused_body = exchange(second_code, wrong_verifier, second_id)
    assert status != 200, ("a wrong PKCE verifier was accepted", status, refused_body)
    assert "access_token" not in refused_body, refused_body
    # And now the *right* verifier does not help either: the refused
    # redemption consumed the code. That is the property that makes an
    # intercepted code useless to the interceptor even on a second attempt.
    status, too_late = exchange(second_code, second_verifier, second_id)
    assert status != 200, ("a code survived a failed redemption", status, too_late)
    assert "access_token" not in too_late, too_late
    print("PKCE: a wrong verifier was refused, and spent the code doing it")

    # --- 16. a code presented by a client it was not issued to -----------
    third_id = register_agent("Third Exchange Agent")
    third_verifier, third_challenge = harness.pkce_pair()
    third_code = approve(flow_url(third_id, third_challenge))
    status, borrowed = exchange(third_code, third_verifier, first_id)
    assert status != 200, ("a code was redeemed by a client it was not issued to",
                           status, borrowed)
    assert "access_token" not in borrowed, borrowed
    print("BINDING: a code issued to one client was refused to another")

    # --- 17. refresh rotation --------------------------------------------
    status, rotated = refresh(refresh_token)
    assert status == 200, (status, rotated)
    rotated_access = rotated.get("access_token")
    rotated_refresh = rotated.get("refresh_token")
    assert rotated_access and rotated_refresh, rotated
    assert rotated_access != access_token, "rotation reissued the same access token"
    assert rotated_refresh != refresh_token, "the refresh token did not rotate"
    drives_the_browser(rotated_access, first_id)
    print("ROTATED: a new pair came back and the new access token drove a tab")

    # --- 18. T-9: replaying a spent refresh token takes the family -------
    status, replay = refresh(refresh_token)
    assert status != 200, ("a consumed refresh token rotated again", status, replay)
    assert "access_token" not in replay, replay
    # An unknown token gets the identical answer, so the endpoint is not an
    # oracle for which tokens or families exist.
    status, unknown = refresh("a-refresh-token-nobody-ever-issued")
    assert unknown == replay, ("a detected reuse is distinguishable from an "
                               "unknown token", replay, unknown)
    # And the family is gone: the access token that rotation just issued no
    # longer resolves, which is reuse detection landing end to end.
    status, _headers, body = request(
        endpoint, b'{"jsonrpc":"2.0","id":1,"method":"tools/list"}',
        Content_Type="application/json", Accept="application/json, text/event-stream",
        Authorization=f"Bearer {rotated_access}")
    assert status == 401, ("the family's access token survived a detected reuse",
                           status, body[:200])
    print("REUSE: a replayed refresh token revoked the family, indistinguishably "
          "from an unknown token")

    # --- 19. a registration alone carries nothing ------------------------
    callback.clear()
    never_approved = register_agent("Never Approved Agent")
    assert callback.last() is None, ("a registration on its own reached the "
                                     "callback", callback.last())
    status, nothing = exchange("a-code-this-client-was-never-issued",
                               first_verifier, never_approved)
    assert status != 200, ("a client that was never approved was issued a token",
                           status, nothing)
    assert "access_token" not in nothing, nothing
    print("UNAPPROVED: a registration on its own holds no code and gets no token")

    # --- 20. a code that went stale --------------------------------------
    # The code section 7 collected and never exchanged. Its lifetime is the
    # constant in `oauth.rs` — sixty seconds, and deliberately not overridable
    # — so this waits out whatever the rest of this file has not already spent.
    aged = time.monotonic() - stale_minted_at
    remaining = 61.0 - aged
    if remaining > 0:
        time.sleep(remaining)
    status, expired = exchange(stale_code, VERIFIER, client_id)
    assert status != 200, ("a code past its lifetime was still redeemable",
                           status, expired)
    assert "access_token" not in expired, expired
    print(f"EXPIRED CODE: a code minted {time.monotonic() - stale_minted_at:.0f}s "
          "ago was refused")

    # --- 21. nothing on the token path took the browser down -------------
    for label, body in (("empty", b""),
                        ("not form encoded", b'{"grant_type":"refresh_token"}'),
                        ("a bare percent", b"grant_type=authorization_code&code=%"),
                        ("no grant type", b"code=abc&client_id=def"),
                        ("an unsupported grant", b"grant_type=password&username=a"),
                        ("oversized", b"grant_type=refresh_token&refresh_token="
                                      + b"A" * 200_000)):
        status, _headers, _body = request(
            token_endpoint, body, Content_Type="application/x-www-form-urlencoded")
        assert status in (400, 401, 403, 413, 431), (label, status)
    assert rpc("tabs_list")["outcome"] == "ok", \
        "a malformed token request took the browser down"
    print("MALFORMED GRANT: six malformed token requests each answered with a "
          "status code")

    print("OAUTH FLOW CHECKS PASSED")
finally:
    if callback is not None:
        callback.stop()
    if tal is not None:
        stop_shell(tal)
    if log is not None:
        log.close()
    xvfb.kill()
    xvfb.wait()  # reap, so the X lock's pid isn't left as a zombie
    shutil.rmtree(tmp, ignore_errors=True)
    if consent_home is not None:
        shutil.rmtree(consent_home, ignore_errors=True)

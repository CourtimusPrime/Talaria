#!/usr/bin/env python3
"""Revocation, end to end: AUTH-02 and Success Criterion 3.

Two agents are authorized the real way — dynamic registration, PKCE, and a
human's actual pointer click on the actual Approve control — and then one of
them is revoked from the Access panel by a human's actual clicks on the actual
Revoke control. What this pins down:

  * The panel lists one row per *approved* client, keyed on the client id the
    client did not choose for itself. A registration nobody approved is inert
    and is not a row — but it is a **count** and a "Forget them" control, so a
    registry filled by an unauthenticated flood is something a human can clear
    from inside the product rather than by quitting and editing `agents.json`
    (CR-04). One click clears it and leaves every approved row alone.
  * The first Revoke click arms **that row only**; the two states carry
    different rect names, so "this row armed and the other did not" is read
    rather than inferred.
  * The second click revokes: the row goes, the client's next request is
    refused with the same 401 an unknown token gets, and — the part this suite
    exists for — the client's **already-open response streams close**, on the
    Streamable-HTTP path *and* on the legacy ``/sse`` one. Both are asserted,
    because the shell's tracking was once keyed on the ``/mcp`` path alone and
    a revoked client's ``/sse`` stream simply kept delivering (CR-02); the two
    live in the same session store under the same client id, so nothing about
    a passing ``/mcp`` assertion implies the other.
  * Every other client is untouched: a fresh request works and both of its own
    open streams are still delivering.
  * The revocation survives a restart of the shell.
  * With nothing authorized, the panel renders its empty state and, since the
    listener is on, the next step beside it.

**Its honest limits, stated because they are the interesting part.**

A request that passed verification a moment before a revoke completes will run
to completion. That is correct and it is not a gap: verification is per
request, and a command already executing against the browser engine is not
interruptible. So this suite deliberately does **not** try to catch a
mid-flight request. It asserts the two things that *are* guaranteed — the next
request is refused, and the open stream closes.

And the stream assertion is made from the **client end**: the read side sees
the stream end. Asserting instead that a fresh request now fails would have
passed whether or not the stream ever closed, which is exactly the shape of
test `04-RESEARCH.md`'s Pitfall 4 warns about. The stream is therefore read
with a raw socket and ``select``, the technique ``mcp_client_test.py``
established, and never with a buffered reader: a reader that has timed out once
refuses every later read, which would turn "still delivering" into a false
negative.

Preconditions: a release build of ``talaria``, an Xvfb display, ``xdotool``.
The shell is started with ``TALARIA_TEST_HOOKS=1`` for ``chrome_rects``.
"""
import json
import os
import re
import select
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

T = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, T)
import harness

# How long to wait for the standalone stream's first keep-alive frame. The
# listener's ping interval is 12s (`http.rs`, `McpAppState::ping_interval`), and
# a frame arriving is what makes "the stream is delivering" a measurement
# rather than an assumption.
PING_WAIT = 20.0
# How long a revoked stream may take to close. The mechanism is immediate — the
# revoke arm hands the listener's runtime the termination and the session's
# transport is shut down, which closes the write half — so this is scheduling
# slack, not a documented bound. If the implementation had fallen back to
# per-keepalive re-verification this would have had to be one ping interval
# plus a margin, and this comment would say so.
CLOSE_WAIT = 8.0

tmp = tempfile.mkdtemp(prefix="talaria-revocation-e2e-")
config = os.path.join(tmp, ".config")
talaria = os.path.join(config, "talaria")
os.makedirs(talaria)
# Outside the suite's own temp directory, which the `finally` removes: a log
# that vanishes with the failure it explains is not a log.
OUT = os.environ.get("TALARIA_E2E_OUT", "/tmp/talaria-e2e")
os.makedirs(OUT, exist_ok=True)
SHELL_LOG = os.path.join(OUT, "revocation-shell.log")


def request(url, body=None, method="POST", **headers):
    """One HTTP round trip. An error status is an answer, not an exception."""
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
    for key, value in headers.items():
        if key.lower() == name.lower():
            return value
    return None


def sse_payload(body):
    """The JSON of a Streamable-HTTP reply, which arrives as one SSE event."""
    text = body.decode()
    for line in text.splitlines():
        if line.startswith("data:"):
            return json.loads(line[len("data:"):].strip())
    return json.loads(text)


class HttpMcpClient:
    """The minimum real Streamable-HTTP MCP client: initialize, notify, call."""

    def __init__(self, endpoint, token, name):
        self.endpoint = endpoint
        self.token = token
        self.name = name
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
        return status, (sse_payload(body) if body and status == 200 else None), body

    def notify(self, method):
        message = {"jsonrpc": "2.0", "method": method, "params": {}}
        status, _headers, _body = request(self.endpoint, json.dumps(message).encode(),
                                          **self._headers())
        return status

    def initialize(self):
        status, reply, _raw = self.call("initialize", {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": self.name, "version": "1.0"}})
        assert status == 200, (status, reply)
        assert self.session_id, "the server issued no session id"
        assert self.notify("notifications/initialized") in (200, 202)
        return reply["result"]


class Stream:
    """A standalone event stream, on a raw socket — ``GET /mcp`` or ``GET /sse``.

    This is the long-lived connection threat T-7 is about: it is opened once,
    it carries server-initiated messages, and — unlike a request/response pair
    — it never asks the server anything again. Nothing about a revoked token
    can reach it through the ordinary per-request check, which is the whole
    reason the shell keeps a registry of live streams and drops a revoked
    client's handles.

    Read with ``select`` on a bare socket rather than through
    ``urllib``/``makefile``: a buffered reader that has timed out once refuses
    every later read, and this class has to be able to say both "nothing yet,
    still open" and "ended" about the same connection.

    Both transports are driven through this one class deliberately. The
    legacy HTTP-plus-event-stream transport is mounted unconditionally by
    ``rust-mcp-axum`` and its ``sse`` feature is enabled transitively, so a
    revoked client's ``/sse`` stream is as real as its ``/mcp`` one — and the
    shell's tracking used to gate on the ``/mcp`` path alone, which meant a
    revoke closed one and not the other (CR-02). Asserting closure with the
    same class on both is what makes that difference visible.

    **The framing matters, and getting it wrong makes this test lie.** The
    response is HTTP/1.1 ``Transfer-Encoding: chunked`` on a keep-alive
    connection, so "the stream ended" is the terminating zero-length chunk, not
    a closed socket — hyper is entitled to hold the TCP connection open for a
    request that will never come. A version of this class that waited for EOF
    would report a stream that had cleanly ended as still delivering, which is
    the false *negative* twin of the false positive this suite is about. Both
    endings are accepted: the zero-length chunk, and a socket that closes.
    """

    def __init__(self, host, port, token, session_id=None, path="/mcp"):
        self.socket = socket.create_connection((host, port), timeout=10)
        self.path = path
        lines = [
            f"GET {path} HTTP/1.1",
            f"Host: {host}:{port}",
            "Accept: text/event-stream",
            f"Authorization: Bearer {token}",
        ]
        # Streamable HTTP names the session the stream belongs to. The legacy
        # `/sse` handshake cannot: the id does not exist until the server mints
        # one while answering, and it comes back inside the body.
        if session_id is not None:
            lines.append(f"Mcp-Session-Id: {session_id}")
        lines.append("Connection: keep-alive")
        self.socket.sendall(("\r\n".join(lines) + "\r\n\r\n").encode())
        self.socket.settimeout(None)
        self.buf = b""
        self.body = b""
        self.ended = False
        # Off until the head is parsed: the response head is not body, and a
        # decoder that ran while it was still arriving would swallow it.
        self.framed = False
        self.chunked = False
        self.status, self.headers = self._read_head()
        self.chunked = "chunked" in self.headers.get("transfer-encoding", "").lower()
        self.framed = True
        self._decode()

    def _read_head(self):
        """Status line and headers, which arrive as soon as the stream opens."""
        deadline = time.monotonic() + 10
        while b"\r\n\r\n" not in self.buf:
            if not self._pump(deadline - time.monotonic()):
                raise AssertionError(f"the stream sent no response head: {self.buf[:200]!r}")
        head, _, self.buf = self.buf.partition(b"\r\n\r\n")
        lines = head.decode(errors="replace").split("\r\n")
        status = int(lines[0].split()[1])
        headers = {}
        for line in lines[1:]:
            key, _, value = line.partition(":")
            headers[key.strip().lower()] = value.strip()
        return status, headers

    def _decode(self):
        """Move whatever complete chunks have arrived into the decoded body.

        A zero-length chunk header is the end of the stream, and it is the only
        ending a keep-alive connection gives.
        """
        if not self.framed:
            return
        if not self.chunked:
            self.body += self.buf
            self.buf = b""
            return
        while True:
            marker = self.buf.find(b"\r\n")
            if marker < 0:
                return
            try:
                size = int(self.buf[:marker].split(b";")[0], 16)
            except ValueError:
                # Not a chunk header. Nothing sensible is left to read.
                self.ended = True
                return
            if size == 0:
                self.ended = True
                return
            if len(self.buf) < marker + 2 + size + 2:
                return  # the chunk is still arriving
            self.body += self.buf[marker + 2:marker + 2 + size]
            self.buf = self.buf[marker + 2 + size + 2:]

    def _pump(self, timeout):
        """One read. False on timeout, False and ``ended`` set on an ending."""
        if timeout <= 0:
            return False
        if not select.select([self.socket], [], [], timeout)[0]:
            return False
        try:
            chunk = self.socket.recv(65536)
        except (ConnectionResetError, OSError):
            # A reset is an ending too, and the honest reading of it here is
            # the same as EOF: the client can no longer receive.
            self.ended = True
            return False
        if not chunk:
            self.ended = True
            return False
        self.buf += chunk
        self._decode()
        return True

    def delivering(self, timeout):
        """True once any byte of stream *body* has arrived within ``timeout``.

        The listener's keep-alive writes a frame every ping interval, so this
        is what turns "the stream is open" into something measured rather than
        assumed — and it means a later closure assertion is about a connection
        that was demonstrably alive.
        """
        deadline = time.monotonic() + timeout
        while not self.body and not self.ended:
            if not self._pump(deadline - time.monotonic()) and time.monotonic() > deadline:
                break
        return bool(self.body) and not self.ended

    def still_open(self, settle=1.5):
        """False once the read side has seen the stream end."""
        self._pump(settle)
        return not self.ended

    def closed_within(self, timeout):
        """True once the read side sees the stream end within ``timeout``.

        **The assertion this suite exists for.** Asserting instead that a fresh
        request now fails would pass whether or not the stream ever closed.
        """
        deadline = time.monotonic() + timeout
        while not self.ended and time.monotonic() < deadline:
            self._pump(min(0.5, deadline - time.monotonic()))
        return self.ended

    def close(self):
        try:
            self.socket.close()
        except OSError:
            pass


def rpc(command, client="revocation-e2e", **params):
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


class NoRedirect(urllib.request.HTTPRedirectHandler):
    """An opener that reports a redirect instead of following it."""

    def redirect_request(self, *args):
        return None


NO_REDIRECT = urllib.request.build_opener(NoRedirect)


def unfollowed(url, method="GET"):
    outgoing = urllib.request.Request(url, method=method)
    try:
        with NO_REDIRECT.open(outgoing, timeout=20) as response:
            return response.status, dict(response.headers), response.read()
    except urllib.error.HTTPError as error:
        return error.code, dict(error.headers), error.read()


def find_window(env):
    for _ in range(15):
        out = subprocess.run(["xdotool", "search", "--name", "Talaria"], env=env,
                             capture_output=True, text=True).stdout.split()
        if out:
            return out[0]
        time.sleep(1)
    raise AssertionError("Talaria window never appeared")


def stop_shell(shell):
    """Stop the shell and wait for its socket to go.

    The socket wait is what makes a restart in this suite honest: without it
    the next `start_shell` can find the old socket still on disk, forward its
    URL to a dying process, and exit."""
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
callbacks = []
streams = []
try:
    log = open(SHELL_LOG, "w")

    # --- 1. a browser with nothing authorized ----------------------------
    port = harness.free_port()
    endpoint = f"http://127.0.0.1:{port}/mcp"
    issuer = f"http://127.0.0.1:{port}"
    harness.write_config(talaria, remote_access={"enabled": True, "port": port})
    tal = harness.start_shell("about:blank", log=log, rust_log="info", wait=10,
                              HOME=tmp, XDG_CONFIG_HOME=config, TALARIA_TEST_HOOKS="1")
    assert wait_until_accepting("127.0.0.1", port), f"the listener never bound {port}"
    assert rpc("tabs_list")["outcome"] == "ok", "the shell is not answering its socket"
    X = harness.x_env()
    wid = find_window(X)
    harness.focus_window(wid, X)

    status, _headers, body = request(
        f"{issuer}/.well-known/oauth-authorization-server", method="GET")
    assert status == 200, (status, body[:200])
    metadata = json.loads(body)
    authorization_endpoint = metadata["authorization_endpoint"]
    token_endpoint = metadata["token_endpoint"]
    registration_endpoint = metadata["registration_endpoint"]
    revocation_endpoint = metadata["revocation_endpoint"]
    print(f"SEEDED: listener on 127.0.0.1:{port}, revocation at {revocation_endpoint}")

    # --- 2. two clients, each authorized by a real human click -----------
    def register(name, callback):
        status, _headers, body = request(
            registration_endpoint,
            json.dumps({"client_name": name, "redirect_uris": [callback.url]}).encode(),
            Content_Type="application/json")
        assert status == 201, (status, body[:400])
        return json.loads(body)["client_id"]

    def consent_state(poll):
        """The parked request's state, as one word, without following the
        redirect a terminal state carries."""
        _status, headers, _body = unfollowed(poll)
        return header(headers, "X-Talaria-Consent")

    def approve(url):
        """Run one authorization to a human's real click, and return the code.

        The wait before the click clears the panel's 1000 ms arm delay. Two
        things happen when it elapses: the button enables, and the "Approve
        turns on in a moment" line above it disappears, which moves the button
        row *up*. `click_rect` reads the rect from the last frame the shell
        drew, so a click aimed at an unarmed frame lands below the button and
        does nothing at all. `oauth_flow_test.py` is where the delay itself is
        asserted; this is only long enough to clear it."""
        status, _headers, page = unfollowed(url)
        assert status == 200, (status, page[:400])
        poll = issuer + re.search(r'url=([^"\']+)', page.decode()).group(1)
        harness.wait_for_rect("consent.approve")
        time.sleep(1.25)
        harness.click_rect("consent.approve", wid, X)
        state = consent_state(poll)
        assert state == "approved", ("the real Approve control did nothing", state)
        harness.wait_for_rect("consent.approve", present=False)
        request(poll, method="GET")

    def authorize(name):
        """Register, approve and exchange — one real client, end to end."""
        callback = harness.loopback_callback()
        callbacks.append(callback)
        client_id = register(name, callback)
        verifier, challenge = harness.pkce_pair()
        callback.clear()
        approve(authorization_endpoint + "?" + urllib.parse.urlencode(
            {"response_type": "code", "client_id": client_id,
             "redirect_uri": callback.url, "code_challenge": challenge,
             "code_challenge_method": "S256", "resource": endpoint,
             "state": f"state-for-{client_id[:8]}"}))
        landed = callback.last()
        assert landed and landed.get("code"), landed
        status, _headers, body = request(
            token_endpoint,
            urllib.parse.urlencode(
                {"grant_type": "authorization_code", "code": landed["code"],
                 "code_verifier": verifier, "client_id": client_id,
                 "redirect_uri": callback.url}).encode(),
            Content_Type="application/x-www-form-urlencoded")
        assert status == 200, (status, body[:400])
        issued = json.loads(body)
        return client_id, issued["access_token"], issued["refresh_token"]

    # Two names, deliberately different in kind. The second is the untrusted
    # claim the row has to render without repeating it as a fact: it tries to
    # take a second line, reorder what follows it, and forge the quotes the
    # panel renders it inside.
    FIRST_NAME = "First Agent"
    HOSTILE_NAME = 'Talaria" ‮verifies\n\nthis agent'
    first_id, first_token, _first_refresh = authorize(FIRST_NAME)
    second_id, second_token, second_refresh = authorize(HOSTILE_NAME)
    assert first_id != second_id, (first_id, second_id)
    print(f"AUTHORIZED: two clients approved by real clicks — {first_id[:8]}, {second_id[:8]}")

    # A third client that registers and is never approved. It must produce no
    # row: a registration on its own is inert.
    unapproved_callback = harness.loopback_callback()
    callbacks.append(unapproved_callback)
    unapproved_id = register("Never Approved", unapproved_callback)

    # --- 3. both clients can drive the browser ---------------------------
    def drives(client, owner):
        """One agent opens a tab, the control socket sees whose it is, and the
        agent closes it again."""
        before = len(rpc("tabs_list")["result"]["tabs"])
        status, opened, _raw = client.call(
            "tools/call", {"name": "tabs_open", "arguments": {"url": "about:blank"}},
            message_id=3)
        assert status == 200, (status, opened)
        assert not opened["result"].get("isError"), opened
        tabs = rpc("tabs_list")["result"]["tabs"]
        mine = [tab for tab in tabs if tab["owner"] == owner]
        assert len(mine) == 1, ("the tab is not owned by the verified client id",
                                [tab["owner"] for tab in tabs])
        status, closed, _raw = client.call(
            "tools/call", {"name": "tabs_close", "arguments": {"tab_id": mine[0]["tab_id"]}},
            message_id=4)
        assert status == 200, (status, closed)
        assert len(rpc("tabs_list")["result"]["tabs"]) == before

    first = HttpMcpClient(endpoint, first_token, "first-e2e")
    first.initialize()
    drives(first, first_id)
    second = HttpMcpClient(endpoint, second_token, "second-e2e")
    second.initialize()
    drives(second, second_id)
    print("DRIVING: both clients opened and closed a real tab, each as its own client id")

    # --- 4. a long-lived stream for each, confirmed delivering -----------
    first_stream = Stream("127.0.0.1", port, first_token, first.session_id)
    streams.append(first_stream)
    second_stream = Stream("127.0.0.1", port, second_token, second.session_id)
    streams.append(second_stream)
    # And one on the legacy transport for each, which is a *separate* session
    # the browser minted server-side. The shell's stream tracking used to be
    # keyed on the `/mcp` path alone, so these two were outside what a revoke
    # could close while sitting in the same session store under the same
    # client id (CR-02).
    first_sse = Stream("127.0.0.1", port, first_token, path="/sse")
    streams.append(first_sse)
    second_sse = Stream("127.0.0.1", port, second_token, path="/sse")
    streams.append(second_sse)
    for label, stream in (("first", first_stream), ("second", second_stream),
                          ("first /sse", first_sse), ("second /sse", second_sse)):
        assert stream.status == 200, (label, stream.status, stream.headers)
        assert "text/event-stream" in stream.headers.get("content-type", ""), \
            (label, stream.headers)
    # Waited for once each, because they share a clock.
    assert first_stream.delivering(PING_WAIT), \
        "the first client's stream opened but never delivered anything"
    assert second_stream.delivering(PING_WAIT), \
        "the second client's stream opened but never delivered anything"
    assert first_sse.delivering(PING_WAIT), \
        "the first client's /sse stream opened but never delivered anything"
    assert second_sse.delivering(PING_WAIT), \
        "the second client's /sse stream opened but never delivered anything"
    print("STREAMS: four open event streams — two on /mcp, two on /sse, all delivering")

    # --- 5. the Access panel lists both ----------------------------------
    harness.click_rect("toolbar.access", wid, X)
    rects, _scale = harness.wait_for_rect("access.row.1")
    assert "access.row.0" in rects and "access.row.1" in rects, sorted(rects)
    assert "access.row.2" not in rects, \
        ("a client that registered and was never approved produced a row",
         unapproved_id, sorted(rects))
    assert "access.revoke.0" in rects and "access.revoke.1" in rects, sorted(rects)
    # The hostile name did not displace anything: both rows and both Revoke
    # buttons are on screen, laid out one under the other, and the panel's own
    # status control is still where it belongs above them.
    assert rects["access.row.0"][1] < rects["access.row.1"][1], rects
    assert rects["access.status"][1] < rects["access.row.0"][1], rects
    for row in (0, 1):
        assert rects[f"access.revoke.{row}"][2] > 0, (row, rects)
    print("PANEL: two rows, none for the unapproved registration, layout intact")

    # --- 5b. the unapproved registration is visible, and clearable here ---
    # CR-04: `/register` takes no credential, so any local account could fill
    # the registry — refused past the cap, never expiring, and rendering no row
    # and therefore no Revoke button. The human saw "Nothing authorized yet"
    # and the only recovery was to quit the browser and edit `agents.json`. It
    # is still not a row, because a registration holds nothing to take away,
    # but it is a count and a control.
    assert "access.waiting" in rects, \
        ("a registration waiting for approval left no trace in the panel",
         unapproved_id, sorted(rects))
    assert "access.forget-unapproved" in rects, sorted(rects)
    assert rects["access.forget-unapproved"][2] > 0, rects
    assert rects["access.waiting"][1] < rects["access.row.0"][1], rects
    harness.click_rect("access.forget-unapproved", wid, X)
    rects, _scale = harness.wait_for_rect("access.waiting", present=False)
    assert "access.forget-unapproved" not in rects, \
        ("the control stayed behind with nothing left to forget", sorted(rects))
    assert "access.row.0" in rects and "access.row.1" in rects, \
        ("forgetting unapproved registrations took an approved client",
         sorted(rects))
    print("WAITING: the unapproved registration was visible and one click cleared it")

    # Which row is which. Rows render newest-authorized first, so the second
    # client is row 0 and the first is row 1 — asserted rather than assumed by
    # revoking one and seeing whose token stops working.
    FIRST_ROW = 1

    # --- 6. one click arms that row and no other -------------------------
    harness.click_rect(f"access.revoke.{FIRST_ROW}", wid, X)
    rects, _scale = harness.wait_for_rect(f"access.confirm-revoke.{FIRST_ROW}")
    assert f"access.revoke.{FIRST_ROW}" not in rects, \
        ("the armed row still draws its unarmed name", sorted(rects))
    other = 1 - FIRST_ROW
    assert f"access.revoke.{other}" in rects, \
        ("the other row lost its plain Revoke", sorted(rects))
    assert f"access.confirm-revoke.{other}" not in rects, \
        ("arming one row armed another", sorted(rects))
    # Nothing was revoked by the first click.
    assert first.call("tools/list", message_id=5)[0] == 200, \
        "the arming click alone revoked something"
    print("ARMED: the first click armed exactly one row and granted nothing")

    # --- 7. the second click revokes -------------------------------------
    # Read the rect from a frame drawn *after* the arming, which
    # `wait_for_rect` above has already done: the armed label is wider than the
    # plain one, and a click scheduled off a stale frame would miss it.
    harness.click_rect(f"access.confirm-revoke.{FIRST_ROW}", wid, X)
    rects, _scale = harness.wait_for_rect("access.row.1", present=False)
    assert "access.row.0" in rects, ("both rows went", sorted(rects))
    # And the panel offers no way to revoke a client it is not listing: one
    # row, one Revoke, and no armed control left over.
    assert "access.revoke.1" not in rects and "access.confirm-revoke.1" not in rects, \
        sorted(rects)
    assert "access.confirm-revoke.0" not in rects, \
        ("the armed state moved onto the surviving row", sorted(rects))
    print("REVOKED: the row disappeared and the surviving row was not armed")

    # --- 8. the revoked client's next request is refused -----------------
    status, _reply, refused_body = first.call("tools/list", message_id=6)
    assert status == 401, (status, refused_body[:200])
    status, _headers, unknown_body = request(
        endpoint, b'{"jsonrpc":"2.0","id":1,"method":"tools/list"}',
        Content_Type="application/json", Accept="application/json, text/event-stream",
        Authorization="Bearer tal_this_token_was_never_issued_by_anyone")
    assert status == 401, (status, unknown_body[:200])
    assert refused_body == unknown_body, (
        "a revoked token is distinguishable from one that was never issued, "
        "which makes the endpoint an oracle for which tokens existed",
        refused_body[:200], unknown_body[:200])
    print("REFUSED: the revoked client's next request is the same 401 an unknown token gets")

    # --- 9. the revoked client's OPEN STREAM closed ----------------------
    # The assertion this suite exists for, and the one `04-RESEARCH.md`'s
    # Pitfall 4 says revocation tests skip. It is made from the client end:
    # the read side sees the stream end. Step 8 above would have passed
    # whether or not this did.
    assert first_stream.closed_within(CLOSE_WAIT), (
        "the revoked client's open stream is still open — its next request is "
        "refused, but it is still receiving from the browser")
    # The same assertion on the legacy transport, which is the half CR-02 was
    # about. It is made the same way and can fail in the same honest direction:
    # nothing here issues a fresh request, so "a new request now fails" cannot
    # satisfy it.
    assert first_sse.closed_within(CLOSE_WAIT), (
        "the revoked client's open /sse stream is still open — the revoke closed "
        "its /mcp stream and left this one delivering")
    print(f"STREAM CLOSED: the revoked client's /mcp and /sse streams both ended "
          f"within {CLOSE_WAIT:.0f}s")

    # --- 10. the other client is untouched -------------------------------
    status, listed, _raw = second.call("tools/list", message_id=7)
    assert status == 200, (status, listed)
    drives(second, second_id)
    assert second_stream.still_open(), \
        "revoking one client closed another client's open stream"
    assert second_sse.still_open(), \
        "revoking one client closed another client's open /sse stream"
    print("UNTOUCHED: the second client still drives the browser and both its "
          "streams are still open")

    # --- 11. revocation survives a restart -------------------------------
    for stream in streams:
        stream.close()
    streams = []
    stop_shell(tal)
    tal = None
    tal = harness.start_shell("about:blank", log=log, rust_log="info", wait=10,
                              HOME=tmp, XDG_CONFIG_HOME=config, TALARIA_TEST_HOOKS="1")
    assert wait_until_accepting("127.0.0.1", port), "the listener never rebound after a restart"
    wid = find_window(X)
    harness.focus_window(wid, X)

    status, _headers, after_restart = request(
        endpoint, b'{"jsonrpc":"2.0","id":1,"method":"tools/list"}',
        Content_Type="application/json", Accept="application/json, text/event-stream",
        Authorization=f"Bearer {first_token}")
    assert status == 401, ("the revoked token works again after a restart", status)
    survivor = HttpMcpClient(endpoint, second_token, "second-e2e")
    survivor.initialize()
    drives(survivor, second_id)
    harness.click_rect("toolbar.access", wid, X)
    rects, _scale = harness.wait_for_rect("access.row.0")
    assert "access.row.1" not in rects, \
        ("the revoked client came back after a restart", sorted(rects))
    print("RESTART: the revocation survived — one row, one working client, one dead token")

    # --- 12. a client may revoke its own token (RFC 7009) ----------------
    # The standard endpoint, which is a different act from the human's revoke:
    # a client disowning its own credential. Always 200, whether or not the
    # token existed.
    def revoke_at_endpoint(token):
        status, _headers, body = request(
            revocation_endpoint,
            urllib.parse.urlencode({"token": token}).encode(),
            Content_Type="application/x-www-form-urlencoded")
        return status, body

    live_status, live_body = revoke_at_endpoint(second_refresh)
    again_status, again_body = revoke_at_endpoint(second_refresh)
    never_status, never_body = revoke_at_endpoint("tal_no_such_token_was_ever_issued")
    assert live_status == again_status == never_status == 200, \
        (live_status, again_status, never_status)
    assert live_body == again_body == never_body, (live_body, again_body, never_body)
    # And it revoked for real: the access token issued alongside that refresh
    # token is gone too, because revoking either half takes the family.
    status, _headers, _body = request(
        endpoint, b'{"jsonrpc":"2.0","id":1,"method":"tools/list"}',
        Content_Type="application/json", Accept="application/json, text/event-stream",
        Authorization=f"Bearer {second_token}")
    assert status == 401, ("revoking a refresh token left its access token live", status)
    print("RFC 7009: one answer for a live, an already-revoked and a never-valid token")

    # --- 13. nothing authorized renders the empty state ------------------
    # The client's own revocation above dropped its tokens but left its
    # registration, which is the endpoint's documented scope. The *human's*
    # revoke is what removes the row, so that is what this uses.
    rects, _scale = harness.wait_for_rect("access.revoke.0")
    harness.click_rect("access.revoke.0", wid, X)
    harness.wait_for_rect("access.confirm-revoke.0")
    harness.click_rect("access.confirm-revoke.0", wid, X)
    rects, _scale = harness.wait_for_rect("access.empty")
    assert "access.row.0" not in rects, ("a row survived the last revoke", sorted(rects))
    # The listener is still on, so the next step is there beside the empty
    # line — the one case where telling somebody where to point a client is
    # useful rather than misleading.
    assert "access.next-step" in rects, \
        ("the empty state offered no next step while the listener was on", sorted(rects))
    assert rects["access.empty"][1] < rects["access.next-step"][1], rects
    print("EMPTY: nothing authorized renders the empty state and its next step")

    print("REVOCATION CHECKS PASSED")
finally:
    for stream in streams:
        stream.close()
    for callback in callbacks:
        callback.stop()
    if tal is not None:
        stop_shell(tal)
    xvfb.kill()
    xvfb.wait()
    if log is not None:
        log.close()
    shutil.rmtree(tmp, ignore_errors=True)

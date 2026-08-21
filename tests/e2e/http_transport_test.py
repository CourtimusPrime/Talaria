#!/usr/bin/env python3
"""E2E: the remote HTTP transport exists, is off unless asked for, binds only
loopback, and refuses everything that reaches it.

What this pins down:

- **Default off (D-04-04), proven by absence.** A shell started with no
  ``config.json`` has nothing accepting connections on the default port. That
  is asserted with a refused connection rather than by reading a setting back,
  because "the flag says off" and "no thread was spawned and no port was bound"
  are different claims and only the second one is the decision.
- **Loopback only.** With the listener on, a connection to this machine's own
  non-loopback address on the same port is refused.
- **Nothing can drive it.** An unauthenticated POST to the MCP endpoint is
  answered ``401`` and has no side effect on the browser: the tab list is
  identical before and after.
- **A page cannot reach it (T-2), on every route that mints a credential.** A
  request carrying an ``Origin`` header is refused outright, because a
  legitimate MCP client sends none and a page always does; so is one whose
  ``Host`` names an address this listener is not bound to, which is the DNS
  rebinding case. Asserted on ``/mcp`` *and* on ``/register``, ``/authorize``,
  ``/authorize/status``, ``/token`` and ``/revoke`` — the SDK dispatches those
  five outside its own middleware chain, so "``/mcp`` refuses it" proves
  nothing about them. The two metadata documents are the deliberate exception
  and are asserted to stay readable uncredentialed and cross-origin: a client
  fetches them *before* it has anything to present, and one that could not
  would never find the flow.
- **An agent cannot navigate to it (T-3).** ``tabs_open`` at the listener's own
  origin is refused with a message that names the reason, and no tab appears —
  and the refusal is a *boundary*, not a rule about URLs the agent typed: an
  agent that assigns ``location.href`` from inside a tab it owns does not reach
  it either. That second half is asserted on the tab's URL over the control
  socket, because the assignment evaluates to the assigned string whether or
  not the load ever begins.
- **SC 4: the other two doors are untouched.** With the listener on, the stdio
  proxy still initializes and lists the same nine tools, and the Unix control
  socket answers throughout — every assertion here goes over it.

Honest limits. This proves the transport *exists* and that it refuses
everything; proving it *serves* the tool surface needs a credential, which
04-05 has. And the chrome assertions read geometry rather than text, so what is
checked is that the bound-state control is the one being drawn, not what it
says.

Standalone (starts its own Xvfb and shell, with ``TALARIA_TEST_HOOKS=1`` for
the rect lookup), with an isolated ``HOME`` and ``XDG_CONFIG_HOME`` because
this suite writes a ``config.json`` and must not touch the real one. Nothing
here reaches the network: every request is to a loopback port this suite
started."""
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness

# `talaria_shell::settings::DEFAULT_REMOTE_PORT`. Carried here so step 1 can
# prove nothing is listening where a default install *would* listen; every
# other step uses a port `harness.free_port()` picked.
DEFAULT_PORT = 8779

# The tool surface both transports serve, from `mcp_client_test.py`.
EXPECTED_TOOLS = ["cookies_read", "download", "evaluate", "navigate", "screenshot",
                  "tabs_close", "tabs_focus", "tabs_list", "tabs_open"]

MCP_BINARY = os.path.join(harness.TARGET, "release", "talaria-mcp")

tmp = tempfile.mkdtemp(prefix="talaria-http-transport-e2e-")
config = os.path.join(tmp, ".config")
talaria = os.path.join(config, "talaria")
os.makedirs(talaria)
SHELL_LOG = os.path.join(tmp, "shell.log")


def start(log):
    return harness.start_shell("about:blank", log=log, rust_log="info", wait=10,
                               HOME=tmp, XDG_CONFIG_HOME=config,
                               TALARIA_TEST_HOOKS="1")


def stop_shell(shell):
    """Stop the shell and wait for its socket to go, so the next start_shell
    does not return against the dying one."""
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


def rpc(command, client="http-transport-e2e", **params):
    """One control-socket request on its own connection — this suite restarts
    the shell, so a long-lived wire would not survive."""
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
    """True when something completes a TCP handshake there."""
    s = socket.socket()
    s.settimeout(timeout)
    try:
        s.connect((host, port))
        return True
    except OSError:
        return False
    finally:
        s.close()


def wait_until_accepting(host, port, seconds=15.0):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if accepts(host, port):
            return True
        time.sleep(0.2)
    return False


def post(url, body=b'{"jsonrpc":"2.0","id":1,"method":"tools/list"}', **headers):
    """POST and return (status, body). An HTTP error status is an answer here,
    not an exception — every interesting case in this suite is a refusal."""
    request = urllib.request.Request(url, data=body, method="POST")
    request.add_header("Content-Type", "application/json")
    request.add_header("Accept", "application/json, text/event-stream")
    for name, value in headers.items():
        request.add_header(name.replace("_", "-"), value)
    try:
        with urllib.request.urlopen(request, timeout=8) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


def raw(path, method="POST", host=None, origin=None, body=b"{}",
        content_type="application/json"):
    """One request on a raw socket, so the ``Host`` header is ours to choose.

    ``urllib`` derives ``Host`` from the URL and offers no honest way to make
    it lie, and a ``Host`` the listener is not bound to is exactly the
    DNS-rebinding case: a page served from ``http://rebind.evil:PORT/`` whose A
    record has been re-pointed at loopback is same-origin with this server as
    far as the browser is concerned, and can read every response it gets — but
    the ``Host`` header it sends still names ``rebind.evil``.

    Returns ``(status, body)``; an error status is an answer here."""
    lines = [f"{method} {path} HTTP/1.1",
             f"Host: {host if host is not None else f'127.0.0.1:{port}'}",
             "Accept: application/json, text/event-stream",
             "Connection: close"]
    if origin is not None:
        lines.append(f"Origin: {origin}")
    payload = body if body is not None else b""
    if body is not None:
        lines.append(f"Content-Type: {content_type}")
        lines.append(f"Content-Length: {len(payload)}")
    s = socket.create_connection(("127.0.0.1", port), timeout=10)
    try:
        s.sendall(("\r\n".join(lines) + "\r\n\r\n").encode() + payload)
        chunks = []
        while True:
            chunk = s.recv(65536)
            if not chunk:
                break
            chunks.append(chunk)
    finally:
        s.close()
    head, _, tail = b"".join(chunks).partition(b"\r\n\r\n")
    return int(head.split(b"\r\n")[0].split()[1]), tail


def non_loopback_address():
    """This machine's own non-loopback IPv4, or None when it has none.

    The UDP `connect` sends nothing — it only asks the routing table which
    local address would be used — so this reaches no network."""
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(("192.0.2.1", 9))  # TEST-NET-1, deliberately unroutable
        address = s.getsockname()[0]
        return None if address.startswith("127.") else address
    except OSError:
        return None
    finally:
        s.close()


def stdio_tools():
    """initialize + tools/list against a freshly spawned talaria-mcp."""
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
              "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                         "clientInfo": {"name": "http-transport-e2e", "version": "1.0"}}})
        init = reply(1)
        assert "result" in init, init
        send({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})
        send({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
        listed = reply(2)
        return sorted(t["name"] for t in listed["result"]["tools"])
    finally:
        try:
            proc.stdin.close()
            proc.wait(timeout=5)
        except (OSError, ValueError, subprocess.TimeoutExpired):
            proc.kill()
            proc.wait()


xvfb = harness.start_xvfb()
log = None
tal = None
try:
    log = open(SHELL_LOG, "w")

    # --- 1. default off, proven by a refused connection ------------------
    # No config.json at all, which is what a fresh install has.
    assert not os.path.exists(os.path.join(talaria, "config.json"))
    tal = start(log)
    assert rpc("tabs_list")["outcome"] == "ok", \
        "the shell is not answering — 'nothing is listening' would prove nothing"
    assert not accepts("127.0.0.1", DEFAULT_PORT), (
        f"something accepted a connection on {DEFAULT_PORT} with no config.json — "
        "remote access must be an absence, not a disabled flag")
    print(f"DEFAULT OFF: nothing accepts 127.0.0.1:{DEFAULT_PORT} without a config.json")

    # --- 2. enabled from config.json, bound, and the chrome says so ------
    stop_shell(tal)
    tal = None
    port = harness.free_port()
    harness.write_config(talaria, remote_access={"enabled": True, "port": port})
    tal = start(log)
    assert wait_until_accepting("127.0.0.1", port), \
        f"the listener never came up on 127.0.0.1:{port}"
    bound = f"127.0.0.1:{port}"
    print(f"BOUND: the listener accepts {bound}")

    rects, _ = harness.wait_for_rect("toolbar.access")
    assert "toolbar.access" in rects, sorted(rects)
    # Opened by a real pointer click at the rect the shell reports for it.
    wid = subprocess.run(["xdotool", "search", "--name", "Talaria"],
                         env=harness.x_env(), capture_output=True,
                         text=True).stdout.split()[-1]
    harness.click_rect("toolbar.access", wid, harness.x_env())
    rects, _ = harness.wait_for_rect("access.disable")
    assert "access.enable" not in rects, \
        ("the panel offered to turn on a listener that is already bound", sorted(rects))
    assert "access.status" in rects, sorted(rects)
    print("CHROME: the Access panel is in its bound state (access.disable, no access.enable)")

    # --- 3. loopback only ------------------------------------------------
    external = non_loopback_address()
    if external is None:
        print("SKIPPED the non-loopback check: this host has no non-loopback IPv4")
    else:
        assert not accepts(external, port), \
            f"the listener accepted a connection on {external}:{port} — it must bind loopback only"
        print(f"LOOPBACK ONLY: {external}:{port} is refused")

    # --- 4. unauthenticated, and with no side effect ---------------------
    before = rpc("tabs_list")
    status, body = post(f"http://{bound}/mcp")
    assert status == 401, (status, body[:200])
    after = rpc("tabs_list")
    assert before["result"] == after["result"], \
        ("an unauthenticated request changed the browser's tab list", before, after)
    print(f"UNAUTHENTICATED: POST /mcp -> {status}, tab list unchanged")

    # --- 5. a page-originated request ------------------------------------
    status, body = post(f"http://{bound}/mcp", Origin="http://evil.example")
    assert status != 200, (status, body[:200])
    assert status in (401, 403), (status, body[:200])
    after = rpc("tabs_list")
    assert before["result"] == after["result"], "an Origin-bearing request touched the browser"
    print(f"ORIGIN: a request carrying an Origin header -> {status}")

    # --- 5b. and every credential endpoint refuses the same two things ---
    # CR-01: the SDK dispatches its auth routes on `compose(&[], ..)`, so
    # /register, /authorize, /token and /revoke carried neither check while
    # /mcp carried both. A rebound page that could reach /register could read
    # back a client_id, drive /authorize, and — after one Approve — read an
    # access token. These four are the endpoints that mint, spend and destroy
    # credentials, so they are the last four that should have been outside it.
    REGISTRATION = json.dumps(
        {"client_name": "origin-probe", "redirect_uris": ["https://evil.example/cb"]}).encode()
    CREDENTIAL_ENDPOINTS = [
        ("/register", "POST", REGISTRATION, "application/json"),
        ("/authorize?response_type=code", "GET", None, None),
        ("/authorize/status", "GET", None, None),
        ("/token", "POST", b"grant_type=authorization_code",
         "application/x-www-form-urlencoded"),
        ("/revoke", "POST", b"token=nothing", "application/x-www-form-urlencoded"),
    ]
    for path, method, payload, content_type in CREDENTIAL_ENDPOINTS:
        kind = {} if content_type is None else {"content_type": content_type}
        status, _body = raw(path, method=method, body=payload,
                            origin="http://evil.example", **kind)
        assert status == 403, \
            (f"{method} {path} carrying an Origin header was not refused", status)
        status, _body = raw(path, method=method, body=payload,
                            host="rebind.evil:1", **kind)
        assert status == 403, \
            (f"{method} {path} addressed to another host was not refused", status)
    print(f"AS ROUTES: {len(CREDENTIAL_ENDPOINTS)} credential endpoints refuse an "
          "Origin header and a rebound Host")

    # And the exemption is exactly two documents wide. Discovery has to work
    # uncredentialed and cross-origin or a conformant client cannot find the
    # flow at all, so this is the half that must *not* regress.
    for document in ("/.well-known/oauth-protected-resource",
                     "/.well-known/oauth-authorization-server"):
        status, body = raw(document, method="GET", body=None,
                           origin="http://evil.example")
        assert status == 200, (document, status, body[:200])
        assert json.loads(body), (document, body[:200])
        status, body = raw(document, method="GET", body=None, host="rebind.evil:1")
        assert status == 200, (document, status, body[:200])
    print("DISCOVERY: both metadata documents stay readable uncredentialed and "
          "cross-origin")

    # --- 6. SC 4: stdio is unaffected while the listener is on -----------
    tools = stdio_tools()
    assert tools == EXPECTED_TOOLS, tools
    print(f"STDIO: talaria-mcp still lists {len(tools)} tools with the listener on")

    # --- 7. T-3: an agent cannot navigate to the listener's own origin ---
    tabs_before = rpc("tabs_list")["result"]["tabs"]
    refused = rpc("tabs_open", url=f"http://{bound}/")
    assert refused["outcome"] == "error", \
        ("an agent opened a tab at Talaria's own listener", refused)
    assert "remote-access listener" in refused["message"], \
        ("the refusal does not name the reason, so an agent cannot tell policy "
         "from a typo", refused)
    tabs_after = rpc("tabs_list")["result"]["tabs"]
    assert len(tabs_after) == len(tabs_before), \
        ("a tab was opened despite the refusal", tabs_before, tabs_after)
    # And the neighbouring port is not swept up in the refusal.
    other = rpc("tabs_open", url=f"http://127.0.0.1:{harness.free_port()}/")
    assert "remote-access listener" not in str(other.get("message", "")), \
        ("the refusal is not keyed on the bound address", other)
    print("LISTENER ORIGIN: tabs_open at the listener's own address is refused by name")

    # --- 7b. and the refusal is a boundary, not a URL-parsing rule -------
    # WR-03: `parse_agent_url` guards the two commands that take a URL and
    # nothing else, so an agent walked around it in one call —
    # `evaluate(tab, "location.href = ...")`, or a window.open, or a link
    # click. The refusal now lives where navigation actually happens, so a
    # page-driven load to the listener's own origin is denied whatever started
    # it. Asserted on the tab's own URL, over the control socket, rather than
    # on what `evaluate` returned: an assignment to `location.href` evaluates
    # to the assigned string whether or not the load ever begins.
    opened = rpc("tabs_open", url="about:blank")
    assert opened["outcome"] == "ok", opened
    agent_tab = opened["result"]["tab"]["tab_id"]
    driven = rpc("evaluate", tab_id=agent_tab,
                 script=f"location.href = 'http://{bound}/register'")
    assert driven["outcome"] == "ok", ("the evaluate itself failed, so this proves "
                                       "nothing about the navigation", driven)
    # Give a load that should never start every chance to have started.
    time.sleep(2.0)
    landed = [tab for tab in rpc("tabs_list")["result"]["tabs"]
              if tab["tab_id"] == agent_tab]
    assert len(landed) == 1, landed
    assert bound not in landed[0]["url"], (
        "an agent drove its own tab onto Talaria's remote-access listener by "
        "assigning location.href — the own-origin refusal is not a boundary",
        landed[0])
    rpc("tabs_close", tab_id=agent_tab)
    print("PAGE-DRIVEN: location.href at the listener's own origin never loads")

    # --- 8. the switch itself: off is one click, on is two ---------------
    # The Access panel is still open from step 2.
    harness.click_rect("access.disable", wid, harness.x_env())
    rects, _ = harness.wait_for_rect("access.enable")
    assert "access.disable" not in rects, sorted(rects)
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline and accepts("127.0.0.1", port):
        time.sleep(0.2)
    assert not accepts("127.0.0.1", port), \
        "the port was still accepting after remote access was switched off"
    assert rpc("tabs_list")["outcome"] == "ok", "switching the listener off took the shell with it"
    # And the refusal it keyed goes with it: nothing is bound, so that address
    # is an ordinary one again.
    allowed = rpc("tabs_open", url=f"http://{bound}/")
    assert "remote-access listener" not in str(allowed.get("message", "")), \
        ("the own-origin refusal outlived the listener it was keyed on", allowed)
    print("DISABLE: one click, the port stops accepting, and the refusal it keyed lifts")

    # Turning it back on takes two clicks: the first only arms the control.
    harness.click_rect("access.enable", wid, harness.x_env())
    rects, _ = harness.wait_for_rect("access.confirm-enable")
    assert "access.enable" not in rects, sorted(rects)
    assert not accepts("127.0.0.1", port), \
        "the first click bound a port — arming must not be enabling"
    harness.click_rect("access.confirm-enable", wid, harness.x_env())
    assert wait_until_accepting("127.0.0.1", port), \
        "the confirming click did not start the listener"
    harness.wait_for_rect("access.disable")
    status, _ = post(f"http://{bound}/mcp")
    assert status == 401, status
    print("ENABLE: the first click only arms; the second binds, and it still refuses")

    print("HTTP TRANSPORT CHECKS PASSED")
finally:
    if tal is not None:
        stop_shell(tal)
    if log is not None:
        log.close()
    xvfb.kill()
    xvfb.wait()  # reap, so the X lock's pid isn't left as a zombie
    shutil.rmtree(tmp, ignore_errors=True)

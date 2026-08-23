#!/usr/bin/env python3
"""The remote view channel, end to end: DIST-01's transport half.

What this pins down, in the order it pins it:

  * **Absence, not refusal.** With remote access off there is no listener, no
    bound port, and therefore no `/view` route at all. That is the default
    installation, and it is an absence rather than a flag consulted per
    request.
  * **The route authenticates in its own right.** An upgrade carrying no
    credential is refused, and the answer is compared **byte for byte** against
    the answer an unknown token gets, so the endpoint is not an oracle for
    which tokens exist. This is the load-bearing assertion of the whole suite:
    the SDK's middleware chain is composed only for its own transport handlers,
    so a merged axum route does not pass through it, and a `/view` that assumed
    inheritance would be origin-checked and unauthenticated.
  * **Cross-site WebSocket hijacking is closed by construction.** A WebSocket
    is exempt from the same-origin policy and has no preflight, so origin
    enforcement is entirely the server's job. An upgrade carrying an `Origin`
    header is refused — and since a browser *always* sends one, a browser-based
    viewer is structurally impossible rather than merely unsupported.
  * **A request addressed to a host this server neither bound nor advertises is
    refused**, which is the DNS-rebinding answer applied to this route too.
  * **The handshake is completed and verified**, not merely sent: the accept
    value is recomputed from the key, so a server that answered `101` without
    completing the handshake fails here.
  * **The snapshot is agent tabs only**, in the tab table's own order, stable
    across repeated reads, and empty rather than absent when there are none.
  * **A tab the human owns is refused**, with bytes identical to a tab that was
    never created.
  * Two viewers may watch one tab; a tab closing underneath them detaches both;
    and a client cannot pin more tabs than the cap allows.

**Its honest limits.** Everything here runs on loopback. That proves the
protocol and the authorisation and proves **nothing** about the network: TLS
termination, the tailnet `Host`, relay latency and the two-machine case are
05-10's and the manual verification's, not this suite's.

Every assertion about what the *browser* did goes over the control socket, not
over the channel under test, so a bug in the transport cannot also be the thing
reporting success.

Preconditions: a release build of ``talaria`` and an Xvfb display.
"""
import json
import os
import shutil
import socket
import sys
import tempfile
import time

T = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, T)
import harness

# The view wire's channel tags (`talaria_protocol::wire`). Spelled here rather
# than imported because there is nothing to import from: the suite is a client
# on the far side of the wire, and writing the tags out is what makes it one.
CHANNEL_CONTROL = 0x01
CHANNEL_TABS = 0x02

# The wire version the server announces. A mismatch here means the shell and
# this suite were built against different wires, which is worth failing on.
PROTOCOL_VERSION = 1

# The cap the shell is started with. Lowered from its default so the cap can be
# reached with three tabs instead of nine — the property under test is that
# there *is* a ceiling and that crossing it is refused, not what the number is.
MAX_ATTACH = 2

tmp = tempfile.mkdtemp(prefix="talaria-view-e2e-")
config = os.path.join(tmp, ".config")
talaria = os.path.join(config, "talaria")
os.makedirs(talaria)
OUT = os.environ.get("TALARIA_E2E_OUT", "/tmp/talaria-e2e")
os.makedirs(OUT, exist_ok=True)
SHELL_LOG = os.path.join(OUT, "remote-view-shell.log")

CLIENT_ID = "client-viewer"
TOKEN = "tal_seeded_view_token_for_the_e2e_suite"
UNKNOWN = "tal_this_token_was_never_issued_by_anyone"


def rpc(command, client="remote-view-e2e", **params):
    """One control-socket request on its own connection.

    The different channel: everything this suite claims about the browser's
    own state is read here rather than over the WebSocket under test."""
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


def view(payload):
    """One control-channel frame, composed the way a real viewer composes it."""
    return bytes([CHANNEL_CONTROL]) + json.dumps(payload).encode()


def read(ws, channel, timeout=6.0):
    """The next message on ``channel``, skipping the others.

    Two channels are in flight at once — control answers and tab snapshots —
    and a reader that took whatever arrived first would be asserting about
    whichever the server happened to send, not about what it asked for."""
    deadline = time.monotonic() + timeout
    while True:
        message = ws.next_message(max(0.0, deadline - time.monotonic()))
        if message is None:
            return None
        if message[0] == channel:
            return json.loads(message[1:])
        if time.monotonic() > deadline:
            return None


def snapshot(ws, timeout=6.0):
    """The next tab snapshot's ids, in the order the server sent them."""
    listed = read(ws, CHANNEL_TABS, timeout)
    assert listed is not None, "the server sent no tab snapshot"
    assert "tabs" in listed, ("the snapshot has no tabs field at all", listed)
    return [tab["tab_id"] for tab in listed["tabs"]]


def connect(port, token=TOKEN, **kwargs):
    """An authorized viewer, past the hello."""
    ws = harness.WebSocket("127.0.0.1", port, token=token, **kwargs)
    assert ws.handshook(), (ws.status, ws.headers)
    hello = read(ws, CHANNEL_CONTROL)
    assert hello == {"view": "hello", "protocol": PROTOCOL_VERSION}, hello
    return ws


def open_agent_tab(client):
    """A tab owned by an agent session, opened over the control socket."""
    reply = rpc("tabs_open", client=client, url="about:blank")
    assert reply["outcome"] == "ok", reply
    return reply["result"]["tab"]["tab_id"]


def tabs_owned_by(owner):
    tabs = rpc("tabs_list")["result"]["tabs"]
    return [tab["tab_id"] for tab in tabs if tab["owner"] == owner]


xvfb = harness.start_xvfb()
log = None
tal = None
sockets = []
try:
    log = open(SHELL_LOG, "w")
    port = harness.free_port()

    # --- 1. remote access off: no listener, so no route ------------------
    tal = harness.start_shell("about:blank", log=log, rust_log="info", wait=10,
                              HOME=tmp, XDG_CONFIG_HOME=config)
    assert rpc("tabs_list")["outcome"] == "ok", "the shell is not answering its socket"
    assert not accepts("127.0.0.1", port), \
        (f"something is listening on {port} with remote access off", port)
    print(f"DEFAULT OFF: nothing accepts 127.0.0.1:{port}, so there is no /view at all")

    # --- 2. switch it on, with one approved client -----------------------
    tal.terminate()
    tal.wait()
    tal = None
    harness.write_config(talaria, remote_access={"enabled": True, "port": port})
    harness.write_agents(talaria, CLIENT_ID, "The Viewer", TOKEN,
                         audience=f"http://127.0.0.1:{port}/mcp")
    tal = harness.start_shell("about:blank", log=log, rust_log="info", wait=10,
                              HOME=tmp, XDG_CONFIG_HOME=config,
                              TALARIA_VIEW_MAX_ATTACH=str(MAX_ATTACH))
    assert wait_until_accepting("127.0.0.1", port), f"the listener never bound {port}"
    print(f"BOUND: the listener accepts 127.0.0.1:{port}")

    # --- 3. an upgrade with no credential is refused, identically --------
    # The assertion this suite exists for. The SDK composes its middleware
    # chain only for its own transport handlers, so a merged axum route does
    # not pass through it — a `/view` that assumed inheritance would be
    # origin-checked and unauthenticated, and only a direct assertion catches
    # that. Comparing against the unknown-token answer is the second half: two
    # refusals that differ are two bits an attacker did not have.
    bare = harness.WebSocket("127.0.0.1", port)
    sockets.append(bare)
    assert bare.status == 401, (bare.status, bare.headers, bare.body[:200])
    unknown = harness.WebSocket("127.0.0.1", port, token=UNKNOWN)
    sockets.append(unknown)
    assert unknown.status == 401, (unknown.status, unknown.body[:200])
    assert (bare.status, bare.body) == (unknown.status, unknown.body), (
        "an uncredentialed upgrade is distinguishable from one carrying a token "
        "that was never issued, which makes /view an oracle for which tokens exist",
        bare.body[:200], unknown.body[:200])
    print("UNAUTHENTICATED: /view refuses an upgrade with no credential, with the "
          "identical answer an unknown token gets")

    # --- 4. an upgrade carrying an Origin header is refused --------------
    # Cross-site WebSocket hijacking. A WebSocket has no preflight and is not
    # subject to the same-origin policy, so this is entirely the server's job.
    # A browser always sends this header, which is what makes a browser-based
    # viewer structurally impossible rather than merely unsupported.
    paged = harness.WebSocket("127.0.0.1", port, token=TOKEN,
                              headers=("Origin: http://rebind.evil",))
    sockets.append(paged)
    assert paged.status == 403, (paged.status, paged.headers, paged.body[:200])
    print("ORIGIN: an upgrade carrying an Origin header is refused, so no browser "
          "can ever open this channel")

    # --- 5. an upgrade addressed to a host this server does not answer to -
    misaddressed = harness.WebSocket("127.0.0.1", port, token=TOKEN,
                                     host_header=f"rebind.evil:{port}")
    sockets.append(misaddressed)
    assert misaddressed.status == 403, (misaddressed.status, misaddressed.body[:200])
    print("HOST: an upgrade addressed to a name this server neither bound nor "
          "advertises is refused")

    # --- 6. a valid upgrade completes, and says which wire it speaks -----
    first = harness.WebSocket("127.0.0.1", port, token=TOKEN)
    sockets.append(first)
    assert first.status == 101, (first.status, first.headers, first.body[:200])
    assert first.handshook(), \
        ("the server answered 101 without a correct Sec-WebSocket-Accept", first.headers)
    hello = read(first, CHANNEL_CONTROL)
    assert hello == {"view": "hello", "protocol": PROTOCOL_VERSION}, hello
    print(f"HANDSHAKE: the upgrade completed with a verified accept value and a "
          f"hello for protocol {PROTOCOL_VERSION}")

    # --- 7. no agent tabs is an EMPTY list, not an error -----------------
    assert tabs_owned_by("me"), "the shell has no Me tab, so this proves nothing"
    listed = read(first, CHANNEL_TABS)
    assert listed is not None, "a viewer of a browser with no agent tabs was told nothing"
    assert "tabs" in listed and listed["tabs"] == [], \
        ("zero agent tabs is not an empty list", listed)
    assert first.still_open(), "an empty snapshot closed the connection"
    print("EMPTY: zero agent tabs is an empty list, the field is present, and the "
          "connection stays open")

    # --- 8. agent tabs only, in creation order, stable across reads ------
    tab_a = open_agent_tab("agent-one")
    me_reply = rpc("open_for_user", url="about:blank")
    assert me_reply["outcome"] == "ok", me_reply
    tab_b = open_agent_tab("agent-two")
    me_tabs = tabs_owned_by("me")
    assert len(me_tabs) >= 2, ("the human-owned tab was not created", me_tabs)

    listed = None
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        candidate = snapshot(first)
        if candidate == [tab_a, tab_b]:
            listed = candidate
            break
    assert listed == [tab_a, tab_b], \
        ("the snapshot is not the two agent tabs in creation order", listed,
         [tab_a, tab_b])
    # A second viewer reads the same table: the order is the tab table's own
    # and is not invented per connection.
    second = connect(port)
    sockets.append(second)
    assert snapshot(second) == [tab_a, tab_b], "repeating the read changed the order"
    for me_tab in me_tabs:
        assert me_tab not in listed, ("a tab the human owns appeared in the snapshot",
                                      me_tab, listed)
    print(f"SNAPSHOT: exactly the two agent tabs {tab_a},{tab_b} in creation order, "
          f"stable across a second read, and none of the human's {me_tabs}")

    # --- 9. a human-owned tab and a tab that never existed refuse alike --
    first.send(view({"view": "attach", "tab": me_tabs[0]}))
    owned = read(first, CHANNEL_CONTROL)
    first.send(view({"view": "attach", "tab": 99999}))
    absent = read(first, CHANNEL_CONTROL)
    assert owned == {"view": "refused"}, ("attaching to the human's tab was not refused",
                                          owned)
    assert owned == absent, (
        "a refused attach distinguishes a tab the human owns from a tab that does "
        "not exist, which makes the channel an enumeration oracle for the human's "
        "own browsing", owned, absent)
    assert json.dumps(owned, sort_keys=True) == json.dumps(absent, sort_keys=True), \
        "the two refusals are not byte-for-byte identical"
    print("REFUSED IDENTICALLY: a tab the human owns and a tab that never existed "
          "produce the same refusal, byte for byte")

    # --- 10. attaching to an agent tab is acknowledged with a viewport ---
    first.send(view({"view": "attach", "tab": tab_a}))
    attached = read(first, CHANNEL_CONTROL)
    assert attached is not None and attached.get("view") == "attached", attached
    assert attached["tab"] == tab_a, attached
    assert attached["width"] > 0 and attached["height"] > 0, \
        ("the acknowledgement carried no usable viewport", attached)
    print(f"ATTACHED: tab {tab_a} acknowledged at "
          f"{attached['width']}x{attached['height']}")

    # --- 11. two viewers on one tab, neither displacing the other --------
    second.send(view({"view": "attach", "tab": tab_a}))
    also = read(second, CHANNEL_CONTROL)
    assert also is not None and also.get("view") == "attached", also
    assert also["tab"] == tab_a, also
    assert first.still_open(), "a second viewer attaching displaced the first"
    assert second.still_open(), second
    print(f"TWO VIEWERS: both connections hold tab {tab_a} and neither displaced "
          f"the other")

    # --- 12. the tab closes underneath both of them ----------------------
    closed = rpc("tabs_close", tab_id=tab_a)
    assert closed["outcome"] == "ok", closed
    for label, ws in (("first", first), ("second", second)):
        notice = read(ws, CHANNEL_CONTROL)
        assert notice == {"view": "detached", "tab": tab_a}, (label, notice)
    remaining = None
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        remaining = snapshot(first)
        if tab_a not in remaining:
            break
    assert remaining is not None and tab_a not in remaining, \
        ("a closed tab is still in the snapshot", tab_a, remaining)
    assert tab_b in remaining, remaining
    print(f"CLOSED UNDERNEATH: both viewers were detached from tab {tab_a} and it "
          f"is gone from the next snapshot")

    # --- 13. the concurrent-attachment cap -------------------------------
    tab_c = open_agent_tab("agent-three")
    tab_d = open_agent_tab("agent-four")
    accepted = []
    refusals = []
    for tab in (tab_b, tab_c, tab_d):
        first.send(view({"view": "attach", "tab": tab}))
        answer = read(first, CHANNEL_CONTROL)
        assert answer is not None, ("no answer to an attach", tab)
        (accepted if answer.get("view") == "attached" else refusals).append(answer)
    assert len(accepted) == MAX_ATTACH, \
        (f"the cap of {MAX_ATTACH} was not what was enforced", accepted, refusals)
    assert refusals == [{"view": "refused"}], \
        ("attaching beyond the cap was not refused, or was refused differently "
         "from everything else", refusals)
    assert first.still_open(), "hitting the cap closed the connection"
    print(f"CAPPED: {MAX_ATTACH} attachments were accepted and the next was refused "
          f"with the same refusal as everything else")

    print("REMOTE VIEW CHECKS PASSED")
finally:
    for ws in sockets:
        ws.close()
    if tal is not None:
        tal.terminate()
        time.sleep(1)
        tal.kill()
        tal.wait()
    xvfb.kill()
    xvfb.wait()
    if log is not None:
        log.close()
    shutil.rmtree(tmp, ignore_errors=True)

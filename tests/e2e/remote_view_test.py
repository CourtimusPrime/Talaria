#!/usr/bin/env python3
"""The remote view channel, end to end: DIST-01's transport half and DIST-02's
input half.

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

And then the input half, DIST-02:

  * **A remote click lands on the element it was aimed at**, proven in both
    directions on a fixture whose links sit at known, well-separated positions
    — and with a *decoy* link one toolbar-height above each of them, so a
    coordinate wrongly shifted by the local window's chrome offset navigates to
    a **named** wrong destination rather than merely to nothing. An assertion
    that only checked "a navigation occurred" would pass with that bug in
    place; this one names the destination it got.
  * **Keystrokes reach the page**, and a key naming something this build does
    not know types nothing rather than something else.
  * **A replayed input message does nothing.** The sequence is strictly
    increasing per connection; a verbatim resend of an accepted message changes
    no page state, and the next greater sequence does.
  * **A tab the human owns refuses remote input** — with the tab *displayed on
    screen*, so the refusal is the server's and not an accident of the page not
    being painted — indistinguishably from a tab id that never existed, and
    without dropping the connection.
  * **Remote input cannot reach the chrome.** A pointer press is aimed at the
    credentials control's *real* rectangle, read from the chrome-geometry test
    hook, and at the history control's, and no panel opens. The history control
    is the discriminating one — a local click there provably *does* open a
    panel in this same run — and it is the one a refactor routing remote input
    through the local window-event path would trip.
  * **A viewer sending input is not a viewer choosing what the human sees:** the
    window's title and the set of focused tabs are unchanged across the whole
    remote input sequence.

**Its honest limits.** Everything here runs on loopback. That proves the
protocol and the authorisation and proves **nothing** about the network: TLS
termination, the tailnet `Host`, relay latency and the two-machine case are
05-10's and the manual verification's, not this suite's.

The input assertions drive a tab that is **on screen**, because until the frame
plan lands an attachment does not show a webview and a hidden one has no hit
test to answer a click. The suite switches the local view to Agents itself, the
way a human would, and says so at the step that does it.

The credentials panel draws no named rectangle while the vault is empty, so
"no panel opened" is measured on the history panel's rows — a probe this suite
proves discriminating before it relies on it. The credentials control is still
the coordinate aimed at, because it is the one whose reachability matters most.

Every assertion about what the *browser* did goes over the control socket, not
over the channel under test, so a bug in the transport cannot also be the thing
reporting success.

Preconditions: a release build of ``talaria`` and an Xvfb display.
"""
import http.server
import json
import os
import shutil
import socket
import socketserver
import subprocess
import sys
import tempfile
import threading
import time

T = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, T)
import harness

# The view wire's channel tags (`talaria_protocol::wire`). Spelled here rather
# than imported because there is nothing to import from: the suite is a client
# on the far side of the wire, and writing the tags out is what makes it one.
CHANNEL_CONTROL = 0x01
CHANNEL_TABS = 0x02
CHANNEL_INPUT = 0x04

# The wire version the server announces. A mismatch here means the shell and
# this suite were built against different wires, which is worth failing on.
PROTOCOL_VERSION = 1

# The cap the shell is started with. Lowered from its default so the cap can be
# reached with three tabs instead of nine — the property under test is that
# there *is* a ceiling and that crossing it is refused, not what the number is.
MAX_ATTACH = 2

# The fixture, and every number in it is load-bearing.
#
# Two destinations at **known, well-separated** vertical positions, so hitting
# the wrong one is detectable rather than indistinguishable from hitting the
# right one — and above each of them a *decoy* band roughly one toolbar-height
# tall. The decoys are the regression assertion for the coordinate trap: the
# local window subtracts its chrome height from a pointer coordinate before
# handing it to a webview, and a remote path that reused that subtraction would
# land every click some forty pixels high. Without the decoys such a click hits
# nothing and the failure reads as "the click did not work"; with them it
# navigates to `/decoy-lower.html`, which names the bug.
#
# Nothing sits in the top 100 px, because the toolbar is drawn *over* the
# webview rather than beside it — so the chrome assertion's coordinate lands in
# a genuinely empty part of the page, and "the URL did not change" means the
# click reached neither a panel nor a link.
FIXTURE = b"""<!doctype html><html><head><meta charset="utf-8"><title>fixture</title>
<style>
  body { margin: 0; font: 16px sans-serif; }
  a { display: block; position: absolute; left: 40px; width: 420px; }
  #decoy-upper { top: 105px;  height: 80px; background: #fee; }
  #upper       { top: 200px;  height: 24px; background: #cfc; }
  #decoy-lower { top: 405px;  height: 80px; background: #fee; }
  #lower       { top: 500px;  height: 24px; background: #ccf; }
  #field       { position: absolute; top: 620px; left: 40px;
                 width: 300px; height: 30px; }
</style></head><body>
<a id="decoy-upper" href="/decoy-upper.html">DECOY ABOVE THE UPPER LINK</a>
<a id="upper" href="/upper.html">UPPER LINK</a>
<a id="decoy-lower" href="/decoy-lower.html">DECOY ABOVE THE LOWER LINK</a>
<a id="lower" href="/lower.html">LOWER LINK</a>
<input id="field" type="text">
</body></html>"""


class Fixture(http.server.BaseHTTPRequestHandler):
    """Every path serves the same page, so a navigation is identified by its
    URL alone and the suite never has to parse a body to know where it went."""

    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(FIXTURE)))
        self.end_headers()
        self.wfile.write(FIXTURE)

    def log_message(self, *args):
        pass

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


def tab_url(tab_id):
    return next(tab["url"] for tab in rpc("tabs_list")["result"]["tabs"]
                if tab["tab_id"] == tab_id)


def focused_tabs():
    """The tab ids the shell considers active, in either view.

    The observable half of "a viewer did not change what the human sees": this
    is `active_me` and `active_agent` together, read over the control socket."""
    return sorted(tab["tab_id"] for tab in rpc("tabs_list")["result"]["tabs"]
                  if tab["focused"])


def wait_for_url(tab_id, want, timeout=12.0):
    """Poll until the tab's URL contains `want`, then return whatever it is.

    Returns the *current* URL either way, so the caller's assertion names the
    destination that was actually reached rather than only failing on a
    timeout — which is the whole point of the decoy links."""
    deadline = time.monotonic() + timeout
    url = tab_url(tab_id)
    while time.monotonic() < deadline:
        if want in url:
            return url
        time.sleep(0.3)
        url = tab_url(tab_id)
    return url


def settle_url(tab_id, seconds=3.0):
    """The tab's URL after `seconds` of nothing happening.

    For the refusals: a navigation that was going to happen has had time to,
    so an unchanged URL is evidence rather than a race won."""
    time.sleep(seconds)
    return tab_url(tab_id)


def evaluate(tab_id, script):
    reply = rpc("evaluate", tab_id=tab_id, script=script)
    assert reply["outcome"] == "ok", (script, reply)
    return reply["result"]["value"]


class Input:
    """A viewer's input channel: the sequence counter, and the frames.

    The counter is the client's half of the wire's contract — `seq` is strictly
    increasing within one connection — and keeping it here means the replay
    assertions have to *reach past* it to resend a number, which is what makes
    them deliberate rather than accidental."""

    def __init__(self, ws, tab):
        self.ws = ws
        self.tab = tab
        self.seq = 0

    def raw(self, message):
        """One input frame exactly as given — no sequence assigned."""
        self.ws.send(bytes([CHANNEL_INPUT]) + json.dumps(message).encode())
        time.sleep(0.15)
        return message

    def send(self, kind, **fields):
        self.seq += 1
        return self.raw({"kind": kind, "tab": self.tab, "seq": self.seq,
                         **fields})

    def click(self, point, tab=None):
        """Move, press and release at `point` — a whole human click.

        Three messages and not one: the wire has no "click", because a client
        that could send one could not express a drag, and the local path has no
        such primitive either."""
        x, y = point
        target = self.tab if tab is None else tab
        for kind, extra in (("mouse_move", {}),
                            ("mouse_button", {"button": "left",
                                              "action": "down"}),
                            ("mouse_button", {"button": "left",
                                              "action": "up"})):
            self.seq += 1
            self.raw({"kind": kind, "tab": target, "seq": self.seq,
                      "x": float(x), "y": float(y), **extra})

    def type_character(self, character):
        last = self.send("key", state="down", key=character)
        self.send("key", state="up", key=character)
        return last


def centres(tab_id, scale):
    """Each fixture element's centre in the tab's own device pixels.

    `getBoundingClientRect` is CSS pixels with the page's top-left as the
    origin; the wire is device pixels with the same origin, so the conversion
    is the window's scale factor and **nothing else**. In particular there is
    no toolbar term: a remote client draws no server toolbar, and adding one
    here would be writing the bug this fixture exists to catch."""
    script = ("const c = id => { const r = document.getElementById(id)"
              ".getBoundingClientRect(); return [r.x + r.width / 2, "
              "r.y + r.height / 2]; };"
              "[c('upper'), c('lower'), c('decoy-upper'), c('decoy-lower'), "
              "c('field')]")
    names = ("upper", "lower", "decoy-upper", "decoy-lower", "field")
    return {name: (x * scale, y * scale)
            for name, (x, y) in zip(names, evaluate(tab_id, script))}


def find_window(env, attempts=20):
    for _ in range(attempts):
        found = subprocess.run(["xdotool", "search", "--name", "Talaria"],
                               env=env, capture_output=True,
                               text=True).stdout.split()
        if found:
            return found[0]
        time.sleep(1)
    raise AssertionError("the Talaria window never appeared")


def window_title(wid, env):
    return subprocess.run(["xdotool", "getwindowname", wid], env=env,
                          capture_output=True, text=True).stdout.strip()


xvfb = harness.start_xvfb()
X = harness.x_env()
log = None
tal = None
fixture = None
sockets = []
try:
    log = open(SHELL_LOG, "w")
    port = harness.free_port()

    # The page the input half drives, served from this process on loopback.
    fixture_port = harness.free_port()
    fixture = socketserver.TCPServer(("127.0.0.1", fixture_port), Fixture)
    threading.Thread(target=fixture.serve_forever, daemon=True).start()
    BASE = f"http://127.0.0.1:{fixture_port}"

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
    # `TALARIA_TEST_HOOKS=1` is what makes `chrome_rects` answer, and the
    # chrome assertion needs it: aiming at a *guessed* toolbar coordinate would
    # prove nothing when the toolbar gains a button.
    tal = harness.start_shell("about:blank", log=log, rust_log="info", wait=10,
                              HOME=tmp, XDG_CONFIG_HOME=config,
                              TALARIA_VIEW_MAX_ATTACH=str(MAX_ATTACH),
                              TALARIA_TEST_HOOKS="1")
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

    # ======================================================================
    # DIST-02: the input half. Everything below drives a real page.
    # ======================================================================

    # --- 14. a fixture page, in an agent tab and in one of the human's ---
    driven = rpc("tabs_open", client="agent-input", url=f"{BASE}/index.html")
    assert driven["outcome"] == "ok", driven
    driven = driven["result"]["tab"]["tab_id"]
    mine = rpc("open_for_user", url=f"{BASE}/index.html")
    assert mine["outcome"] == "ok", mine
    mine = max(tabs_owned_by("me"))
    assert wait_for_url(driven, "/index.html").endswith("/index.html"), \
        ("the agent tab never reached the fixture", tab_url(driven))
    assert wait_for_url(mine, "/index.html").endswith("/index.html"), \
        ("the human's tab never reached the fixture", tab_url(mine))

    # --- 15. put the agent's tab on screen, the way a human would ---------
    # Until the frame plan lands, an attachment does not show a webview, and a
    # hidden webview has no hit test to answer a click with. So the *local*
    # human switches to the Agents view — a real click on the real toggle —
    # and the suite then measures that the remote input reached the page.
    # Nothing below this line switches anything: that is the point of doing it
    # here, before the measurement starts.
    wid = find_window(X)
    rpc("tabs_focus", tab_id=driven)
    harness.click_rect("toolbar.agents", wid, X)
    time.sleep(1.5)
    assert window_title(wid, X).startswith("fixture"), \
        ("the agent's fixture tab is not the one on screen", window_title(wid, X))
    print(f"ON SCREEN: the local human switched to the Agents view and tab "
          f"{driven} is displayed")

    # --- 16. a viewer attached to it -------------------------------------
    viewer = connect(port)
    sockets.append(viewer)
    viewer.send(view({"view": "attach", "tab": driven}))
    ack = read(viewer, CHANNEL_CONTROL)
    assert ack is not None and ack.get("view") == "attached", ack
    _, scale = harness.chrome_rects()
    where = centres(driven, scale)
    keys = Input(viewer, driven)

    # --- 17. a remote click lands on the LOWER link ----------------------
    # The regression assertion, and the reason the fixture carries decoys: an
    # assertion that merely checked "a navigation occurred" would pass with the
    # local toolbar offset wrongly applied. This one names what it hit.
    keys.click(where["lower"])
    landed = wait_for_url(driven, "/lower.html")
    assert landed.endswith("/lower.html"), (
        "a remote click aimed at the lower link did not reach it", landed,
        where["lower"])
    assert "/decoy-lower.html" not in landed, (
        "the click landed one toolbar-height high — the local window's chrome "
        "offset was applied to a remote coordinate", landed)
    assert "/upper.html" not in landed, ("the click hit the upper link", landed)
    print(f"AIMED LOWER: a remote click at {where['lower']} reached "
          f"{landed.rsplit('/', 1)[-1]}, not the decoy above it")

    # The decoy is *reachable*, which is what stops the assertion above being
    # vacuous: aimed at deliberately, it navigates. So "not the decoy" is a
    # statement about where the click went, not about a link that never worked.
    assert rpc("navigate", tab_id=driven, url=f"{BASE}/index.html")["outcome"] == "ok"
    wait_for_url(driven, "/index.html")
    keys.click(where["decoy-lower"])
    landed = wait_for_url(driven, "/decoy-lower.html")
    assert landed.endswith("/decoy-lower.html"), (
        "the decoy band is not clickable, so 'not the decoy' proves nothing",
        landed)
    print("DECOY REACHABLE: aimed at the band one toolbar-height above the "
          "lower link, a remote click lands on it — so the offset regression "
          "would be named rather than merely missed")

    # --- 18. and the other direction, at the UPPER link ------------------
    # Two assertions in opposite directions is what makes the coordinate
    # mapping proven rather than coincidental.
    assert rpc("navigate", tab_id=driven, url=f"{BASE}/index.html")["outcome"] == "ok"
    wait_for_url(driven, "/index.html")
    keys.click(where["upper"])
    landed = wait_for_url(driven, "/upper.html")
    assert landed.endswith("/upper.html"), (
        "a remote click aimed at the upper link did not reach it", landed,
        where["upper"])
    assert "/decoy-upper.html" not in landed, (
        "the click landed one toolbar-height high", landed)
    assert "/lower.html" not in landed, ("the click hit the lower link", landed)
    print(f"AIMED UPPER: a remote click at {where['upper']} reached "
          f"{landed.rsplit('/', 1)[-1]}, not the decoy above it")

    # --- 19. keystrokes reach the page, and an unknown key types nothing --
    assert rpc("navigate", tab_id=driven, url=f"{BASE}/index.html")["outcome"] == "ok"
    wait_for_url(driven, "/index.html")
    where = centres(driven, scale)
    keys.click(where["field"])
    time.sleep(0.5)
    for character in "hey":
        keys.type_character(character)
    # A key naming something this build does not carry: refused, and not
    # substituted with the nearest thing it might have meant.
    keys.send("key", state="down", named="Warp")
    keys.send("key", state="up", named="Warp")
    time.sleep(1.0)
    typed = evaluate(driven, "document.getElementById('field').value")
    assert typed == "hey", ("the remote keystrokes did not arrive intact", typed)
    print(f"TYPED: the field reads {typed!r}, and the unmapped key name added "
          f"nothing")

    # --- 20. a replayed input message does nothing -----------------------
    replayed = keys.type_character("!")
    time.sleep(0.8)
    grown = evaluate(driven, "document.getElementById('field').value")
    assert grown == "hey!", grown
    keys.raw(replayed)          # verbatim, sequence and all
    keys.raw(replayed)
    time.sleep(1.0)
    after = evaluate(driven, "document.getElementById('field').value")
    assert after == "hey!", ("a replayed input message was applied a second "
                             "time", after)
    keys.type_character("?")    # the next greater sequence, which does apply
    time.sleep(1.0)
    after = evaluate(driven, "document.getElementById('field').value")
    assert after == "hey!?", ("a message with a greater sequence was dropped",
                              after)
    assert viewer.still_open(), "a replayed message closed the connection"
    print("REPLAY: resending an accepted input message verbatim changed "
          "nothing, and the next greater sequence did")

    # --- 21. remote input cannot reach the chrome ------------------------
    # The chrome is where the credentials control, the bookmark star and a
    # downloads row's open control live — the surfaces `ChromeRect`'s own doc
    # comment refused to expose to agents, for exactly this reason. This is the
    # assertion a future refactor routing remote input through the local
    # window-event path would trip.
    #
    # The probe is the history panel's rows, because the credentials panel
    # draws no named rectangle while the vault is empty. It is proved
    # discriminating first, with a *local* click on the same control.
    before_title = window_title(wid, X)
    before_focused = focused_tabs()
    before_url = tab_url(driven)

    harness.click_rect("toolbar.history", wid, X)
    harness.wait_for_rect("history.row.0")
    harness.click_rect("toolbar.history", wid, X)
    harness.wait_for_rect("history.row.0", present=False)
    print("PROBE: a local click on the history control opens the panel and a "
          "second one closes it, so its rows are a probe that discriminates")

    rects, scale = harness.chrome_rects()
    for control in ("toolbar.credentials", "toolbar.history"):
        x, y, width, height = rects[control]
        # The same rectangle `click_rect` converts through the *window's*
        # origin for a local click. A remote message has no window and no
        # chrome strip above the page, so these numbers land in the page's own
        # empty top band — which is the whole of why the chrome is out of
        # reach.
        keys.click(((x + width / 2) * scale, (y + height / 2) * scale))
    time.sleep(1.5)
    harness.wait_for_rect("history.row.0", present=False, timeout=3.0)
    assert tab_url(driven) == before_url, (
        "a remote click at a toolbar coordinate navigated the page",
        before_url, tab_url(driven))
    print("CHROME: remote clicks at the credentials control's and the history "
          "control's real coordinates opened no panel and navigated nothing")

    # --- 22. and none of it moved what the human is looking at -----------
    assert window_title(wid, X) == before_title, (
        "remote input changed the displayed tab or the view mode",
        before_title, window_title(wid, X))
    assert focused_tabs() == before_focused, (
        "remote input changed an active tab", before_focused, focused_tabs())
    print(f"UNMOVED: the window still reads {before_title!r} and the focused "
          f"tabs are still {before_focused}")

    # --- 23. a tab the human owns refuses, with that tab on screen -------
    # On screen deliberately: a refusal measured against a hidden webview would
    # be indistinguishable from a click that simply had nothing to hit. The
    # human's own tab is displayed, the coordinates are the ones that provably
    # worked on the agent's copy of the same page, and nothing happens.
    rpc("tabs_focus", tab_id=mine)
    harness.click_rect("toolbar.me", wid, X)
    time.sleep(1.5)
    assert window_title(wid, X).startswith("fixture"), \
        ("the human's fixture tab is not the one on screen",
         window_title(wid, X))

    viewer.send(view({"view": "attach", "tab": mine}))
    refused_mine = read(viewer, CHANNEL_CONTROL)
    viewer.send(view({"view": "attach", "tab": 424242}))
    refused_absent = read(viewer, CHANNEL_CONTROL)
    assert refused_mine == {"view": "refused"}, refused_mine
    assert refused_mine == refused_absent, (
        "a tab the human owns is distinguishable from one that never existed",
        refused_mine, refused_absent)

    mine_before = tab_url(mine)
    keys.click(where["lower"], tab=mine)
    keys.click(where["lower"], tab=424242)
    assert settle_url(mine, 3.0) == mine_before, (
        "remote input reached a tab the human owns", mine_before,
        tab_url(mine))
    assert viewer.still_open(), \
        "naming the human's tab dropped the connection instead of refusing"
    print(f"HUMAN'S TAB: input aimed at the displayed Me tab {mine} did "
          f"nothing, indistinguishably from a tab id that never existed, and "
          f"the connection stayed open")

    print("REMOTE VIEW CHECKS PASSED")
finally:
    for ws in sockets:
        ws.close()
    if fixture is not None:
        fixture.shutdown()
        fixture.server_close()
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

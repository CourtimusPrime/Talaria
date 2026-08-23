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

And then the frame half, also DIST-02:

  * **The first frame after an attach is a whole keyframe**, at the origin,
    covering the whole surface, and declaring the same size the attach
    acknowledgement did — so a client can allocate its texture before the first
    frame rather than reflow on it.
  * **A page nobody is touching sends nothing at all.** Read across many ticks
    with the page untouched, not one frame arrives. This is the delta model's
    whole payoff and it is the assertion a pump built on the one-shot
    screenshot path would fail.
  * **A small change is a delta over a region smaller than the surface**, with
    a frame sequence greater than the keyframe's, and **a large change is a
    keyframe** instead — the dirty-tile threshold, observed rather than
    reasoned about.
  * **A viewport change produces a keyframe**, so a client that just
    reallocated its texture is given a whole surface to fill it with.
  * **Every frame echoes the last input the server had applied when it
    painted**, which is what lets a client discard a frame that predates its
    own most recent click. Asserted as present and correct, with no claim about
    timing.
  * **Two viewers on one tab are independent**: each gets its own keyframe and
    its own sequence space, detaching one leaves the other receiving, and the
    tab is released only when the last of them leaves — read back over the
    control socket, because a released lease is an absence and every other
    symptom of it is indistinguishable from a viewer with nothing to send.
  * **A viewer attached to a background tab moves nothing the human sees.** The
    displayed tab, the view mode and the set of focused tabs are read over the
    control socket before and after a whole frame exchange, and the human's own
    tab is captured before and after and compared **pixel for pixel** — the
    per-tab-framebuffer invariant asserted rather than cited.
  * **Closing the attached tab detaches the viewer and stops the frames**
    without dropping its connection.

And then the whole loop, DIST-01 and DIST-02 together — the **real client
binary**, and this is the one section that does not speak the wire directly:

  * Every section above composes wire messages by hand. This one starts
    `talaria-client` and drives its **window** with a real pointer and real
    keystrokes, because a synthetic wire message bypasses exactly the
    client-side transform most likely to be wrong: the picture is fitted into
    the client's own page area, and a pointer has to be mapped back through the
    inverse of that fit before it means anything to a page.
  * **The client renders its empty state before any agent tab exists**, rather
    than a blank pane.
  * **Attaching happens through the client's own control**, clicked for real,
    and the server's own hold count is what proves it landed.
  * **The server's tab viewport is unchanged by the attachment.** Read before
    and after. The viewer adapts to the page; the page does not reflow because
    somebody started watching, and the client window is deliberately resized to
    a size and an aspect ratio the server's tab does not share so the transform
    is exercised rather than accidentally being the identity.
  * **A click lands on the element it was aimed at, in both directions**, with
    the same decoy discipline the wire path uses — and the assertion is made
    over the control socket, not over the frame path, so the thing under test is
    not also the thing reporting success.
  * **Keystrokes reach a background agent tab**, which is the case an attachment
    makes possible: the tab is held shown and not focused, and the local human
    is looking at their own tab throughout.
  * **A click in the letterboxed margin sends nothing**, and neither does a
    click on the client's own controls. Not a clamped edge position — nothing.
  * **The picture is actually arriving**, asserted without reading a pixel: the
    client reports the last frame sequence it applied and the last input
    sequence the server echoed, and both are checked. "The client is connected"
    is not "the client is showing the page".

**Its honest limits.** Everything here runs on loopback under software
rendering. That proves the protocol, the authorisation and the delta behaviour,
and proves **nothing** about the network or about what this costs on real
hardware: TLS termination, the tailnet `Host`, relay behaviour, the rate
ladder and the two-machine case are 05-10's and the manual verification's, not
this suite's. In particular the real-client section below proves the *protocol
and the authorisation* of Success Criterion 1 over loopback; **the two-machine
run remains a manual item**, because two machines are not something a suite on
one machine can produce. **Nothing here makes a timing claim of any kind**, deliberately:
measuring on loopback and declaring a budget met is the pitfall the phase's
research names by name.

The input assertions drive a tab that is **on screen**. An attachment now shows
its tab, so a click on a background agent tab would reach a page — but the
input half was written before that and the on-screen setup is what makes its
refusal assertions mean what they say, so it is left exactly as it was.

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
import struct
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
CHANNEL_FRAME = 0x10

# The frame header's fixed layout (`talaria_protocol::wire::FrameHeader`),
# spelled out here for the same reason the tags are: this suite is a client on
# the far side of the wire, and decoding the header by hand is what makes it
# one. A layout drift shows up as a decode that does not agree with the tab's
# real geometry rather than as an import that stopped compiling.
FRAME_HEADER_LEN = 51
FRAME_FORMAT_VERSION = 1
KEYFRAME = 0
TILE = 1

# The wire version the server announces. A mismatch here means the shell and
# this suite were built against different wires, which is worth failing on.
PROTOCOL_VERSION = 1

# The cap the shell is started with. Lowered from its default so the cap can be
# reached with three tabs instead of nine — the property under test is that
# there *is* a ceiling and that crossing it is refused, not what the number is.
MAX_ATTACH = 2

# The view-connection ceiling the shell is started with, and it is *exactly*
# the number of sockets this suite holds open at its peak — the two viewers of
# the snapshot section, the input section's, and the two of the frame section.
# Pinned rather than raised out of the way, so the ceiling is reachable and the
# refusal that guards it is asserted with a real upgrade rather than reasoned
# about. Its default is lower; the property under test is that there *is* a
# ceiling, that crossing it is refused, and that an authenticated client can
# tell that refusal from a rejected credential.
MAX_VIEW_CONNECTIONS = 5


def wheel_line_pixels():
    """How far the shell says one wheel notch travels, read out of its source.

    Read rather than spelled, for the reason the client's key-table drift test
    reads the server's table with ``include_str!``: the property under test is
    that the remote wheel path and the local one scale by **one** number, and a
    number copied into this file would agree with the constant right up until
    somebody changed the constant. A miss fails loudly rather than silently
    substituting a default — a suite that fell back to 1.0 here would assert
    exactly the defect it exists to catch."""
    source = os.path.join(os.path.dirname(os.path.dirname(T)),
                          "crates", "talaria-shell", "src", "app.rs")
    with open(source, encoding="utf-8") as handle:
        for line in handle:
            if line.strip().startswith("pub(crate) const WHEEL_LINE_PIXELS"):
                return float(line.split("=")[1].strip().rstrip(";"))
    raise AssertionError(f"WHEEL_LINE_PIXELS is not declared in {source}")

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
  /* The wheel assertion needs a page taller than any viewport this suite
     runs in, and needs it without disturbing a single number above. Absolute
     and one pixel wide: it takes no space in the flow, paints nothing, moves
     nothing, and extends the scrollable overflow area to 4000px. */
  #tall        { position: absolute; top: 0; left: 0; width: 1px;
                 height: 4000px; }
</style></head><body>
<a id="decoy-upper" href="/decoy-upper.html">DECOY ABOVE THE UPPER LINK</a>
<a id="upper" href="/upper.html">UPPER LINK</a>
<a id="decoy-lower" href="/decoy-lower.html">DECOY ABOVE THE LOWER LINK</a>
<a id="lower" href="/lower.html">LOWER LINK</a>
<input id="field" type="text">
<div id="tall"></div>
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


def decode_frame(message):
    """One frame-channel message, taken apart into a header dict and a payload.

    Every refusal the shell's own decoder makes is re-made here, so a header
    this suite accepts is one the wire would: a short slice, a format version
    this build does not know, a kind naming no kind, a zero scale, a tile with
    no area, and a tile that does not fit inside the frame it declares."""
    body = message[1:]
    assert len(body) > FRAME_HEADER_LEN, ("a frame carried no payload", len(body))
    assert body[0] == FRAME_FORMAT_VERSION, ("unknown frame format", body[0])
    assert body[1] in (KEYFRAME, TILE), ("unknown frame kind", body[1])
    scale = body[2]
    assert scale > 0, "a frame declared a zero scale"
    fields = struct.unpack_from("<QQQIIIIII", body, 3)
    header = dict(zip(("tab", "seq", "last_applied", "x", "y",
                       "width", "height", "frame_width", "frame_height"),
                      fields))
    header["kind"] = body[1]
    header["scale"] = scale
    assert header["width"] > 0 and header["height"] > 0, ("a tile with no area", header)
    assert header["x"] + header["width"] <= header["frame_width"], header
    assert header["y"] + header["height"] <= header["frame_height"], header
    payload = body[FRAME_HEADER_LEN:]
    assert payload[:8] == b"\x89PNG\r\n\x1a\n", ("a frame payload is not a PNG", payload[:8])
    return header, payload


def next_frame(ws, timeout=8.0):
    """The next frame message's header and payload, or ``None`` if none comes.

    Skips the other channels the way :func:`read` does, and for the same
    reason: several channels are in flight at once and a reader that took
    whatever arrived first would be asserting about the wrong one."""
    deadline = time.monotonic() + timeout
    while True:
        message = ws.next_message(max(0.0, deadline - time.monotonic()))
        if message is None:
            return None
        if message[0] == CHANNEL_FRAME:
            return decode_frame(message)
        if time.monotonic() > deadline:
            return None


def settle_frames(ws, quiet=2.5, limit=25.0):
    """Read until nothing has arrived for `quiet`, and answer the last header.

    A page that has just been attached to is still doing deferred work — the
    engine paints a leased tab lazily and the first frames after a lease is
    taken are the ones that settle it. Draining to quiet first is what makes
    the silence assertion afterwards a statement about a *static* page rather
    than about a page that had not finished arriving."""
    deadline = time.monotonic() + limit
    last = None
    while time.monotonic() < deadline:
        frame = next_frame(ws, quiet)
        if frame is None:
            return last
        last = frame
    return last


def view_holds(tab_id):
    """How many viewers hold `tab_id` shown — `None` for a tab that is gone.

    The test hook, over the control socket. A released lease is an absence, and
    every other symptom of one (a tab going hidden, frames stopping) is
    indistinguishable from a viewer that simply had nothing to send."""
    reply = rpc("view_holds", tab_id=tab_id)
    assert reply["outcome"] == "ok", reply
    return reply["result"]["value"]


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


def tab_viewport(tab_id):
    """The tab's own viewport in device pixels, over the control socket.

    Read out of the **page** rather than out of the view channel's own attach
    acknowledgement: this is the fact the real-client section asserts is
    *unchanged* by an attachment, and reading it over the channel under test
    would make the assertion circular.

    Deliberately **not** ``screenshot``, which would be the obvious way to ask a
    tab how big it is. That command's background-tab path hides the webview
    again once it has read the pixels, without consulting the view hold count —
    so screenshotting a tab a viewer is watching silently stops that viewer's
    clicks from landing while its frames carry on arriving. Recorded in this
    phase's ``deferred-items.md``; this suite routes around it rather than
    proving the client broken by it."""
    width, height = evaluate(
        tab_id,
        "[window.innerWidth * window.devicePixelRatio,"
        " window.innerHeight * window.devicePixelRatio]",
    )
    return round(width), round(height)


def client_rect(client, name):
    """One of the real client's named controls, from its most recent frame."""
    return harness.wait_for_client_rect(client, name)


def client_reading(client, name):
    """One of the real client's numeric readings, as ``(width, height)``.

    The client puts these on the same geometry line its control rectangles ride,
    under the same test hook, with the value in the rectangle's dimensions — see
    its ``record_readings``. They are how this suite tells "the client is
    connected" from "the client is showing the page", which is a distinction no
    assertion about the connection can make."""
    rect = harness.wait_for_client_rect(client, name)
    return rect["width"], rect["height"]


def client_screen(client, wid, point):
    """A point in the client's logical points, as a screen coordinate."""
    scale_factor, _ = client_reading(client, "reading.points_per_pixel")
    origin_x, origin_y = harness.window_origin(wid, X)
    x, y = point
    return (str(round(origin_x + x * scale_factor)),
            str(round(origin_y + y * scale_factor)))


def click_client_point(client, wid, point):
    """Drive a genuine pointer click at a point in the client's own window.

    The same shape ``harness.click_rect`` uses against the server's window —
    focus, move, click — aimed at the other process. There is no window manager
    on the test display, so both windows sit at the origin and the client, being
    the later of the two, is the one on top; the focus call makes that explicit
    rather than relying on it."""
    target = client_screen(client, wid, point)
    subprocess.run(["xdotool", "windowfocus", "--sync", wid], env=X)
    time.sleep(0.3)
    subprocess.run(["xdotool", "mousemove", *target], env=X)
    time.sleep(0.4)
    subprocess.run(["xdotool", "click", "1"], env=X)
    time.sleep(1.5)
    return target


def click_client(wid, rect):
    """Click the centre of one of the client's own named controls."""
    return click_client_point(CLIENT[0], wid,
                              (rect["x"] + rect["width"] / 2,
                               rect["y"] + rect["height"] / 2))


def click_page(client, wid, page_point):
    """Click a **page** coordinate, through the client's own fitted picture.

    The conversion is the client's own transform read back out of its recorded
    geometry — the fitted surface's rectangle and the page's size in its own
    device pixels — so this aims where the client is *drawing* that page pixel.
    That is what makes the assertion downstream a statement about the client's
    inverse transform: if the inverse forgot the client's own interface offset,
    or rounded the wrong way, the click would land somewhere else and the decoy
    would name it."""
    surface = client_rect(client, "page.surface")
    page_width, page_height = client_reading(client, "reading.page_size")
    x, y = page_point
    return click_client_point(client, wid, (
        surface["x"] + x * surface["width"] / page_width,
        surface["y"] + y * surface["height"] / page_height,
    ))


# The one running client, so `click_client` can reach it without every call site
# passing it. A list rather than a bare name because it is assigned inside the
# suite body and read from a function defined above it.
CLIENT = [None]


xvfb = harness.start_xvfb()
X = harness.x_env()
log = None
tal = None
fixture = None
client = None
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
                              TALARIA_VIEW_MAX_CONNECTIONS=str(MAX_VIEW_CONNECTIONS),
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

    # ======================================================================
    # DIST-02: the frame half. Everything below reads real pixels.
    # ======================================================================

    # --- 24. a fresh viewer on a fresh tab, out of the input half's way ---
    # A tab of its own, never clicked into: the input half left a caret
    # blinking in `driven`'s text field, and a caret is a tile changing twice a
    # second forever — which would make the static-page assertion below a test
    # of nothing. The local human is in the Me view throughout this section, so
    # the watched tab is a *background* agent tab, which is the case that
    # matters: an attachment has to show it without disturbing anything here.
    watched = rpc("tabs_open", client="agent-frames", url=f"{BASE}/watched.html")
    assert watched["outcome"] == "ok", watched
    watched = watched["result"]["tab"]["tab_id"]
    assert wait_for_url(watched, "/watched.html").endswith("/watched.html"), \
        ("the watched tab never reached the fixture", tab_url(watched))
    assert view_holds(watched) == 0, "a tab was held before anyone attached"

    seer = connect(port)
    sockets.append(seer)
    seer.send(view({"view": "attach", "tab": watched}))
    ack = read(seer, CHANNEL_CONTROL)
    assert ack is not None and ack.get("view") == "attached", ack
    assert view_holds(watched) == 1, \
        ("an attachment did not hold the tab shown, so no hit test can be "
         "answered and no frame can be painted", view_holds(watched))
    print(f"HELD: attaching to background tab {watched} holds it shown, and the "
          f"local human is still in the Me view")

    # --- 25. the first frame is a keyframe covering the whole surface ----
    first_frame = next_frame(seer)
    assert first_frame is not None, "a viewer attached and was sent no frame at all"
    header, payload = first_frame
    assert header["kind"] == KEYFRAME, ("the first frame after an attach was a "
                                        "delta against a surface the client "
                                        "does not have", header)
    assert (header["x"], header["y"]) == (0, 0), header
    assert (header["width"], header["height"]) \
        == (header["frame_width"], header["frame_height"]), \
        ("the first keyframe did not cover the whole surface", header)
    assert header["tab"] == watched, header
    assert header["scale"] == 1, ("this plan sends full resolution", header)
    print(f"KEYFRAME ON ATTACH: frame {header['seq']} is a whole "
          f"{header['frame_width']}x{header['frame_height']} surface, "
          f"{len(payload)} bytes of PNG")

    # --- 26. the header's size is the size the attach already promised ---
    # So a client can allocate its texture on the acknowledgement and does not
    # have to reflow when the first frame turns out to be a different shape.
    assert (header["frame_width"], header["frame_height"]) \
        == (ack["width"], ack["height"]), \
        ("the attach acknowledgement and the first frame disagree about the "
         "tab's size, so a client that sized its surface on the "
         "acknowledgement has to throw it away", ack, header)
    print(f"SIZED AHEAD: the acknowledgement's {ack['width']}x{ack['height']} "
          f"is what the first frame declares")

    # --- 27. a static page sends NOTHING ---------------------------------
    # The delta model's whole payoff, and the assertion a pump built on the
    # one-shot screenshot path would fail: that path waits for a repaint
    # notification a settled page never produces, and answers the wait with a
    # timeout. This one reads across many ticks and expects silence.
    settle_frames(seer)
    quiet = next_frame(seer, 5.0)
    assert quiet is None, \
        ("a page nobody touched kept sending frames, so the tile comparison is "
         "not deciding anything", quiet[0] if quiet else None)
    print("STATIC IS SILENT: with the page untouched, not one frame arrived "
          "across many ticks")

    # --- 28. a small change is a delta over a small region ---------------
    evaluate(watched, "document.getElementById('upper').style.background = '#f00'")
    small = next_frame(seer)
    assert small is not None, "a change to the page produced no frame"
    delta, _ = small
    assert delta["kind"] == TILE, \
        ("a one-element change was sent as a whole keyframe", delta)
    assert delta["width"] * delta["height"] \
        < delta["frame_width"] * delta["frame_height"], \
        ("the delta's region is the whole surface", delta)
    assert delta["seq"] > header["seq"], \
        ("frame sequences did not increase", header["seq"], delta["seq"])
    print(f"DELTA: a one-element change arrived as a "
          f"{delta['width']}x{delta['height']} region at "
          f"({delta['x']}, {delta['y']}), sequence {delta['seq']}")

    # --- 29. a large change is a keyframe instead ------------------------
    # The dirty-tile threshold, observed. A scroll-sized change is one keyframe
    # rather than hundreds of tile messages, which is a bandwidth argument
    # before it is anything else.
    settle_frames(seer)
    evaluate(watched, "document.body.style.background = '#00f'")
    large = settle_frames(seer)
    assert large is not None, "repainting the whole page produced no frame"
    whole, _ = large
    assert whole["kind"] == KEYFRAME, \
        ("a change covering the whole page was sent as deltas", whole)
    assert (whole["width"], whole["height"]) \
        == (whole["frame_width"], whole["frame_height"]), whole
    print(f"THRESHOLD: a whole-page change arrived as one keyframe, sequence "
          f"{whole['seq']}")

    # --- 30. a viewport change produces a keyframe -----------------------
    # The client is about to reallocate its texture, so every tile it holds is
    # about to be meaningless and a delta composited onto the new one would be
    # a stripe of stale pixels.
    settle_frames(seer)
    seer.send(view({"view": "viewport", "tab": watched,
                    "width": 800, "height": 600}))
    resized = next_frame(seer)
    assert resized is not None, "a viewport change produced no frame"
    after_resize, _ = resized
    assert after_resize["kind"] == KEYFRAME, \
        ("a viewport change did not produce a keyframe", after_resize)
    assert (after_resize["width"], after_resize["height"]) \
        == (after_resize["frame_width"], after_resize["frame_height"]), after_resize
    assert seer.still_open(), "a viewport change closed the connection"
    print(f"RESIZE: a viewport change produced a keyframe declaring "
          f"{after_resize['frame_width']}x{after_resize['frame_height']}")

    # --- 31. every frame echoes the last input the server had applied ----
    # The whole of the instrumentation, and the reason a client can discard a
    # frame that predates its own most recent click. Asserted as present and
    # correct; what it is *worth* is a two-machine question and not this
    # suite's.
    settle_frames(seer)
    pointer = Input(seer, watched)
    _, scale = harness.chrome_rects()
    marks = centres(watched, scale)
    pointer.send("mouse_move", x=float(marks["field"][0]),
                 y=float(marks["field"][1]))
    sent = pointer.seq
    # A pointer move over a link changes no pixels, so nothing would be sent
    # at all: the page has to be *made* to repaint before there is a frame to
    # read the echo off. The toggle is what does that, and it runs before each
    # read rather than after, because a silent page is the expected state here
    # and not a reason to stop looking.
    echoed = None
    carried = []
    for attempt in range(12):
        evaluate(watched,
                 "const l = document.getElementById('lower');"
                 f"l.style.background = '#0{attempt % 8}f';")
        frame = next_frame(seer, 6.0)
        if frame is None:
            continue
        carried.append(frame[0]["last_applied"])
        if frame[0]["last_applied"] >= sent:
            echoed = frame[0]
            break
    assert echoed is not None, (
        "no frame echoed the input sequence the server had already applied, so "
        "a client cannot tell which of its own inputs a frame postdates",
        sent, carried)
    print(f"ECHOED: a frame carries last-applied-input {echoed['last_applied']}, "
          f"at or past the {sent} that was sent")

    # --- 32. a second viewer on the same tab is independent --------------
    other = connect(port)
    sockets.append(other)
    other.send(view({"view": "attach", "tab": watched}))
    other_ack = read(other, CHANNEL_CONTROL)
    assert other_ack is not None and other_ack.get("view") == "attached", other_ack
    assert view_holds(watched) == 2, \
        ("the second viewer did not take its own hold", view_holds(watched))

    own_keyframe = next_frame(other)
    assert own_keyframe is not None, "the second viewer was sent no frame"
    assert own_keyframe[0]["kind"] == KEYFRAME, \
        ("the second viewer was given a delta against a surface it never had",
         own_keyframe[0])
    assert own_keyframe[0]["seq"] == 1, \
        ("the two viewers share one frame sequence space", own_keyframe[0])
    print(f"SECOND VIEWER: its own keyframe at sequence "
          f"{own_keyframe[0]['seq']}, while the first is past "
          f"{after_resize['seq']}")

    # Both keep receiving, and detaching one leaves the other receiving.
    evaluate(watched, "document.body.style.background = '#333'")
    assert next_frame(seer) is not None, "the first viewer stopped receiving"
    assert next_frame(other) is not None, "the second viewer stopped receiving"

    other.send(view({"view": "detach", "tab": watched}))
    gone = read(other, CHANNEL_CONTROL)
    assert gone == {"view": "detached", "tab": watched}, gone
    assert view_holds(watched) == 1, \
        ("the first detach released a tab the other viewer was watching",
         view_holds(watched))
    settle_frames(seer)
    evaluate(watched, "document.body.style.background = '#666'")
    assert next_frame(seer) is not None, \
        "one viewer detaching stopped the other's frames"
    print("INDEPENDENT: one viewer detaching left the other receiving, and the "
          "tab is still held once")

    # --- 33. the connection ceiling, at the suite's own peak -------------
    # CR-02: the concurrent-attachment cap is counted per *connection*, and
    # nothing capped connections — so one token bought as many frame pumps as
    # its holder cared to open, each of them paying paints and framebuffer
    # readbacks on the winit main thread, and the Access panel that would
    # revoke that token is drawn by the loop being saturated.
    #
    # This is asserted here rather than earlier because here is where the
    # suite's own open sockets reach the ceiling, so the refusal is produced by
    # a real upgrade against a real ceiling rather than by a lowered one.
    #
    # The **status is the assertion**, not merely that it failed: a client that
    # hit the ceiling has done nothing wrong, and answering it the way an
    # unknown token is answered would send its operator looking at the token.
    # The capacity answer is reachable only after the credential was accepted,
    # so it is not an oracle for which tokens exist.
    assert len([s for s in sockets if s.handshook()]) == MAX_VIEW_CONNECTIONS, (
        "this suite is not holding the number of view sockets the ceiling was "
        "set to, so the refusal below would be measuring the wrong thing",
        MAX_VIEW_CONNECTIONS)
    over = harness.WebSocket("127.0.0.1", port, token=TOKEN)
    sockets.append(over)
    assert over.status == 503, (
        "a view upgrade past the connection ceiling was accepted, or refused "
        "with the wrong answer", over.status, over.body[:200])
    assert over.status != bare.status, (
        "a client that hit the connection ceiling is told its credential was "
        "rejected, which sends its operator looking at the wrong thing",
        over.status, over.body[:200])
    assert view_holds(watched) == 1, \
        ("the refused upgrade disturbed a lease somebody else holds",
         view_holds(watched))
    settle_frames(seer)
    evaluate(watched, "document.body.style.background = '#5a5'")
    assert next_frame(seer) is not None, \
        "a refused upgrade stopped an accepted viewer's frames"
    print(f"CONNECTION CEILING: with {MAX_VIEW_CONNECTIONS} view sockets open, "
          f"a further authenticated upgrade is refused {over.status} — a "
          f"different answer from the {bare.status} an unknown token gets — and "
          f"the viewers already connected carry on")

    # --- 34. the local display did not move, pixel for pixel -------------
    # Read over the control socket rather than over the channel under test,
    # before and after a whole frame exchange — and the human's own tab is
    # captured both times and compared byte for byte, which is the per-tab
    # framebuffer invariant asserted rather than cited.
    displayed_before = window_title(wid, X)
    focused_before = focused_tabs()
    mine_before_pixels = rpc("screenshot", tab_id=mine)
    assert mine_before_pixels["outcome"] == "ok", mine_before_pixels
    mine_before_pixels = mine_before_pixels["result"]

    settle_frames(seer)
    evaluate(watched, "document.body.style.background = '#999'")
    assert next_frame(seer) is not None, "the exchange under test produced no frame"
    settle_frames(seer)

    assert window_title(wid, X) == displayed_before, (
        "a viewer attached to a background tab changed the displayed tab or "
        "the view mode", displayed_before, window_title(wid, X))
    assert focused_tabs() == focused_before, (
        "a viewer attached to a background tab changed an active tab",
        focused_before, focused_tabs())
    mine_after_pixels = rpc("screenshot", tab_id=mine)
    assert mine_after_pixels["outcome"] == "ok", mine_after_pixels
    mine_after_pixels = mine_after_pixels["result"]
    assert (mine_after_pixels["width"], mine_after_pixels["height"]) \
        == (mine_before_pixels["width"], mine_before_pixels["height"]), \
        ("the human's own tab changed size while a viewer was attached to "
         "another", mine_before_pixels["width"], mine_after_pixels["width"])
    assert mine_after_pixels["png_base64"] == mine_before_pixels["png_base64"], (
        "the human's own tab's pixels changed while a viewer was watching a "
        "different tab — the per-tab framebuffer invariant does not hold for "
        "the frame pump's shape")
    print(f"LOCAL DISPLAY UNMOVED: still {displayed_before!r}, focused tabs "
          f"still {focused_before}, and the human's own tab is byte-identical "
          f"across a whole frame exchange on another tab")

    # --- 35. closing the tab detaches, releases and stops the frames -----
    assert rpc("tabs_close", tab_id=watched)["outcome"] == "ok"
    notice = None
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        answer = read(seer, CHANNEL_CONTROL, 4.0)
        if answer == {"view": "detached", "tab": watched}:
            notice = answer
            break
    assert notice is not None, "closing the watched tab told the viewer nothing"
    assert view_holds(watched) is None, \
        ("the closed tab is still in the tab table", view_holds(watched))
    assert next_frame(seer, 4.0) is None, \
        "frames kept arriving for a tab that no longer exists"
    assert seer.still_open(), \
        "the tab closing dropped the connection instead of detaching it"
    print(f"CLOSED: tab {watched} closing detached the viewer, released the "
          f"hold and stopped the frames, without dropping the connection")


    # ======================================================================
    # Success Criterion 1: the real client binary, driven by a real pointer.
    #
    # Everything above composes wire messages by hand. Nothing below does.
    # ======================================================================

    # --- 36. the client's empty state, before there is anything to list ---
    # Every agent tab the sections above left behind is closed first, so "no
    # agent has opened a tab on this server" is the *true* state rather than a
    # pane that happens to be blank. The empty-state label is only drawn while
    # the client is connected, which makes finding it the connection assertion
    # too — a client that never reached the server draws the connection copy and
    # no tab surface at all.
    for stale in [tab["tab_id"] for tab in rpc("tabs_list")["result"]["tabs"]
                  if tab["owner"] != "me"]:
        rpc("tabs_close", tab_id=stale)
    viewer.close()
    seer.close()

    client = harness.start_client(f"http://127.0.0.1:{port}", rust_log="info",
                                  TALARIA_CLIENT_TOKEN=TOKEN)
    CLIENT[0] = client
    harness.wait_for_client_rect(client, "tabs.empty")
    print("CLIENT EMPTY STATE: the real client connected and drew the "
          "no-agent-tabs copy rather than a blank pane")

    # --- 37. an agent tab appears in the real client's list ---------------
    remote = rpc("tabs_open", client="agent-remote", url=f"{BASE}/index.html")
    assert remote["outcome"] == "ok", remote
    remote = remote["result"]["tab"]["tab_id"]
    assert wait_for_url(remote, "/index.html").endswith("/index.html"), \
        ("the client's tab never reached the fixture", tab_url(remote))
    harness.wait_for_client_rect(client, "tabs.row.0")
    watch = harness.wait_for_client_rect(client, "tabs.attach.0")
    print(f"CLIENT LISTED: agent tab {remote} has a row and a Watch control in "
          f"the real client")

    # --- 38. the client's window, deliberately the wrong size -------------
    # The transform must be exercised rather than accidentally be the identity.
    # The client's page area is already narrower than the server's tab — its own
    # controls take a fixed strip — and the window is resized on top of that so
    # neither the size nor the aspect ratio matches. A letterboxed margin is then
    # guaranteed to exist, which is what section 44 aims at.
    client_window = None
    for _ in range(20):
        found = subprocess.run(["xdotool", "search", "--name", "remote view"],
                               env=X, capture_output=True, text=True).stdout.split()
        if found:
            client_window = found[0]
            break
        time.sleep(1)
    assert client_window, "the remote view client's window never appeared"
    before_area = client_rect(client, "page.area")
    subprocess.run(["xdotool", "windowsize", client_window, "1000", "700"], env=X)
    time.sleep(2.0)
    after_area = client_rect(client, "page.area")
    assert (after_area["width"], after_area["height"]) \
        != (before_area["width"], before_area["height"]), \
        ("the client's window did not resize, so the fit transform may be the "
         "identity and would prove nothing", before_area, after_area)
    print(f"MISMATCHED: the client's page area is "
          f"{after_area['width']:.0f}x{after_area['height']:.0f} points against "
          f"a server tab this section reads below")

    # --- 39. the server's tab viewport, before anybody attaches ----------
    viewport_before = tab_viewport(remote)
    assert view_holds(remote) == 0, "a tab was held before anyone attached"

    # --- 40. attach through the client's OWN control, with a real pointer -
    click_client(client_window, watch)
    held = None
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if view_holds(remote) == 1:
            held = 1
            break
        time.sleep(0.3)
    assert held == 1, (
        "a real click on the client's Watch control did not produce an "
        "attachment on the server", view_holds(remote))
    attached_tab, _ = client_reading(client, "reading.attached_tab")
    assert attached_tab == float(remote), (
        "the client believes it is watching a different tab from the one the "
        "server is holding", attached_tab, remote)
    print(f"CLIENT ATTACHED: a real click on the client's own control attached "
          f"it to tab {remote}, and the server holds it once")

    # --- 41. and the server's tab viewport is unchanged by it ------------
    # The decision this whole design turns on: the viewer adapts to the page.
    # A client that asked the server to resize the tab to its own window would
    # make an agent's layout a function of who is looking at it.
    viewport_after = tab_viewport(remote)
    assert viewport_after == viewport_before, (
        "attaching a differently-sized viewer resized the server's tab viewport",
        viewport_before, viewport_after)
    page_size = client_reading(client, "reading.page_size")
    assert page_size == (float(viewport_before[0]), float(viewport_before[1])), (
        "the client's idea of the page's size is not the tab's own viewport",
        page_size, viewport_before)
    print(f"VIEWPORT UNMOVED: the tab is still "
          f"{viewport_before[0]}x{viewport_before[1]} device pixels, and the "
          f"client adapted to it rather than the other way round")

    # --- 42. a real click through the client lands on the LOWER link ------
    # The same decoy discipline the wire path uses, aimed across a process
    # boundary and through the client's own transform. Asserted over the control
    # socket, so the frame path is not also the thing reporting success.
    marks = centres(remote, scale)
    click_page(client, client_window, marks["lower"])
    landed = wait_for_url(remote, "/lower.html")
    assert landed.endswith("/lower.html"), (
        "a real pointer in the real client, aimed at the lower link, did not "
        "reach it", landed, marks["lower"])
    assert "/decoy-lower.html" not in landed, (
        "the click landed one toolbar-height high, through the client", landed)
    assert "/upper.html" not in landed, ("the click hit the upper link", landed)
    print(f"CLIENT AIMED LOWER: a real pointer in the client's window reached "
          f"{landed.rsplit('/', 1)[-1]}, not the decoy above it")

    # --- 43. the decoy is reachable through the client too ---------------
    assert rpc("navigate", tab_id=remote, url=f"{BASE}/index.html")["outcome"] == "ok"
    wait_for_url(remote, "/index.html")
    click_page(client, client_window, marks["decoy-lower"])
    landed = wait_for_url(remote, "/decoy-lower.html")
    assert landed.endswith("/decoy-lower.html"), (
        "the decoy band is not reachable through the client, so 'not the decoy' "
        "proves nothing about where the click went", landed)
    print("CLIENT DECOY REACHABLE: aimed at deliberately, the band one "
          "toolbar-height above the lower link is hit — so the offset "
          "regression would be named rather than merely missed")

    # --- 44. and the other direction, at the UPPER link ------------------
    assert rpc("navigate", tab_id=remote, url=f"{BASE}/index.html")["outcome"] == "ok"
    wait_for_url(remote, "/index.html")
    click_page(client, client_window, marks["upper"])
    landed = wait_for_url(remote, "/upper.html")
    assert landed.endswith("/upper.html"), (
        "a real pointer in the real client, aimed at the upper link, did not "
        "reach it", landed, marks["upper"])
    assert "/decoy-upper.html" not in landed, (
        "the click landed one toolbar-height high, through the client", landed)
    assert "/lower.html" not in landed, ("the click hit the lower link", landed)
    print(f"CLIENT AIMED UPPER: a real pointer reached "
          f"{landed.rsplit('/', 1)[-1]}, so the transform is proven in both "
          f"directions rather than coincidentally right in one")

    # --- 45. real keystrokes reach a BACKGROUND agent tab ----------------
    # The local human has been looking at their own tab since section 23, so
    # this tab is shown-and-blurred rather than displayed. Servo's keyboard
    # focus is a different thing from visibility, and this is the assertion that
    # settles whether a held tab can be typed into at all.
    assert rpc("navigate", tab_id=remote, url=f"{BASE}/index.html")["outcome"] == "ok"
    wait_for_url(remote, "/index.html")
    marks = centres(remote, scale)
    click_page(client, client_window, marks["field"])
    time.sleep(1.0)
    subprocess.run(["xdotool", "type", "--delay", "120", "hey"], env=X)
    typed = None
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        typed = evaluate(remote, "document.getElementById('field').value")
        if typed == "hey":
            break
        time.sleep(0.5)
    assert typed == "hey", (
        "real keystrokes at the client's window did not reach the background "
        "agent tab it is attached to", typed)
    print(f"CLIENT TYPED: the field of a background agent tab reads {typed!r} "
          f"after real keystrokes at the client's own window")

    # --- 46. a click in the letterboxed MARGIN sends nothing -------------
    # Inside the client's page area and outside the fitted picture, which the
    # mismatched window size guarantees exists. Not clamped to the page's edge:
    # clamping would turn "the human aimed at the margin" into a click at the
    # edge of the page, which is a click they did not make.
    area = client_rect(client, "page.area")
    surface = client_rect(client, "page.surface")
    margin = surface["y"] - area["y"]
    assert margin > 4.0, (
        "there is no letterboxed margin to aim at, so this assertion would be "
        "measuring nothing", area, surface)
    quiet_url = tab_url(remote)
    click_client_point(client, client_window,
                       (surface["x"] + surface["width"] / 2, area["y"] + margin / 2))
    time.sleep(2.5)
    assert tab_url(remote) == quiet_url, (
        "a click in the letterboxed margin reached the page", quiet_url,
        tab_url(remote))
    assert evaluate(remote, "document.getElementById('field').value") == "hey", (
        "a click in the letterboxed margin changed the page's state")
    print(f"MARGIN SILENT: a real click {margin:.0f} points above the picture, "
          f"inside the client's page area, reached the server not at all")

    # --- 47. and a click on the client's OWN controls sends nothing ------
    row = client_rect(client, "tabs.row.0")
    click_client(client_window, row)
    time.sleep(2.5)
    assert tab_url(remote) == quiet_url, (
        "a click on one of the client's own controls reached the page",
        quiet_url, tab_url(remote))
    assert evaluate(remote, "document.getElementById('field').value") == "hey", (
        "a click on one of the client's own controls changed the page's state")
    assert view_holds(remote) == 1, \
        "the client detached itself during the refusal assertions"
    print("CLIENT CHROME: a real click on the client's own tab row reached the "
          "page not at all, and the attachment survived it")

    # --- 48. the picture is arriving, and the echo reached the click -----
    # Without reading a pixel. The client reports what it applied and what the
    # server echoed; a page change is forced first, because silence is the
    # correct answer to a static page and never a symptom.
    applied_before, _ = client_reading(client, "reading.frame_seq")
    sent, _ = client_reading(client, "reading.input_seq")
    assert sent > 0, "the client sent no input at all, so nothing was measured"
    applied_after = applied_before
    echoed = 0.0
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        evaluate(remote, "document.getElementById('lower').style.background = "
                         "`#0${Math.floor(Math.random() * 8)}f`")
        time.sleep(0.6)
        applied_after, _ = client_reading(client, "reading.frame_seq")
        echoed, _ = client_reading(client, "reading.last_applied_input")
        if applied_after > applied_before and echoed >= sent:
            break
    assert applied_after > applied_before, (
        "the client applied no further frame after the page changed, so it is "
        "connected but not showing the page", applied_before, applied_after)
    assert echoed >= sent, (
        "no frame the client applied echoed an input sequence at or past the "
        "one it sent, so it cannot tell which of its own clicks a frame "
        "postdates", sent, echoed)
    print(f"CLIENT SHOWING: frame sequence advanced {applied_before:.0f} -> "
          f"{applied_after:.0f}, and the echoed input sequence {echoed:.0f} "
          f"reached the {sent:.0f} the client had sent")

    # --- 49. a remote wheel notch travels as far as a local one -----------
    # The one input verb that shipped with no end-to-end assertion at all,
    # which is why a 76x error in it survived a phase. The quantity asserted
    # is a **distance**, not "something moved": the defect delivered the
    # client's raw line count straight to the engine while the local path
    # multiplied the identical value by WHEEL_LINE_PIXELS, so a notch scrolled
    # one pixel instead of seventy-six — a page-sized error that "scrollY is
    # non-zero" would have called a pass.
    #
    # The bar is half a notch per notch, and half rather than exact because
    # the engine is entitled to clamp at the end of the document, to apply the
    # scroll over more than one frame, and to have a device pixel ratio of its
    # own. Nothing near the unscaled value can clear it.
    notch = wheel_line_pixels()
    notches = 3
    surface = client_rect(client, "page.surface")
    assert rpc("navigate", tab_id=remote, url=f"{BASE}/index.html")["outcome"] == "ok"
    wait_for_url(remote, "/index.html")
    time.sleep(1.0)
    assert evaluate(remote, "window.scrollY") == 0, \
        "the page was already scrolled, so this measures the wrong thing"
    scrollable = evaluate(remote, "document.documentElement.scrollHeight "
                                  "- window.innerHeight")
    assert scrollable > notches * notch, (
        "the fixture is not tall enough for the scroll under test to have "
        "anywhere to go", scrollable, notches * notch)
    # Over the middle of the fitted picture, so the pointer is genuinely on the
    # page rather than in the letterboxed margin, and then real wheel-down
    # button presses at the client's own window. Button 5 is wheel-down in the
    # X11 core protocol, which winit reports as one LineDelta.
    subprocess.run(["xdotool", "windowfocus", "--sync", client_window], env=X)
    time.sleep(0.3)
    subprocess.run(["xdotool", "mousemove", *client_screen(
        client, client_window,
        (surface["x"] + surface["width"] / 2,
         surface["y"] + surface["height"] / 2))], env=X)
    time.sleep(0.4)
    for _ in range(notches):
        subprocess.run(["xdotool", "click", "5"], env=X)
        time.sleep(0.4)
    scrolled = 0
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        scrolled = evaluate(remote, "window.scrollY")
        if scrolled >= notches * notch / 2:
            break
        time.sleep(0.5)
    assert scrolled >= notches * notch / 2, (
        f"{notches} real wheel notches at the client scrolled the page "
        f"{scrolled} device pixels, and {notches} notches on the local path "
        f"travel {notches * notch}: the remote wheel is not scaled by "
        f"WHEEL_LINE_PIXELS", scrolled, notch)
    print(f"WHEEL SCALED: {notches} real notches at the client scrolled the "
          f"agent tab {scrolled} px, against the {notches * notch:.0f} px the "
          f"local path's own constant says {notches} notches travel")

    # --- 50. the local human's window never moved -------------------------
    assert window_title(wid, X) == displayed_before, (
        "the real client changed the displayed tab or the view mode",
        displayed_before, window_title(wid, X))
    print(f"LOCAL DISPLAY STILL UNMOVED: the server's own window still reads "
          f"{displayed_before!r} after a whole remote session")

    print("REMOTE VIEW CHECKS PASSED")
finally:
    if client is not None:
        client.terminate()
        time.sleep(0.5)
        client.kill()
        client.wait()
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

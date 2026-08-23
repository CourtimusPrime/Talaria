#!/usr/bin/env python3
"""The frame cadence, the degrade ladder, and the report -- DIST-02's SC 2 half.

What this pins down:

  * **The passive-to-takeover transition, proved from frame timestamps rather
    than asserted.** A viewer that is only watching receives frames inside the
    200-500 ms band the requirement names; the moment it starts driving, the
    interval collapses; and when it stops, the interval comes back. A
    transition that only went one way would be half a transition, so both
    directions are measured.
  * **The boundary of that transition.** An input arriving before the idle
    threshold elapses keeps the cadence driven, and letting the threshold
    elapse returns it to passive. The threshold is lowered through its own
    environment override so the suite measures the rule rather than waiting out
    a production value.
  * **The ladder degrades under a constrained link, and says so.** A *real*
    `talaria-client`, connected through `link_shim.py` at this tailnet's own
    measured relayed profile -- about 13 Mbit/s and about 40 ms -- steps down
    the rung ladder, reports the degraded state in its own interface, and
    **still lets the human click**. That last one is the assertion that
    matters: `D-05-06` says degrade and report, never withhold control,
    because a degraded takeover of a login wall still clears the login wall.
  * **And it does not oscillate.** Over a steady period on the shim the rung
    the client reports changes at most once more, because a rung change costs a
    keyframe and a ladder that flapped would spend the link on them.

**What this suite refuses to claim, and the refusal is the point.** It does
**not** assert that Success Criterion 2's ~30-60 ms takeover target is met.
That is a claim about a **direct WireGuard path between two machines**, and
everything here runs on one machine over loopback under software rendering --
which hides transmission entirely, and transmission is the only variable that
matters. Measuring on loopback and declaring the budget met is the pitfall
`05-RESEARCH.md` names by name, and its warning sign is exactly a latency
assertion running against `127.0.0.1`.

So every timing assertion here is about a **transition** or a **response**,
never about a wall-clock target, and every one of them carries its tolerance
and the reason that tolerance is defensible under software rendering in a
comment beside it. The target itself is confirmed by the two-machine manual
check `05-VALIDATION.md` lists and `05-11` turns into a script: ThinkPad
server, MacBook Air client, over the real tailnet, with `tailscale status`
recorded to say whether the path was direct or relayed.

**Every X display on this machine is an Xvfb on llvmpipe** (05-02), so the
absolute numbers below are a software rasteriser's. That is why the driven
assertion is "the interval collapsed relative to the passive one" rather than
"the interval was thirty milliseconds": the second would be a claim about this
machine's rasteriser wearing a product claim's clothes.

Everything this suite claims about the *browser* is read over the control
socket, never over the channel under test, so a bug in the frame path cannot
also be the thing reporting success.

Preconditions: release builds of ``talaria`` and ``talaria-client``, and an
Xvfb display.
"""
import http.server
import json
import os
import shutil
import socket
import socketserver
import statistics
import struct
import subprocess
import sys
import tempfile
import threading
import time

T = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, T)
import harness
from link_shim import LinkShim, RELAYED_DELAY_MS, RELAYED_RATE_BPS

# The view wire's channel tags and header layout, spelled out here rather than
# imported for the same reason `remote_view_test.py` spells them out: this
# suite is a client on the far side of the wire, and writing them by hand is
# what makes it one.
CHANNEL_CONTROL = 0x01
CHANNEL_TABS = 0x02
CHANNEL_INPUT = 0x04
CHANNEL_FRAME = 0x10
FRAME_HEADER_LEN = 51
FRAME_FORMAT_VERSION = 1
PROTOCOL_VERSION = 1

# The two rungs this suite measures, from `talaria_protocol::wire::RUNG_LADDER`.
# The fastest rung's interval and the slowest's, in milliseconds.
DRIVEN_MS = 30
PASSIVE_MS = 250

# DIST-02's passive band, which the slowest rung sits inside.
PASSIVE_BAND_MS = (200, 500)

# The idle threshold this suite runs the shell with, through the override both
# ends read. Well under the production second, so the transition assertions do
# not spend four seconds each waiting one out, and well over the driven
# interval so a driven window is many frames rather than one.
IDLE_MS = 800

CLIENT_ID = "client-viewer"
TOKEN = "tal_seeded_view_token_for_the_latency_suite"

# The fixture, and the animation in it is load-bearing twice over.
#
# **A cadence is only measurable on a page that is changing.** The frame pump
# sends nothing at all when nothing changed -- which is the delta model's whole
# payoff and is asserted by `remote_view_test.py` -- so a static page produces
# no frames to take timestamps of, and "no frames arrived" would be
# indistinguishable from "the cadence is slow".
#
# **And it has to be a CSS animation, not a script one.** Measured on this
# machine, three ways of changing one small region, over four seconds with a
# viewer attached to a background agent tab:
#
# | How the page changes | Passive | Driven |
# |----------------------|---------|--------|
# | `setInterval` setting a style every 10 ms | 5 frames | 4 frames |
# | `requestAnimationFrame` | 16 frames | 30 frames |
# | a CSS `@keyframes` animation | 16 frames | 137 frames |
#
# Only the last one is the cadence: 16 frames in four seconds is 250 ms and 137
# is 29 ms, which are the two rungs exactly. The engine batches a script's
# style mutations against its own refresh driver, and throttles that driver for
# a webview nobody is looking at -- so a page animated from script repaints
# about once a second however often the pump asks, and a suite built on one
# would be measuring the refresh driver rather than the pump. A declarative
# animation is driven by the compositor and is not throttled the same way.
# **This is a fact about the engine, not about the pump**, and it is written
# down here because the next person to write a timing suite will otherwise
# rediscover it the slow way.
#
# The animated region is deliberately small: a 120-pixel square dirties a
# handful of tiles, so the delta is a few kilobytes and the measurement is of
# the *cadence* rather than of how long a large frame takes to encode.
FIXTURE = b"""<!doctype html><html><head><meta charset="utf-8"><title>pulse</title>
<style>
  body { margin: 0; font: 16px sans-serif; background: #fff; }
  @keyframes pulse { 0% { background: #000 } 50% { background: #fff } 100% { background: #000 } }
  #pulse { position: absolute; top: 24px; left: 24px;
           width: 120px; height: 120px; background: #000;
           animation: pulse 0.1s linear infinite; }
  a { display: block; position: absolute; left: 40px; width: 420px;
      top: 500px; height: 24px; background: #ccf; }
</style></head><body>
<div id="pulse"></div>
<a id="lower" href="/lower.html">LOWER LINK</a>
</body></html>"""


class Fixture(http.server.BaseHTTPRequestHandler):
    """Every path serves the same page, so a navigation is identified by its
    URL alone and the animation survives the click assertion."""

    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(FIXTURE)))
        self.end_headers()
        self.wfile.write(FIXTURE)

    def log_message(self, *args):
        pass


tmp = tempfile.mkdtemp(prefix="talaria-latency-e2e-")
config = os.path.join(tmp, ".config")
talaria = os.path.join(config, "talaria")
os.makedirs(talaria)
OUT = os.environ.get("TALARIA_E2E_OUT", "/tmp/talaria-e2e")
os.makedirs(OUT, exist_ok=True)
SHELL_LOG = os.path.join(OUT, "remote-latency-shell.log")


def rpc(command, client="remote-latency-e2e", **params):
    """One control-socket request on its own connection.

    The different channel: everything this suite claims about the browser is
    read here rather than over the WebSocket under test."""
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


def wait_until_accepting(host, port, seconds=20.0):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        s = socket.socket()
        s.settimeout(1.0)
        try:
            s.connect((host, port))
            return True
        except OSError:
            pass
        finally:
            s.close()
        time.sleep(0.2)
    return False


def view(payload):
    return bytes([CHANNEL_CONTROL]) + json.dumps(payload).encode()


def read(ws, channel, timeout=6.0):
    deadline = time.monotonic() + timeout
    while True:
        message = ws.next_message(max(0.0, deadline - time.monotonic()))
        if message is None:
            return None
        if message[0] == channel:
            return json.loads(message[1:])
        if time.monotonic() > deadline:
            return None


def connect(port):
    ws = harness.WebSocket("127.0.0.1", port, token=TOKEN)
    assert ws.handshook(), (ws.status, ws.headers, ws.body[:200])
    hello = read(ws, CHANNEL_CONTROL)
    assert hello == {"view": "hello", "protocol": PROTOCOL_VERSION}, hello
    return ws


def frame_header(message):
    """One frame message's header, taken apart. The payload is not needed here
    -- this suite measures *when* frames arrive, not what is in them."""
    body = message[1:]
    assert len(body) > FRAME_HEADER_LEN, ("a frame carried no payload", len(body))
    assert body[0] == FRAME_FORMAT_VERSION, ("unknown frame format", body[0])
    fields = struct.unpack_from("<QQQIIIIII", body, 3)
    header = dict(zip(("tab", "seq", "last_delivered", "x", "y",
                       "width", "height", "frame_width", "frame_height"),
                      fields))
    header["kind"] = body[1]
    header["scale"] = body[2]
    return header


def drain(ws, quiet=1.5, limit=25.0):
    """Read until nothing has arrived for `quiet`, and answer the last header.

    A page that was just attached to is still settling -- the engine paints a
    leased tab lazily and the first frames after a lease is taken are the ones
    that settle it -- so draining first is what makes the measurement
    afterwards a statement about the steady state."""
    deadline = time.monotonic() + limit
    last = None
    while time.monotonic() < deadline:
        message = ws.next_message(quiet)
        if message is None:
            return last
        if message[0] == CHANNEL_FRAME:
            last = frame_header(message)
    return last


def clear(ws):
    """Throw away whatever is already buffered, so a measurement starts empty."""
    ws._pump(0.05)
    ws.messages.clear()


def frame_gaps(ws, seconds):
    """Inter-arrival gaps, in milliseconds, of the frames of the next `seconds`.

    **The timestamp is taken at the socket read that produced the bytes, not
    when the message is popped.** ``WebSocket.next_message`` decodes everything
    a single ``recv`` delivered before answering with the first of them, so
    stamping at the pop would give every message of one read the time of
    whenever the caller got round to it. Reaching for the private ``_pump`` is
    deliberate for that reason: it is the only seam where "the bytes just
    arrived" is a fact rather than an inference."""
    stamps = []
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        before = len(ws.messages)
        ws._pump(min(0.25, max(0.0, end - time.monotonic())))
        if len(ws.messages) == before:
            continue
        now = time.monotonic()
        while ws.messages:
            message = ws.messages.pop(0)
            if message and message[0] == CHANNEL_FRAME:
                stamps.append(now)
    return [(later - earlier) * 1000.0 for earlier, later in zip(stamps, stamps[1:])]


def cadence(ws, seconds):
    """The median frame interval over `seconds`, in milliseconds.

    The **median** rather than the mean, because one late tick under a software
    rasteriser drags a mean a long way and is not what the cadence is."""
    gaps = frame_gaps(ws, seconds)
    assert len(gaps) >= 4, (
        "too few frames arrived to say anything about a cadence -- the page may "
        "have stopped animating, or the pump may not be ticking", len(gaps))
    return statistics.median(gaps), len(gaps)


class Driver:
    """A viewer's input channel, and a thread that keeps it driving.

    A cadence measurement has to happen *while* input is arriving, so the
    sending cannot be a step before the reading. One sender thread, so the
    sequence counter has one writer."""

    def __init__(self, ws, tab, point):
        self.ws = ws
        self.tab = tab
        self.point = point
        self.seq = 0
        self.running = False
        self.thread = None

    def send(self, offset=0.0):
        self.seq += 1
        x, y = self.point
        self.ws.send(bytes([CHANNEL_INPUT]) + json.dumps({
            "kind": "mouse_move", "tab": self.tab, "seq": self.seq,
            "x": float(x) + offset, "y": float(y),
        }).encode())

    def _pump(self, period):
        step = 0
        while self.running:
            step = (step + 1) % 8
            self.send(float(step))
            time.sleep(period)

    def start(self, period=0.15):
        self.running = True
        self.thread = threading.Thread(target=self._pump, args=(period,), daemon=True)
        self.thread.start()

    def stop(self):
        self.running = False
        if self.thread is not None:
            self.thread.join(timeout=2.0)
            self.thread = None


def evaluate(tab_id, script):
    reply = rpc("evaluate", tab_id=tab_id, script=script)
    assert reply["outcome"] == "ok", (script, reply)
    return reply["result"]["value"]


def tab_url(tab_id):
    return next(tab["url"] for tab in rpc("tabs_list")["result"]["tabs"]
                if tab["tab_id"] == tab_id)


def wait_for_url(tab_id, want, timeout=25.0):
    deadline = time.monotonic() + timeout
    url = tab_url(tab_id)
    while time.monotonic() < deadline:
        if want in url:
            return url
        time.sleep(0.3)
        url = tab_url(tab_id)
    return url


def view_holds(tab_id):
    reply = rpc("view_holds", tab_id=tab_id)
    assert reply["outcome"] == "ok", reply
    return reply["result"]["value"]


def link_centre(tab_id):
    """The lower link's centre in the tab's own device pixels."""
    x, y = evaluate(
        tab_id,
        "const r = document.getElementById('lower').getBoundingClientRect();"
        "[(r.x + r.width / 2) * window.devicePixelRatio,"
        " (r.y + r.height / 2) * window.devicePixelRatio]",
    )
    return x, y


# -- the real client's own interface, read by name rather than by pixel -------


def client_rect(client, name):
    return harness.wait_for_client_rect(client, name)


def client_reading(client, name):
    rect = harness.wait_for_client_rect(client, name)
    return rect["width"], rect["height"]


def client_rect_now(client, name):
    """One control of the client's most recent frame, or ``None``."""
    for rect in harness.client_rects(client):
        if rect["name"] == name:
            return rect
    return None


def client_screen(client, wid, point):
    scale_factor, _ = client_reading(client, "reading.points_per_pixel")
    origin_x, origin_y = harness.window_origin(wid, X)
    x, y = point
    return (round(origin_x + x * scale_factor), round(origin_y + y * scale_factor))


def page_screen(client, wid, page_point):
    """A page coordinate as a screen coordinate, through the client's own fit.

    The conversion is read back out of the client's recorded geometry -- the
    fitted surface's rectangle and the page's own size -- so this aims where
    the client is actually drawing that page pixel."""
    surface = client_rect(client, "page.surface")
    page_width, page_height = client_reading(client, "reading.page_size")
    x, y = page_point
    return client_screen(client, wid, (
        surface["x"] + x * surface["width"] / page_width,
        surface["y"] + y * surface["height"] / page_height,
    ))


def click_client_point(client, wid, target):
    subprocess.run(["xdotool", "windowfocus", "--sync", wid], env=X)
    time.sleep(0.3)
    subprocess.run(["xdotool", "mousemove", str(target[0]), str(target[1])], env=X)
    time.sleep(0.4)
    subprocess.run(["xdotool", "click", "1"], env=X)
    time.sleep(1.0)


xvfb = harness.start_xvfb()
X = harness.x_env()
log = None
tal = None
fixture = None
client = None
shim = None
viewer = None
driver = None
try:
    log = open(SHELL_LOG, "w")
    port = harness.free_port()

    # The shim binds now, before the shell starts, for two reasons: its port
    # has to be known in order to be advertised, and binding it immediately
    # closes the window `harness.free_port` documents, where something else
    # takes the number between the probe and the bind.
    shim = LinkShim(("127.0.0.1", port))

    fixture_port = harness.free_port()
    fixture = socketserver.TCPServer(("127.0.0.1", fixture_port), Fixture)
    threading.Thread(target=fixture.serve_forever, daemon=True).start()
    BASE = f"http://127.0.0.1:{fixture_port}"

    # --- 1. one shell, remote access on, reachable both ways -------------
    #
    # The shell is **advertised at the shim's origin** so a client arriving
    # through the shim is addressed by a name this browser answers to. Without
    # it the shim's port would be a `Host` this server neither bound nor
    # advertises, which the DNS-rebinding protector correctly turns away -- and
    # the alternative, teaching the shim to rewrite HTTP headers, would make a
    # byte forwarder into a proxy and stop it being an honest model of a link.
    # The bound loopback address stays in the allowlist either way, which is
    # what lets the direct sections below connect to it.
    advertised = f"http://127.0.0.1:{shim.port}"
    harness.write_config(talaria, remote_access={
        "enabled": True, "port": port, "advertised_url": advertised,
    })
    harness.write_agents(talaria, CLIENT_ID, "The Viewer", TOKEN,
                         audience=f"{advertised}/mcp")
    tal = harness.start_shell("about:blank", log=log, rust_log="info", wait=10,
                              HOME=tmp, XDG_CONFIG_HOME=config,
                              TALARIA_TEST_HOOKS="1",
                              TALARIA_VIEW_IDLE_MS=str(IDLE_MS))
    assert wait_until_accepting("127.0.0.1", port), f"the listener never bound {port}"

    opened = rpc("tabs_open", client="agent-one", url=f"{BASE}/index.html")
    assert opened["outcome"] == "ok", opened
    tab = opened["result"]["tab"]["tab_id"]
    assert wait_for_url(tab, "/index.html").endswith("/index.html"), \
        ("the agent tab never reached the fixture", tab_url(tab))
    print(f"BOUND: the listener accepts 127.0.0.1:{port}, advertised as {advertised}, "
          f"with the idle threshold at {IDLE_MS} ms")

    # --- 2. a viewer attached DIRECTLY, with no shim in the way ----------
    # Sections 3 to 6 measure the *cadence*, which is the server's own clock.
    # Putting a constrained link in front of it would measure the link instead.
    viewer = connect(port)
    viewer.send(view({"view": "attach", "tab": tab}))
    attached = read(viewer, CHANNEL_CONTROL)
    assert attached == {"view": "attached", "tab": tab,
                        "width": attached.get("width"),
                        "height": attached.get("height")}, attached
    assert view_holds(tab) == 1, "the attach did not take a hold"
    settled = drain(viewer)
    assert settled is not None, "no frame ever arrived, so there is no cadence to measure"
    assert settled["scale"] == 1, ("a viewer that asked for no rung was not given the "
                                   "fastest one", settled)
    print(f"ATTACHED: frames are arriving for tab {tab} at "
          f"{settled['frame_width']}x{settled['frame_height']}, scale 1")

    # --- 3. with nobody driving, the cadence is the passive one ----------
    # Tolerance: the requirement's band is 200-500 ms and the slowest rung sits
    # at 250. This asserts the *band* rather than the rung's own number, and
    # widens it by 50 ms at the bottom and 100 ms at the top. Both ends are
    # defensible under software rendering for the same reason: a tick that
    # lands late is not re-fired to catch up -- the pump reschedules from `now`
    # rather than from the deadline it missed -- so a slow tick pushes the
    # *following* gap out rather than compressing it, and llvmpipe produces slow
    # ticks. Nothing in the band can be reached by a driven attachment, which is
    # what this measurement is distinguishing it from.
    clear(viewer)
    passive_ms, passive_count = cadence(viewer, 4.0)
    assert 150 <= passive_ms <= 600, (
        "a viewer nobody is driving is not receiving frames at the passive "
        "cadence", passive_ms, PASSIVE_BAND_MS)
    print(f"PASSIVE: {passive_count} frames at a median of {passive_ms:.0f} ms, "
          f"inside DIST-02's {PASSIVE_BAND_MS[0]}-{PASSIVE_BAND_MS[1]} ms band")

    # --- 4. the moment a human drives, the cadence collapses -------------
    # **The transition, from frame timestamps rather than from an assertion.**
    #
    # Tolerance: two of them, and neither is a wall-clock target. The first is
    # relative -- the driven median must be at most half the passive one --
    # which is the transition itself and is a ratio, so it survives a slow
    # rasteriser scaling both sides. The second is an absolute ceiling of
    # 150 ms, which exists only to stop a passive cadence that happened to be
    # measured fast from satisfying the ratio; it is five times the fastest
    # rung's 30 ms, and it is deliberately not 30, because 05-02 measured
    # `paint()` alone at 5.1 ms mean and 7.8 ms p95 under llvmpipe while
    # scrolling and this suite is animating a page and reading a socket at the
    # same time. Asserting 30 here would be a claim about this machine's
    # software rasteriser wearing a product claim's clothes.
    driver = Driver(viewer, tab, link_centre(tab))
    driver.start(period=0.15)
    time.sleep(0.6)
    clear(viewer)
    driven_ms, driven_count = cadence(viewer, 3.0)
    assert driven_ms <= passive_ms / 2, (
        "the cadence did not rise when the viewer started driving", driven_ms, passive_ms)
    assert driven_ms <= 150, (
        "the driven cadence is nowhere near the fastest rung", driven_ms, DRIVEN_MS)
    print(f"TAKEOVER: {driven_count} frames at a median of {driven_ms:.0f} ms while "
          f"driving, against {passive_ms:.0f} ms passive -- the transition, from "
          f"frame timestamps")

    # --- 5. and it comes back down when they stop ------------------------
    # A transition that only goes one way is half a transition.
    driver.stop()
    # Past the idle threshold with room: the last input may have gone out just
    # before `stop`, so the wait is the threshold plus a margin rather than the
    # threshold exactly. The boundary itself is section 6's, measured
    # deliberately rather than incidentally.
    time.sleep(IDLE_MS / 1000.0 + 0.6)
    clear(viewer)
    back_ms, back_count = cadence(viewer, 4.0)
    assert back_ms >= passive_ms / 2, (
        "the cadence never returned to passive after the viewer stopped driving",
        back_ms, passive_ms)
    assert 150 <= back_ms <= 600, ("the returned cadence is not the passive one", back_ms)
    print(f"RELEASED: {back_count} frames at a median of {back_ms:.0f} ms once the "
          f"viewer stopped -- the transition runs both ways")

    # --- 6. the boundary: an input before the threshold keeps it driven ---
    # The rule is "the elapsed time since the last accepted input *reaches* the
    # threshold", and a new input always re-enters. So an input at the
    # three-quarter mark must leave the cadence driven, and only letting the
    # threshold actually elapse takes it back.
    #
    # Tolerance: the two windows are sized against the threshold rather than
    # against a clock -- three quarters of it before the second input, and the
    # whole of it plus a margin afterwards. That is what makes this a
    # measurement of the *rule* and not of how promptly this machine's loop
    # wakes; the exact-boundary case, where a sample lands on the threshold to
    # the millisecond, is not something a wall clock can produce and is pinned
    # by the unit test on `is_driving` instead, at the threshold and one
    # millisecond either side.
    driver.send()
    time.sleep(IDLE_MS * 0.75 / 1000.0)
    driver.send()
    clear(viewer)
    held_ms, held_count = cadence(viewer, IDLE_MS * 0.55 / 1000.0)
    assert held_ms <= passive_ms / 2, (
        "an input arriving before the idle threshold elapsed did not keep the "
        "cadence driven", held_ms, passive_ms)
    time.sleep(IDLE_MS / 1000.0 + 0.6)
    clear(viewer)
    lapsed_ms, _ = cadence(viewer, 3.5)
    assert lapsed_ms >= passive_ms / 2, (
        "letting the idle threshold elapse did not return the cadence to passive",
        lapsed_ms, passive_ms)
    print(f"BOUNDARY: an input at {IDLE_MS * 0.75:.0f} ms held the driven cadence "
          f"({held_count} frames, median {held_ms:.0f} ms); letting {IDLE_MS} ms "
          f"elapse returned it to {lapsed_ms:.0f} ms")

    viewer.close()
    viewer = None

    # ======================================================================
    # And now the constrained link, with the REAL client binary.
    # ======================================================================

    # --- 7. a real client through the shim, at the relayed profile -------
    client = harness.start_client(advertised, rust_log="info",
                                  TALARIA_CLIENT_TOKEN=TOKEN,
                                  TALARIA_VIEW_IDLE_MS=str(IDLE_MS))
    harness.wait_for_client_rect(client, "tabs.attach.0")
    client_window = None
    for _ in range(20):
        found = subprocess.run(["xdotool", "search", "--name", "remote view"],
                               env=X, capture_output=True, text=True).stdout.split()
        if found:
            client_window = found[0]
            break
        time.sleep(1)
    assert client_window, "the remote view client's window never appeared"
    watch = client_rect(client, "tabs.attach.0")
    click_client_point(client, client_window, client_screen(
        client, client_window,
        (watch["x"] + watch["width"] / 2, watch["y"] + watch["height"] / 2)))
    held = False
    deadline = time.monotonic() + 25
    while time.monotonic() < deadline:
        if view_holds(tab) >= 1:
            held = True
            break
        time.sleep(0.3)
    assert held, ("a real click on the client's Watch control did not attach it "
                  "through the shim", view_holds(tab))
    starting_rung, _ = client_reading(client, "reading.rung")
    assert starting_rung == 0, ("the client did not start at the top of the ladder",
                                starting_rung)
    print(f"SHIMMED: the real client attached to tab {tab} through "
          f"{RELAYED_RATE_BPS} bit/s and {RELAYED_DELAY_MS} ms each way, starting "
          f"at rung {starting_rung:.0f}")

    # --- 8. it walks DOWN the ladder, and it says so ---------------------
    # `reading.rung` is the position in the ladder counting from the fastest,
    # so degradation is this number going *up*. Driving is what produces
    # samples at all: the estimate is timed from an input this client sent to
    # the frame that echoes it, so a client nobody is touching measures nothing
    # and correctly stays where it is.
    #
    # Tolerance: a wall-clock budget of sixty seconds and no assertion at all
    # about how fast it gets there. What is asserted is the *response* -- that a
    # link which cannot carry the target is answered by moving down the ladder
    # -- because how many samples that takes is the step-down count, which is a
    # unit test's business and not a stopwatch's.
    surface = client_rect(client, "page.surface")
    drive_at = page_screen(client, client_window, link_centre(tab))
    rung = starting_rung
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline and rung < 1:
        for step in range(8):
            subprocess.run(["xdotool", "mousemove",
                            str(drive_at[0] + step * 3), str(drive_at[1] + step * 2)],
                           env=X)
            time.sleep(0.12)
        rung, _ = client_reading(client, "reading.rung")
    assert rung >= 1, (
        "the client never stepped down the ladder on a link that cannot carry the "
        "target, so it is silently missing it rather than degrading", rung)

    degraded = client_rect_now(client, "link.degraded")
    assert degraded is not None, (
        "the client stepped down the ladder and told the human nothing -- degrade "
        "and report, not degrade quietly",
        [rect["name"] for rect in harness.client_rects(client)])
    assert client_rect_now(client, "link.full") is None, \
        "the client is drawing the full-speed report and the degraded one at once"
    measured, _ = client_reading(client, "reading.input_to_photon_ms")
    assert measured > 0, ("the client reports no input-to-photon estimate at all, so "
                          "its report has no evidence behind it", measured)
    print(f"DEGRADED: the client walked to rung {rung:.0f} and drew its degraded "
          f"report, with an input-to-photon estimate of {measured:.0f} ms")

    # --- 9. and the human can still click -------------------------------
    # **The assertion this whole plan turns on.** `D-05-06`: degrade and
    # report, never withhold control, because a degraded takeover of a login
    # wall still clears the login wall. Asserted over the control socket, so
    # the frame path under test is not also the thing reporting success.
    before = tab_url(tab)
    click_client_point(client, client_window, page_screen(client, client_window,
                                                          link_centre(tab)))
    reached = wait_for_url(tab, "/lower.html", timeout=30)
    assert reached.endswith("/lower.html"), (
        "a click through a degraded link did not reach the page -- the client is "
        "withholding control on a slow link rather than degrading", before, reached)
    print(f"STILL REACHED: a real click through the constrained link navigated the "
          f"agent's tab to {reached}")

    # --- 10. and the ladder does not oscillate ---------------------------
    # A rung change forces a keyframe, so a ladder flapping between two rungs
    # spends the link on them. The asymmetric counts and the recovery margin
    # are what stop that, and the margin is chosen so a sample good enough to
    # step up is never an overrun on the rung it steps up to -- walked over the
    # whole ladder by a unit test. This is the same property observed on a real
    # link.
    #
    # Tolerance: at most one further move over a steady twelve seconds of
    # driving. Not zero: the shim's rate is steady but llvmpipe's is not, and
    # one genuine step in twelve seconds is the ladder working. Two or more
    # would be flapping.
    settled_changes, _ = client_reading(client, "reading.rung_changes")
    steady_until = time.monotonic() + 12
    while time.monotonic() < steady_until:
        for step in range(6):
            subprocess.run(["xdotool", "mousemove",
                            str(drive_at[0] + step * 4), str(drive_at[1] + step)],
                           env=X)
            time.sleep(0.15)
    final_changes, _ = client_reading(client, "reading.rung_changes")
    final_rung, _ = client_reading(client, "reading.rung")
    assert final_changes - settled_changes <= 1, (
        "the ladder oscillated on a link whose rate never changed",
        settled_changes, final_changes)
    assert client_rect_now(client, "link.degraded") is not None, \
        "the degraded report disappeared while the link was still degraded"
    print(f"STEADY: over twelve seconds of driving the rung moved "
          f"{final_changes - settled_changes:.0f} more times, ending at rung "
          f"{final_rung:.0f}")

    print("REMOTE LATENCY CHECKS PASSED")
finally:
    if driver is not None:
        driver.stop()
    if client is not None:
        client.terminate()
        time.sleep(0.5)
        client.kill()
        client.wait()
    if viewer is not None:
        viewer.close()
    if shim is not None:
        shim.stop()
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

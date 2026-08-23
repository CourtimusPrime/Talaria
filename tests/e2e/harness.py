"""Shared launcher for the e2e suites that start their own shell.

Isolation knobs (so a suite can run next to a long soak on the default
display without killing it):

- ``TALARIA_E2E_DISPLAY``  X display to create with Xvfb (default ``:99``).
- ``XDG_RUNTIME_DIR``      inherited by the shell — point it at a private
  directory to get a private control socket (``$XDG_RUNTIME_DIR/talaria.sock``).
  With it unset the shell falls back to ``talaria-$UID/talaria.sock`` inside
  the temp directory, a per-UID directory it creates at mode 0700; see
  ``socket_path`` below, which mirrors ``talaria_protocol::socket_path``.

Only Talaria processes attached to *this* display are killed before launch;
shells on other displays are left alone.

It also carries the chrome-geometry helpers the UI suites drive clicks with —
``chrome_rects``, ``wait_for_rect``, ``click_rect`` — because two suites need
them and a hardcoded toolbar coordinate copied into both is exactly the thing
they exist to delete. They need a shell started with ``TALARIA_TEST_HOOKS=1``.

``free_port``, ``write_config`` and ``write_agents`` live here for the same
reason: more than one suite needs to launch a shell whose ``config.json`` says
something particular, on a port nothing else is holding, with an ``agents.json``
that already holds an approved client. ``pkce_pair`` and ``loopback_callback``
are here for that reason too: the OAuth suites this phase adds all need a
client-side PKCE pair and a redirect target to be sent back to.
"""
import base64
import hashlib
import http.server
import json
import os
import select
import signal
import socket
import struct
import subprocess
import threading
import time
import urllib.parse

REPO = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
DISPLAY = os.environ.get("TALARIA_E2E_DISPLAY", ":99")
# Cargo writes to $CARGO_TARGET_DIR when it is set, not to ./target. CI sets it
# so the runner's throwaway checkout shares one warm dependency build instead of
# recompiling 934 crates per run, and the binary then does not exist under REPO
# at all. Honour the same variable rather than assuming the default location.
TARGET = os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target")
BINARY = os.path.join(TARGET, "release", "talaria")
# The remote view client's binary, named beside the shell's rather than in the
# suite that first needs it: the plan that builds it should not have to reach
# into this file's constant block from a later wave to add one line.
CLIENT_BINARY = os.path.join(TARGET, "release", "talaria-client")


def socket_path():
    """Where the shell puts its control socket.

    Mirrors ``talaria_protocol::socket_path``: ``$XDG_RUNTIME_DIR`` when that
    variable is set, otherwise a per-UID ``talaria-$UID`` directory under the
    temp directory, plus the socket filename.  The suites carry this same
    expression inline rather than importing it — each one must stay runnable
    standalone against an already-running shell."""
    return os.environ.get("XDG_RUNTIME_DIR",
                          f"{os.environ.get('TMPDIR', '/tmp')}/talaria-{os.getuid()}") \
        + "/talaria.sock"


SOCK = socket_path()


def x_env(**extra):
    env = dict(os.environ, DISPLAY=DISPLAY)
    env.update(extra)
    return env


def kill_shells_on_display(display=DISPLAY):
    """SIGKILL talaria processes whose environment carries DISPLAY=display."""
    needle = f"DISPLAY={display}".encode()
    for pid in os.listdir("/proc"):
        if not pid.isdigit():
            continue
        try:
            with open(f"/proc/{pid}/comm", "rb") as f:
                if f.read().strip() != b"talaria":
                    continue
            with open(f"/proc/{pid}/environ", "rb") as f:
                if needle in f.read().split(b"\0"):
                    os.kill(int(pid), signal.SIGKILL)
        except (OSError, ProcessLookupError):
            continue


def _pid_alive(pid):
    """True for a live process; False for none or a zombie (killed but not
    yet reaped by its parent — still listed in /proc)."""
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("State:"):
                    return "Z" not in line.split()[1]
    except OSError:
        return False
    return False


def _clear_stale_x_lock(display):
    """A SIGKILLed Xvfb leaves /tmp/.X<n>-lock behind; a new server refuses
    to start while it exists. Remove it once its owning pid is gone."""
    number = display.lstrip(":").split(".")[0]
    lock = f"/tmp/.X{number}-lock"
    for _ in range(50):
        try:
            with open(lock) as f:
                pid = int(f.read().strip() or 0)
        except FileNotFoundError:
            return
        except ValueError:
            pid = 0
        if pid and _pid_alive(pid):
            time.sleep(0.1)
            continue
        for path in (lock, f"/tmp/.X11-unix/X{number}"):
            try:
                os.remove(path)
            except FileNotFoundError:
                pass
        return


def start_xvfb(display=DISPLAY, settle=2.0):
    subprocess.run(["pkill", "-f", f"[X]vfb {display}( |$)"], check=False)
    kill_shells_on_display(display)
    time.sleep(settle)  # let a previous Xvfb fully release the display
    for attempt in range(5):
        _clear_stale_x_lock(display)
        xvfb = subprocess.Popen(["Xvfb", display, "-screen", "0", "1280x800x24"],
                                stderr=subprocess.DEVNULL)
        for _ in range(20):  # wait until the display actually accepts connections
            if subprocess.run(["xdpyinfo", "-display", display], stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL).returncode == 0:
                return xvfb
            if xvfb.poll() is not None:
                break  # refused to start (stale lock / race) — retry
            time.sleep(0.5)
        xvfb.kill()
        time.sleep(1.0)
    raise RuntimeError(f"Xvfb {display} never came up")


def start_shell(url, log=subprocess.DEVNULL, rust_log="error", wait=8.0, **env_extra):
    env = x_env(RUST_LOG=rust_log, **env_extra)
    shell = subprocess.Popen([BINARY, url], env=env, stdout=log, stderr=log)
    # Ready when the control socket answers a hello (or after `wait` seconds).
    deadline = time.monotonic() + wait
    while time.monotonic() < deadline:
        if os.path.exists(SOCK):
            break
        time.sleep(0.1)
    time.sleep(max(0.0, deadline - time.monotonic()))  # first paint settle
    return shell


def free_port():
    """A loopback port nothing is listening on right now.

    Binds port 0, reads what the kernel assigned, and lets go. That leaves a
    small race — something else could take the number between the close here
    and the shell's own bind — and the suites accept it deliberately: they run
    sequentially, and the alternative, a fixed constant, would collide far more
    often against this machine's 32768-60999 ephemeral range, where an ordinary
    outbound connection can already be holding the number."""
    s = socket.socket()
    try:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]
    finally:
        s.close()


def write_config(talaria_dir, **keys):
    """Seed ``config.json`` in a suite's isolated config directory.

    Written *before* ``start_shell``, which is the same seed-the-store-before-
    launch technique ``history_test.py`` and ``bookmarks_test.py`` already use:
    the shell reads its settings once, at startup, so this is the only moment a
    test can decide what they say. ``talaria_dir`` is the ``talaria``
    subdirectory of the suite's ``XDG_CONFIG_HOME``."""
    os.makedirs(talaria_dir, exist_ok=True)
    path = os.path.join(talaria_dir, "config.json")
    with open(path, "w") as f:
        json.dump(dict(keys), f)
    return path


def write_agents(talaria_dir, client_id, client_name, token, **kwargs):
    """Seed ``agents.json`` with one approved client holding one access token.

    **What this is:** a pre-existing authorization record — exactly what a
    prior approval leaves behind on disk. A human approved this client once,
    the browser minted a token, and what stayed is the client row plus the
    SHA-256 of the token. Writing that file before launch is the same
    seed-the-store-before-launch technique ``history_test.py`` and
    ``bookmarks_test.py`` use, and it is the only moment a test can decide what
    the store says, because the shell reads it once at startup.

    **What this is not:** a bypass. It grants nothing a real approval would not
    have granted, no code path in the shell treats a seeded record differently
    from a minted one, and there is no environment variable or hook that mints
    or accepts a token. The digest is computed here with ``hashlib.sha256`` —
    the same mapping ``agents::digest_of`` uses — so the suite and the shell
    agree on what a token means without the shell having to expose anything.
    Anyone who can write this file already runs as the user and could read the
    config directory anyway.

    ``audience`` must be the shell's canonical resource identifier for the port
    it is about to bind (``http://127.0.0.1:PORT/mcp``), because verification
    compares it byte for byte — see ``oauth::canonical_resource``.

    **When a suite configures an advertised URL** (``write_config`` with
    ``remote_access.advertised_url``), that identifier is built from *that*
    origin and not from the loopback address — ``https://NAME:PORT/mcp`` — and
    the audience seeded here must match it byte for byte too, because the
    advertised spelling is then the only canonical one. No existing suite
    configures one, so no existing suite changes.

    ``extra_tokens`` takes a list of ``(token, audience)`` pairs for the cases
    a suite needs a *second* credential for, such as one minted for another
    resource server. They belong to the same client and their own family.

    04-07's revocation suite needs this helper too, which is why it lives here
    rather than in one suite."""
    audience = kwargs.pop("audience")
    scope = kwargs.pop("scope", "talaria:drive")
    now_ms = kwargs.pop("now_ms", int(time.time() * 1000))
    ttl_ms = kwargs.pop("ttl_ms", 60 * 60 * 1000)
    extra_tokens = kwargs.pop("extra_tokens", [])
    assert not kwargs, f"unknown write_agents keyword(s): {sorted(kwargs)}"

    def record(raw, aud, family):
        return {"digest": hashlib.sha256(raw.encode()).hexdigest(),
                "kind": "access", "client_id": client_id, "family_id": family,
                "expires_at_ms": now_ms + ttl_ms, "audience": aud,
                "scope": scope, "consumed_at_ms": None}

    document = {
        "clients": [{"client_id": client_id, "client_name": client_name,
                     "redirect_uris": [], "registered_at_ms": now_ms,
                     "authorized_at_ms": now_ms}],
        "tokens": [record(token, audience, "family-0")] +
                  [record(raw, aud, f"family-{n + 1}")
                   for n, (raw, aud) in enumerate(extra_tokens)],
    }
    os.makedirs(talaria_dir, exist_ok=True)
    path = os.path.join(talaria_dir, "agents.json")
    with open(path, "w") as f:
        json.dump(document, f)
    os.chmod(path, 0o600)
    return path


def chrome_rects(client="e2e-rects"):
    """Where the shell is drawing each named chrome control right now.

    Returns ``(rects, scale)``: ``rects`` maps a name (``toolbar.credentials``,
    ``history.row.1``, ``downloads.open.0``) to an ``(x, y, width, height)``
    tuple in logical points with the window's top-left as the origin, and
    ``scale`` is the window's scale factor.

    ``chrome_rects`` is a test hook — the shell refuses it as an unknown
    command unless it was started with ``TALARIA_TEST_HOOKS=1``, so a suite
    using this must pass that in. It replaces the hardcoded toolbar
    coordinates the UI suites used to carry, which moved every time the
    toolbar gained a button.

    The reply describes the last frame the shell drew, so a caller that has
    just changed what is on screen should poll rather than read once — that is
    what ``wait_for_rect`` is for.
    """
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(SOCK)
    try:
        f = s.makefile("rw")
        f.write(json.dumps({"type": "hello", "client": client}) + "\n")
        f.flush()
        assert json.loads(f.readline())["type"] == "hello_ack"
        f.write(json.dumps({"type": "request", "id": 1,
                            "command": "chrome_rects"}) + "\n")
        f.flush()
        while True:
            m = json.loads(f.readline())
            if m.get("type") == "reply" and m.get("id") == 1:
                break
    finally:
        s.close()
    assert m["outcome"] == "ok", \
        ("chrome_rects was refused — was the shell started with "
         "TALARIA_TEST_HOOKS=1?", m)
    rects = {r["name"]: (r["x"], r["y"], r["width"], r["height"])
             for r in m["result"]["rects"]}
    return rects, m["result"]["scale"]


def wait_for_rect(name, timeout=8.0, present=True):
    """Poll until `name` is being drawn (or, with present=False, is not).

    Returns the rects mapping. Raises rather than returning a stale answer:
    a suite that goes on to click a control the chrome is not drawing would
    click whatever is underneath it instead."""
    deadline = time.monotonic() + timeout
    while True:
        rects, scale = chrome_rects()
        if (name in rects) == present:
            return rects, scale
        if time.monotonic() > deadline:
            raise AssertionError(
                f"{name} was {'never' if present else 'still'} drawn after "
                f"{timeout}s; the chrome is showing {sorted(rects)}")
        time.sleep(0.2)


def window_origin(wid, env):
    """The window's top-left in screen coordinates."""
    out = subprocess.run(["xdotool", "getwindowgeometry", "--shell", wid],
                         env=env, capture_output=True, text=True).stdout
    values = dict(line.split("=", 1) for line in out.splitlines() if "=" in line)
    return int(values["X"]), int(values["Y"])


def focus_window(wid, env, settle=0.3):
    """Give the window keyboard and pointer focus, and let it settle."""
    subprocess.run(["xdotool", "windowfocus", "--sync", wid], env=env)
    time.sleep(settle)


def click_rect(name, wid, env, settle=1.5, timeout=8.0, focus=True):
    """Click the centre of the named chrome control, for real.

    Looks the control up with ``wait_for_rect``, converts its logical centre
    to screen pixels through the window's own origin and scale factor, and
    drives a genuine pointer click at it with ``xdotool`` — the same input
    path a person's mouse takes. Returns the screen point it clicked.

    ``focus=False`` skips the focus call and the settle that follows it, for
    the one kind of assertion where the *latency* of the click is the thing
    under test — the consent panel's arm delay, where a click has to land
    inside a 1000 ms window to prove anything. Focus the window once up front
    with :func:`focus_window` and pass ``focus=False`` there; everywhere else
    the default is what you want, because a click at an unfocused window is a
    click that may only raise it."""
    rects, scale = wait_for_rect(name, timeout=timeout)
    x, y, width, height = rects[name]
    origin_x, origin_y = window_origin(wid, env)
    point = (str(round(origin_x + (x + width / 2) * scale)),
             str(round(origin_y + (y + height / 2) * scale)))
    if focus:
        focus_window(wid, env)
    subprocess.run(["xdotool", "mousemove", *point, "click", "1"], env=env)
    time.sleep(settle)
    return point


def stop(shell, xvfb):
    shell.terminate()
    time.sleep(1)
    shell.kill()
    shell.wait()
    xvfb.kill()
    xvfb.wait()  # reap, so the X lock's pid isn't left as a zombie


def pkce_pair():
    """A PKCE ``(verifier, challenge)`` pair, S256.

    The client half of RFC 7636: a high-entropy verifier the client keeps, and
    the URL-safe base64 of its SHA-256 that the client sends to the
    authorization endpoint. Talaria advertises ``S256`` and accepts nothing
    else — ``plain`` would let a "verifier" equal the challenge, which is no
    protection at all — so there is deliberately no option here to produce one.

    The padding is stripped because RFC 7636 specifies base64url *without*
    padding and Talaria's challenge check rejects ``=``.
    """
    verifier = base64.urlsafe_b64encode(os.urandom(32)).decode().rstrip("=")
    digest = hashlib.sha256(verifier.encode()).digest()
    challenge = base64.urlsafe_b64encode(digest).decode().rstrip("=")
    return verifier, challenge


class LoopbackCallback:
    """The redirect target a native OAuth client binds for itself.

    An ``http.server.HTTPServer`` on **port 0** — the kernel picks the number —
    served from a daemon thread, recording the query of every request it is
    redirected to.

    Binding port 0 is not a testing convenience: it is exactly the case RFC
    8252's loopback exception exists for. A native client cannot know its
    callback port at registration time, so an authorization server is required
    to allow any port for a loopback redirect URI, and that is the single field
    Talaria's otherwise-exact redirect matching ignores. A suite that
    registered a fixed port would never exercise it.
    """

    def __init__(self):
        self.received = []
        received = self.received

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                parsed = urllib.parse.urlsplit(self.path)
                received.append(
                    {key: values[-1] for key, values
                     in urllib.parse.parse_qs(parsed.query).items()})
                body = b"received"
                try:
                    self.send_response(200)
                    self.send_header("Content-Type", "text/plain")
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                except OSError:
                    pass

            def log_message(self, *args):
                pass

        self.server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
        self.port = self.server.server_address[1]
        self.url = f"http://127.0.0.1:{self.port}/callback"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def last(self):
        """The query of the most recent redirect, or ``None``."""
        return self.received[-1] if self.received else None

    def clear(self):
        del self.received[:]

    def stop(self):
        self.server.shutdown()
        self.server.server_close()


def loopback_callback():
    """A started :class:`LoopbackCallback`. Stop it in a ``finally``."""
    return LoopbackCallback()


# --- a standard-library WebSocket client -------------------------------------

WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


def ws_accept(key):
    """RFC 6455's handshake proof: base64(sha1(key + GUID))."""
    return base64.b64encode(hashlib.sha1((key + WS_GUID).encode()).digest()).decode()


class WebSocket:
    """A WebSocket client on a raw socket, standard library only.

    **What it pins down.** The request line and every header are written by
    hand, which is the point rather than an inconvenience: the origin-refusal
    and wrong-host assertions are one extra header each, and no library gets
    to decide what goes on the wire. The handshake proof is **verified** —
    ``Sec-WebSocket-Accept`` is recomputed from the key that was sent — so a
    proxy or a server that answered ``101`` without completing the handshake
    is caught rather than assumed to have worked.

    A non-``101`` answer is a value, not an exception: ``status``, ``headers``
    and ``body`` are readable, which is how the refusal assertions compare one
    answer against another byte for byte.

    Read with ``select`` on a bare socket rather than through a buffered
    reader, the technique ``revocation_test.py``'s ``Stream`` established and
    for the same reason: a reader that has timed out once refuses every later
    read, and this class has to be able to say both "nothing yet, still open"
    and "ended" about the same connection. ``closed_within`` is the assertion
    a revoked viewer needs, and copying its shape is deliberate — asserting
    instead that a fresh upgrade now fails would pass whether or not the
    socket ever closed.

    **Its honest limits.** It does not reassemble fragmented messages (this
    server sends none), it does not negotiate extensions, and it treats a
    close frame, an EOF and a reset alike as an ending — which is the honest
    reading of all three from the read side: the client can no longer receive.
    """

    def __init__(self, host, port, token=None, path="/view", host_header=None,
                 headers=(), timeout=10):
        self.socket = socket.create_connection((host, port), timeout=timeout)
        self.key = base64.b64encode(os.urandom(16)).decode()
        lines = [
            f"GET {path} HTTP/1.1",
            f"Host: {host_header or f'{host}:{port}'}",
            "Upgrade: websocket",
            "Connection: Upgrade",
            f"Sec-WebSocket-Key: {self.key}",
            "Sec-WebSocket-Version: 13",
        ]
        if token is not None:
            lines.append(f"Authorization: Bearer {token}")
        lines.extend(headers)
        self.socket.sendall(("\r\n".join(lines) + "\r\n\r\n").encode())
        self.socket.settimeout(None)
        self.buf = b""
        self.messages = []
        self.ended = False
        self.framed = False
        self.body = b""
        self.status, self.headers = self._read_head()
        if self.status == 101:
            self.framed = True
            self._decode()
        else:
            self.ended = True
            self._read_body()

    # -- handshake ------------------------------------------------------------

    def _read_head(self):
        deadline = time.monotonic() + 10
        while b"\r\n\r\n" not in self.buf:
            if not self._pump(deadline - time.monotonic()):
                raise AssertionError(f"no response head: {self.buf[:200]!r}")
        head, _, self.buf = self.buf.partition(b"\r\n\r\n")
        lines = head.decode(errors="replace").split("\r\n")
        status = int(lines[0].split()[1])
        headers = {}
        for line in lines[1:]:
            key, _, value = line.partition(":")
            headers[key.strip().lower()] = value.strip()
        return status, headers

    def _read_body(self):
        """A refusal's body, so two refusals can be compared byte for byte."""
        length = int(self.headers.get("content-length", 0) or 0)
        deadline = time.monotonic() + 5
        while len(self.buf) < length and time.monotonic() < deadline:
            if not self._pump(deadline - time.monotonic()) and self.ended:
                break
        self.body, self.buf = self.buf[:length], self.buf[length:]

    def handshook(self):
        """True once the upgrade completed *and* the proof checks out."""
        return (self.status == 101
                and self.headers.get("sec-websocket-accept") == ws_accept(self.key)
                and self.headers.get("upgrade", "").lower() == "websocket")

    # -- framing --------------------------------------------------------------

    def _decode(self):
        """Move whatever complete frames have arrived into ``messages``."""
        if not self.framed:
            return
        while True:
            if len(self.buf) < 2:
                return
            opcode = self.buf[0] & 0x0F
            masked = self.buf[1] & 0x80
            length = self.buf[1] & 0x7F
            offset = 2
            if length == 126:
                if len(self.buf) < 4:
                    return
                length = struct.unpack(">H", self.buf[2:4])[0]
                offset = 4
            elif length == 127:
                if len(self.buf) < 10:
                    return
                length = struct.unpack(">Q", self.buf[2:10])[0]
                offset = 10
            mask = b""
            if masked:
                if len(self.buf) < offset + 4:
                    return
                mask = self.buf[offset:offset + 4]
                offset += 4
            if len(self.buf) < offset + length:
                return
            payload = self.buf[offset:offset + length]
            self.buf = self.buf[offset + length:]
            if mask:
                payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
            if opcode == 0x8:      # close
                self.ended = True
                return
            elif opcode == 0x9:    # ping -> pong, so the server keeps us
                self.send(payload, opcode=0xA)
            elif opcode == 0xA:    # pong
                pass
            else:                  # text, binary, continuation
                self.messages.append(payload)

    def send(self, payload, opcode=0x2):
        """One masked frame. A client must mask; a server must not."""
        mask = os.urandom(4)
        length = len(payload)
        if length < 126:
            header = bytes([0x80 | opcode, 0x80 | length])
        elif length < 65536:
            header = bytes([0x80 | opcode, 0x80 | 126]) + struct.pack(">H", length)
        else:
            header = bytes([0x80 | opcode, 0x80 | 127]) + struct.pack(">Q", length)
        body = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.socket.sendall(header + mask + body)

    # -- reading --------------------------------------------------------------

    def _pump(self, timeout):
        """One read. False on timeout, False and ``ended`` set on an ending."""
        if timeout <= 0:
            return False
        if not select.select([self.socket], [], [], timeout)[0]:
            return False
        try:
            chunk = self.socket.recv(65536)
        except (ConnectionResetError, OSError):
            self.ended = True
            return False
        if not chunk:
            self.ended = True
            return False
        self.buf += chunk
        self._decode()
        return True

    def next_message(self, timeout=5.0):
        """The next decoded message, or ``None`` if none arrives in time."""
        deadline = time.monotonic() + timeout
        while not self.messages and not self.ended:
            if not self._pump(deadline - time.monotonic()) and time.monotonic() > deadline:
                break
        return self.messages.pop(0) if self.messages else None

    def delivering(self, timeout=5.0):
        """True once any message has arrived, which makes "open" a measurement."""
        return self.next_message(timeout) is not None

    def still_open(self, settle=1.0):
        """False once the read side has seen the connection end."""
        self._pump(settle)
        return not self.ended

    def closed_within(self, timeout):
        """True once the read side sees the connection end within ``timeout``.

        The assertion a revoked viewer needs, made from the side that would
        notice. Asserting instead that a fresh upgrade now fails would pass
        whether or not this socket ever closed."""
        deadline = time.monotonic() + timeout
        while not self.ended and time.monotonic() < deadline:
            self._pump(min(0.5, deadline - time.monotonic()))
        return self.ended

    def close(self):
        try:
            self.socket.close()
        except OSError:
            pass

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
"""
import json
import os
import signal
import socket
import subprocess
import time

REPO = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
DISPLAY = os.environ.get("TALARIA_E2E_DISPLAY", ":99")
# Cargo writes to $CARGO_TARGET_DIR when it is set, not to ./target. CI sets it
# so the runner's throwaway checkout shares one warm dependency build instead of
# recompiling 934 crates per run, and the binary then does not exist under REPO
# at all. Honour the same variable rather than assuming the default location.
TARGET = os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target")
BINARY = os.path.join(TARGET, "release", "talaria")


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


def click_rect(name, wid, env, settle=1.5, timeout=8.0):
    """Click the centre of the named chrome control, for real.

    Looks the control up with ``wait_for_rect``, converts its logical centre
    to screen pixels through the window's own origin and scale factor, and
    drives a genuine pointer click at it with ``xdotool`` — the same input
    path a person's mouse takes. Returns the screen point it clicked."""
    rects, scale = wait_for_rect(name, timeout=timeout)
    x, y, width, height = rects[name]
    origin_x, origin_y = window_origin(wid, env)
    point = (str(round(origin_x + (x + width / 2) * scale)),
             str(round(origin_y + (y + height / 2) * scale)))
    subprocess.run(["xdotool", "windowfocus", "--sync", wid], env=env)
    time.sleep(0.3)
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

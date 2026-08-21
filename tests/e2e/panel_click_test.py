#!/usr/bin/env python3
"""E2E: what the chrome panels' rows and buttons actually do when clicked.

The stores behind History, Bookmarks and Downloads are covered by their own
suites, and every one of them stops at the file on disk. This suite covers the
other end: a real pointer click on a real widget, driven with ``xdotool`` at
the rect the shell reports for it through the ``chrome_rects`` test hook. The
six behaviours below were the ones 03-VERIFICATION.md left as human items
because "no headless harness in this repo drives an egui click"; they are all
driven here.

What this pins down:

- clicking a history row navigates the *displayed* tab to that row's URL and
  closes the panel in the same click — the close proved twice over, by the
  panel's rows no longer being drawn and by the next Ctrl+H *opening* it;
- clicking a bookmark row does the same, which is BROWSE-02's "return to it"
  half end to end;
- Clear history takes two clicks, and the confirming state does not survive
  the panel closing: the first click clears nothing and arms the control (the
  chrome reports it under a different name once armed), a close and reopen
  disarms it, and a single click after that still clears nothing — that last
  assertion is the one that proves the reset rather than assuming it;
- clicking a downloads row's Open launches ``xdg-open`` at all (BROWSE-04's
  "opened from it" half), and launches it on **exactly the path the row
  stores**: the same filename is downloaded twice so the second uniquifies to
  ``report (1).pdf``, and Open is pressed on the *first* row, whose argv must
  be the first path verbatim — not the requested name, not the uniquified
  sibling. That is T-03-01, the phase's one high-severity security truth;
- an Open that cannot spawn leaves the browser running, shows an inline notice
  with a Dismiss control, and dismisses cleanly;
- and, last, that the ``chrome_rects`` hook every step above leans on is
  simply not there in a shell started without ``TALARIA_TEST_HOOKS=1`` — where
  the human's own controls are on screen is not an agent's to read.

Two honest limits. The chrome reports geometry, not text, so the notice's
*wording* is not read here: what is asserted is that the Dismiss control is on
screen (it is drawn only while a notice exists) and that the shell logged the
filename and the OS error the label is built from. And the failure is provoked
by taking ``xdg-open`` off the shell's PATH, not by deleting the downloaded
file — ``spawn`` does not stat the argument, so a missing file would launch
the handler quite happily and fail somewhere this browser cannot see.

``xdg-open`` is a **fake** for the whole run: a shell script first on the
child's PATH that appends its argv to a file and exits. It is written to a
temp directory at runtime and never committed, so nothing here launches a real
application, and the argv assertions are exact rather than inferred.

Standalone (starts its own Xvfb + shell, with ``TALARIA_TEST_HOOKS=1`` for the
rect lookup): it needs exclusive input focus, and an isolated home, because
history, bookmarks, ``downloads.json`` and the engine profile all live under
the platform config directory. Pages and the downloaded body come from a local
HTTP server, so nothing here touches the network.
"""
import http.server
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness

NAME = "report.pdf"
BODY = b"%PDF-1.4 pretend this is a report\n" * 8
PAGES = {
    "/one": b"<!doctype html><title>Panel One</title><h1>one</h1>",
    "/two": b"<!doctype html><title>Panel Two</title><h1>two</h1>",
}


class Fixtures(http.server.BaseHTTPRequestHandler):
    """Two pages to visit and one body to download."""

    def do_GET(self):
        path = self.path.split("?")[0]
        if path == "/report":
            self.body(BODY, "application/octet-stream")
        elif path in PAGES:
            self.body(PAGES[path], "text/html")
        else:
            self.send_error(404)

    def body(self, payload, kind):
        try:
            self.send_response(200)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except OSError:
            pass

    def log_message(self, *args):
        pass


tmp = tempfile.mkdtemp(prefix="talaria-panel-click-e2e-")
config = os.path.join(tmp, ".config")
talaria = os.path.join(config, "talaria")
downloads = os.path.join(tmp, "Downloads")
shim_dir = os.path.join(tmp, "bin")
# The whole of the PATH for the failure phase: a directory with nothing in it,
# so `xdg-open` cannot be found anywhere and `spawn` fails with ENOENT. Leaving
# the real PATH in place and only removing the shim would not do it — execvp
# keeps searching, and this machine has a real /usr/bin/xdg-open on it.
no_opener_dir = os.path.join(tmp, "no-opener")
for directory in (talaria, downloads, shim_dir, no_opener_dir):
    os.makedirs(directory)
# dirs::download_dir() reads this file (dirs-sys resolves $XDG_CONFIG_HOME then
# $HOME/.config); without it there is no downloads directory to find.
with open(os.path.join(config, "user-dirs.dirs"), "w") as f:
    f.write('XDG_DOWNLOAD_DIR="$HOME/Downloads"\n')

HISTORY = os.path.join(talaria, "history.jsonl")
BOOKMARKS = os.path.join(talaria, "bookmarks.json")
STORE = os.path.join(talaria, "downloads.json")
OPENED = os.path.join(tmp, "xdg-open.argv")
SHELL_LOG = os.path.join(tmp, "shell.log")

# The fake opener. One line per argument, so an Open that passed anything
# other than exactly one path shows up as the wrong number of lines rather
# than being quietly absorbed.
SHIM = os.path.join(shim_dir, "xdg-open")
with open(SHIM, "w") as f:
    f.write("#!/bin/sh\n")
    f.write("printf '%s\\n' \"$@\" >> " + json.dumps(OPENED) + "\n")
os.chmod(SHIM, 0o755)

SHIM_PATH = shim_dir + ":" + os.environ.get("PATH", "")

server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixtures)
threading.Thread(target=server.serve_forever, daemon=True).start()
BASE = f"http://127.0.0.1:{server.server_address[1]}"


def start(path=SHIM_PATH, log=subprocess.DEVNULL, hooks="1"):
    """`hooks` is what gates `chrome_rects`; the last section starts a shell
    without it, which is what an ordinary run of the browser looks like."""
    extra = {"TALARIA_TEST_HOOKS": hooks} if hooks else {}
    return harness.start_shell("about:blank", log=log, rust_log="warn",
                               HOME=tmp, XDG_CONFIG_HOME=config, PATH=path,
                               **extra)


def stop_shell(shell):
    """Stop the shell and wait for its socket to go, so the next start_shell
    does not return against the dying one (and does not get its URL forwarded
    to it by the single-instance path)."""
    shell.terminate()
    for _ in range(50):
        if shell.poll() is not None:
            break
        time.sleep(0.1)
    else:
        shell.kill()
    shell.wait()
    for _ in range(50):
        if not os.path.exists(harness.SOCK):
            return
        time.sleep(0.1)
    try:
        os.remove(harness.SOCK)
    except FileNotFoundError:
        pass


def rpc(command, client="panel-click-e2e", **params):
    """One request on its own connection — the suite restarts the shell, so a
    long-lived wire would not survive."""
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


def history_rows():
    try:
        with open(HISTORY) as f:
            return [json.loads(line) for line in f.read().splitlines() if line.strip()]
    except FileNotFoundError:
        return []


def json_rows(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (FileNotFoundError, ValueError):
        return []


def opened():
    """Every argument the fake xdg-open has been handed, in order."""
    try:
        with open(OPENED) as f:
            return [line for line in f.read().splitlines() if line]
    except FileNotFoundError:
        return []


def wait_for(read, want, timeout=15.0):
    """Poll `read` until it returns at least `want` rows. Stores are written
    from the event loop a tick after the reply that reported the work, so
    every count here is polled rather than read once."""
    deadline = time.monotonic() + timeout
    rows = read()
    while time.monotonic() < deadline and len(rows) < want:
        time.sleep(0.2)
        rows = read()
    return rows


def displayed():
    """The tab the human is looking at: the focused Me-owned one. The suite
    opens no agent tabs, so there is exactly one."""
    r = rpc("tabs_list")
    assert r["outcome"] == "ok", r
    me = [t for t in r["result"]["tabs"] if t["owner"] == "me" and t["focused"]]
    assert len(me) == 1, me
    return me[0]


def wait_for_displayed(suffix, timeout=20.0):
    """Poll until the displayed tab's URL ends with `suffix`."""
    deadline = time.monotonic() + timeout
    tab = displayed()
    while time.monotonic() < deadline and not tab["url"].endswith(suffix):
        time.sleep(0.3)
        tab = displayed()
    assert tab["url"].endswith(suffix), \
        (f"the displayed tab never reached {suffix}", tab)
    return tab


xvfb = harness.start_xvfb()
X = harness.x_env()
tal = None
log = None


def find_window():
    for _ in range(15):
        out = subprocess.run(["xdotool", "search", "--name", "Talaria"], env=X,
                             capture_output=True, text=True).stdout.split()
        if out:
            return out[0]
        time.sleep(1)
    raise AssertionError("Talaria window never appeared")


try:
    tal = start()
    wid = find_window()

    def press(*keys):
        """A real keystroke at the real window."""
        subprocess.run(["xdotool", "windowfocus", "--sync", wid], env=X)
        time.sleep(0.3)
        subprocess.run(["xdotool", "key", *keys], env=X)
        time.sleep(1.2)

    def click(name):
        return harness.click_rect(name, wid, X)

    assert history_rows() == [], "the isolated home already had history in it"
    assert json_rows(BOOKMARKS) == [], "the isolated home already had bookmarks in it"
    assert json_rows(STORE) == [], "the isolated home already had downloads in it"

    # --- clicking a history row navigates and closes the panel -------------
    for page in ("/one", "/two"):
        r = rpc("open_for_user", url=BASE + page)
        assert r["outcome"] == "ok", r
    rows = wait_for(history_rows, 2)
    assert [row["url"] for row in rows] == [BASE + "/one", BASE + "/two"], rows
    wait_for_displayed("/two")

    press("ctrl+h")
    rects, _ = harness.wait_for_rect("history.row.1")
    # Newest first, so row 1 is the older visit — /one, the page the displayed
    # tab is *not* on. Which row was clicked is not taken on trust: the URL
    # asserted below is the one only that row carries.
    assert "history.row.0" in rects, sorted(rects)
    click("history.row.1")
    tab = wait_for_displayed("/one")
    print("HISTORY row click navigated the displayed tab to", tab["url"])

    harness.wait_for_rect("history.row.0", present=False)
    print("HISTORY panel closed on the same click")
    # The other half of the same claim: the panel is closed, not merely
    # emptied, so the next Ctrl+H opens it rather than closing it again.
    press("ctrl+h")
    harness.wait_for_rect("history.row.0")
    print("HISTORY the next Ctrl+H opened it again")
    press("ctrl+h")

    # --- clicking a bookmark row navigates and closes the panel ------------
    # BROWSE-02's "return to it" half. Bookmark the page being displayed,
    # go somewhere else, then come back through the row.
    press("ctrl+d")
    marks = wait_for(lambda: json_rows(BOOKMARKS), 1)
    assert len(marks) == 1, marks
    assert marks[0]["url"] == BASE + "/one", marks
    print("BOOKMARKED", marks[0]["url"])

    r = rpc("open_for_user", url=BASE + "/two")
    assert r["outcome"] == "ok", r
    wait_for_displayed("/two")

    press("ctrl+b")
    harness.wait_for_rect("bookmarks.row.0")
    click("bookmarks.row.0")
    tab = wait_for_displayed("/one")
    print("BOOKMARK row click returned the displayed tab to", tab["url"])
    harness.wait_for_rect("bookmarks.row.0", present=False)
    press("ctrl+b")
    harness.wait_for_rect("bookmarks.row.0")
    print("BOOKMARKS panel closed on the click, and reopens")
    press("ctrl+b")

    # --- Clear history takes two clicks, and the confirm resets -----------
    time.sleep(2)  # let the navigations above finish landing their rows
    before = history_rows()
    assert len(before) >= 2, before

    press("ctrl+h")
    rects, _ = harness.wait_for_rect("history.clear")
    assert "history.confirm-clear" not in rects, \
        ("the Clear control opened already armed", sorted(rects))

    click("history.clear")
    time.sleep(1.5)
    assert history_rows() == before, \
        ("the first click on Clear history cleared it", history_rows())
    harness.wait_for_rect("history.confirm-clear")
    print("CLEAR first click armed the confirm and cleared nothing")

    # Close and reopen: the half-pressed clear must not survive it.
    press("ctrl+h")
    press("ctrl+h")
    rects, _ = harness.wait_for_rect("history.clear")
    assert "history.confirm-clear" not in rects, \
        ("the confirm survived the panel closing", sorted(rects))

    # The assertion that proves the reset rather than assuming it: one click
    # on a control that reset is an arm, and a control that did NOT reset
    # would have cleared the store on this very click.
    click("history.clear")
    time.sleep(1.5)
    assert history_rows() == before, \
        ("a single click cleared history after a close and reopen — the "
         "confirm state persisted across the close", history_rows())
    print("CLEAR the confirm reset when the panel closed")

    # And now, armed again, confirm for real.
    click("history.confirm-clear")
    time.sleep(1.5)
    assert history_rows() == [], ("the confirming click did not clear history",
                                  history_rows())
    harness.wait_for_rect("history.clear", present=False)
    harness.wait_for_rect("history.row.0", present=False)
    print("CLEAR confirmed —", len(before), "rows gone")
    press("ctrl+h")

    # --- Open launches xdg-open on exactly the path the row stores --------
    # T-03-01. The same requested name twice, so the second file is written as
    # `report (1).pdf`; Open is then pressed on the FIRST row, whose stored
    # path is the only one of the two that a re-derivation from the requested
    # filename could not produce.
    for _ in range(2):
        r = rpc("download", url=BASE + "/report", filename=NAME)
        assert r["outcome"] == "ok", r
    rows = wait_for(lambda: json_rows(STORE), 2)
    assert len(rows) == 2, rows
    first, second = rows
    assert os.path.basename(first["path"]) == NAME, first
    assert "(1)" in os.path.basename(second["path"]), second
    assert opened() == [], ("something launched the opener before the click",
                            opened())

    press("ctrl+j")
    rects, _ = harness.wait_for_rect("downloads.open.1")
    # Newest first again, so display row 1 is the first download — the row
    # whose path has no "(1)" in it.
    assert "downloads.open.0" in rects, sorted(rects)
    click("downloads.open.1")

    launched = wait_for(opened, 1, timeout=10.0)
    assert launched, "clicking Open launched nothing at all"
    assert launched == [first["path"]], \
        ("Open did not launch the path the row stores", launched, first["path"],
         second["path"])
    assert "(1)" not in launched[0], \
        ("Open launched the uniquified sibling, not the row that was clicked",
         launched)
    print("OPEN launched xdg-open on", launched[0])
    press("ctrl+j")

    # --- a failed Open says so inline, dismisses, and does not take the
    #     browser down with it --------------------------------------------
    # Restarted with a PATH that has no `xdg-open` on it, so the spawn itself
    # fails. The downloads list is on disk, so both rows come back.
    stop_shell(tal)
    log = open(SHELL_LOG, "w")
    tal = start(path=no_opener_dir, log=log)
    wid = find_window()
    assert json_rows(STORE) == rows, "the downloads list did not survive the restart"

    press("ctrl+j")
    rects, _ = harness.wait_for_rect("downloads.open.0")
    assert "downloads.dismiss-error" not in rects, \
        ("the Downloads panel opened already showing an error", sorted(rects))

    click("downloads.open.0")
    # The Dismiss control is drawn only while a notice is showing, so its
    # presence is the notice's presence.
    harness.wait_for_rect("downloads.dismiss-error")
    print("FAILED open showed an inline notice with a Dismiss control")

    # The notice's wording is not readable from here, but the two values it is
    # built from are: the shell logs the path it tried and the OS error, and
    # the label is `format!("Couldn't open {filename} — {error}")` over the
    # same pair.
    log.flush()
    with open(SHELL_LOG) as f:
        text = f.read()
    assert f"could not open {second['path']}" in text, \
        ("the failed Open did not report the path it tried", text[-2000:])
    assert "No such file or directory" in text, \
        ("the failed Open did not report the OS error", text[-2000:])
    print("FAILED open named the file and the OS error")

    # The valuable half: it is a notice, not a crash.
    assert tal.poll() is None, ("the shell exited on a failed Open", tal.returncode)
    assert rpc("tabs_list")["outcome"] == "ok", "the shell stopped answering"

    click("downloads.dismiss-error")
    rects, _ = harness.wait_for_rect("downloads.dismiss-error", present=False)
    assert "downloads.open.0" in rects, \
        ("Dismiss took the list with it", sorted(rects))
    assert json_rows(STORE) == rows, "a failed Open changed the downloads list"
    assert opened() == launched, ("a failed Open launched something after all",
                                  opened())
    print("DISMISSED the notice; the list and the shell are intact")

    # --- the hook this suite leans on is not there in an ordinary run -----
    # Where the human's own controls are on screen is not an agent's to read:
    # an agent that could would know where to aim synthetic input at the
    # credentials button, the bookmark star or a downloads row. Started
    # without TALARIA_TEST_HOOKS, the command has to answer the way an
    # unrecognised one does, and say nothing about a feature being withheld.
    stop_shell(tal)
    tal = start(hooks=None)
    refused = rpc("chrome_rects")
    assert refused["outcome"] == "error", \
        ("chrome_rects answered a shell started without TALARIA_TEST_HOOKS",
         refused)
    assert refused["message"] == "unknown command", \
        ("the refusal advertises the hook rather than denying it exists",
         refused)
    # And the rest of the surface is unaffected — this is a gate on one
    # command, not a shell that stopped answering.
    assert rpc("tabs_list")["outcome"] == "ok", "the gated shell stopped answering"
    print("GATED chrome_rects is refused as an unknown command without the hook")

    print("PANEL CLICK CHECKS PASSED")
finally:
    server.shutdown()
    if tal is not None:
        stop_shell(tal)
    if log is not None:
        log.close()
    xvfb.kill()
    xvfb.wait()  # reap, so the X lock's pid isn't left as a zombie
    shutil.rmtree(tmp, ignore_errors=True)

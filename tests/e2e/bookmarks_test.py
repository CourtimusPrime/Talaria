#!/usr/bin/env python3
"""E2E: bookmarks through the real keyboard path (BROWSE-02).

Bookmarking is a human-only chrome action. There is deliberately no
control-socket command and no MCP tool that adds, removes or reads a bookmark,
so — unlike history, which rides real navigation an agent can trigger — the
only way to exercise this from a test is to press the key a person would press.
This suite drives ``Ctrl+D`` and ``Ctrl+B`` at the real window with ``xdotool``
and asserts on ``bookmarks.json`` itself, which is also exactly what a restart
reads.

What this pins down:

- ``Ctrl+D`` on a loaded page writes one bookmark carrying that page's URL, its
  title and a timestamp;
- ``Ctrl+D`` again *removes* it rather than adding a second copy — the toggle,
  proven through the keyboard rather than only in the store's unit tests;
- ``Ctrl+D`` a third time bookmarks it again, so the toggle is a toggle and not
  a one-way trip;
- ``Ctrl+B`` (the Bookmarks panel) leaves the shell answering — this suite does
  not verify what the panel *looks* like, which is 03-VALIDATION.md's
  human-judgment row; it verifies that the shortcut neither crashes nor wedges
  the event loop;
- and the bookmark survives a full shell restart against the same config
  directory, which is BROWSE-02's literal success criterion.

Standalone (starts its own Xvfb + shell): it needs an isolated home, because
bookmarks, the vault and the engine profile all live under the platform config
directory and the developer's real ones must not be touched. The page comes
from a local HTTP server so nothing here depends on the network — and so the
title asserted below is one this file controls.
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

TITLE = "Bookmarks Keep"
PAGES = {
    "/keep": f"<!doctype html><title>{TITLE}</title><h1>keep</h1>".encode(),
}


class Pages(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = PAGES.get(self.path.split("?")[0], b"<!doctype html><title>x</title>")
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def make_home(prefix):
    """A temp home with an empty talaria config directory inside it."""
    home = tempfile.mkdtemp(prefix=prefix)
    config = os.path.join(home, ".config")
    talaria = os.path.join(config, "talaria")
    os.makedirs(talaria)
    return home, config, talaria


def start(home, config, url, log=subprocess.DEVNULL):
    """The shell on a real page: the star acts on the displayed tab, so there
    has to be one with a resolvable URL and title."""
    return harness.start_shell(url, log=log, rust_log="warn",
                               HOME=home, XDG_CONFIG_HOME=config)


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


def rpc(command, client="bookmarks-e2e", **params):
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


def bookmarks(talaria):
    """The stored list; an absent or unparseable file reads as empty, which is
    the same thing the shell itself does with it."""
    try:
        with open(os.path.join(talaria, "bookmarks.json")) as f:
            return json.load(f)
    except (FileNotFoundError, ValueError):
        return []


def wait_for_count(talaria, count, timeout=10.0):
    """The star's write happens on the event loop a tick after the keypress
    lands, so every count here is polled rather than read once."""
    deadline = time.monotonic() + timeout
    entries = bookmarks(talaria)
    while time.monotonic() < deadline and len(entries) != count:
        time.sleep(0.2)
        entries = bookmarks(talaria)
    return entries


home, config, talaria = make_home("talaria-bookmarks-e2e-")

server = http.server.HTTPServer(("127.0.0.1", 0), Pages)
threading.Thread(target=server.serve_forever, daemon=True).start()
PAGE = f"http://127.0.0.1:{server.server_address[1]}/keep"

xvfb = harness.start_xvfb()
X = harness.x_env()
tal = None


def key(*args):
    subprocess.run(["xdotool"] + list(args), env=X)


try:
    tal = start(home, config, PAGE)

    wid = None
    for _ in range(15):
        out = subprocess.run(["xdotool", "search", "--name", "Talaria"], env=X,
                             capture_output=True, text=True).stdout.split()
        if out:
            wid = out[0]
            break
        time.sleep(1)
    assert wid, "Talaria window never appeared"

    def focus():
        """Focus synchronously before every synthetic keypress."""
        key("windowfocus", "--sync", wid)
        time.sleep(0.3)

    # The star reads the *displayed* tab's URL, so wait for the page to be
    # that tab's URL before pressing anything — otherwise the first Ctrl+D
    # could bookmark about:blank and the assertion below would be about the
    # wrong thing.
    for _ in range(40):
        r = rpc("tabs_list")
        if r["outcome"] == "ok" and any(
                t["owner"] == "me" and "/keep" in t["url"] for t in r["result"]["tabs"]):
            break
        time.sleep(0.5)
    else:
        raise AssertionError("the starting page never loaded in a me-owned tab")

    assert bookmarks(talaria) == [], "the isolated home already had bookmarks in it"

    # --- Ctrl+D bookmarks the displayed page ------------------------------
    focus()
    key("key", "ctrl+d")
    entries = wait_for_count(talaria, 1)
    assert len(entries) == 1, entries
    assert entries[0]["url"] == PAGE, entries
    assert entries[0]["title"] == TITLE, entries
    assert entries[0]["created_at_ms"] > 0, entries
    print("BOOKMARKED", entries[0]["url"], "-", entries[0]["title"])

    # --- Ctrl+D again removes it rather than duplicating it ---------------
    # BROWSE-02's toggle: the same URL is never stored twice, and the second
    # press is a remove, not a second add and not a title overwrite.
    focus()
    key("key", "ctrl+d")
    entries = wait_for_count(talaria, 0)
    assert entries == [], ("the second Ctrl+D did not remove the bookmark", entries)
    print("TOGGLED off — the list is empty again")

    # --- Ctrl+D a third time bookmarks it again ---------------------------
    focus()
    key("key", "ctrl+d")
    entries = wait_for_count(talaria, 1)
    assert len(entries) == 1, entries
    assert entries[0]["url"] == PAGE, entries
    print("TOGGLED back on — one entry")

    # --- Ctrl+B opens the panel without wedging the shell -----------------
    # The panel's contents are a human-judgment check (03-VALIDATION.md); what
    # is automatable here is that the shortcut is survivable, which is worth
    # asserting because the panel replaces the page and reroutes input.
    focus()
    key("key", "ctrl+b")
    time.sleep(1.5)
    r = rpc("tabs_list")
    assert r["outcome"] == "ok", ("the shell stopped answering after Ctrl+B", r)
    assert len(bookmarks(talaria)) == 1, "opening the panel changed the store"
    print("PANEL Ctrl+B — shell still answering,", len(r["result"]["tabs"]), "tabs")

    # --- restart survival: BROWSE-02's literal success criterion ----------
    stop_shell(tal)
    tal = start(home, config, PAGE)
    entries = bookmarks(talaria)
    assert len(entries) == 1, entries
    assert entries[0]["url"] == PAGE, entries
    assert entries[0]["title"] == TITLE, entries
    r = rpc("tabs_list")
    assert r["outcome"] == "ok", r
    print("RESTART the bookmark survived")

    print("BOOKMARKS CHECKS PASSED")
finally:
    if tal is not None:
        stop_shell(tal)
    xvfb.kill()
    xvfb.wait()
    server.shutdown()
    shutil.rmtree(home, ignore_errors=True)

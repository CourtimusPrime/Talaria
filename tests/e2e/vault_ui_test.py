#!/usr/bin/env python3
"""E2E: the credentials capture path, driven through the real chrome (CRED-03).

Plan 02-09 made the vault writable; nothing in the browser called that API, so
``cookies_read`` returned ``{"entries": []}`` on every fresh install and one of
the product's headline features was inert. This suite proves the other half:
a credential a *user types into Talaria* reaches the encrypted vault and comes
back out of the same command an agent uses.

What this pins down, in order:

- the credentials panel opens from the toolbar and the shell stays live while
  it is up (a screenshot over the socket still answers);
- a site, a username and a password typed into the panel's three fields and
  saved are returned by ``cookies_read`` for that host — chrome input, UI
  intent, ``Vault::upsert``, encrypted write, and the agent-facing read path,
  end to end;
- the save is durable: a fresh shell on the same home still returns it;
- deleting the row through the panel makes ``cookies_read`` return nothing.

Every assertion above is made over the control socket. The *input* mechanism is
not the deliverable and is deliberately kept as coordinate-free as it can be:
the toolbar button is found by name through the ``chrome_rects`` test hook and
clicked where the chrome says it is, and everything inside the panel is driven
by Tab and Enter from the field the panel focuses on open. A panel row's
vertical position depends on whether a one-shot vault notice is showing, which
depends on whether *this machine* has a usable keychain — not something a
committed test may depend on.

Standalone (starts its own Xvfb + shell, with ``TALARIA_TEST_HOOKS=1`` so the
rect lookup answers): it needs exclusive input focus, and an isolated home,
because the vault, the key file and the engine profile all live under the
platform config directory.

The password is generated per run rather than hardcoded, so nothing
credential-shaped is committed.
"""
import base64
import json
import os
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness

# The key-glyph button in the toolbar, looked up by name rather than
# hardcoded. It used to be a coordinate that tracked the toolbar's control
# order — Me / Agents / separator / back / forward / reload / new tab /
# separator / history / bookmark-star / bookmarks / downloads / settings /
# **credentials** — so every plan that inserted a control ahead of it moved
# the constant, and every one of those moves was discovered as a red suite
# (239 -> 283 -> 341 -> 370 -> 399, four times in Phase 3 alone). The shell
# now reports the rect it actually laid the button out at, so the next
# inserted button costs nothing here.
CREDENTIALS_BUTTON = "toolbar.credentials"

# Tab presses from the site field to the first stored row's delete button:
# username, password, reveal, delete. The Save button sits between password and
# reveal but is disabled on a freshly-opened panel (the drafts are empty), and
# egui does not give keyboard focus to a disabled widget.
TABS_TO_DELETE = 4

SITE = "vault-ui.example"
USERNAME = "capture-user"
PASSWORD = secrets.token_hex(16)


def make_home():
    home = tempfile.mkdtemp(prefix="talaria-vault-ui-e2e-")
    config = os.path.join(home, ".config")
    os.makedirs(os.path.join(config, "talaria"))
    return home, config


def start(home, config):
    return harness.start_shell("about:blank", rust_log="warn",
                               HOME=home, XDG_CONFIG_HOME=config,
                               TALARIA_TEST_HOOKS="1")


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


def rpc(command, **params):
    """One request on its own connection — the suite restarts the shell
    between phases, so a long-lived wire would not survive."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(harness.SOCK)
    try:
        f = s.makefile("rw")
        f.write(json.dumps({"type": "hello", "client": "vault-ui-e2e"}) + "\n")
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


def read(domain):
    r = rpc("cookies_read", domain=domain)
    assert r["outcome"] == "ok", r
    return r["result"]["entries"]


home, config = make_home()
xvfb = harness.start_xvfb()
tal = None
try:
    tal = start(home, config)
    X = harness.x_env()

    def key(*args):
        subprocess.run(["xdotool"] + list(args), env=X)

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
        """Focus synchronously before every synthetic input."""
        key("windowfocus", "--sync", wid)
        time.sleep(0.3)

    def open_panel():
        """Click the real toolbar button, at the rect the chrome reports for
        it — still a genuine pointer click on the button this suite's
        docstring claims to cover, just no longer at a guessed coordinate."""
        harness.click_rect(CREDENTIALS_BUTTON, wid, X)

    assert read(SITE) == [], "the isolated home already had credentials in it"

    # --- open the panel --------------------------------------------------
    open_panel()
    # The panel replaces the page, so this is a liveness check, not a pixel
    # comparison: the shell is still serving the socket with the panel up.
    tab_id = rpc("tabs_list")["result"]["tabs"][0]["tab_id"]
    shot = rpc("screenshot", tab_id=tab_id)
    assert shot["outcome"] == "ok", shot
    png = base64.b64decode(shot["result"]["png_base64"])
    print("PANEL open, shell still live:", len(png), "bytes of screenshot")

    # --- type a credential and save it -----------------------------------
    # The panel puts the caret in the site field when it opens, so the whole
    # form is reachable with Tab from here.
    focus()
    key("type", "--delay", "40", SITE)
    time.sleep(0.3)
    key("key", "Tab")
    time.sleep(0.3)
    key("type", "--delay", "40", USERNAME)
    time.sleep(0.3)
    key("key", "Tab")
    time.sleep(0.3)
    key("type", "--delay", "40", PASSWORD)
    time.sleep(0.5)
    key("key", "Return")  # Enter in the password field is the save
    time.sleep(2)

    entries = read(SITE)
    assert len(entries) == 1, entries
    assert entries[0]["username"] == USERNAME, entries
    assert entries[0]["password"] == PASSWORD, \
        "the credential typed into the chrome did not reach the vault"
    print("SAVED and readable through cookies_read:", SITE, entries[0]["username"])

    # --- durable across a restart ----------------------------------------
    stop_shell(tal)
    tal = start(home, config)

    entries = read(SITE)
    assert len(entries) == 1, entries
    assert entries[0]["password"] == PASSWORD, \
        "the credential did not survive a restart — it was only ever in memory"
    print("RESTART the saved credential is still there")

    # --- delete it from the panel ----------------------------------------
    wid = None
    for _ in range(15):
        out = subprocess.run(["xdotool", "search", "--name", "Talaria"], env=X,
                             capture_output=True, text=True).stdout.split()
        if out:
            wid = out[0]
            break
        time.sleep(1)
    assert wid, "the restarted Talaria window never appeared"

    open_panel()
    focus()
    for _ in range(TABS_TO_DELETE):
        key("key", "Tab")
        time.sleep(0.3)
    key("key", "Return")
    time.sleep(2)

    assert read(SITE) == [], \
        "deleting the row in the panel left the credential in the vault"
    print("DELETED the row is gone from cookies_read")

    print("VAULT UI CHECKS PASSED")
finally:
    if tal is not None:
        stop_shell(tal)
    xvfb.kill()
    xvfb.wait()
    shutil.rmtree(home, ignore_errors=True)

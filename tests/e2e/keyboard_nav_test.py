#!/usr/bin/env python3
"""Keyboard chrome navigation: Ctrl+T / Ctrl+L+Enter / Ctrl+W via xdotool.
Standalone: starts its own Xvfb + talaria."""
import os, subprocess, time, socket, json, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness

xvfb = harness.start_xvfb()
tal = harness.start_shell("https://example.com")
X = harness.x_env()

def key(*args):
    subprocess.run(["xdotool"] + list(args), env=X)

def tabs():
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(harness.SOCK)
    f = s.makefile("rw")
    f.write(json.dumps({"type": "hello", "client": "kbd-test"}) + "\n"); f.flush(); f.readline()
    f.write(json.dumps({"type": "request", "id": 1, "command": "tabs_list"}) + "\n"); f.flush()
    return json.loads(f.readline())["result"]["tabs"]

try:
    wid = None
    for _ in range(15):
        out = subprocess.run(["xdotool", "search", "--name", "Talaria"], env=X,
                             capture_output=True, text=True).stdout.split()
        if out:
            wid = out[0]
            break
        time.sleep(1)
    assert wid, "Talaria window never appeared"
    key("windowfocus", "--sync", wid); time.sleep(0.5)
    n0 = len([t for t in tabs() if t["owner"] == "me"])
    key("key", "ctrl+t"); time.sleep(2)
    n1 = len([t for t in tabs() if t["owner"] == "me"])
    assert n1 == n0 + 1, (n0, n1)
    print("ctrl+t opens a tab")
    key("key", "ctrl+l"); time.sleep(1)
    key("key", "ctrl+a"); key("type", "--delay", "30", "example.com"); time.sleep(0.5)
    key("key", "Return"); time.sleep(6)
    assert any("example.com" in t["url"] for t in tabs() if t["owner"] == "me")
    print("ctrl+l + type + enter navigates")
    def focused_url():
        return next(t["url"] for t in tabs() if t["owner"] == "me" and t["focused"])
    key("key", "alt+Left"); time.sleep(4)
    assert "example.com" not in focused_url(), focused_url()
    print("alt+left goes back")
    key("key", "alt+Right"); time.sleep(4)
    assert "example.com" in focused_url(), focused_url()
    print("alt+right goes forward")
    key("key", "ctrl+w"); time.sleep(2)
    n2 = len([t for t in tabs() if t["owner"] == "me"])
    assert n2 == n1 - 1, (n1, n2)
    print("ctrl+w closes the tab")
    print("KEYBOARD NAV CHECKS PASSED")
finally:
    harness.stop(tal, xvfb)

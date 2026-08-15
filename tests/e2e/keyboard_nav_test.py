#!/usr/bin/env python3
"""Keyboard chrome navigation: Ctrl+T / Ctrl+L+Enter / Ctrl+W via xdotool.
Standalone: starts its own Xvfb + talaria."""
import os, subprocess, time, socket, json, sys

REPO = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
subprocess.run(["pkill", "-f", "[X]vfb :99"], check=False)
subprocess.run(["pkill", "-x", "talaria"], check=False)
time.sleep(0.5)
xvfb = subprocess.Popen(["Xvfb", ":99", "-screen", "0", "1280x800x24"], stderr=subprocess.DEVNULL)
time.sleep(1)
env = dict(os.environ, DISPLAY=":99", RUST_LOG="error")
tal = subprocess.Popen([os.path.join(REPO, "target/release/talaria"), "https://example.com"],
                       env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(8)
X = dict(os.environ, DISPLAY=":99")

def key(*args):
    subprocess.run(["xdotool"] + list(args), env=X)

def tabs():
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(os.environ.get("XDG_RUNTIME_DIR", "/tmp") + "/talaria.sock")
    f = s.makefile("rw")
    f.write(json.dumps({"type": "hello", "client": "kbd-test"}) + "\n"); f.flush(); f.readline()
    f.write(json.dumps({"type": "request", "id": 1, "command": "tabs_list"}) + "\n"); f.flush()
    return json.loads(f.readline())["result"]["tabs"]

try:
    wid = subprocess.run(["xdotool", "search", "--name", "Talaria"], env=X,
                         capture_output=True, text=True).stdout.split()[0]
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
    key("key", "ctrl+w"); time.sleep(2)
    n2 = len([t for t in tabs() if t["owner"] == "me"])
    assert n2 == n1 - 1, (n1, n2)
    print("ctrl+w closes the tab")
    print("KEYBOARD NAV CHECKS PASSED")
finally:
    tal.terminate(); time.sleep(1); tal.kill(); xvfb.kill()

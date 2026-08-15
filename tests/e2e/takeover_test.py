#!/usr/bin/env python3
"""Takeover core: switch to the Agents view via a real mouse click, then a
human click inside the agent's tab navigates that tab. Standalone (starts
Xvfb + talaria)."""
import json, os, socket, subprocess, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness

xvfb = harness.start_xvfb()
tal = harness.start_shell("https://servo.org")
X = harness.x_env()

try:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(harness.SOCK)
    f = s.makefile("rw")
    f.write(json.dumps({"type": "hello", "client": "takeover-test"}) + "\n"); f.flush(); f.readline()
    rid = 0

    def rpc(command, **params):
        global rid
        rid += 1
        f.write(json.dumps({"type": "request", "id": rid, "command": command, **params}) + "\n")
        f.flush()
        while True:
            m = json.loads(f.readline())
            if m.get("type") == "reply" and m.get("id") == rid:
                return m

    tid = rpc("tabs_open", url="https://example.com/")["result"]["tab"]["tab_id"]
    time.sleep(6)
    wid = None
    for _ in range(15):
        out = subprocess.run(["xdotool", "search", "--name", "Talaria"], env=X,
                             capture_output=True, text=True).stdout.split()
        if out:
            wid = out[0]
            break
        time.sleep(1)
    assert wid, "Talaria window never appeared"
    subprocess.run(["xdotool", "windowfocus", "--sync", wid], env=X)
    subprocess.run(["xdotool", "mousemove", "65", "11", "click", "1"], env=X)
    time.sleep(3)
    r = rpc("evaluate", tab_id=tid,
            script="const a=document.querySelector('a'); const r=a.getBoundingClientRect(); "
                   "[r.x+r.width/2, r.y+r.height/2]")
    lx, ly = r["result"]["value"]
    subprocess.run(["xdotool", "mousemove", str(int(lx)), str(int(ly) + 45)], env=X)
    time.sleep(0.5)
    subprocess.run(["xdotool", "click", "1"], env=X)
    time.sleep(6)
    url = next(t["url"] for t in rpc("tabs_list")["result"]["tabs"] if t["tab_id"] == tid)
    assert "iana.org" in url, url
    print("human click navigated the agent tab:", url)
    print("TAKEOVER CHECKS PASSED")
finally:
    harness.stop(tal, xvfb)

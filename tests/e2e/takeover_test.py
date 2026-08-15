#!/usr/bin/env python3
"""Takeover core: switch to the Agents view via a real mouse click, then a
human click inside the agent's tab navigates that tab. Standalone (starts
Xvfb + talaria)."""
import json, os, socket, subprocess, sys, time

REPO = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
subprocess.run(["pkill", "-f", "[X]vfb :99"], check=False)
subprocess.run(["pkill", "-x", "talaria"], check=False)
time.sleep(0.5)
xvfb = subprocess.Popen(["Xvfb", ":99", "-screen", "0", "1280x800x24"], stderr=subprocess.DEVNULL)
time.sleep(1)
env = dict(os.environ, DISPLAY=":99", RUST_LOG="error")
tal = subprocess.Popen([os.path.join(REPO, "target/release/talaria"), "https://servo.org"],
                       env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(8)
X = dict(os.environ, DISPLAY=":99")

try:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(os.environ.get("XDG_RUNTIME_DIR", "/tmp") + "/talaria.sock")
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
    wid = subprocess.run(["xdotool", "search", "--name", "Talaria"], env=X,
                         capture_output=True, text=True).stdout.split()[0]
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
    tal.terminate(); time.sleep(1); tal.kill(); xvfb.kill()

#!/usr/bin/env python3
"""Orchestrate: Xvfb + talaria + control-socket e2e + chrome screenshots."""
import os
import signal
import subprocess
import sys
import time

T = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
DISPLAY = ":99"

subprocess.run(["pkill", "-f", "[X]vfb :99"], check=False)
subprocess.run(["pkill", "-x", "talaria"], check=False)
time.sleep(0.5)

xvfb = subprocess.Popen(["Xvfb", DISPLAY, "-screen", "0", "1280x800x24"],
                        stderr=subprocess.DEVNULL)
time.sleep(1)

env = dict(os.environ, DISPLAY=DISPLAY, RUST_LOG="warn")
tal_log = open(os.path.join(T, "talaria.log"), "w")
talaria = subprocess.Popen([os.path.join(REPO, "target/release/talaria"),
                            "https://servo.org"], env=env,
                           stdout=tal_log, stderr=tal_log)
time.sleep(12)

def shot(name):
    xwd = os.path.join(T, name + ".xwd")
    png = os.path.join(T, name + ".png")
    subprocess.run(["xwd", "-root", "-silent", "-out", xwd],
                   env=dict(os.environ, DISPLAY=DISPLAY), check=True)
    subprocess.run(["convert", xwd, png], check=True)
    print("shot:", png)

rc = 1
try:
    if talaria.poll() is not None:
        print("TALARIA DIED EARLY, exit", talaria.returncode)
        sys.exit(2)
    shot("chrome-me")
    rc = subprocess.run([sys.executable, os.path.join(os.path.dirname(os.path.abspath(__file__)), "control_socket_test.py"), T]).returncode
    print("e2e exit:", rc)
    # Agent tab open during e2e was closed at the end; reopen one so the
    # Agents view has content, then click the Agents toggle and screenshot.
    if rc == 0:
        import base64, json, socket
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect(os.environ.get("XDG_RUNTIME_DIR", "/tmp") + "/talaria.sock")
        f = s.makefile("rw")
        def send(o):
            f.write(json.dumps(o) + "\n"); f.flush()
        send({"type": "hello", "client": "screenshot-agent"})
        f.readline()
        send({"type": "request", "id": 1, "command": "tabs_open",
              "url": "https://example.com/"})
        f.readline()
        time.sleep(6)
        # Click the "Agents" toggle (approx 60,18 logical -> same in px here).
        subprocess.run(["xdotool", "mousemove", "60", "16", "click", "1"],
                       env=dict(os.environ, DISPLAY=DISPLAY), check=False)
        time.sleep(2)
        shot("chrome-agents")
finally:
    talaria.send_signal(signal.SIGTERM)
    time.sleep(1)
    talaria.kill()
    xvfb.kill()
    tal_log.close()

print("tail of talaria.log:")
print("".join(open(os.path.join(T, "talaria.log")).readlines()[-8:]))
sys.exit(rc)

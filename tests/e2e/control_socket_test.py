#!/usr/bin/env python3
"""E2E: drive Talaria's control socket like an agent would."""
import base64
import json
import os
import socket
import sys
import time

SOCK = os.environ.get("XDG_RUNTIME_DIR", "/tmp") + "/talaria.sock"
OUT = sys.argv[1] if len(sys.argv) > 1 else "/tmp/talaria-e2e"
os.makedirs(OUT, exist_ok=True)

s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
for attempt in range(30):
    try:
        s.connect(SOCK)
        break
    except OSError:
        time.sleep(1)
else:
    sys.exit("could not connect to " + SOCK)

f = s.makefile("rw")
req_id = 0

def send(obj):
    f.write(json.dumps(obj) + "\n")
    f.flush()

def rpc(command, **params):
    global req_id
    req_id += 1
    send({"type": "request", "id": req_id, "command": command, **params})
    while True:
        line = f.readline()
        if not line:
            sys.exit("connection closed")
        msg = json.loads(line)
        if msg.get("type") == "reply" and msg.get("id") == req_id:
            return msg

send({"type": "hello", "client": "e2e-test"})
ack = json.loads(f.readline())
assert ack["type"] == "hello_ack", ack
print("HELLO_ACK session", ack["session_id"])

r = rpc("tabs_list")
print("TABS_LIST:", json.dumps(r)[:200])
assert r["outcome"] == "ok" and len(r["result"]["tabs"]) == 1

r = rpc("tabs_open", url="https://example.com/")
print("TABS_OPEN:", json.dumps(r)[:200])
assert r["outcome"] == "ok"
tab_id = r["result"]["tab"]["tab_id"]

time.sleep(8)  # let it load

r = rpc("evaluate", tab_id=tab_id, script="document.title")
print("EVALUATE title:", json.dumps(r)[:200])
assert r["outcome"] == "ok", r

r = rpc("evaluate", tab_id=tab_id,
        script="document.querySelector('h1').textContent")
print("EVALUATE h1:", json.dumps(r)[:200])

r = rpc("evaluate", tab_id=tab_id, script="1 + 41")
print("EVALUATE math:", json.dumps(r)[:200])
assert r["outcome"] == "ok" and r["result"]["value"] == 42, r

r = rpc("screenshot", tab_id=tab_id)
assert r["outcome"] == "ok", json.dumps(r)[:300]
png = base64.b64decode(r["result"]["png_base64"])
path = os.path.join(OUT, "agent-screenshot.png")
open(path, "wb").write(png)
print("SCREENSHOT:", len(png), "bytes ->", path,
      r["result"]["width"], "x", r["result"]["height"])

r = rpc("cookies_read", domain="example.com")
print("COOKIES_READ:", json.dumps(r)[:200])
assert r["outcome"] == "ok"

r = rpc("tabs_list")
tabs = r["result"]["tabs"]
print("TABS_LIST after:", json.dumps(tabs)[:400])
assert len(tabs) == 2
owners = sorted(t["owner"] for t in tabs)
assert owners == ["e2e-test", "me"], owners

r = rpc("navigate", tab_id=tab_id, url="https://servo.org/")
print("NAVIGATE:", json.dumps(r)[:150])
assert r["outcome"] == "ok"

r = rpc("tabs_close", tab_id=tab_id)
print("TABS_CLOSE:", json.dumps(r)[:150])
assert r["outcome"] == "ok"

r = rpc("tabs_list")
assert len(r["result"]["tabs"]) == 1

print("ALL CONTROL-SOCKET CHECKS PASSED")

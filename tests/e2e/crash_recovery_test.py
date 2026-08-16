#!/usr/bin/env python3
"""Crash-recovery path: simulate a WebContent crash (via the
TALARIA_TEST_HOOKS evaluate hook), check agent-facing behavior, recover
via navigate. Expects a running shell started with TALARIA_TEST_HOOKS=1."""
import json
import os
import socket
import sys
import time

SOCK = os.environ.get("XDG_RUNTIME_DIR",
                      f"{os.environ.get('TMPDIR', '/tmp')}/talaria-{os.getuid()}") \
    + "/talaria.sock"

s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
for _ in range(30):
    try:
        s.connect(SOCK)
        break
    except OSError:
        time.sleep(1)
else:
    sys.exit("could not connect")

f = s.makefile("rw")
req_id = 0

def send(o):
    f.write(json.dumps(o) + "\n")
    f.flush()

def rpc(command, **params):
    global req_id
    req_id += 1
    send({"type": "request", "id": req_id, "command": command, **params})
    while True:
        msg = json.loads(f.readline())
        if msg.get("type") == "reply" and msg.get("id") == req_id:
            return msg

send({"type": "hello", "client": "crash-test"})
assert json.loads(f.readline())["type"] == "hello_ack"

r = rpc("tabs_open", url="https://example.com/")
tab_id = r["result"]["tab"]["tab_id"]
time.sleep(6)

r = rpc("evaluate", tab_id=tab_id, script="__talaria_sim_crash__")
assert r["outcome"] == "ok", r
print("crash simulated on tab", tab_id)

r = rpc("tabs_list")
tab = next(t for t in r["result"]["tabs"] if t["tab_id"] == tab_id)
assert tab["crashed"] is True, tab
print("tabs_list reports crashed=true")

r = rpc("evaluate", tab_id=tab_id, script="1+1")
assert r["outcome"] == "error" and "crashed" in r["message"], r
print("evaluate on crashed tab -> error:", r["message"])

r = rpc("screenshot", tab_id=tab_id)
assert r["outcome"] == "error" and "crashed" in r["message"], r
print("screenshot on crashed tab -> error")

r = rpc("navigate", tab_id=tab_id, url="https://example.com/")
assert r["outcome"] == "ok", r
time.sleep(5)

r = rpc("tabs_list")
tab = next(t for t in r["result"]["tabs"] if t["tab_id"] == tab_id)
assert tab["crashed"] is False, tab
print("navigate recovered the tab (crashed=false)")

r = rpc("evaluate", tab_id=tab_id, script="document.title")
assert r["outcome"] == "ok" and r["result"]["value"] == "Example Domain", r
print("evaluate works again after recovery")

rpc("tabs_close", tab_id=tab_id)
print("CRASH-RECOVERY CHECKS PASSED")

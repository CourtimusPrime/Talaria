#!/usr/bin/env python3
"""Command timeout (hung evaluate) + agent-tab persistence across session
end. Expects a running shell with TALARIA_COMMAND_TIMEOUT_SECS=3."""
import json, os, socket, time

SOCK = os.environ.get("XDG_RUNTIME_DIR",
                      f"{os.environ.get('TMPDIR', '/tmp')}/talaria-{os.getuid()}") \
    + "/talaria.sock"
rid = 0

def conn(name):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(SOCK)
    f = s.makefile("rw")
    f.write(json.dumps({"type": "hello", "client": name}) + "\n"); f.flush(); f.readline()
    return s, f

def rpc(f, command, **params):
    global rid
    rid += 1
    f.write(json.dumps({"type": "request", "id": rid, "command": command, **params}) + "\n")
    f.flush()
    while True:
        m = json.loads(f.readline())
        if m.get("type") == "reply" and m.get("id") == rid:
            return m

s1, f1 = conn("hang-test")
tid = rpc(f1, "tabs_open", url="https://example.com/")["result"]["tab"]["tab_id"]
time.sleep(5)
t0 = time.monotonic()
r = rpc(f1, "evaluate", tab_id=tid, script="while(true){}")
dt = time.monotonic() - t0
assert r["outcome"] == "error" and 2.5 < dt < 6, (r, dt)
print(f"hung evaluate -> error in {dt:.1f}s")
assert rpc(f1, "tabs_list")["outcome"] == "ok"
print("connection usable after timeout")
s1.close()
time.sleep(1)
_, f2 = conn("session-check")
owners = [t["owner"] for t in rpc(f2, "tabs_list")["result"]["tabs"]]
assert "hang-test" in owners, owners
print("agent tab persists after session end")
print("TIMEOUT + SESSION CHECKS PASSED")

#!/usr/bin/env python3
"""Command timeout (hung evaluate), connection pipelining, and agent-tab
persistence across session end.

The pipelining half is the shell-side proof for MCP-11: two request lines are
written back to back on one connection — a never-terminating evaluate, then a
tabs_list — and the tabs_list must reply first and long before the evaluate's
timeout. Before that change the shell awaited each outcome before reading the
next line, so one wedged tab stalled everything else on the session for the
full command timeout. Writing both lines before reading either is legal: the
protocol IDs every request and documents out-of-order replies.

Expects a running shell with TALARIA_COMMAND_TIMEOUT_SECS=3."""
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

def send(f, command, **params):
    """Write one request line without waiting for its reply."""
    global rid
    rid += 1
    f.write(json.dumps({"type": "request", "id": rid, "command": command, **params}) + "\n")
    f.flush()
    return rid

def next_reply(f):
    """The next reply in arrival order, skipping any unsolicited event."""
    while True:
        m = json.loads(f.readline())
        if m.get("type") == "reply":
            return m

def reply(f, want):
    while True:
        m = next_reply(f)
        if m["id"] == want:
            return m

def rpc(f, command, **params):
    return reply(f, send(f, command, **params))

s1, f1 = conn("hang-test")
tid = rpc(f1, "tabs_open", url="https://example.com/")["result"]["tab"]["tab_id"]
time.sleep(5)

# Both lines out before either reply is read.
t0 = time.monotonic()
hung = send(f1, "evaluate", tab_id=tid, script="while(true){}")
listed = send(f1, "tabs_list")
first = next_reply(f1)
tl = time.monotonic() - t0
assert first["id"] == listed and first["outcome"] == "ok", first
r = reply(f1, hung)
dt = time.monotonic() - t0
assert r["outcome"] == "error" and 2.5 < dt < 6, (r, dt)
# Both ends: the tabs_list landed while the evaluate was still outstanding,
# and by a margin no ordering fluke accounts for.
assert tl < 1.5 and dt - tl > 1.5, (tl, dt)
print(f"tabs_list overtook the wedged evaluate: {tl:.1f}s vs {dt:.1f}s")
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

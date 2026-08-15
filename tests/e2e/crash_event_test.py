#!/usr/bin/env python3
"""Unsolicited events over the control socket: a second client observes
tab_crashed and tab_closed events for a tab it doesn't own. Expects a running
shell with TALARIA_TEST_HOOKS=1."""
import json, os, socket, time

SOCK = os.environ.get("XDG_RUNTIME_DIR", "/tmp") + "/talaria.sock"

def conn(name):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(SOCK)
    f = s.makefile("rw")
    f.write(json.dumps({"type": "hello", "client": name}) + "\n"); f.flush(); f.readline()
    return s, f

rid = 0
def rpc(f, command, **params):
    global rid
    rid += 1
    f.write(json.dumps({"type": "request", "id": rid, "command": command, **params}) + "\n")
    f.flush()
    while True:
        m = json.loads(f.readline())
        if m.get("type") == "reply" and m.get("id") == rid:
            return m

_, actor = conn("event-actor")
watcher_sock, watcher = conn("event-watcher")
watcher_sock.settimeout(10)

tid = rpc(actor, "tabs_open", url="https://example.com/")["result"]["tab"]["tab_id"]
time.sleep(4)

rpc(actor, "evaluate", tab_id=tid, script="__talaria_sim_crash__")
event = json.loads(watcher.readline())
print("watcher got:", event)
assert event == {"type": "event", "event": "tab_crashed", "tab_id": tid}, event
print("tab_crashed event delivered to non-owner client")

rpc(actor, "tabs_close", tab_id=tid)
event = json.loads(watcher.readline())
print("watcher got:", event)
assert event == {"type": "event", "event": "tab_closed", "tab_id": tid}, event
print("tab_closed event delivered")
print("CRASH EVENT CHECKS PASSED")

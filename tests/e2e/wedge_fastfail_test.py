#!/usr/bin/env python3
"""Same-tab fail-fast on a wedged page, and the takeover route surviving it.

Two things are pinned down here. First, a second evaluate against a tab whose
script thread is already busy is refused immediately with a distinct busy
error naming the tab, instead of queueing behind the first one and burning the
whole command timeout — that is MCP-10's same-tab half (02-05 closed the
cross-tab half). Second, the human's route back into a wedged tab stays open:
tabs_list, screenshot and tabs_focus all still answer while the wedged
evaluate is outstanding, because a page that has wedged itself is exactly when
a takeover is needed.

The blanket command timeout is still the outer bound, not something fail-fast
replaced: the first evaluate is read back at the end and must still end in the
timeout error, and a further evaluate afterwards must no longer be refused as
busy.

Writing two request lines before reading either reply is legal and is the only
way to get two evaluates genuinely in flight: the protocol IDs every request
and documents out-of-order replies.

Expects a running shell with TALARIA_COMMAND_TIMEOUT_SECS=3."""
import base64, json, os, socket, time

SOCK = os.environ.get("XDG_RUNTIME_DIR",
                      f"{os.environ.get('TMPDIR', '/tmp')}/talaria-{os.getuid()}") \
    + "/talaria.sock"
TIMEOUT_SECS = int(os.environ.get("TALARIA_COMMAND_TIMEOUT_SECS", "3"))
BUSY = "busy — a previous evaluate is still running"
rid = 0
held = {}

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

def reply(f, want):
    """The reply for `want`, holding any other reply that overtakes it.

    Replies arrive out of order here by construction, and the wedged call's
    reply must survive being overtaken — it is read back at the end."""
    if want in held:
        return held.pop(want)
    while True:
        m = json.loads(f.readline())
        if m.get("type") != "reply":
            continue
        if m["id"] == want:
            return m
        held[m["id"]] = m

def rpc(f, command, **params):
    return reply(f, send(f, command, **params))

_, f = conn("wedge-fastfail")
tid = rpc(f, "tabs_open", url="https://example.com/")["result"]["tab"]["tab_id"]
time.sleep(4)

# FAIL FAST: two evaluates on the SAME tab, both lines out before either reply
# is read. The first never terminates, so servo never fires its callback.
t0 = time.monotonic()
hung = send(f, "evaluate", tab_id=tid, script="while(true){}")
second = send(f, "evaluate", tab_id=tid, script="1+1")
r = reply(f, second)
dt = time.monotonic() - t0
assert r["outcome"] == "error", r
assert BUSY in r["message"] and f"tab {tid} " in r["message"], r
# Bounded at both ends: a refusal, not a call that merely finished early, and
# far enough under the command timeout that no timeout accounts for it.
assert dt < 1.0, (dt, r)
print(f"second evaluate on the busy tab refused in {dt:.2f}s: {r['message']}")

# TAKEOVER ROUTE (D-07): every non-script command still answers on the wedged
# tab while its evaluate is outstanding. If the busy check is ever generalised
# across the dispatch, these are what fail.
listed = rpc(f, "tabs_list")
assert listed["outcome"] == "ok", listed
assert tid in [t["tab_id"] for t in listed["result"]["tabs"]], listed
print("tabs_list still answers and lists the wedged tab")

shot = rpc(f, "screenshot", tab_id=tid)
assert shot["outcome"] == "ok", json.dumps(shot)[:300]
png = base64.b64decode(shot["result"]["png_base64"])
assert len(png) > 1000 and png[:8] == b"\x89PNG\r\n\x1a\n", len(png)
print(f"screenshot still answers on the wedged tab: {len(png)} bytes")

focused = rpc(f, "tabs_focus", tab_id=tid)
assert focused["outcome"] == "ok", focused
print("tabs_focus still answers on the wedged tab")

held_dt = time.monotonic() - t0
assert hung not in held and held_dt < TIMEOUT_SECS, (held_dt, held)
print(f"all three answered while the wedged evaluate was still outstanding "
      f"({held_dt:.1f}s < {TIMEOUT_SECS}s)")

# TIMEOUT IS STILL THE OUTER BOUND (D-06): fail-fast was added in front of it,
# so the first evaluate still ends in the existing timeout error.
r = reply(f, hung)
dt = time.monotonic() - t0
assert r["outcome"] == "error" and "timed out" in r["message"], r
assert TIMEOUT_SECS - 0.5 < dt < TIMEOUT_SECS + 3, (dt, r)
print(f"wedged evaluate still ends in the timeout error at {dt:.1f}s: {r['message']}")

# RECOVERY: the in-flight entry is gone, so a later evaluate is not refused as
# busy. It may still fail for a page reason — the tab's script thread is wedged
# for good — but not with the busy wording.
r = rpc(f, "evaluate", tab_id=tid, script="1+1")
assert BUSY not in r.get("message", ""), r
print(f"tab no longer refused as busy after the timeout: {json.dumps(r)[:120]}")

assert rpc(f, "tabs_close", tab_id=tid)["outcome"] == "ok"
print("wedged tab closed")
print("WEDGE FASTFAIL CHECKS PASSED")

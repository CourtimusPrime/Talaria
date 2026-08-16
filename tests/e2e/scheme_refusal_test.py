#!/usr/bin/env python3
"""Scheme allowlist on the agent URL path (MCP-09): tabs_open and navigate
refuse file:, javascript:, blob: and every other scheme outside the allowlist,
name the rejected scheme in the error, and leave nothing opened or
half-navigated behind. http, https, about:blank and data: still work —
about:blank because popup adoption depends on that exact literal. Also pins
the human path as deliberately unrestricted: open_for_user goes through
resolve_location rather than parse_agent_url and still opens a local file.
Expects a running shell."""
import json, os, socket

SOCK = os.environ.get("XDG_RUNTIME_DIR",
                      f"{os.environ.get('TMPDIR', '/tmp')}/talaria-{os.getuid()}") \
    + "/talaria.sock"
OUT = os.path.abspath(os.environ.get("TALARIA_E2E_OUT", "/tmp/talaria-e2e"))

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

def tabs(f):
    return rpc(f, "tabs_list")["result"]["tabs"]

def ids(f):
    return {t["tab_id"] for t in tabs(f)}

_, agent = conn("scheme-refusal")
opened = []
before = ids(agent)

# --- refusals: tabs_open ---------------------------------------------------
# /etc/hostname is world-readable on every Linux box, so a pass here means the
# refusal happened, not that the file was missing.
for url, scheme in [("file:///etc/hostname", "file"),
                    ("javascript:1+1", "javascript"),
                    ("blob:https://example.com/abc", "blob")]:
    r = rpc(agent, "tabs_open", url=url)
    assert r["outcome"] == "error", r
    assert f"scheme {scheme}" in r["message"], r
    assert "not allowed for agents" in r["message"], r
    assert ids(agent) == before, (url, tabs(agent))
    print(f"tabs_open refused {url}: {r['message']}")

# --- refusals: navigate ----------------------------------------------------
r = rpc(agent, "tabs_open", url="https://example.com/")
assert r["outcome"] == "ok", r
tid = r["result"]["tab"]["tab_id"]
opened.append(tid)

r = rpc(agent, "navigate", tab_id=tid, url="file:///etc/hostname")
assert r["outcome"] == "error", r
assert "scheme file" in r["message"], r
tab = next(t for t in tabs(agent) if t["tab_id"] == tid)
assert tab["url"] == "https://example.com/", tab
print("navigate refused file: and left the tab on", tab["url"])

# --- refusals are per-call, not shared state -------------------------------
# Three rejected-scheme requests written back to back before any reply is
# read: each one has to come back with its own refusal, whatever the order.
batch = [{"type": "request", "id": rid + 1, "command": "tabs_open",
          "url": "file:///etc/hostname"},
         {"type": "request", "id": rid + 2, "command": "navigate", "tab_id": tid,
          "url": "javascript:1"},
         {"type": "request", "id": rid + 3, "command": "tabs_open",
          "url": "blob:https://example.com/x"}]
for m in batch:
    agent.write(json.dumps(m) + "\n")
agent.flush()
seen = {}
while len(seen) < len(batch):
    m = json.loads(agent.readline())
    if m.get("type") == "reply":
        seen[m["id"]] = m
rid += len(batch)
for m in batch:
    r = seen[m["id"]]
    assert r["outcome"] == "error", (m, r)
    assert "not allowed for agents" in r["message"], (m, r)
assert ids(agent) == before | {tid}, tabs(agent)
tab = next(t for t in tabs(agent) if t["tab_id"] == tid)
assert tab["url"] == "https://example.com/", tab
print("interleaved rejected-scheme calls each got their own refusal")

# --- allowlist positives ---------------------------------------------------
r = rpc(agent, "tabs_open", url="about:blank")
assert r["outcome"] == "ok", r
assert r["result"]["tab"]["url"] == "about:blank", r
opened.append(r["result"]["tab"]["tab_id"])
print("tabs_open about:blank ok (popup adoption depends on this literal)")

r = rpc(agent, "tabs_open", url="data:text/html,<title>data page</title><h1>d</h1>")
assert r["outcome"] == "ok", r
assert r["result"]["tab"]["title"] == "data page", r
opened.append(r["result"]["tab"]["tab_id"])
print("tabs_open data: ok and the document rendered")

# --- the human path is not the agent path (D-02) ---------------------------
# open_for_user is the wire command a second `talaria <url>` launch sends; it
# resolves through resolve_location. If someone ever "simplifies" by pointing
# both paths at the allowlist, this is the assertion that fails.
os.makedirs(OUT, exist_ok=True)
page = os.path.join(OUT, "human-local-page.html")
open(page, "w").write("<!doctype html><title>human local page</title><h1>local</h1>")

r = rpc(agent, "open_for_user", url="file://" + page)
assert r["outcome"] == "ok", r
human = r["result"]["tab"]["tab_id"]
opened.append(human)
tab = next(t for t in tabs(agent) if t["tab_id"] == human)
assert tab["owner"] == "me", tab
assert tab["url"] == "file://" + page, tab
assert tab["title"] == "human local page", tab
print("open_for_user still opens a local file for the human:", tab["url"])

# --- leave the shared shell as it was found --------------------------------
for tab_id in opened:
    rpc(agent, "tabs_close", tab_id=tab_id)
assert ids(agent) == before, tabs(agent)

print("SCHEME REFUSAL CHECKS PASSED")

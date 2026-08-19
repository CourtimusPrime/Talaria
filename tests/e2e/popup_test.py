#!/usr/bin/env python3
"""E2E: pages that open other pages — `window.open` and `target=_blank`.

Same bug class as the link-click fix: servo's default delegate drops the
request, so page-driven tab creation was a silent no-op. The contract this
pins down:

- both routes create a real tab, owned by whoever owned the opener;
- the owner is told, unprompted, that the tab appeared and which page made it;
- the popup is reachable as a tab (evaluate / screenshot / close);
- the opener stays usable, and either side can outlive the other;
- `window.close()` from the popup removes the tab and emits `tab_closed`.

Socket-driven: runs against the shell run_all.py already has up. Pages come
from a local HTTP server so nothing here depends on the network.
"""
import base64
import http.server
import json
import os
import socket
import sys
import threading
import time

SOCK = os.environ.get("XDG_RUNTIME_DIR",
                      f"{os.environ.get('TMPDIR', '/tmp')}/talaria-{os.getuid()}") \
    + "/talaria.sock"

OPENER = b"""<!doctype html><title>opener</title>
<a id=lnk href="/child?via=link" target="_blank">open in a new tab</a>
"""
CHILD = b"""<!doctype html><title>popup child</title><h1 id=h>child page</h1>"""


class Pages(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = CHILD if self.path.startswith("/child") else OPENER
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


server = http.server.HTTPServer(("127.0.0.1", 0), Pages)
threading.Thread(target=server.serve_forever, daemon=True).start()
BASE = f"http://127.0.0.1:{server.server_address[1]}"

s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
for _ in range(30):
    try:
        s.connect(SOCK)
        break
    except OSError:
        time.sleep(1)
else:
    sys.exit("could not connect to " + SOCK)

f = s.makefile("rw")
req_id = 0
events = []


def rpc(command, **params):
    """Send a request; collect any events that arrive while we wait."""
    global req_id
    req_id += 1
    f.write(json.dumps({"type": "request", "id": req_id, "command": command, **params}) + "\n")
    f.flush()
    while True:
        line = f.readline()
        if not line:
            sys.exit("connection closed")
        msg = json.loads(line)
        if msg.get("type") == "event":
            events.append(msg)
        elif msg.get("type") == "reply" and msg.get("id") == req_id:
            return msg


def tabs():
    r = rpc("tabs_list")
    assert r["outcome"] == "ok", r
    return r["result"]["tabs"]


def wait_for_event(name, timeout=10, **fields):
    """Wait for an unsolicited event matching `name` and `fields`.

    Polls tabs_list purely to pump the socket — rpc() is what drains events
    off the shared stream. The event itself is not what polling produces; if
    it were, it would not be worth having."""
    deadline = time.monotonic() + timeout
    while True:
        for e in events:
            if e.get("event") == name and all(e.get(k) == v for k, v in fields.items()):
                events.remove(e)
                return e
        if time.monotonic() > deadline:
            raise AssertionError(f"no {name} event matching {fields} in {timeout}s: {events}")
        rpc("tabs_list")
        time.sleep(0.25)


def wait_for_tab(known, timeout=15):
    """Poll tabs_list until a tab id outside `known` shows up, loaded."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        fresh = [t for t in tabs() if t["tab_id"] not in known]
        if fresh and not fresh[0]["loading"]:
            return fresh[0]
        time.sleep(0.25)
    raise AssertionError(f"no new tab within {timeout}s (have {[t['tab_id'] for t in tabs()]})")


f.write(json.dumps({"type": "hello", "client": "e2e-popup"}) + "\n")
f.flush()
ack = json.loads(f.readline())
assert ack["type"] == "hello_ack", ack

r = rpc("tabs_open", url=BASE + "/opener")
assert r["outcome"] == "ok", r
parent = r["result"]["tab"]["tab_id"]
# Earlier suites in the same shell may have left agent tabs behind, so say
# outright which tab is active rather than assuming this one is.
assert rpc("tabs_focus", tab_id=parent)["outcome"] == "ok"
before = {t["tab_id"] for t in tabs()}
print("OPENER tab", parent)

# --- window.open ---------------------------------------------------------
r = rpc("evaluate", tab_id=parent, script=f"typeof window.open('{BASE}/child?via=open', '_blank')")
assert r["outcome"] == "ok", r
assert r["result"]["value"] == "object", r  # a real Window, not undefined
popup = wait_for_tab(before)
assert "via=open" in popup["url"], popup
assert popup["owner"] == "e2e-popup", popup  # inherits the opener's owner
assert popup["title"] == "popup child", popup
print("WINDOW_OPEN -> tab", popup["tab_id"], popup["url"])

# The adopted tab's id appears in no reply this session will ever receive, so
# the event is the only way to learn it without polling. It names the opener
# too, because that is the part `tabs_list` could never have supplied.
opened = wait_for_event("tab_opened", tab_id=popup["tab_id"], opener_tab_id=parent)
print("TAB_OPENED event:", json.dumps(opened))

# The opener was this agent's active tab, so its popup fronts the Agents view
# — without disturbing the human's Me view, which still has its own active tab.
assert popup["focused"] is True, popup
assert next(t for t in tabs() if t["tab_id"] == parent)["focused"] is False
assert any(t["focused"] and t["owner"] == "me" for t in tabs()), tabs()
print("POPUP is active in the agent view; Me view untouched")

# The popup is a first-class tab: scriptable and capturable.
r = rpc("evaluate", tab_id=popup["tab_id"], script="document.getElementById('h').textContent")
assert r["outcome"] == "ok" and r["result"]["value"] == "child page", r
r = rpc("screenshot", tab_id=popup["tab_id"])
assert r["outcome"] == "ok", json.dumps(r)[:300]
assert len(base64.b64decode(r["result"]["png_base64"])) > 0
print("POPUP evaluate + screenshot ok")

# The opener survives its popup and is still scriptable.
r = rpc("evaluate", tab_id=parent, script="document.title")
assert r["outcome"] == "ok" and r["result"]["value"] == "opener", r
print("OPENER still usable")

# --- target=_blank link click -------------------------------------------
# The opener is a background tab now (its own popup took the view), so this
# popup must stay behind rather than yanking the view a second time.
before = {t["tab_id"] for t in tabs()}
r = rpc("evaluate", tab_id=parent, script="document.getElementById('lnk').click(); 'clicked'")
assert r["outcome"] == "ok", r
blank = wait_for_tab(before)
assert wait_for_event("tab_opened", tab_id=blank["tab_id"], opener_tab_id=parent)
assert "via=link" in blank["url"], blank
assert blank["owner"] == "e2e-popup", blank
assert blank["focused"] is False, blank
assert next(t for t in tabs() if t["tab_id"] == popup["tab_id"])["focused"] is True
print("TARGET_BLANK -> tab", blank["tab_id"], blank["url"], "(stays behind)")

# --- window.close() from the popup --------------------------------------
events.clear()
r = rpc("evaluate", tab_id=blank["tab_id"], script="window.close(); 'bye'")
# The reply may be ok or an error depending on how fast the webview goes
# away; what matters is the tab leaving the table and the event firing.
deadline = time.monotonic() + 10
while time.monotonic() < deadline:
    if blank["tab_id"] not in {t["tab_id"] for t in tabs()}:
        break
    time.sleep(0.25)
else:
    raise AssertionError(f"tab {blank['tab_id']} survived window.close()")
closed = [e for e in events if e.get("event") == "tab_closed" and e.get("tab_id") == blank["tab_id"]]
assert closed, f"no tab_closed event for {blank['tab_id']}: {events}"
print("WINDOW_CLOSE removed the tab and pushed tab_closed")

# --- the popup outlives its opener --------------------------------------
r = rpc("tabs_close", tab_id=parent)
assert r["outcome"] == "ok", r
r = rpc("evaluate", tab_id=popup["tab_id"], script="1 + 41")
assert r["outcome"] == "ok" and r["result"]["value"] == 42, r
print("POPUP outlives its opener")

r = rpc("tabs_close", tab_id=popup["tab_id"])
assert r["outcome"] == "ok", r
assert popup["tab_id"] not in {t["tab_id"] for t in tabs()}

server.shutdown()
print("ALL POPUP CHECKS PASSED")

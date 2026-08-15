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
# tabs_open replies once the page has loaded: title known, loading false,
# and an immediate evaluate runs in the loaded document (no InternalError).
assert r["result"]["tab"]["title"] == "Example Domain", r
assert r["result"]["tab"]["loading"] is False, r

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
# navigate also waits for the load, and returns the tab as it now is.
assert r["result"]["tab"]["url"] == "https://servo.org/", r
assert r["result"]["tab"]["loading"] is False, r
r = rpc("evaluate", tab_id=tab_id, script="location.href")
assert r["outcome"] == "ok" and r["result"]["value"] == "https://servo.org/", r
print("EVALUATE right after navigate sees the new page")
r = rpc("evaluate", tab_id=tab_id, script="throw new Error('boom')")
assert r["outcome"] == "error" and "boom" in r["message"], r
print("EVALUATE error carries the script's message")

# Promises are awaited; top-level await works in both single-expression and
# multi-statement (return) forms; rejections become errors.
r = rpc("evaluate", tab_id=tab_id, script="new Promise(res => setTimeout(() => res(41 + 1), 150))")
assert r["outcome"] == "ok" and r["result"]["value"] == 42, r
r = rpc("evaluate", tab_id=tab_id, script="await fetch(location.href).then(x => x.status)")
assert r["outcome"] == "ok" and r["result"]["value"] == 200, r
r = rpc("evaluate", tab_id=tab_id, script="const x = await Promise.resolve(2); return x * 21")
assert r["outcome"] == "ok" and r["result"]["value"] == 42, r
r = rpc("evaluate", tab_id=tab_id, script="Promise.reject(new Error('nope'))")
assert r["outcome"] == "error" and "nope" in r["message"], r
r = rpc("evaluate", tab_id=tab_id, script="var __persist = 5; __persist")
r = rpc("evaluate", tab_id=tab_id, script="__persist")
assert r["outcome"] == "ok" and r["result"]["value"] == 5, r
print("EVALUATE awaits promises / top-level await / globals persist")

# A page whose CSP forbids eval still evaluates (raw fallback).
import http.server, threading
class CspPage(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = b"<!doctype html><title>csp page</title><h1 id=h>hello csp</h1>"
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Security-Policy", "script-src 'self'; default-src 'self'")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *args):
        pass
csp_server = http.server.HTTPServer(("127.0.0.1", 0), CspPage)
threading.Thread(target=csp_server.serve_forever, daemon=True).start()
r = rpc("navigate", tab_id=tab_id, url=f"http://127.0.0.1:{csp_server.server_address[1]}/")
assert r["outcome"] == "ok" and r["result"]["tab"]["title"] == "csp page", r
r = rpc("evaluate", tab_id=tab_id, script="document.getElementById('h').textContent")
assert r["outcome"] == "ok" and r["result"]["value"] == "hello csp", r
print("EVALUATE works on a CSP page that forbids eval")
csp_server.shutdown()

r = rpc("tabs_close", tab_id=tab_id)
print("TABS_CLOSE:", json.dumps(r)[:150])
assert r["outcome"] == "ok"

r = rpc("tabs_list")
assert len(r["result"]["tabs"]) == 1

print("ALL CONTROL-SOCKET CHECKS PASSED")

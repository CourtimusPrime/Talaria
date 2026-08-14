#!/usr/bin/env python3
"""Drive talaria-mcp as a real MCP client over stdio: initialize, list tools,
call every tool end-to-end against the running shell."""
import base64
import json
import os
import subprocess
import sys

REPO = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
T = os.environ.get("TALARIA_E2E_OUT", "/tmp/talaria-e2e")
os.makedirs(T, exist_ok=True)

proc = subprocess.Popen(
    [os.path.join(REPO, "target/release/talaria-mcp")],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    text=True, bufsize=1,
)

msg_id = 0

def rpc(method, params=None, notify=False):
    global msg_id
    m = {"jsonrpc": "2.0", "method": method}
    if params is not None:
        m["params"] = params
    if not notify:
        msg_id += 1
        m["id"] = msg_id
    proc.stdin.write(json.dumps(m) + "\n")
    proc.stdin.flush()
    if notify:
        return None
    while True:
        line = proc.stdout.readline()
        if not line:
            err = proc.stderr.read()
            sys.exit(f"talaria-mcp died: {err[-800:]}")
        try:
            r = json.loads(line)
        except json.JSONDecodeError:
            continue
        if r.get("id") == msg_id:
            return r

def call(tool, args):
    r = rpc("tools/call", {"name": tool, "arguments": args})
    if "error" in r:
        return {"_rpc_error": r["error"]}
    return r["result"]

r = rpc("initialize", {
    "protocolVersion": "2025-06-18",
    "capabilities": {},
    "clientInfo": {"name": "mcp-e2e", "version": "1.0"},
})
assert "result" in r, r
print("INIT ok, server:", r["result"]["serverInfo"]["name"],
      "protocol:", r["result"]["protocolVersion"])
rpc("notifications/initialized", {}, notify=True)

r = rpc("tools/list", {})
tools = sorted(t["name"] for t in r["result"]["tools"])
print("TOOLS:", tools)
expected = ["cookies_read", "download", "evaluate", "navigate", "screenshot",
            "tabs_close", "tabs_focus", "tabs_list", "tabs_open"]
assert tools == expected, tools

r = call("tabs_list", {})
assert not r.get("isError"), r
print("tabs_list:", r["content"][0]["text"][:120])

r = call("tabs_open", {"url": "https://example.com/"})
assert not r.get("isError"), r
tab = json.loads(r["content"][0]["text"])["tab"]
tab_id = tab["tab_id"]
print("tabs_open -> tab", tab_id, "owner", tab["owner"])
assert tab["owner"] == "mcp-e2e", tab

import time
time.sleep(6)

r = call("evaluate", {"tab_id": tab_id, "script": "document.title"})
assert not r.get("isError"), r
value = json.loads(r["content"][0]["text"])["value"]
print("evaluate title:", value)
assert value == "Example Domain", value

r = call("screenshot", {"tab_id": tab_id})
assert not r.get("isError"), r
img = next(c for c in r["content"] if c["type"] == "image")
png = base64.b64decode(img["data"])
open(os.path.join(T, "mcp-screenshot.png"), "wb").write(png)
print("screenshot:", len(png), "bytes", img["mimeType"])
assert img["mimeType"] == "image/png" and len(png) > 10000

r = call("cookies_read", {"domain": "example.com"})
assert not r.get("isError"), r
print("cookies_read:", r["content"][0]["text"][:80])

r = call("download", {
    "url": "https://raw.githubusercontent.com/servo/servo/main/README.md",
    "filename": "talaria-mcp-test-download.md",
})
assert not r.get("isError"), r
dl = json.loads(r["content"][0]["text"])
print("download:", dl)
assert os.path.exists(dl["path"]) and dl["bytes"] > 100
os.remove(dl["path"])

r = call("navigate", {"tab_id": tab_id, "url": "https://servo.org/"})
assert not r.get("isError"), r
print("navigate ok")

r = call("tabs_focus", {"tab_id": tab_id})
assert not r.get("isError"), r
print("tabs_focus ok")

r = call("tabs_close", {"tab_id": tab_id})
assert not r.get("isError"), r
print("tabs_close ok")

# Error paths
r = call("evaluate", {"tab_id": 9999, "script": "1"})
assert r.get("isError") or "_rpc_error" in r, r
print("error path (bad tab) ok:", json.dumps(r)[:120])

proc.stdin.close()
proc.wait(timeout=5)
print("ALL MCP CLIENT CHECKS PASSED")

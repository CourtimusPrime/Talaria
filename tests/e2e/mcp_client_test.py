#!/usr/bin/env python3
"""Drive talaria-mcp as a real MCP client over stdio: initialize, list tools,
call every tool end-to-end against the running shell, prove two tool calls
pipeline, and prove tab lifecycle events arrive as MCP notifications on the
owning session and on no other.

Reads the proxy's stdout off the raw fd with its own buffer rather than
proc.stdout.readline(): this suite times reads out on purpose, and a Python
buffered reader that has once timed out refuses every later read. Notifications
share the stream with responses, so anything a response overtakes is buffered
rather than discarded."""
import base64
import json
import os
import select
import subprocess
import sys
import time

REPO = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
# See the TARGET comment in harness.py: cargo writes to $CARGO_TARGET_DIR when
# it is set, and CI sets it.
TARGET = os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target")
BINARY = os.path.join(TARGET, "release/talaria-mcp")
T = os.environ.get("TALARIA_E2E_OUT", "/tmp/talaria-e2e")
os.makedirs(T, exist_ok=True)


class Client:
    """One talaria-mcp process, driven as an MCP client over stdio."""

    def __init__(self, name):
        self.name = name
        self.proc = subprocess.Popen(
            [BINARY],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, bufsize=1,
        )
        self.fd = self.proc.stdout.fileno()
        self.buf = b""
        self.notes = []
        self.msg_id = 0

    def write(self, message):
        self.proc.stdin.write(json.dumps(message) + "\n")
        self.proc.stdin.flush()

    def readline(self, timeout):
        """One decoded message, or None if nothing arrived within `timeout`.
        Non-JSON stdout lines are skipped, as they always were."""
        deadline = time.monotonic() + timeout
        while True:
            while b"\n" not in self.buf:
                left = deadline - time.monotonic()
                if left <= 0 or not select.select([self.fd], [], [], left)[0]:
                    return None
                chunk = os.read(self.fd, 65536)
                if not chunk:
                    sys.exit(f"talaria-mcp ({self.name}) died: "
                             f"{self.proc.stderr.read()[-800:]}")
                self.buf += chunk
            line, _, self.buf = self.buf.partition(b"\n")
            try:
                return json.loads(line)
            except json.JSONDecodeError:
                continue

    def send(self, method, params=None, notify=False):
        """Write one message without waiting for its response."""
        m = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            m["params"] = params
        if notify:
            self.write(m)
            return None
        self.msg_id += 1
        m["id"] = self.msg_id
        self.write(m)
        return self.msg_id

    def send_call(self, tool, args):
        return self.send("tools/call", {"name": tool, "arguments": args})

    def response(self, wanted, timeout=60):
        """The response to one of `wanted` ids. A notification met on the way
        is buffered, never dropped — dropping one here would hide exactly the
        bug the notification assertions below exist to catch."""
        while True:
            m = self.readline(timeout)
            assert m is not None, \
                f"{self.name}: no response to {wanted} in {timeout}s; notes={self.notes}"
            if "id" not in m:
                self.notes.append(m)
            elif m["id"] in wanted:
                return m

    def rpc(self, method, params=None, notify=False):
        i = self.send(method, params, notify)
        return None if i is None else self.response({i})

    def call(self, tool, args):
        r = self.rpc("tools/call", {"name": tool, "arguments": args})
        if "error" in r:
            return {"_rpc_error": r["error"]}
        return r["result"]

    def initialize(self):
        r = self.rpc("initialize", {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": self.name, "version": "1.0"},
        })
        assert "result" in r, r
        self.rpc("notifications/initialized", {}, notify=True)
        return r["result"]

    def open_tab(self, url="https://example.com/"):
        r = self.call("tabs_open", {"url": url})
        assert not r.get("isError"), r
        return json.loads(r["content"][0]["text"])["tab"]["tab_id"]

    def expect_note(self, event, tab_id, why, seconds=15):
        """A JSON-RPC message with no id carrying this event name and this tab
        id — enough for a client to act without a follow-up tabs_list."""
        want = {"event": event, "tab_id": tab_id}
        deadline = time.monotonic() + seconds
        while True:
            for note in self.notes:
                if note.get("params", {}).get("data") == want:
                    assert note.get("method") == "notifications/message", note
                    self.notes.remove(note)
                    print(f"{self.name} notified: {json.dumps(note['params'])}")
                    return note
            left = deadline - time.monotonic()
            assert left > 0, (f"{self.name}: no {event} notification for tab {tab_id} "
                              f"within {seconds}s ({why}); saw {self.notes}")
            m = self.readline(left)
            if m is not None and "id" not in m:
                self.notes.append(m)

    def expect_adoption(self, opener_tab_id, why, seconds=20):
        """The adoption notification, matched by opener rather than by tab.

        The new tab's id cannot be known in advance — no reply to this session
        ever carries it, which is the whole reason the event exists — so this
        matches on the opener and returns what it learned."""
        deadline = time.monotonic() + seconds
        while True:
            for note in self.notes:
                data = note.get("params", {}).get("data", {})
                if data.get("event") == "tab_opened" \
                        and data.get("opener_tab_id") == opener_tab_id:
                    assert note.get("method") == "notifications/message", note
                    self.notes.remove(note)
                    print(f"{self.name} notified: {json.dumps(note['params'])}")
                    return data["tab_id"]
            left = deadline - time.monotonic()
            assert left > 0, (f"{self.name}: no tab_opened notification for opener "
                              f"{opener_tab_id} within {seconds}s ({why}); saw {self.notes}")
            m = self.readline(left)
            if m is not None and "id" not in m:
                self.notes.append(m)

    def expect_silence(self, why, seconds=2.0):
        """No unsolicited message of ANY shape — not merely none naming a
        particular tab id, so a regression leaking a differently-shaped
        notification fails here too."""
        assert not self.notes, f"{self.name}: buffered notifications: {self.notes}"
        assert not self.buf, f"{self.name}: buffered bytes: {self.buf!r}"
        m = self.readline(seconds)
        assert m is None, f"{self.name}: unsolicited message ({why}): {m}"
        print(f"{self.name} stayed silent for {seconds}s ({why})")

    def shutdown(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=5)
        except (OSError, ValueError, subprocess.TimeoutExpired):
            self.proc.kill()
            self.proc.wait()


client = Client("mcp-e2e")

init = client.initialize()
print("INIT ok, server:", init["serverInfo"]["name"],
      "protocol:", init["protocolVersion"])
# The notification below is `notifications/message`, which a conformant client
# only accepts from a server that declared the logging capability.
assert "logging" in init["capabilities"], init["capabilities"]
print("CAPABILITIES:", json.dumps(init["capabilities"]))

r = client.rpc("tools/list", {})
tools = sorted(t["name"] for t in r["result"]["tools"])
print("TOOLS:", tools)
expected = ["cookies_read", "download", "evaluate", "navigate", "screenshot",
            "tabs_close", "tabs_focus", "tabs_list", "tabs_open"]
assert tools == expected, tools

r = client.call("tabs_list", {})
assert not r.get("isError"), r
print("tabs_list:", r["content"][0]["text"][:120])

r = client.call("tabs_open", {"url": "https://example.com/"})
assert not r.get("isError"), r
tab = json.loads(r["content"][0]["text"])["tab"]
tab_id = tab["tab_id"]
print("tabs_open -> tab", tab_id, "owner", tab["owner"])
assert tab["owner"] == "mcp-e2e", tab

time.sleep(6)

r = client.call("evaluate", {"tab_id": tab_id, "script": "document.title"})
assert not r.get("isError"), r
value = json.loads(r["content"][0]["text"])["value"]
print("evaluate title:", value)
assert value == "Example Domain", value

r = client.call("screenshot", {"tab_id": tab_id})
assert not r.get("isError"), r
img = next(c for c in r["content"] if c["type"] == "image")
png = base64.b64decode(img["data"])
open(os.path.join(T, "mcp-screenshot.png"), "wb").write(png)
print("screenshot:", len(png), "bytes", img["mimeType"])
assert img["mimeType"] == "image/png" and len(png) > 10000

r = client.call("cookies_read", {"domain": "example.com"})
assert not r.get("isError"), r
print("cookies_read:", r["content"][0]["text"][:80])

r = client.call("download", {
    "url": "https://raw.githubusercontent.com/servo/servo/main/README.md",
    "filename": "talaria-mcp-test-download.md",
})
assert not r.get("isError"), r
dl = json.loads(r["content"][0]["text"])
print("download:", dl)
assert os.path.exists(dl["path"]) and dl["bytes"] > 100
os.remove(dl["path"])

r = client.call("navigate", {"tab_id": tab_id, "url": "https://servo.org/"})
assert not r.get("isError"), r
print("navigate ok")

r = client.call("tabs_focus", {"tab_id": tab_id})
assert not r.get("isError"), r
print("tabs_focus ok")

r = client.call("tabs_close", {"tab_id": tab_id})
assert not r.get("isError"), r
print("tabs_close ok")
# That close was this session's own tab, so it also produces a notification.
client.expect_note("tab_closed", tab_id, "closed the tool-surface tab")

# Error paths
r = client.call("evaluate", {"tab_id": 9999, "script": "1"})
assert r.get("isError") or "_rpc_error" in r, r
print("error path (bad tab) ok:", json.dumps(r)[:120])

# Concurrency (MCP-11): two tool calls in flight at once on one session, with
# no read between them. The tabs_list must come back first and fast — if the
# proxy holds its connection lock across a round trip, it cannot.
#
# The second call is issued a beat after the first rather than in the same
# breath. Both are dispatched concurrently by the server runtime, so with a
# serialising connection it is a coin flip which one reaches the lock first;
# the beat makes the slow call the definite holder, and the assertion then
# fails on a serialising connection every time instead of half the time.

slow_tab = client.open_tab()
time.sleep(6)

# A synchronous busy-wait, so no promise polling is involved and the call
# still finishes inside the shared shell's 3s command timeout.
slow = client.send_call("evaluate", {
    "tab_id": slow_tab,
    "script": "var t=Date.now();while(Date.now()-t<1500){};'slow'",
})
time.sleep(0.4)
t0 = time.monotonic()
fast = client.send_call("tabs_list", {})
order, seen, fast_dt = [], {}, None
while len(seen) < 2:
    r = client.response({slow, fast})
    if r["id"] == fast:
        fast_dt = time.monotonic() - t0
    order.append(r["id"])
    seen[r["id"]] = r
assert not seen[slow]["result"].get("isError"), seen[slow]
assert not seen[fast]["result"].get("isError"), seen[fast]
assert json.loads(seen[slow]["result"]["content"][0]["text"])["value"] == "slow", seen[slow]
assert order[0] == fast, f"tabs_list was gated on the slow evaluate: {order}"
assert fast_dt < 0.5, f"tabs_list took {fast_dt:.1f}s — it waited on the evaluate"
print(f"CONCURRENT tool calls: tabs_list returned in {fast_dt:.2f}s "
      "while the evaluate was still outstanding")

r = client.call("tabs_close", {"tab_id": slow_tab})
assert not r.get("isError"), r
client.expect_note("tab_closed", slow_tab, "closed the concurrency tab")

# Tab lifecycle notifications (AGENT-04). A second proxy process, a separate
# MCP session, is spawned and kept live so the silence assertion means "a
# connected session that would see a fan-out saw nothing" rather than "a
# session that never connected saw nothing" — the proxy connects to the
# control socket lazily, on its first tool call.
observer = Client("mcp-e2e-observer")
try:
    observer.initialize()
    observer_tab = observer.open_tab()
    print("observer session live, owns tab", observer_tab)

    # --- own-session delivery: crash, then close, on the actor's tab ---
    crashed = client.open_tab()
    r = client.call("evaluate", {"tab_id": crashed, "script": "__talaria_sim_crash__"})
    assert not r.get("isError"), r
    client.expect_note("tab_crashed", crashed, "own tab crashed")
    print("tab_crashed reached the owning MCP session as a notification")

    r = client.call("tabs_close", {"tab_id": crashed})
    assert not r.get("isError"), r
    client.expect_note("tab_closed", crashed, "own tab closed")
    print("tab_closed reached the owning MCP session as a notification")

    # --- adoption: a tab this session never asked for ---
    # A page the agent is driving calls window.open. The popup is adopted into
    # the opener's session, so the agent now owns a tab whose id appears in no
    # reply it will ever get. tabs_list could tell it *that* a tab exists; only
    # the event tells it which page produced one, and tells it without polling.
    opener = client.open_tab()
    r = client.call("evaluate", {
        "tab_id": opener,
        "script": "typeof window.open('https://example.com/?adopted=1', '_blank')",
    })
    assert not r.get("isError"), r
    adopted = client.expect_adoption(opener, "own page called window.open")
    assert adopted != opener, adopted
    print("tab_opened reached the owning MCP session, naming opener", opener)

    for tab in (adopted, opener):
        r = client.call("tabs_close", {"tab_id": tab})
        assert not r.get("isError"), r
        client.expect_note("tab_closed", tab, "cleaning up the adoption tabs")

    # --- cross-session silence ---
    # The observer owns none of those tabs. The shell addresses each event to
    # the session that owns the tab; the proxy must forward, never fan out.
    observer.expect_silence("another MCP session's tab crashed, opened and closed")

    # The observer's own close proves that silence was real addressing and not
    # a dead notification path on that process.
    r = observer.call("tabs_close", {"tab_id": observer_tab})
    assert not r.get("isError"), r
    observer.expect_note("tab_closed", observer_tab, "observer closed its own tab")
    print("the observer's notification path is live — its silence was addressing")
    client.expect_silence("the observer closed its own tab")
finally:
    observer.shutdown()

client.shutdown()
print("ALL MCP CLIENT CHECKS PASSED")

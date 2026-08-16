#!/usr/bin/env python3
"""Owner-addressed lifecycle events over the control socket: a session gets
tab_crashed and tab_closed for its OWN tabs, another session gets nothing at
all, and a human-owned tab produces no agent-facing event. Expects a running
shell with TALARIA_TEST_HOOKS=1.

Reads the raw socket rather than makefile(): a buffered reader that has once
timed out refuses every later read ("cannot read from timed out object"), and
this suite times a read out on purpose, twice, per connection."""
import json, os, socket

SOCK = os.environ.get("XDG_RUNTIME_DIR",
                      f"{os.environ.get('TMPDIR', '/tmp')}/talaria-{os.getuid()}") \
    + "/talaria.sock"

class Wire:
    """One control-socket client. Events share the reply stream, so anything
    unsolicited that a reply overtakes is buffered rather than discarded."""

    def __init__(self, name):
        self.name = name
        self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.s.connect(SOCK)
        self.buf = b""
        self.events = []
        self.rid = 0
        self.send({"type": "hello", "client": name})
        assert self.readline(10)["type"] == "hello_ack"

    def send(self, message):
        self.s.sendall((json.dumps(message) + "\n").encode())

    def readline(self, timeout):
        """One decoded message, or None if nothing arrived within `timeout`."""
        while b"\n" not in self.buf:
            self.s.settimeout(timeout)
            try:
                chunk = self.s.recv(65536)
            except (socket.timeout, TimeoutError):
                return None
            assert chunk, f"{self.name}: shell closed the connection"
            self.buf += chunk
        line, _, self.buf = self.buf.partition(b"\n")
        return json.loads(line)

    def rpc(self, command, **params):
        self.rid += 1
        self.send({"type": "request", "id": self.rid, "command": command, **params})
        while True:
            m = self.readline(10)
            assert m is not None, f"{self.name}: no reply to {command}"
            if m.get("type") == "event":
                self.events.append(m)
            elif m.get("type") == "reply" and m.get("id") == self.rid:
                return m

    def expect_event(self, want):
        while want not in self.events:
            m = self.readline(10)
            assert m is not None, f"{self.name}: no {want}; saw {self.events}"
            if m.get("type") == "event":
                self.events.append(m)
        self.events.remove(want)
        print(f"{self.name} got: {want}")

    def expect_silence(self, why, seconds=1.5):
        """No unsolicited message of ANY shape — not merely none naming a
        particular tab id, so a regression leaking a differently-shaped event
        fails here too."""
        assert not self.events, f"{self.name}: buffered events: {self.events}"
        assert not self.buf, f"{self.name}: buffered bytes: {self.buf!r}"
        m = self.readline(seconds)
        assert m is None, f"{self.name}: unsolicited message ({why}): {m}"
        print(f"{self.name} stayed silent for {seconds}s ({why})")

    def open_tab(self):
        return self.rpc("tabs_open", url="https://example.com/")["result"]["tab"]["tab_id"]

actor = Wire("event-actor")
observer = Wire("event-observer")

# --- own-tab delivery: crash ---
crashed = actor.open_tab()
actor.rpc("evaluate", tab_id=crashed, script="__talaria_sim_crash__")
actor.expect_event({"type": "event", "event": "tab_crashed", "tab_id": crashed})
print("tab_crashed delivered to the owning session")

# --- own-tab delivery: close ---
closed = actor.open_tab()
actor.rpc("tabs_close", tab_id=closed)
actor.expect_event({"type": "event", "event": "tab_closed", "tab_id": closed})
print("tab_closed delivered to the owning session")

# Clean up the crashed tab; its close is the owner's event too.
actor.rpc("tabs_close", tab_id=crashed)
actor.expect_event({"type": "event", "event": "tab_closed", "tab_id": crashed})

# --- cross-session silence ---
# The observer owns none of the tabs above and must have seen nothing while
# all three of the actor's events were delivered.
observer.expect_silence("actor's tabs crashed and closed")

# --- human-owned silence ---
# open_for_user creates a tab owned by Me, not by the requesting session. An
# implementation that keyed the filter on "the session that asked" instead of
# on the tab's owner would deliver a close event here.
mine = actor.rpc("open_for_user", url="https://example.com/")["result"]["tab"]["tab_id"]
actor.rpc("tabs_close", tab_id=mine)
actor.expect_silence("closed a human-owned tab")
observer.expect_silence("another session closed a human-owned tab")
print("closing a human-owned tab produced no event on either connection")

print("CRASH EVENT CHECKS PASSED")

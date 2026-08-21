#!/usr/bin/env python3
"""E2E: local browsing history (BROWSE-01).

History is captured from inside a servo delegate callback, filtered by tab
owner, and written to a plain ``history.jsonl`` under the config directory.
None of that is observable through the control socket — there is deliberately
no command and no MCP tool that reads history back — so this suite asserts on
the file itself, which is also exactly what a restart reads.

What this pins down:

- a completed navigation in one of the human's own tabs becomes one row
  carrying the page's URL, its title, and a timestamp;
- a second navigation to a *different* page appends rather than replacing, so
  the store is a log and not a set;
- a navigation in an **agent-owned** tab produces no row at all — the negative
  case, asserted directly rather than inferred from the absence of a positive
  one;
- the rows survive a full shell restart against the same config directory,
  which is BROWSE-01's literal success criterion;
- and, empirically, what Servo 0.4.0 does about an in-page ``pushState``
  navigation (03-RESEARCH.md's Open Question 1). That last step deliberately
  makes no assertion about the outcome either way: the answer was unknown when
  this was written, so the suite *reports* it — see PUSHSTATE PROBE in the
  output — and asserts only that nothing regressed or crashed.

Standalone (starts its own Xvfb + shell): it needs an isolated home, because
history, the vault and the engine profile all live under the platform config
directory and the developer's real ones must not be touched. Pages come from a
local HTTP server so nothing here depends on the network.
"""
import http.server
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness

PAGES = {
    "/one": b"<!doctype html><title>History One</title><h1>one</h1>",
    "/two": b"<!doctype html><title>History Two</title><h1>two</h1>",
    "/agent": b"<!doctype html><title>Agent Only</title><h1>agent</h1>",
    "/spa": b"<!doctype html><title>History SPA</title><h1 id=h>spa</h1>",
}


class Pages(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = PAGES.get(self.path.split("?")[0], b"<!doctype html><title>x</title>")
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def make_home(prefix):
    """A temp home with an empty talaria config directory inside it."""
    home = tempfile.mkdtemp(prefix=prefix)
    config = os.path.join(home, ".config")
    talaria = os.path.join(config, "talaria")
    os.makedirs(talaria)
    return home, config, talaria


def start(home, config, log=subprocess.DEVNULL):
    return harness.start_shell("about:blank", log=log, rust_log="warn",
                               HOME=home, XDG_CONFIG_HOME=config)


def stop_shell(shell):
    """Stop the shell and wait for its socket to go, so the next start_shell
    does not return against the dying one (and does not get its URL forwarded
    to it by the single-instance path)."""
    shell.terminate()
    for _ in range(50):
        if shell.poll() is not None:
            break
        time.sleep(0.1)
    else:
        shell.kill()
    shell.wait()
    for _ in range(50):
        if not os.path.exists(harness.SOCK):
            return
        time.sleep(0.1)
    try:
        os.remove(harness.SOCK)
    except FileNotFoundError:
        pass


def rpc(command, client="history-e2e", **params):
    """One request on its own connection — the suite restarts the shell
    between phases, so a long-lived wire would not survive.

    `client` is what labels the session, and a tab opened through `tabs_open`
    belongs to whichever session asked for it: that is how the agent-owned tab
    below becomes agent-owned."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(harness.SOCK)
    try:
        f = s.makefile("rw")
        f.write(json.dumps({"type": "hello", "client": client}) + "\n")
        f.flush()
        assert json.loads(f.readline())["type"] == "hello_ack"
        f.write(json.dumps({"type": "request", "id": 1, "command": command,
                            **params}) + "\n")
        f.flush()
        while True:
            m = json.loads(f.readline())
            if m.get("type") == "reply" and m.get("id") == 1:
                return m
    finally:
        s.close()


def history_rows(talaria):
    """Every parseable row in history.jsonl; an absent file reads as none."""
    try:
        with open(os.path.join(talaria, "history.jsonl")) as f:
            text = f.read()
    except FileNotFoundError:
        return []
    rows = []
    for line in text.splitlines():
        if line.strip():
            rows.append(json.loads(line))
    return rows


def wait_for_rows(talaria, count, timeout=15.0):
    """History is written from the event loop one tick after the reply that
    reported the load, so every count here is polled rather than read once."""
    deadline = time.monotonic() + timeout
    rows = history_rows(talaria)
    while time.monotonic() < deadline and len(rows) < count:
        time.sleep(0.2)
        rows = history_rows(talaria)
    return rows


home, config, talaria = make_home("talaria-history-e2e-")

server = http.server.HTTPServer(("127.0.0.1", 0), Pages)
threading.Thread(target=server.serve_forever, daemon=True).start()
BASE = f"http://127.0.0.1:{server.server_address[1]}"

xvfb = harness.start_xvfb()
tal = None
try:
    tal = start(home, config)

    # --- one completed navigation in the human's own tab ------------------
    r = rpc("open_for_user", url=BASE + "/one")
    assert r["outcome"] == "ok", r
    rows = wait_for_rows(talaria, 1)
    assert len(rows) == 1, rows
    assert rows[0]["url"] == BASE + "/one", rows
    assert rows[0]["title"] == "History One", rows
    assert rows[0]["visited_at_ms"] > 0, rows
    print("RECORDED", rows[0]["url"], "-", rows[0]["title"])

    # --- a second, different page appends rather than replacing -----------
    r = rpc("open_for_user", url=BASE + "/two")
    assert r["outcome"] == "ok", r
    rows = wait_for_rows(talaria, 2)
    assert len(rows) == 2, rows
    assert [row["url"] for row in rows] == [BASE + "/one", BASE + "/two"], rows
    assert rows[1]["title"] == "History Two", rows
    print("APPENDED second row, in visit order")

    # --- an agent's own tab is not the human's history --------------------
    # BROWSE-01's negative case. `tabs_open` on an agent session produces a
    # TabOwner::Agent tab; the drain filters on TabOwner::Me, so a completed
    # load there must leave the file untouched.
    r = rpc("tabs_open", client="history-e2e-agent", url=BASE + "/agent")
    assert r["outcome"] == "ok", r
    agent_tab = r["result"]["tab"]["tab_id"]
    # Give the write path as long as a real one would have taken, so "no row"
    # means the filter held rather than that nothing had happened yet.
    time.sleep(3.0)
    rows = history_rows(talaria)
    assert len(rows) == 2, ("an agent-owned navigation reached the human's history", rows)
    assert all("/agent" not in row["url"] for row in rows), rows
    print("AGENT tab", agent_tab, "recorded nothing —", len(rows), "rows unchanged")

    # --- an agent may not navigate one of the human's own tabs ------------
    # CR-02, pinned as the attack rather than as its consequence: `tabs_list`
    # hands an agent the human's tabs by design, so the refusal has to live on
    # `navigate` itself. Both halves are asserted — the command is refused,
    # *and* no row appears — because a refusal that still wrote the row would
    # be the worse of the two failures.
    r = rpc("tabs_list", client="history-e2e-agent")
    assert r["outcome"] == "ok", r
    me_tabs = [t for t in r["result"]["tabs"] if t["owner"] == "me"]
    assert me_tabs, ("tabs_list showed an agent no Me tab, so the refusal is untested",
                     r["result"]["tabs"])
    me_tab = me_tabs[0]["tab_id"]

    r = rpc("navigate", client="history-e2e-agent", tab_id=me_tab,
            url=BASE + "/agent")
    assert r["outcome"] == "error", ("an agent navigated one of the human's own tabs", r)
    assert "tabs_open" in r["message"], r
    print("NAVIGATE on Me tab", me_tab, "refused:", r["message"])

    # As long as a real navigation would have taken, so "no row" means the
    # refusal held rather than that nothing had happened yet.
    time.sleep(3.0)
    rows = history_rows(talaria)
    assert len(rows) == 2, ("an agent forged a row into the human's history", rows)
    assert all("/agent" not in row["url"] for row in rows), rows
    # The tab the agent aimed at is still where the human left it.
    r = rpc("tabs_list")
    tab = next(t for t in r["result"]["tabs"] if t["tab_id"] == me_tab)
    assert "/agent" not in tab["url"], ("the refused navigate still moved the tab", tab)
    print("ME tab still on", tab["url"], "—", len(rows), "rows unchanged")

    # --- restart survival: BROWSE-01's literal success criterion ----------
    stop_shell(tal)
    tal = start(home, config)
    rows = history_rows(talaria)
    assert len(rows) == 2, rows
    assert [row["url"] for row in rows] == [BASE + "/one", BASE + "/two"], rows
    r = rpc("tabs_list")
    assert r["outcome"] == "ok", r
    print("RESTART both rows survived")

    # --- empirical: what does an in-page pushState do? --------------------
    # 03-RESEARCH.md Open Question 1. Capture is gated on LoadStatus::Complete
    # and it was not known whether Servo 0.4.0 re-fires that (or anything
    # else this shell listens to) for a same-document navigation. This step
    # exists to *find out*, so it deliberately asserts nothing about the
    # delta. The invariants it does assert are defensive: rows must not be
    # lost, and the shell must still be answering.
    r = rpc("open_for_user", url=BASE + "/spa")
    assert r["outcome"] == "ok", r
    spa_tab = r["result"]["tab"]["tab_id"]
    rows = wait_for_rows(talaria, 3)
    before = len(rows)

    r = rpc("evaluate", tab_id=spa_tab,
            script="history.pushState(null, '', '#probed'); location.hash")
    assert r["outcome"] == "ok", r
    print("PUSHSTATE evaluate returned:", r["result"]["value"])
    time.sleep(3.0)
    after = len(history_rows(talaria))
    print(f"PUSHSTATE PROBE: {before} -> {after} rows")
    if after > before:
        print("PUSHSTATE PROBE: Complete re-fires for a same-document "
              "navigation — SPA route changes are already captured")
    else:
        print("PUSHSTATE PROBE: no new row — same-document navigation is not "
              "captured (the accepted v1 gap in RESEARCH.md Pitfall 4)")

    assert after >= before, ("a pushState cost the store rows", before, after)
    r = rpc("tabs_list")
    assert r["outcome"] == "ok", ("the shell stopped answering after a pushState", r)

    print("HISTORY CHECKS PASSED")
finally:
    if tal is not None:
        stop_shell(tal)
    xvfb.kill()
    xvfb.wait()
    server.shutdown()
    shutil.rmtree(home, ignore_errors=True)

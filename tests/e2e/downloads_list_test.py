#!/usr/bin/env python3
"""E2E: a completed download becomes a downloads-list row (BROWSE-04).

`download_bounds_test.py` already pins what happens to the *bytes* — the cap,
the timeout, the non-clobbering create. This suite pins the other half, the
part that only exists once `AppEvent::DownloadCompleted` reaches the main
thread: that the shell records what it wrote, in `downloads.json`, with the
path it actually wrote rather than the name that was asked for.

What this pins down:

- a successful `download` appends exactly one entry, carrying the requested
  filename, the source url, the byte count, and the written path;
- a second download of the **same** requested filename gets its own row,
  keyed on `create_unique`'s uniquified path (`report (1).pdf`), rather than
  overwriting or merging with the first — the adjacency edge the store's own
  unit tests describe, here proved end to end against a real fetch;
- both paths appear verbatim in the file, so nothing normalised or re-derived
  either one — this is the security-relevant property, since the panel's
  `Open` button launches exactly this string;
- the row records that an agent asked for it, which is what the panel shows
  the human above that same button;
- a download that fails (here: one refused for exceeding the byte cap) adds
  no row at all, so the list never claims a file that is not there;
- the list survives a restart.

Standalone (starts its own Xvfb + shell): the cap is read from the shell
process's environment, which the shared run_all.py shell does not carry.
HOME and XDG_CONFIG_HOME point at a temp directory, so the downloads
directory, the engine profile, the vault and `downloads.json` are all the
suite's own — note that `downloads.json` lives under XDG_CONFIG_HOME/talaria,
not in the downloads directory itself.
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

CAP = 4096
COMMAND_TIMEOUT = 10
BODY = b"%PDF-1.4 pretend this is a report\n" * 8
NAME = "report.pdf"


class Fixtures(http.server.BaseHTTPRequestHandler):
    """Two bodies: a small one that succeeds, and one over the cap."""

    def do_GET(self):
        if self.path == "/report":
            self.body(BODY)
        elif self.path == "/over-cap":
            self.body(b"z" * (CAP + 1))
        else:
            self.send_error(404)

    def body(self, payload):
        try:
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except OSError:
            pass

    def log_message(self, *args):
        pass


tmp = tempfile.mkdtemp(prefix="talaria-downloads-list-e2e-")
downloads = os.path.join(tmp, "Downloads")
config = os.path.join(tmp, ".config")
os.makedirs(downloads)
os.makedirs(config)
# dirs::download_dir() reads this file (dirs-sys resolves $XDG_CONFIG_HOME
# then $HOME/.config); without it there is no downloads directory to find.
with open(os.path.join(config, "user-dirs.dirs"), "w") as f:
    f.write('XDG_DOWNLOAD_DIR="$HOME/Downloads"\n')

STORE = os.path.join(config, "talaria", "downloads.json")

server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixtures)
threading.Thread(target=server.serve_forever, daemon=True).start()
BASE = f"http://127.0.0.1:{server.server_address[1]}"


def read_store():
    """The stored array, or [] when the file is not there yet."""
    try:
        with open(STORE) as f:
            return json.load(f)
    except (OSError, ValueError):
        return []


def raw_store():
    try:
        with open(STORE) as f:
            return f.read()
    except OSError:
        return ""


def stop_shell(shell):
    """Stop the shell and wait for its socket to go, so the next start_shell
    does not return against the dying one — and does not get its URL forwarded
    to it by the single-instance path. Lifted verbatim from
    ``bookmarks_test.py``, which needs it for the same restart check.
    """
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


def start():
    return harness.start_shell("about:blank",
                               log=subprocess.DEVNULL,
                               rust_log="warn",
                               HOME=tmp,
                               XDG_CONFIG_HOME=config,
                               TALARIA_MAX_DOWNLOAD_BYTES=str(CAP),
                               TALARIA_COMMAND_TIMEOUT_SECS=str(COMMAND_TIMEOUT))


def wait_for_rows(count, timeout=10.0):
    """Poll until the store holds `count` rows.

    The completion event is asynchronous relative to the socket reply: the
    background thread answers the agent and posts `DownloadCompleted` to the
    event loop, which appends on its next turn. A bounded poll is the honest
    way to wait for that, rather than a fixed sleep that is either flaky or
    slow.
    """
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        rows = read_store()
        if len(rows) >= count:
            return rows
        time.sleep(0.1)
    return read_store()


xvfb = harness.start_xvfb()
tal = start()

try:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(harness.SOCK)
    f = s.makefile("rw")
    f.write(json.dumps({"type": "hello", "client": "downloads-list"}) + "\n")
    f.flush()
    assert json.loads(f.readline())["type"] == "hello_ack"
    rid = 0

    def rpc(command, **params):
        global rid
        rid += 1
        f.write(json.dumps({"type": "request", "id": rid, "command": command, **params}) + "\n")
        f.flush()
        while True:
            m = json.loads(f.readline())
            if m.get("type") == "reply" and m.get("id") == rid:
                return m

    assert read_store() == [], "the isolated home already had a downloads list in it"

    # --- one completed download becomes one row --------------------------
    first = rpc("download", url=BASE + "/report", filename=NAME)
    assert first["outcome"] == "ok", first
    rows = wait_for_rows(1)
    assert len(rows) == 1, f"expected one row, got {rows}"
    row = rows[0]
    assert row["filename"] == NAME, row
    assert row["path"] == first["result"]["path"], \
        "the stored path is not the path the shell reported writing"
    assert os.path.basename(row["path"]) == NAME, row
    assert row["url"] == BASE + "/report", row
    assert row["bytes"] == len(BODY), row
    assert row["completed_at_ms"] > 0, row
    # CR-04(a): the row records who asked. This request came over the control
    # socket, so the answer is the agent — recorded, not filtered on, and read
    # by the Downloads panel above the button that hands the file to the OS.
    assert row["requested_by_agent"] is True, \
        ("an agent-requested download was recorded without its provenance", row)
    assert os.path.getsize(row["path"]) == len(BODY), \
        "the row names a path that is not the file that was written"
    print("FIRST row:", os.path.basename(row["path"]), row["bytes"], "bytes")

    # --- the same requested name gets its own row, on its own path -------
    second = rpc("download", url=BASE + "/report", filename=NAME)
    assert second["outcome"] == "ok", second
    rows = wait_for_rows(2)
    assert len(rows) == 2, f"the second download did not get its own row: {rows}"
    later = rows[1]
    assert later["filename"] == NAME, later
    assert later["path"] != row["path"], \
        "two same-named downloads share one path"
    assert "(1)" in os.path.basename(later["path"]), \
        f"the second path was not uniquified: {later['path']}"
    assert later["path"] == second["result"]["path"], later
    # The first row must be untouched — not merged, not overwritten, not
    # re-pointed at the newer file.
    assert rows[0]["path"] == row["path"], "the first row was rewritten"
    assert rows[0]["bytes"] == row["bytes"], "the first row's byte count changed"
    for entry in rows:
        assert os.path.getsize(entry["path"]) == len(BODY), entry
    print("SECOND row:", os.path.basename(later["path"]),
          "(distinct from", os.path.basename(row["path"]) + ")")

    # --- both paths are present verbatim, unnormalised --------------------
    raw = raw_store()
    for entry in rows:
        # json.dumps of the bare string gives the exact escaped form the file
        # must contain, quotes included.
        needle = json.dumps(entry["path"])
        assert needle in raw, f"{needle} is not present verbatim in downloads.json"
    print("BOTH paths present verbatim in downloads.json")

    # --- a failed download adds nothing -----------------------------------
    refused = rpc("download", url=BASE + "/over-cap", filename="too-big.bin")
    assert refused["outcome"] == "error", refused
    assert "cap" in refused["message"], refused
    # Give the event loop the same window a successful download would have
    # had, so "no row" means no row rather than "not yet".
    time.sleep(2)
    rows_after = read_store()
    assert len(rows_after) == 2, \
        f"a refused download produced a list row: {rows_after}"
    assert not any(e["filename"] == "too-big.bin" for e in rows_after), rows_after
    print("REFUSED download added no row")

    # --- the list survives a restart --------------------------------------
    f.close()
    s.close()
    stop_shell(tal)
    tal = start()
    reloaded = read_store()
    assert len(reloaded) == 2, f"the list did not survive a restart: {reloaded}"
    assert [e["path"] for e in reloaded] == [e["path"] for e in rows], \
        "the reloaded list is not the list that was written"
    print("RESTART kept", len(reloaded), "rows in order")

    print("DOWNLOADS LIST CHECKS PASSED")
finally:
    server.shutdown()
    if tal is not None:
        stop_shell(tal)
    xvfb.kill()
    xvfb.wait()  # reap, so the X lock's pid isn't left as a zombie
    shutil.rmtree(tmp, ignore_errors=True)

#!/usr/bin/env python3
"""E2E: the bounds on `download` — an agent writing to the user's disk.

Before this suite the command copied the whole body with no cap (fill the
disk), created the destination with a truncating open (silently clobber a
same-named file the user already had), and ran on a detached thread with no
timeout (keep writing long after the agent was told the call failed).

What this pins down:

- a body of exactly the cap succeeds and reports the cap as its byte count;
  a body of the cap plus one byte is refused, names the cap, and leaves no
  file at the destination the request asked for;
- a name collision uniquifies rather than clobbering, and both files survive
  intact — including when two requests for the same name are in flight;
- a stalling server ends inside the command-timeout window, not after it,
  and leaves nothing behind;
- a filename carrying a path separator is still refused.

Standalone (starts its own Xvfb + shell): the cap is read from the shell
process's environment, which the shared run_all.py shell does not carry.
HOME and XDG_CONFIG_HOME point at a temp directory so the downloads
directory, the engine profile and the vault are all the suite's own.
"""
import http.server
import json
import os
import shutil
import socket
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness

CAP = 1024
COMMAND_TIMEOUT = 5  # download_timeout() derives 3s from this
SMALL = b"x" * 64


class Fixtures(http.server.BaseHTTPRequestHandler):
    """Four bodies: at the cap, one byte over it, small, and never-arrives."""

    def do_GET(self):
        if self.path == "/at-cap":
            self.body(b"a" * CAP)
        elif self.path == "/over-cap":
            self.body(b"b" * (CAP + 1))
        elif self.path == "/small":
            self.body(SMALL)
        elif self.path == "/stall":
            # Headers promise a body, then almost none of it arrives. The
            # shell's own read timeout is the only thing that can end this.
            self.headers_for(100000)
            try:
                self.wfile.write(b"partial")
                self.wfile.flush()
                time.sleep(COMMAND_TIMEOUT * 8)
            except OSError:
                pass
        else:
            self.send_error(404)

    def headers_for(self, length):
        try:
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(length))
            self.end_headers()
        except OSError:
            pass

    def body(self, payload):
        self.headers_for(len(payload))
        try:
            self.wfile.write(payload)
        except OSError:
            pass

    def log_message(self, *args):
        pass


tmp = tempfile.mkdtemp(prefix="talaria-download-e2e-")
downloads = os.path.join(tmp, "Downloads")
config = os.path.join(tmp, ".config")
os.makedirs(downloads)
os.makedirs(config)
# dirs::download_dir() reads this file (dirs-sys resolves $XDG_CONFIG_HOME
# then $HOME/.config); without it there is no downloads directory to find.
with open(os.path.join(config, "user-dirs.dirs"), "w") as f:
    f.write('XDG_DOWNLOAD_DIR="$HOME/Downloads"\n')

server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixtures)
threading.Thread(target=server.serve_forever, daemon=True).start()
BASE = f"http://127.0.0.1:{server.server_address[1]}"

xvfb = harness.start_xvfb()
tal = harness.start_shell("about:blank",
                          HOME=tmp,
                          XDG_CONFIG_HOME=config,
                          TALARIA_MAX_DOWNLOAD_BYTES=str(CAP),
                          TALARIA_COMMAND_TIMEOUT_SECS=str(COMMAND_TIMEOUT))

try:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(harness.SOCK)
    f = s.makefile("rw")
    f.write(json.dumps({"type": "hello", "client": "download-bounds"}) + "\n")
    f.flush()
    assert json.loads(f.readline())["type"] == "hello_ack"
    rid = 0

    def send(command, **params):
        """Write one request line without waiting for its reply."""
        global rid
        rid += 1
        f.write(json.dumps({"type": "request", "id": rid, "command": command, **params}) + "\n")
        f.flush()
        return rid

    def reply(want):
        while True:
            m = json.loads(f.readline())
            if m.get("type") == "reply" and m.get("id") == want:
                return m

    def rpc(command, **params):
        return reply(send(command, **params))

    def dl(name, route):
        return rpc("download", url=BASE + route, filename=name)

    # --- cap boundary ----------------------------------------------------
    r = dl("at-cap.bin", "/at-cap")
    assert r["outcome"] == "ok", r
    assert r["result"]["bytes"] == CAP, r
    assert os.path.getsize(os.path.join(downloads, "at-cap.bin")) == CAP
    print("AT CAP ok:", r["result"]["path"], r["result"]["bytes"], "bytes")

    r = dl("over-cap.bin", "/over-cap")
    assert r["outcome"] == "error", r
    assert str(CAP) in r["message"] and "cap" in r["message"], r
    assert not os.path.exists(os.path.join(downloads, "over-cap.bin")), \
        "the partial file survived a cap refusal"
    print("OVER CAP refused:", r["message"])

    # --- no clobber ------------------------------------------------------
    first = dl("note.txt", "/small")
    assert first["outcome"] == "ok", first
    second = dl("note.txt", "/small")
    assert second["outcome"] == "ok", second
    assert second["result"]["path"] != first["result"]["path"], second
    assert "(1)" in os.path.basename(second["result"]["path"]), second
    for path in (first["result"]["path"], second["result"]["path"]):
        assert os.path.getsize(path) == len(SMALL), f"{path} was truncated"
    print("NO CLOBBER:", os.path.basename(first["result"]["path"]),
          "+", os.path.basename(second["result"]["path"]))

    # --- same name, both requests written before either reply is read ----
    a = send("download", url=BASE + "/small", filename="race.dat")
    b = send("download", url=BASE + "/small", filename="race.dat")
    ra, rb = reply(a), reply(b)
    assert ra["outcome"] == "ok" and rb["outcome"] == "ok", (ra, rb)
    assert ra["result"]["path"] != rb["result"]["path"], (ra, rb)
    for r in (ra, rb):
        assert os.path.getsize(r["result"]["path"]) == len(SMALL), r
    print("CONCURRENT same name ->", os.path.basename(ra["result"]["path"]),
          "+", os.path.basename(rb["result"]["path"]))

    # --- timeout and cleanup ---------------------------------------------
    t0 = time.monotonic()
    r = dl("stalled.bin", "/stall")
    elapsed = time.monotonic() - t0
    assert r["outcome"] == "error", r
    # Both ends: slow enough to prove the download's own bound fired, and
    # inside the command timeout so the agent got this message rather than
    # the control socket's generic one.
    assert 1.0 <= elapsed < COMMAND_TIMEOUT, f"{elapsed:.1f}s outside the bound"
    assert "script still running" not in r["message"], r
    print(f"STALL bounded at {elapsed:.1f}s (< {COMMAND_TIMEOUT}s):", r["message"])
    time.sleep(2)  # a detached thread still writing would land here
    assert not os.path.exists(os.path.join(downloads, "stalled.bin")), \
        "the detached thread kept writing after the agent got its error"
    print("STALL left nothing behind")

    # --- existing filename validation is unchanged -----------------------
    r = dl("../escape.bin", "/small")
    assert r["outcome"] == "error" and r["message"] == "bad filename", r
    r = dl("sub/escape.bin", "/small")
    assert r["outcome"] == "error" and r["message"] == "bad filename", r
    print("PATH SEPARATOR still refused")

    print("DOWNLOAD BOUNDS CHECKS PASSED")
finally:
    server.shutdown()
    harness.stop(tal, xvfb)
    shutil.rmtree(tmp, ignore_errors=True)

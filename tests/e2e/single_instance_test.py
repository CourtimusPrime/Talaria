#!/usr/bin/env python3
"""Single instance: launching `talaria <url>` while a shell already owns the
control socket hands the URL to the running shell (a new Me tab, window
brought forward) and exits 0 — instead of stealing the socket path and
leaving the first shell unreachable to agents when it quits. Expects a
running shell."""
import json, os, socket, subprocess, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness

def tabs():
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(harness.SOCK)
    f = s.makefile("rw")
    f.write(json.dumps({"type": "hello", "client": "single-instance-test"}) + "\n"); f.flush(); f.readline()
    f.write(json.dumps({"type": "request", "id": 1, "command": "tabs_list"}) + "\n"); f.flush()
    result = json.loads(f.readline())["result"]["tabs"]
    s.close()
    return result

before = tabs()
t0 = time.monotonic()
second = subprocess.run([harness.BINARY, "example.org"], env=harness.x_env(),
                        capture_output=True, text=True, timeout=30)
elapsed = time.monotonic() - t0
assert second.returncode == 0, (second.returncode, second.stderr)
assert "already running" in second.stderr, second.stderr
assert elapsed < 10, elapsed
print(f"second launch exited 0 in {elapsed:.1f}s: {second.stderr.strip()}")

after = tabs()
new = [t for t in after if t["tab_id"] not in {t["tab_id"] for t in before}]
assert len(new) == 1 and new[0]["owner"] == "me" and "example.org" in new[0]["url"], new
print("URL opened as a Me tab in the running shell:", new[0]["url"])
assert tabs()  # socket still served by the first shell
print("SINGLE-INSTANCE CHECKS PASSED")

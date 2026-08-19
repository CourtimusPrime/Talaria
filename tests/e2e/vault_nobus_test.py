#!/usr/bin/env python3
"""No session D-Bus: the shell still starts and still serves commands.

`Vault::load` runs on the main thread during startup and asks the OS keychain
for the vault key. On Linux that is the Secret Service over D-Bus, and with no
DBUS_SESSION_BUS_ADDRESS libdbus does not fail — it tries to *start* a bus
itself, and blocks forever when it cannot. That hang is the whole browser: the
control socket answers `hello` from its own thread while the event loop never
services a command, so the shell looks alive and serves nothing.

This is what a headless server, a container, an SSH session and a CI job all
look like, and it is invisible in an interactive run, where autolaunch finds the
desktop session's bus at /run/user/$UID/bus and returns at once. So this suite
builds that environment on purpose: no bus address, and a private empty
XDG_RUNTIME_DIR with no bus in it.

Starts its own shell. Assumes no session bus is needed by anything else here."""
import json, os, shutil, socket, sys, tempfile, time

# Both must be set before harness is imported: harness computes SOCK from
# XDG_RUNTIME_DIR at import time, and start_shell hands os.environ to the child.
RT = tempfile.mkdtemp(prefix="tal-nobus-", dir="/tmp")  # short: AF_UNIX caps sun_path at ~108
os.chmod(RT, 0o700)
os.environ["XDG_RUNTIME_DIR"] = RT
os.environ.pop("DBUS_SESSION_BUS_ADDRESS", None)

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness

# 3s matches the tightest timeout run_all.py imposes anywhere. The bug made
# every command hit its timeout no matter how large it was, so a small one here
# is not a flakiness risk — it is the assertion.
TIMEOUT = "3"

xvfb = harness.start_xvfb()
shell = harness.start_shell("about:blank", rust_log="warn", wait=12,
                            TALARIA_TEST_HOOKS="1", TALARIA_COMMAND_TIMEOUT_SECS=TIMEOUT)
try:
    assert shell.poll() is None, f"shell died at startup, exit {shell.returncode}"

    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(harness.SOCK)
    f = s.makefile("rw")
    f.write(json.dumps({"type": "hello", "client": "vault-nobus-test"}) + "\n"); f.flush()
    ack = json.loads(f.readline())
    assert ack["type"] == "hello_ack", ack

    # tabs_list touches no network and no page. If this times out, the event
    # loop is not running, and nothing else in the suite means anything.
    t0 = time.monotonic()
    f.write(json.dumps({"type": "request", "id": 1, "command": "tabs_list"}) + "\n"); f.flush()
    r = json.loads(f.readline())
    elapsed = time.monotonic() - t0
    assert r["outcome"] == "ok", r
    assert elapsed < float(TIMEOUT), f"tabs_list took {elapsed:.2f}s with a {TIMEOUT}s timeout"
    print(f"tabs_list answered in {elapsed:.2f}s with no session bus: {len(r['result']['tabs'])} tab(s)")

    # A second command proves the loop kept serving rather than answering once
    # from a queue that had drained before the vault blocked.
    f.write(json.dumps({"type": "request", "id": 2, "command": "tabs_list"}) + "\n"); f.flush()
    assert json.loads(f.readline())["outcome"] == "ok"
    print("second command served: the event loop is running, not draining")

    # The fix's first layer is not provoking autolaunch at all. If libdbus had
    # tried, it would have left its scaffolding here.
    spawned = sorted(n for n in os.listdir(RT) if n != "talaria.sock")
    assert not spawned, f"D-Bus autolaunch was provoked, it created {spawned}"
    print("no bus scaffolding in the runtime dir: autolaunch was never triggered")

    s.close()
    print("NO-BUS STARTUP CHECKS PASSED")
finally:
    harness.stop(shell, xvfb)
    shutil.rmtree(RT, ignore_errors=True)

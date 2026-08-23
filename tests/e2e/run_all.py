#!/usr/bin/env python3
"""Full e2e regression in one go.

Phase 1: one shell (TALARIA_TEST_HOOKS=1, TALARIA_COMMAND_TIMEOUT_SECS=3)
serves the socket-driven suites: control_socket, scheme_refusal,
crash_recovery, crash_event, timeout_session, wedge_fastfail, mcp_client,
single_instance, popup.  Phase 2: the standalone suites (keyboard_nav, takeover,
download_bounds, vault, vault_ui, history, bookmarks, downloads_list,
panel_click, http_transport, oauth_flow, revocation, remote_view,
remote_latency, vault_nobus) each start their own shell.

oauth_flow_test, revocation_test and remote_latency_test are the slow ones.
The first two each authorize clients through a real consent click, and
revocation_test additionally waits
out a keep-alive interval to prove a stream is delivering before it asserts
that revoking closes it — on the Streamable-HTTP stream, the legacy /sse
stream, and the remote view WebSocket. remote_view_test restarts its shell
once, to prove that with remote access off there is no /view route at all
before switching it on.  remote_latency_test is slow for a different reason:
it deliberately waits out cadence windows in both directions, and then drives
a real client through a constrained link until the rate ladder answers.

Honours TALARIA_E2E_DISPLAY / XDG_RUNTIME_DIR (see harness.py) so it can run
next to a soak on the default display.  Exit code = number of failed suites.
"""
import os
import subprocess
import sys
import time

T = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, T)
import harness

OUT = os.environ.get("TALARIA_E2E_OUT", "/tmp/talaria-e2e")
os.makedirs(OUT, exist_ok=True)
PY = sys.executable
results = {}


def run(name, args, env=None):
    t0 = time.monotonic()
    proc = subprocess.run([PY, os.path.join(T, name + ".py"), *args], env=env,
                          capture_output=True, text=True)
    ok = proc.returncode == 0
    results[name] = ok
    print(f"[{'PASS' if ok else 'FAIL'}] {name} ({time.monotonic()-t0:.0f}s)")
    if not ok:
        print(proc.stdout[-2000:])
        print(proc.stderr[-2000:])
    return ok


xvfb = harness.start_xvfb(settle=0.5)
log = open(os.path.join(OUT, "run_all-shell.log"), "w")
shell = harness.start_shell("https://servo.org", log=log, rust_log="warn", wait=12,
                            TALARIA_TEST_HOOKS="1", TALARIA_COMMAND_TIMEOUT_SECS="3")
try:
    if shell.poll() is not None:
        print("shell died early, exit", shell.returncode)
        sys.exit(99)
    env = dict(os.environ, TALARIA_E2E_OUT=OUT)
    run("control_socket_test", [OUT], env)
    run("scheme_refusal_test", [], env)
    run("crash_recovery_test", [], env)
    run("crash_event_test", [], env)
    run("timeout_session_test", [], env)
    run("wedge_fastfail_test", [], env)
    run("mcp_client_test", [], env)
    run("single_instance_test", [], env)
    run("popup_test", [], env)
finally:
    harness.stop(shell, xvfb)
    log.close()

# vault_nobus_test goes last because it is the only suite that rewrites
# XDG_RUNTIME_DIR and unsets DBUS_SESSION_BUS_ADDRESS for its own child; it does
# that in its own process, so nothing above it sees either change.
for name in ("keyboard_nav_test", "takeover_test", "download_bounds_test", "vault_test",
             "vault_ui_test", "history_test", "bookmarks_test", "downloads_list_test",
             "panel_click_test", "http_transport_test", "oauth_flow_test",
             "revocation_test", "remote_view_test", "remote_latency_test",
             "vault_nobus_test"):
    run(name, [])

failed = [n for n, ok in results.items() if not ok]
print("failed:", failed or "none")
sys.exit(len(failed))

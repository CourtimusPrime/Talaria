# Testing Patterns

**Analysis Date:** 2026-08-15

Two independent layers:

| Layer | Location | Runner | Scope |
|-------|----------|--------|-------|
| Rust unit tests | inline `#[cfg(test)] mod tests` in crate sources | `cargo test` | pure logic only (serde round-trips, domain matching) |
| Python e2e | `tests/e2e/` | `python3 tests/e2e/run_all.py` | the real binary under Xvfb, driven through the control socket, xdotool, and the MCP stdio server |

Nearly all behavioural coverage lives in the e2e layer. New features are pinned down there, not in `cargo test`.

## Test Framework

**Rust:**
- Built-in `#[test]` harness. No `rstest`, `proptest`, `mockall`, or `tokio::test` anywhere.
- No test config files; no `tests/` integration-test directory inside any crate.

**Python:**
- **Stdlib only** — no pytest, no unittest, no dependencies. Suites are top-level scripts using bare `assert` and `sys.exit`.
- Modules used: `socket` (AF_UNIX), `json`, `subprocess`, `http.server` + `threading` (local fixture pages), `base64`, `time`, `os`.
- The "framework" is `tests/e2e/harness.py` plus the `rpc()` helper each suite defines.

**Run Commands:**
```bash
cargo build --release              # required first — e2e drives target/release/talaria
cargo test                         # the 3 Rust unit tests
python3 tests/e2e/run_all.py       # full e2e regression; exit code = number of failed suites

# Run alongside a soak you don't want disturbed (own display + own socket):
TALARIA_E2E_DISPLAY=:98 XDG_RUNTIME_DIR=/tmp/talaria-e2e-rt python3 tests/e2e/run_all.py

python3 tests/e2e/control_socket_test.py /tmp/out   # one suite, against an already-running shell
python3 tests/e2e/run_control_socket_e2e.py         # socket pass + chrome screenshots
```

**Prerequisites:** `Xvfb`, `xdotool`, `xdpyinfo`, `xwd`, ImageMagick `convert`, and built `target/release/talaria` + `target/release/talaria-mcp`.

## Test File Organization

**Rust — co-located,** at the bottom of the module it tests:
- `crates/talaria-protocol/src/lib.rs:141` — `mod tests` covering wire round-trips.
- `crates/talaria-shell/src/vault.rs:171` — `mod tests` covering domain matching.

**Python — flat directory,** `tests/e2e/`:
```
tests/e2e/
├── harness.py                   # shared Xvfb + shell launcher
├── run_all.py                   # full regression orchestrator
├── run_control_socket_e2e.py    # socket pass + chrome screenshots
├── overnight_lock.py            # guards against two soak loops on one branch
├── README.md                    # prerequisites + manual a11y checklist
├── control_socket_test.py       # protocol surface: tabs/evaluate/screenshot/cookies/navigate
├── crash_recovery_test.py       # simulated WebContent crash + recovery via navigate
├── crash_event_test.py          # unsolicited tab_crashed / tab_closed events to a 2nd client
├── timeout_session_test.py      # hung evaluate hits the command timeout; tabs outlive sessions
├── mcp_client_test.py           # real MCP stdio client against talaria-mcp
├── single_instance_test.py      # second `talaria <url>` launch hands URL to the running shell
├── popup_test.py                # window.open and target=_blank create real tabs
├── keyboard_nav_test.py         # ctrl+t / ctrl+l / alt+arrows / ctrl+w via xdotool
└── takeover_test.py             # click the Agents toggle, then click inside an agent tab
```
`tests/e2e/*.png`, `*.xwd`, `*.log` and `__pycache__/` are gitignored (`.gitignore`).

## Test Structure

**Rust — arrange/act/assert with a local constructor helper:**
```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn entry(url: &str) -> CredentialEntry { /* … */ }

    #[test]
    fn domain_matching() {
        let vault = Vault { path: PathBuf::new(), cipher: None, entries: vec![/* … */] };
        assert_eq!(vault.matching("google.com").len(), 1);
        assert_eq!(vault.matching("example.com").len(), 0);
    }
}
```
(`crates/talaria-shell/src/vault.rs`). Test fns are named after the behaviour (`domain_matching`, `round_trip_request`, `reply_shapes`) — no `test_` prefix. Structs are built by literal, using private fields directly, rather than through constructors that would touch the filesystem or keychain.

**Python — linear script, no test functions.** Each suite:
1. docstring stating what it pins down and its preconditions,
2. connect (or launch via `harness`),
3. a sequence of `rpc(...)` calls with `assert` after each, passing the whole reply as the assert message: `assert r["outcome"] == "ok", r`,
4. a `print("… CHECKS PASSED")` sentinel on the last line.

Failure signalling is the process exit code — an `AssertionError` traceback is the report. Suites that launch their own shell wrap the body in `try: … finally: harness.stop(tal, xvfb)`.

**The shared RPC idiom** (copied per-suite by design, so each file runs standalone):
```python
SOCK = os.environ.get("XDG_RUNTIME_DIR", "/tmp") + "/talaria.sock"

def rpc(command, **params):
    global req_id
    req_id += 1
    send({"type": "request", "id": req_id, "command": command, **params})
    while True:                      # skip unsolicited events until our id comes back
        msg = json.loads(f.readline())
        if msg.get("type") == "reply" and msg.get("id") == req_id:
            return msg
```
The `while True` id-match loop is mandatory: replies can arrive out of order and `Event` messages interleave on the same socket.

## Harness: Xvfb, control socket, xdotool

`tests/e2e/harness.py` is the only shared code. Its contract:

**Isolation knobs** (so a suite can run beside a long soak):
- `TALARIA_E2E_DISPLAY` — X display to create (default `:99`), exposed as `harness.DISPLAY`.
- `XDG_RUNTIME_DIR` — inherited by the shell, giving a private control socket at `$XDG_RUNTIME_DIR/talaria.sock` (`harness.SOCK`).
- `TALARIA_E2E_OUT` — artifact directory (default `/tmp/talaria-e2e`).

**`start_xvfb(display, settle)`** — kills any prior `Xvfb <display>`, calls `kill_shells_on_display()`, then loops up to 5 attempts: `_clear_stale_x_lock()` → `Popen(["Xvfb", display, "-screen", "0", "1280x800x24"])` → poll `xdpyinfo -display <display>` until the server accepts connections. Two hard-won details are encoded here and must not be simplified away:
- `kill_shells_on_display()` scans `/proc/<pid>/comm` for `talaria` and `/proc/<pid>/environ` for `DISPLAY=<display>`, so **only shells on this display are killed** — a soak on another display survives.
- `_clear_stale_x_lock()` removes `/tmp/.X<n>-lock` and `/tmp/.X11-unix/X<n>` left behind by a SIGKILLed Xvfb, but only once the owning pid is gone or reaped (`_pid_alive` treats a zombie as dead).

**`start_shell(url, log, rust_log, wait, **env_extra)`** — launches `target/release/talaria <url>` with `DISPLAY` set, then waits for the control socket file to appear and sleeps out the remainder of `wait` for first paint. Extra env goes in as kwargs; that's how test hooks are enabled.

**`stop(shell, xvfb)`** — `terminate` → 1s → `kill` → `wait`, then `xvfb.kill(); xvfb.wait()` — the `wait()` matters so the X lock's pid isn't left as a zombie and the next run's lock-clearing works.

**`x_env(**extra)`** — `os.environ` plus `DISPLAY`; every `xdotool`/`xwd` subprocess must be given this env.

**xdotool usage** (`tests/e2e/keyboard_nav_test.py`, `tests/e2e/takeover_test.py`, `tests/e2e/run_control_socket_e2e.py`):
- Find the window by title first, retrying up to 15× at 1s: `xdotool search --name Talaria`.
- Focus synchronously before sending input: `xdotool windowfocus --sync <wid>`.
- Keys: `xdotool key ctrl+t`, typing with `xdotool type --delay 30 example.com`.
- Mouse: `xdotool mousemove <x> <y> click 1`; chrome coordinates are hardcoded logical points (the Agents toggle sits at roughly `60,16`), and webview coordinates are computed by `evaluate`-ing `getBoundingClientRect()` in the page and adding the ~45px toolbar offset.
- After every synthetic input, `time.sleep()` (0.5s for chrome, 2–6s for anything that loads a page) before asserting via the socket.

## Mocking

**There is none, and that is the policy.** No mock framework in either language. The e2e layer drives the real Servo engine, the real socket, and the real `talaria-mcp` binary.

**Substitutes for mocking:**
- **In-process HTTP fixture servers.** For anything that must not depend on the network, suites start `http.server.HTTPServer(("127.0.0.1", 0), Handler)` on a daemon thread and navigate to `http://127.0.0.1:<port>/`. Used for the CSP-forbids-eval case in `tests/e2e/control_socket_test.py` and for opener/child pages in `tests/e2e/popup_test.py`. Bind port `0` and read it back from `server.server_address[1]`; suppress logging by overriding `log_message`. Call `server.shutdown()` when done.
- **Test hooks compiled into the shell.** `TALARIA_TEST_HOOKS=1` enables a magic script token: `evaluate` of `__talaria_sim_crash__` fakes a WebContent crash (`crates/talaria-shell/src/app.rs:972`). This is how `crash_recovery_test.py` and `crash_event_test.py` get a crash without killing a process.
- **Env-var time compression.** `TALARIA_COMMAND_TIMEOUT_SECS=3` shrinks the 30s default so `timeout_session_test.py` can assert a hung `while(true){}` evaluate fails in `2.5 < dt < 6` seconds.

**What NOT to mock:** the engine, the socket protocol, the MCP transport, page loads. If a behaviour can only be verified with a fake, it probably belongs in a Rust unit test instead.

## Fixtures and Factories

**Test data is inline.** HTML fixtures are module-level `bytes` literals:
```python
OPENER = b"""<!doctype html><title>opener</title>
<a id=lnk href="/child?via=link" target="_blank">open in a new tab</a>
"""
CHILD = b"""<!doctype html><title>popup child</title><h1 id=h>child page</h1>"""
```
(`tests/e2e/popup_test.py`). There is no fixtures directory.

**Live external URLs are used as stable fixtures:** `https://example.com/` (title asserted as exactly `Example Domain`), `https://servo.org/`, and a raw githubusercontent README for the download tool. Suites that must be offline-safe use the local HTTP server instead.

**Artifacts** (screenshots, logs) go to `TALARIA_E2E_OUT` / `/tmp/talaria-e2e`, never into the repo — `tests/e2e/*.png|xwd|log` are gitignored.

## Coverage

**No coverage tooling and no enforced target.** Rust unit coverage is deliberately tiny (3 tests, all pure functions); everything stateful is covered end-to-end.

**Orchestration in `tests/e2e/run_all.py`:**
- **Phase 1** — one shell started with `TALARIA_TEST_HOOKS=1 TALARIA_COMMAND_TIMEOUT_SECS=3 RUST_LOG=warn`, reused by the socket-driven suites in order: `control_socket`, `crash_recovery`, `crash_event`, `timeout_session`, `mcp_client`, `single_instance`, `popup`. A `shell.poll() is not None` guard exits `99` if the shell dies early.
- **Phase 2** — the standalone suites `keyboard_nav` and `takeover`, each starting their own Xvfb + shell (they need exclusive input focus).
- Each suite runs as a subprocess with `capture_output=True`; on failure the last 2000 chars of stdout and stderr are printed. Result line: `[PASS|FAIL] <name> (<secs>s)`. **Exit code = number of failed suites.**

**Adding a suite:** write `tests/e2e/<feature>_test.py` with the standard docstring + `rpc` idiom, then add a `run("<feature>_test", [], env)` line in the phase-1 block of `tests/e2e/run_all.py` if it can share a shell, or to the phase-2 tuple if it needs its own display/focus.

**Manual, not automated:** screen-reader/accesskit behaviour cannot be checked headlessly. `tests/e2e/README.md` carries a 5-step VoiceOver checklist (Me/Agents toggle labels, toolbar control labels, tab-strip announcements, crashed-tab heading, keyboard-only pass) to run on macOS before a release.

**Soak protection:** `tests/e2e/overnight_lock.py` plus the `.overnight-lock` file prevent two overnight loops running on one branch.

## Common Patterns

**Wait-for-condition, never a bare sleep, when a resource is involved:**
```python
for attempt in range(30):
    try:
        s.connect(SOCK)
        break
    except OSError:
        time.sleep(1)
else:
    sys.exit("could not connect to " + SOCK)
```
The `for/else` form turns exhaustion into a clear failure message.

**Error-path testing** — assert both the outcome and the message substring:
```python
r = rpc("evaluate", tab_id=tab_id, script="throw new Error('boom')")
assert r["outcome"] == "error" and "boom" in r["message"], r

r = rpc("evaluate", tab_id=tab_id, script="1+1")
assert r["outcome"] == "error" and "crashed" in r["message"], r
```

**Timing assertions bound both ends,** so a passing test proves the timeout fired rather than the call merely failing fast: `assert r["outcome"] == "error" and 2.5 < dt < 6, (r, dt)` (`tests/e2e/timeout_session_test.py`).

**Multi-client event testing** — open a second connection as a passive watcher, `settimeout(10)` it, and `readline()` for the unsolicited event while a first connection acts:
```python
_, actor = conn("event-actor")
watcher_sock, watcher = conn("event-watcher")
watcher_sock.settimeout(10)
rpc(actor, "evaluate", tab_id=tid, script="__talaria_sim_crash__")
assert json.loads(watcher.readline()) == {"type": "event", "event": "tab_crashed", "tab_id": tid}
```
(`tests/e2e/crash_event_test.py`).

**Ownership assertions** — tabs opened over the socket must be owned by the client name from `hello` (or `clientInfo.name` over MCP), and the human's tab stays `me`:
```python
owners = sorted(t["owner"] for t in tabs)
assert owners == ["e2e-test", "me"], owners
```

**MCP client testing** (`tests/e2e/mcp_client_test.py`) — speak JSON-RPC 2.0 line-by-line to `target/release/talaria-mcp` over stdin/stdout with `bufsize=1`: `initialize` (protocolVersion `2025-06-18`) → `notifications/initialized` notification → `tools/list` asserted against an exact sorted tool list → `tools/call` for every tool → an error path with a bogus `tab_id`. Non-JSON stdout lines are skipped; if stdout closes, the last 800 chars of stderr are reported.

**Screenshot verification** is by size and dimensions, not pixel comparison: decode the base64 PNG, write it to the artifact dir, and assert `len(png) > 10000` / `mimeType == "image/png"`. Full-chrome shots go through `xwd -root -silent -out …` then ImageMagick `convert` (`tests/e2e/run_control_socket_e2e.py`).

---

*Testing analysis: 2026-08-15*

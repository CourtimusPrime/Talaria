# E2E tests

Require a built `target/release/talaria` (+ `talaria-mcp`) and Xvfb.

- `run_control_socket_e2e.py` — launches Xvfb + talaria, drives the control
  socket directly (`control_socket_test.py`), screenshots the chrome in both
  Me and Agents views.
- `mcp_client_test.py` — full MCP stdio client pass against a running shell:
  initialize, tools/list, and every tool called end-to-end (expects the shell
  already running with `DISPLAY` set, e.g. started by the runner above).

- `run_all.py` — the whole regression: one shell for the socket-driven suites
  (control_socket, crash_recovery, crash_event, timeout_session, mcp_client),
  then the standalone keyboard_nav and takeover suites.

```sh
cargo build --release
python3 tests/e2e/run_all.py
```

To run next to a shell you don't want disturbed (e.g. a soak on `:99`), give
the suites their own display and control socket:

```sh
TALARIA_E2E_DISPLAY=:98 XDG_RUNTIME_DIR=/tmp/talaria-e2e-rt python3 tests/e2e/run_all.py
```

Launchers only kill Talaria processes attached to their own display.

## Manual accessibility check (not automated)

Screen-reader output can't be verified headlessly, so accesskit/VoiceOver is a
manual checklist, run on macOS before a release:

1. Build and launch Talaria on macOS; enable VoiceOver (Cmd+F5).
2. Confirm the Me/Agents toggle announces its label and selected state.
3. Tab through toolbar controls: Back / Forward / Reload / New tab and the URL
   bar must each announce a meaningful label (they carry hover-text labels; if
   VoiceOver reads nothing, accesskit isn't wired — see OVERNIGHT_LOG.md
   needs-your-call).
4. Confirm tab-strip entries announce title + close control, and the crashed
   state announces the "This tab crashed" heading and Reload button.
5. Keyboard-only pass: Ctrl+L / Ctrl+T / Ctrl+W / Ctrl+Tab all work with
   VoiceOver running.

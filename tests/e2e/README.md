# E2E tests

Require a built `target/release/talaria` (+ `talaria-mcp`) and Xvfb.

- `run_control_socket_e2e.py` — launches Xvfb + talaria, drives the control
  socket directly (`control_socket_test.py`), screenshots the chrome in both
  Me and Agents views.
- `mcp_client_test.py` — full MCP stdio client pass against a running shell:
  initialize, tools/list, and every tool called end-to-end (expects the shell
  already running with `DISPLAY` set, e.g. started by the runner above).

```sh
cargo build --release
python3 tests/e2e/run_control_socket_e2e.py
```

# Talaria

A lightweight, lightning-fast web browser for humans and agents.

Talaria embeds [Servo](https://servo.org) (via the `servo` crate, a.k.a. libservo) as its
rendering engine. Tabs are split between a **Me** view (human-driven) and an **Agents**
view (agent-driven), with a built-in OAuth'ed MCP server so any agent — including Claude
Desktop — can drive the browser using the user's own local credentials and sessions.
A human can watch any agent session live and take it over directly (e.g. to complete a
login or a Cloudflare challenge), then let the agent continue in the same session.

See [SPEC.md](SPEC.md) for the full set of product and architecture decisions.

## Architecture

- **Engine**: Servo via libservo (crates.io `servo`), compiled in — content pane only.
- **Shell/chrome**: a custom winit embedder with **egui** chrome (the servoshell
  pattern) — decided architecture, not a stopgap. egui draws directly into the same GL
  context as Servo's compositor, so chrome rendering needs no webview at all and takes
  no dependency on the experimental Servo-WRY backend. Icons: egui-phosphor.
- **Agent surface**: a local control socket (`talaria-protocol`) driven by the
  `talaria-mcp` stdio MCP server; the same envelope schema later carries distributed
  mode over Tailscale.

## Status

Working single-window browser: multi-tab Me/Agents views with live takeover, MCP tool
surface (evaluate-centric), encrypted credential vault, crash recovery, persistent
engine profile. Soak-tested 4h under Xvfb. See `SPEC.md` for decisions and
`OVERNIGHT_LOG.md` for the latest test-and-fix run.

## Building

```sh
cargo build --release
cargo run --release -p talaria-shell -- https://example.com
```

Requires Rust ≥ 1.88 and Servo's Linux/macOS build prerequisites (`python3`, `pkg-config`,
`cmake`, `clang`, fontconfig/freetype dev headers).

## Connecting an agent (MCP)

With the Talaria browser running, point any MCP client at the `talaria-mcp`
stdio server. Claude Desktop example (`claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "talaria": { "command": "/path/to/target/release/talaria-mcp" }
  }
}
```

Tools: `tabs_list`, `tabs_open`, `tabs_close`, `tabs_focus`, `navigate`,
`evaluate` (arbitrary in-page JS — the primary interaction primitive),
`screenshot`, `cookies_read`, `download`. `tabs_open` and `navigate` wait for
the page to load before returning (`loading: true` in the reply means it gave
up waiting after ~20s), so a following `evaluate` acts on the page you asked
for. `evaluate` awaits returned promises and supports top-level `await`
(single expression returned directly; multi-statement scripts need `return`).
Tabs opened by an agent appear in
the browser's **Agents** view, labeled by the client's MCP identity; switch
to that view to watch the session live or take it over directly (e.g. to
complete a login), then let the agent continue.

Keyboard: `Ctrl+L` URL bar (selects the URL; `Esc` abandons the edit),
`Ctrl+T` new tab, `Ctrl+W` close tab, `Ctrl+Tab`/`Ctrl+Shift+Tab` cycle tabs,
`Ctrl+R`/`F5` reload, `Alt+Left`/`Alt+Right` back/forward. `Tab`/`Shift+Tab`
move between toolbar controls once the chrome has focus.

## Continuous integration

Every push and every pull request runs the same four checks, in this order, and a
failure at any one of them stops the run:

1. `cargo build --release --locked` for the whole workspace
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test`
4. the full end-to-end suite, `python3 tests/e2e/run_all.py`

There is no path filter on the workflow, so a push that changes no Rust source still
runs all four — a filtered push reports green without having verified anything.

The end-to-end step needs a virtual framebuffer. The suite drives the real binary
through a window, so the job installs `Xvfb` and the X utilities the harness shells
out to (`xdpyinfo`, `xdotool`, `xwd`, ImageMagick `convert`, `pkill`) alongside
Servo's build prerequisites, and points the shell at a job-private
`XDG_RUNTIME_DIR` so it gets its own control socket.

The lockfile is deliberately load-bearing (see the comment at the top of
`Cargo.toml`), so CI builds `--locked`. A separate weekly job,
`.github/workflows/lockfile-audit.yml`, resolves dependencies from scratch on
purpose — it is *expected* to fail, and it prints the pinned-dependency
diagnosis and its recovery command when it does. It never runs on push, and it
never writes a lockfile back.

## License

Talaria is dual-licensed under [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE).

Files derived from Servo's embedding examples carry MPL-2.0 headers (file-level copyleft);
Servo and SpiderMonkey are consumed as MPL-2.0 dependencies.

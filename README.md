# Charon

A lightweight, lightning-fast web browser for humans and agents.

Charon embeds [Servo](https://servo.org) (via the `servo` crate, a.k.a. libservo) as its
rendering engine. Tabs are split between a **Me** view (human-driven) and an **Agents**
view (agent-driven), with a built-in OAuth'ed MCP server so any agent — including Claude
Desktop — can drive the browser using the user's own local credentials and sessions.
A human can watch any agent session live and take it over directly (e.g. to complete a
login or a Cloudflare challenge), then let the agent continue in the same session.

See [SPEC.md](SPEC.md) for the full set of product and architecture decisions.

## Status

Early feasibility stage. Current milestone: prove the Servo-based content pane builds and
renders real pages on the target machines (`crates/charon-shell`, a minimal winit shell).

## Building

```sh
cargo build --release
cargo run --release -p charon-shell -- https://example.com
```

Requires Rust ≥ 1.88 and Servo's Linux/macOS build prerequisites (`python3`, `pkg-config`,
`cmake`, `clang`, fontconfig/freetype dev headers).

## License

Charon is dual-licensed under [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE).

Files derived from Servo's embedding examples carry MPL-2.0 headers (file-level copyleft);
Servo and SpiderMonkey are consumed as MPL-2.0 dependencies.

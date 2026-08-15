# Overnight test-and-improve loop — 2026-08-16

Branch: `overnight/2026-08-16`. Started 09:34 +0400. Hard stop: 8 hours or 25 iterations.
Baseline: main after the four close-outs (crash-event push, per-webview framebuffers, Phosphor icons, VoiceOver checklist) on top of the merged 2026-08-15 loop.

## Needs your call

*(none yet)*

## Summary

- Iterations: (running)
- Issues found & fixed: (running)
- Reverted: 0

## Log

*(one line per iteration: issue → fix → verified how)*
1. [speed] Re-baseline after per-webview framebuffers: startup/socket 21ms, displayed capture 4–5ms, background capture 2–82ms (frame-ready path healthy) — all unchanged or better. New cost surfaced: ~12MB/tab (own framebuffer + page) and ~31MB allocator retention after closing 10 tabs; a heavy page (servo.org) costs ~200MB engine-side. Observation only — lazy framebuffer allocation is a possible future optimization, not tonight's redesign.
2. [stability] 200 rapid connect/hello/disconnect session churns (a third dying mid-request): RSS 303→304MB, thread count flat, sessions map drains, events still delivered to a fresh client afterwards — the new per-connection writer/event channels don't leak.
3. [speed] Idle CPU with a static page: 0.0% over a 20s window — ControlFlow::Wait sleeps properly, no repaint loop. Clean audit.
4. [design] Agents view listed agent tabs flat with a bare count → tabs now group under their owning session's label (Phosphor robot icon + client name), and tabs whose session disconnected fall into a "disconnected" group (tabs outlive sessions by design) → verified: screenshot with three concurrent MCP sessions shows per-session grouping. Follow-up noted (not done): pushing tab events as MCP notifications through the stdio proxy — socket-level events exist for distributed clients; MCP-side push is a separate design question.

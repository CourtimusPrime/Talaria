# Overnight test-and-improve loop — 2026-08-16

Branch: `overnight/2026-08-16`. Started 09:34 +0400. Hard stop: 8 hours or 25 iterations.
Baseline: main after the four close-outs (crash-event push, per-webview framebuffers, Phosphor icons, VoiceOver checklist) on top of the merged 2026-08-15 loop.

## Needs your call

1. **In-flight working-tree changes found mid-loop (~11:20)** — repo dir renamed open-browser→talaria, and `talaria-protocol/src/lib.rs` has an uncommitted `loading: bool` field on TabInfo (with block-until-loaded semantics sketched in its doc comment), shell side not yet implemented — the shell won't compile until that lands. Looks like your (or another session's) work in progress, so the loop did NOT touch, finish, or revert it, and stopped rebuilding the shell from this point; remaining loop activity uses the already-built binary (soak + final e2e regression are unaffected). Also present: `.claude/HANDOFF-KzpAG.md` deleted in the working tree and a `.README.md.swp` vim swap — left alone.

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
5. [tests] keyboard_nav and takeover standalone suites raced a just-killed Xvfb when run back-to-back (bit the 08-15 final sweep) → they now wait for :99 to actually accept connections (xdpyinfo poll) and retry the window search → syntax-verified now; exercised in this loop's final regression (running them mid-soak would kill the soak's display). Soak checkpoint 50min: 153 cycles ok, RSS flat 477–478MB (+20MB vs loop 1 — the per-tab framebuffer, expected).

# Overnight test-and-improve loop — 2026-08-15

Branch: `overnight/2026-08-15`. Started 01:39 +04. Hard stop: 8 hours (~09:40) or 25 iterations.

## Needs your call

*(judgment calls surfaced during the loop land here — none yet)*

- **Note, not a bug:** the mission brief says "Tauri-shelled" with "React chrome". The current build is a winit shell with egui chrome, per SPEC's own dependency-status finding (tauri-runtime-verso dormant since 2025-10; custom winit embedder until the 16-weeks-out re-check). Accessibility work below targets the egui chrome accordingly. Not redesigning this tonight.

## Summary

- Issues found: (running)
- Fixed: (running)
- Reverted: 0

## Log

*(one line per iteration: issue → fix → verified how)*
1. [agent-use] No end-to-end test of the real MCP stdio path existed → added tests/e2e/mcp_client_test.py driving talaria-mcp as a real client (initialize, tools/list, all 9 tools incl. error paths) + vendored control-socket e2e into tests/e2e/ → verified: full pass against live shell under Xvfb (owner labeled "mcp-e2e", download lands in ~/Downloads, bad-tab returns isError).
2. [stability] Screenshot of a non-displayed tab returned the displayed tab's pixels (hidden webview's pipeline is throttled; its paint() never reaches the shared framebuffer) → replaced sync capture with a pending-capture queue: hide displayed, show target, wait for notify_new_frame_ready (or 1.5s timeout), then paint + read_to_image + restore; WaitUntil scheduling wakes the loop for timeouts; frame-ready flag set inside the delegate (no painting there — painter-borrow re-entrancy) → verified: agent-screenshot.png now shows example.com (54KB, was byte-identical servo.org before); control-socket e2e + full MCP client regression both pass. Side effect: displayed tab may briefly show the captured tab (<1.5s) during background capture.

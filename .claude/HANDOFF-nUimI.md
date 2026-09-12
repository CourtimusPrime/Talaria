# HANDOFF.md

## Current Task

Talaria Phase 5 execution: Testing remote view client on real two-machine setup (MacBook Air to ThinkPad over Tailscale). Discovered sidebar flickering during cursor movement; diagnosed and fixed during this session.

## Key Decisions Made

**Root cause identified (NOT a network/latency problem):** `EguiGlow::run` (egui_glow 0.34.3) forwards viewport commands but discards the repaint delay output. When egui requests another frame to finish an animation (~83ms hover fade on sidebar buttons), the client's loop never hears it. The defect only surfaced on the real two-machine test because automated suites run both ends on one host under Xvfb, where Servo's continuous frame delivery masks the issue.

**Fix approach:** After `egui.run()` builds a frame, check `egui.egui_ctx.has_requested_repaint()` and call `window.request_redraw()` if true. Request self-settles once animation completes; idle client draws nothing.

**Verified:** Code compiles cleanly (clippy -D warnings), 90 unit tests pass.

## Files Changed

- `crates/talaria-client/src/main.rs`: Added 21-line repaint request block after egui frame build (lines 366–387), with detailed comment explaining the defect, symptom, and fix
- `CHANGELOG.md`: Prepended fix entry documenting discovery context and implementation

## Next Steps

1. Phase 5 gate remains `human_needed` with 2 unverified behaviors:
   - **SC 1** — Two-machine client over Tailscale can list, watch, drive agent tabs (transport half)
   - **SC 2** — Remote takeover latency stays ~30–60ms on direct WireGuard path (direct-path half)

2. Still needed for SC 2 evidence before token expires (~10:26 UTC):
   - Client's left panel reading: "Input to picture: about N ms"
   - Whether rung sentence changed while cursor moved
   - Takeover 'feel' impression (subjective, distinct from measurement)

3. Flicker fix does not depend on the link being up; it was a bonus diagnostic during two-machine run setup.

## Open Questions

- SC 2 measurements: recorded millisecond latency estimate + whether it stayed at rung 0 (`link.full`)?
- Has the fix eliminated the sidebar flicker in follow-up tests?

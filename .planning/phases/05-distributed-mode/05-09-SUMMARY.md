---
phase: 05-distributed-mode
plan: 09
subsystem: ui
tags: [client, egui, texture, fit-transform, input, xdotool, e2e, png, winit]

# Dependency graph
requires:
  - phase: 05-distributed-mode
    provides: "05-03's `FrameHeader`, its fixed 51-byte layout, `FRAME_HEADER_LEN`, the six structural refusals its decoder makes, and `InputMessage`'s both-or-neither key rule — all reused rather than restated"
  - phase: 05-distributed-mode
    provides: "05-06's shared key-name table in `keyutils.rs`, its server-side sequence rule, its viewport containment test and its no-cached-position discipline — the client half of which this plan supplies"
  - phase: 05-distributed-mode
    provides: "05-07's `talaria-client` crate, its connection thread, its `ConnectionState`, its chrome-geometry hook and `harness.start_client`/`client_rects`/`wait_for_client_rect`"
  - phase: 05-distributed-mode
    provides: "05-08's frame pump: the hold that makes a background tab answer a hit test, keyframe-then-delta selection, the per-attachment sequence, and `last_applied_input` stamped into every header"
provides:
  - "`present::fit` / `present::Fit` — the one transform (scale plus origin plus the page's own size), computed in one place, with `Fit::surface_rect` as the single expression of the client's own interface offset"
  - "`present::decide` / `present::Application` — the ordering rule as one comparison, covering the before-any-keyframe case and the delta-after-a-higher-keyframe case by construction"
  - "`present::decode` — a frame payload into the interface's own image type, refusing anything that is not eight-bit RGBA of exactly the declared size"
  - "`present::Presenter` — keyframe and delta texture application through egui's own partial upload, surface resize on a differing keyframe, and clear on detach"
  - "`input::page_position` — the inverse of `fit`, with the floor rounding rule and no position moved to the nearest edge"
  - "`input::Capture` / `input::Aim` — window-system input into wire input, the monotonic sequence that outlives a reconnect, and the three refusals"
  - "`net::Outbound` — the client's send half, a queue drained by the connection thread, with `control` and `input` as its whole vocabulary"
  - "`net::Update::{Attached, Detached, Refused, Frame}` and `net::FramePayload` — the frame channel decoded rather than discarded, with a `Debug` that prints a length rather than a page of pixels"
  - "Client controls: a per-row Watch/Stop control, an attachment indication, a recorded `page.area` and `page.surface`, and the `reading.*` numeric channel"
  - "`tests/e2e/remote_view_test.py` sections 35–48 — the real client binary driven by a real pointer and real keystrokes, asserted over the control socket"
affects: [05-10, 05-11]

# Tech tracking
tech-stack:
  added: ["png 0.17 (already in the lock; an edge onto an existing node, zero added packages)"]
  patterns:
    - "One transform, computed in one module and inverted in another, with the offset that both need expressed once as a method both call — so applying it twice or forgetting it is not expressible at either end"
    - "Refuse rather than move to the nearest valid value: a position outside the picture sends nothing, because the nearest valid value is a click nobody made"
    - "A test-only numeric channel riding an existing geometry hook, with the convention stated at the function rather than inferred by its reader"
    - "A cross-crate agreement test by `include_str!` of the other crate's source, where linking it is forbidden — the list is walked, not asserted in a comment"
    - "Take the two values that travel rather than the window-system event that carries them, so the seam is both testable and honest about what crosses a machine boundary"

key-files:
  created:
    - crates/talaria-client/src/present.rs
    - crates/talaria-client/src/input.rs
  modified:
    - crates/talaria-client/src/net.rs
    - crates/talaria-client/src/main.rs
    - crates/talaria-client/src/chrome.rs
    - crates/talaria-client/Cargo.toml
    - Cargo.lock
    - tests/e2e/remote_view_test.py
    - CHANGELOG.md
    - .planning/phases/05-distributed-mode/deferred-items.md

key-decisions:
  - "The picture is uploaded through egui's own texture manager (`TextureHandle::set` and `set_partial`) rather than through a raw GL paint callback: `set_partial` *is* the sub-region upload the tile protocol exists for, it issues the same `glTexSubImage2D`, and it needs no `unsafe` in a crate whose acceptance criteria forbid `unwrap()` and whose whole point is being small"
  - "The scale denominator composes with the **fit**, not with the upload: the page's size is what is fitted and the texture is stretched across the result, so a half-resolution frame occupies the same on-screen rectangle as a full one"
  - "`Fit` carries the page's size in device pixels, so the inverse can refuse a coordinate outside the page without recomputing anything the forward direction already knew"
  - "The fitted surface's rectangle is half-open: its top-left is inside and its bottom-right is not, which makes the page's last pixel reachable and the pixel past it unreachable without either being a special case"
  - "The client's controls are a fixed-width **side** panel rather than a strip above the page, so the page area's aspect ratio has nothing to do with the server's and a letterboxed margin is a real place a pointer can be"
  - "`Capture::key` takes the logical key and the state, not a `winit::event::KeyEvent`: most of that type's fields have no meaning on the far end and one of them is not constructible outside winit at all"
  - "A wheel travels in whichever mode the window system reported and is not converted: how far a line is is a question about the page being scrolled, and the server is the end that knows the answer"
  - "The attach acknowledgement's size is deliberately not used to size the surface — it is a starting size and never a contract, and sizing from the weaker fact would put a wrongly-scaled frame on screen for exactly one frame, during which a click would land somewhere else"
  - "The end-to-end suite reads the tab's viewport with `evaluate` rather than with `screenshot`, because `screenshot` re-hides a held tab (see Deviations) — and the function says so at its own doc comment rather than in a commit message"

patterns-established:
  - "Pattern: when a criterion forbids a word, say the thing without the word and record the substitution — `input.rs` says 'moved to the nearest edge' where the prose would have said the forbidden one, and the meaning is unchanged"
  - "Pattern: expose the two numbers that distinguish 'connected' from 'showing the page', because every other symptom of a stalled picture is indistinguishable from a static one"
  - "Pattern: a refused message must not spend a sequence number, or the far end's echo reports a value nothing ever sent"

requirements-completed: [DIST-01, DIST-02]

coverage:
  - id: D1
    description: "The client applies a keyframe to its whole surface and a delta to exactly the region the header names, uploading per frame rather than rebuilding"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "cargo test -p talaria-client present::tests::the_first_frame_of_an_attachment_must_be_a_keyframe, ::a_delta_names_exactly_its_own_region, ::a_keyframe_of_a_different_size_replaces_and_resizes, ::a_delta_against_a_surface_of_another_size_is_discarded"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'CLIENT SHOWING' (the applied frame sequence advances after a page change)"
        status: pass
    human_judgment: false
  - id: D2
    description: "A frame whose sequence does not exceed the last applied for that attachment is discarded, including a delta arriving after a higher-numbered keyframe"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "present::tests::a_sequence_that_does_not_exceed_the_last_applied_is_discarded, ::a_delta_arriving_after_a_higher_numbered_keyframe_is_discarded"
        status: pass
    human_judgment: false
  - id: D3
    description: "A payload that does not decode costs one frame: the picture already held is untouched and the connection undisturbed"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "present::tests::a_payload_that_is_not_an_image_decodes_to_nothing, ::a_payload_of_the_wrong_size_decodes_to_nothing; net::tests::a_frame_shorter_than_its_own_header_is_discarded, ::a_header_sized_run_of_bytes_that_is_not_a_header_is_discarded_too"
        status: pass
    human_judgment: false
  - id: D4
    description: "One fit transform, defined in present.rs and inverted in input.rs, with the client's own interface offset expressed once"
    requirement: "DIST-02"
    verification:
      - kind: other
        ref: "grep -c 'fn fit' crates/talaria-client/src/present.rs is 1 and crates/talaria-client/src/input.rs is 0; the offset appears once, in Fit::surface_rect, which both directions call"
        status: pass
      - kind: unit
        ref: "present::tests::a_wider_content_area_leaves_the_margin_on_the_left_and_right, ::a_taller_content_area_leaves_the_margin_above_and_below, ::an_exactly_matching_content_area_has_no_margin_and_unit_scale; input::tests::the_top_left_of_the_surface_is_the_pages_own_origin, ::one_point_inside_the_far_edges_is_the_pages_last_pixel"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'CLIENT AIMED LOWER', 'CLIENT DECOY REACHABLE', 'CLIENT AIMED UPPER'"
        status: pass
    human_judgment: false
  - id: D5
    description: "A pointer outside the fitted surface — the letterboxed margin, or the client's own controls — sends nothing rather than a position moved to the page's edge"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "input::tests::one_point_outside_the_left_edge_yields_nothing, ::one_point_outside_the_right_edge_yields_nothing, ::one_point_outside_the_top_edge_yields_nothing, ::one_point_outside_the_bottom_edge_yields_nothing, ::a_position_over_the_clients_own_interface_yields_nothing"
        status: pass
      - kind: other
        ref: "grep -ci 'clamp' crates/talaria-client/src/input.rs is 0"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'MARGIN SILENT' and 'CLIENT CHROME'"
        status: pass
    human_judgment: false
  - id: D6
    description: "The floor rounding rule, so a pointer anywhere within the screen pixel showing page pixel n means n"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "input::tests::the_rounding_rule_is_the_floor_rather_than_the_nearest"
        status: pass
      - kind: other
        ref: "grep -ci 'floor' crates/talaria-client/src/input.rs is 5"
        status: pass
    human_judgment: false
  - id: D7
    description: "The scale denominator is honoured in both the upload size and the fit, so a half-resolution frame occupies the same on-screen area as a full one"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "present::tests::a_half_resolution_frame_occupies_the_same_area_as_a_full_one, ::the_upload_size_of_a_half_resolution_keyframe_is_the_texture_size, ::a_keyframe_whose_denominator_changed_is_a_replacement_at_the_new_scale; input::tests::a_half_resolution_picture_maps_to_the_pages_own_coordinates"
        status: pass
    human_judgment: false
  - id: D8
    description: "The client's input sequence starts above zero, strictly increases, is never reset by a reconnect, and is not spent on a message that is not sent"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "input::tests::the_sequence_starts_above_zero_and_strictly_increases, ::a_sequence_is_not_spent_on_a_message_that_is_not_sent"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'CLIENT SHOWING' (the echoed input sequence reaches the one the client sent)"
        status: pass
    human_judgment: false
  - id: D9
    description: "The three refusals: nothing while not connected, nothing for a tab not attached to, nothing for a key this build cannot name"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "input::tests::nothing_is_sent_while_the_connection_is_not_established, ::nothing_is_sent_for_a_tab_this_client_is_not_attached_to, ::a_key_this_build_cannot_name_types_nothing_rather_than_something_else, ::a_button_with_no_known_pointer_position_sends_nothing"
        status: pass
    human_judgment: false
  - id: D10
    description: "Every key name the client can emit is one the server's shared table maps, asserted by walking the list rather than by inspection"
    requirement: "DIST-02"
    verification:
      - kind: unit
        ref: "input::tests::every_name_this_client_can_emit_is_one_the_server_maps, ::the_super_key_is_folded_onto_the_name_the_server_knows"
        status: pass
    human_judgment: false
  - id: D11
    description: "The server's tab is never resized because a viewer attached: the client adapts to the page and asks for nothing"
    requirement: "DIST-02"
    verification:
      - kind: other
        ref: "grep -rn 'Viewport' crates/talaria-client/src/ returns 0 — no wire message requesting a server-side resize exists in the client at all; grep -ci 'resize' present.rs is 5, all of the client's own surface"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'VIEWPORT UNMOVED' (the tab's viewport read before and after an attachment from a differently-sized client)"
        status: pass
    human_judgment: false
  - id: D12
    description: "A human at a client can take over an agent's live session: click both directions, type, and watch — proven with the real binary, a real pointer and real keystrokes, asserted over the control socket"
    requirement: "DIST-01"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'CLIENT EMPTY STATE', 'CLIENT LISTED', 'MISMATCHED', 'CLIENT ATTACHED', 'CLIENT AIMED LOWER', 'CLIENT DECOY REACHABLE', 'CLIENT AIMED UPPER', 'CLIENT TYPED'"
        status: pass
    human_judgment: false
  - id: D13
    description: "Remote keyboard input reaches a held-but-not-focused background agent tab — 05-08's open question"
    requirement: "DIST-02"
    verification:
      - kind: e2e
        ref: "python3 tests/e2e/remote_view_test.py — 'CLIENT TYPED' (the field of a background agent tab reads 'hey' after real keystrokes at the client's window, with the local human in the Me view)"
        status: pass
    human_judgment: false
  - id: D14
    description: "The server's local code paths are unchanged by this plan"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "git status --porcelain crates/talaria-shell/src/ is empty; git diff --stat tests/e2e/takeover_test.py and tests/e2e/panel_click_test.py are both empty"
        status: pass
      - kind: e2e
        ref: "takeover_test and panel_click_test both PASS unmodified inside python3 tests/e2e/run_all.py"
        status: pass
    human_judgment: false
  - id: D15
    description: "Success Criterion 1's two-machine half: a client on a *second* machine, over Tailscale, driving the server's agent tabs"
    verification: []
    human_judgment: true
    rationale: "The end-to-end suite runs one shell and one client on one machine over loopback. That proves the protocol, the authorisation, the transform and the takeover; it proves nothing about TLS termination, the tailnet `Host`, relay behaviour or two clocks. Two machines are not something a suite on one machine can produce, and 05-11 owns the script."

# Metrics
duration: 175min
completed: 2026-08-23
status: complete
---

# Phase 5 Plan 09: Client Present and Capture Summary

**A human at a `talaria-client` can now watch an agent's tab and take it over — clicking on the
element they aimed at in both directions, typing into a page the local human is not even looking at,
and sending nothing at all from the letterboxed margin — through one transform defined in
`present.rs` and inverted in `input.rs`, with the server's tab never resized because somebody
started watching.**

## Performance

- **Duration:** 175 min
- **Tasks:** 3 of 3, each committed on its own
- **Files created:** 2 (`present.rs`, `input.rs`); modified: 7
- **Unit tests:** 448 → **486** (+38: 401 shell, 59 client, 24 protocol, 2 mcp)
- **End-to-end:** **23/23 PASS**, `failed: none`, exit 0

## Accomplishments

- **Success Criterion 1's loopback half is closed, and closed honestly.** The end-to-end suite
  starts the **real** `talaria-client` binary against the shell's loopback listener, attaches
  through the client's **own control** with a real `xdotool` pointer, clicks links through the
  client's window, types with real keystrokes, and asserts every effect **over the control socket**
  — a different channel from the one under test, so a bug in the frame path cannot also be the thing
  reporting success. What remains is the two-machine run, which is named as a manual item (D15)
  rather than implied.

- **One transform, and the source count says so.** `fn fit` appears once in `present.rs` and zero
  times in `input.rs`. The offset that both directions need is expressed once — `Fit::surface_rect`
  — and both call it, so applying it twice or forgetting it is not expressible at either end. The
  end-to-end proof is the client-side counterpart of 05-06's: a click aimed at the lower link
  reaches the lower link, a click aimed at the decoy band one toolbar-height above it reaches the
  **decoy** (so "not the decoy" is a statement about where the click went), and the upper link is
  hit in the other direction.

- **Nothing is moved to the nearest valid position.** `grep -ci 'clamp'` over `input.rs` is 0, and
  the module header says what it does instead and why: a click at the edge of the page is a click
  nobody made, at the place a confirm button lives. Unit-tested one point outside each of the four
  edges, exactly on each boundary, and one point inside — plus, end to end, a real click 127 points
  above the picture and inside the client's page area that reached the server not at all.

- **The viewer adapts to the page.** `Viewport` appears **nowhere** in the client's source: there is
  no code path that asks the server for a size. The suite reads the tab's viewport before and after
  an attachment from a client whose window it has deliberately resized to 1000×700 (page area
  644×684 points against a 1200×800 tab, so neither the size nor the aspect ratio matches) and finds
  it unchanged.

- **05-08's open question is answered rather than deferred again.** Remote keystrokes **do** reach a
  held-but-not-focused background agent tab. The local human sits in the Me view looking at their
  own tab throughout; the field on the agent's page reads `hey`. No hold was weakened to get there.
  `deferred-items.md` records the resolution against the entry that raised it.

- **"Connected" is told from "showing the page".** The client reports the last frame sequence it
  applied and the last input sequence the server echoed, on the same geometry line its control
  rectangles ride. The suite asserts the frame sequence advanced after a page change and that the
  echo reached the sequence the client had sent. Without those two numbers, a client whose picture
  never arrived would pass every other assertion in the section.

- **A real bug found by a test written to pass.** See Deviations: `screenshot` on a tab a viewer is
  watching silently stops that viewer's clicks landing. It is a shell bug, out of this plan's scope,
  recorded rather than fixed, and routed around.

### The transform, exactly

Given a frame declaring `frame_width × frame_height` at `scale_denominator` *d*, and a content area
*A* (the client's window minus its own controls, in logical points):

```
page       = (frame_width · d, frame_height · d)          # device pixels, the space input lives in
scale      = min(A.width / page.width, A.height / page.height)
origin     = max(((A.size − page · scale) / 2), 0)        # the letterbox margin, two sides
surface    = Rect(A.min + origin, page · scale)           # Fit::surface_rect — the one expression
                                                          # of the client's own interface offset
```

Forward, `present.rs` draws the texture into `surface`. Inverse, `input.rs`:

```
page_x = floor((window.x − surface.min.x) / scale)        # floor, not nearest
page_y = floor((window.y − surface.min.y) / scale)
send only if 0 ≤ page_x < page.width and 0 ≤ page_y < page.height
```

**The floor, and why it is a real question.** A pointer anywhere within the screen pixel that shows
page pixel *n* maps to *n*, which is what a human means by "I clicked on that pixel". Rounding to
nearest gives the right half of every pixel to its neighbour — arbitrary until somebody aims at a
one-pixel target, and then not. Unit-tested at twice life size, where one page pixel is two points
wide and both of them must map to the same page pixel.

**The rectangle is half-open**: its top-left corner is inside and its bottom-right corner is not,
which makes the page's last pixel reachable and the pixel past it unreachable without either being a
special case.

**How the denominator composes.** With the fit, not with the upload. A half-resolution frame is a
*smaller picture of the same page*, so it is the page's size that is fitted and the texture is
stretched across the result — asserted by a test that fits a 1200×800 surface at *d*=1 and a 600×400
surface at *d*=2 into the same area and requires the two `surface_rect`s to be equal. Fitting the
texture's own size instead would shrink a page on screen the first time 05-10's ladder stepped down,
which reads as a rendering fault rather than as a protocol one.

### The ordering rule, and the client's three refusals

**Ordering** is one comparison. A frame applies only when its sequence **exceeds** the last one
applied for that attachment; a frame arriving before any keyframe is discarded because a delta has
nothing to composite over; a delta whose declared frame size is not the size held is discarded
because its tile coordinates were computed against a different picture. The awkward case — a delta
arriving after a keyframe with a higher sequence — needs no rule of its own: the keyframe already
raised the high-water mark past it.

**The three refusals** sit at the top of the send path: not connected, not attached, and a key this
build cannot name. Each is a check with a one-line comment, and each complements rather than
duplicates the server's: the server refuses because it must not trust a peer, and the client refuses
because a message it knows will be refused spends a sequence number and confuses the latency
estimate that reads it. A fourth property falls out of the same discipline and is tested
separately — **a refused message must not spend a sequence number**, or the frame header's echo
would report a value nothing ever sent.

### The end-to-end section's geometry, for a later reader

| Quantity | Value in the run recorded here |
|----------|-------------------------------|
| Server tab viewport | 1200 × 800 device pixels |
| Client window after `xdotool windowsize` | 1000 × 700 |
| Client page area | 644 × 684 logical points |
| Fitted picture | 644 × 429 points at scale ≈ 0.537 |
| Letterbox margin | 127 points, top and bottom |

Neither the size nor the aspect ratio matches, so the transform is exercised rather than
accidentally being the identity, and the margin the suite aims at is a real place a pointer can be.
The client's controls are a fixed 340-point **side** panel rather than a strip above the page,
which is what guarantees that.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 — Blocking] `png` added to the client's manifest**

- **Found during:** Task 1, writing `present::decode`.
- **Issue:** the frame channel carries PNG bytes and the client had no decoder. Every alternative
  was worse: hand-writing an inflate implementation in a crate whose acceptance criteria forbid
  `unwrap()`, or not decoding at all and failing the plan's own behaviour list.
- **Why this is not the package install Rule 3 excludes:** nothing was installed and nothing was
  resolved. `png = "0.17"` is already declared in `[workspace.dependencies]`, already in
  `Cargo.lock` at 0.17.16, and is the **other end of this exact line** — the server's frame encoder.
  The manifest gained `png = { workspace = true }` and the lock gained exactly one line,
  `"png 0.17.16"` inside `talaria-client`'s dependency array, hand-edited the way 05-07 landed this
  crate's dependencies. `cargo build --release --locked` is the proof: a resolution would fail
  against the committed lock rather than quietly rewriting it.
- **Cost:** `git diff --stat Cargo.lock` is `1 +`. Zero added packages; `png`'s own five
  dependencies were already in the tree through the shell.
- **The root `Cargo.toml` is untouched.** The plan's verification item 6 names `Cargo.toml` and
  `Cargo.lock`; the first holds exactly, the second is the one line above, gated and stated.

**2. [Rule 2 — Missing critical functionality] `reading.points_per_pixel`**

- **Found during:** Task 3, writing the screen-coordinate conversion.
- **Issue:** every rectangle the client records is in logical points, and a suite converting one to a
  screen coordinate needs the client's **own** scale factor to finish the job. Under Xvfb it is 1
  and the omission would have been invisible; on the machine pair this whole phase exists for the two
  windows may be on displays with different scale factors, which is the ordinary case.
- **Fix:** one more `reading.` entry, from `ui.ctx().pixels_per_point()`.

### Deliberate divergences from the plan text

**3. The word the acceptance criterion forbids is not in the module it forbids it in**

The plan's Task 2 action asks the module header to say "it does not clamp"; its acceptance criterion
requires `grep -ci 'clamp' crates/talaria-client/src/input.rs` to be **0**, and the prompt repeats
that as a hard gate. The criterion wins, and the meaning is preserved rather than dropped: the header
says the module "does not pull an outside position to the nearest edge", and spells out the failure
in full — "a click they did not make, at the edge of the page, which is where a confirm button
lives". `present.rs` and this summary are free to use the ordinary word and do.

**4. The picture goes through egui's texture manager, not a raw GL paint callback**

The plan's Task 1 action says to "hold the surface as a texture in the graphics context" and "draw
it through the paint callback", by analogy with how the server's chrome draws its engine's surface.
The analogy does not fully transfer, and the difference favours the simpler path. The server's
chrome has no *pixels* — it has a texture the compositor owns and must hand to GL directly. This
client has the pixels, on the CPU, as decoded bytes. `egui::TextureHandle::set_partial` **is** the
sub-region upload the tile protocol exists for: it issues the same `glTexSubImage2D` through
`egui_glow`'s painter, and it needs no `unsafe`, no manual GL state and no framebuffer juggling in a
crate whose criteria forbid `unwrap()`. The properties the plan asked for all hold — uploaded per
frame rather than rebuilt, a delta is a sub-region upload, the texture is resized on a differing
keyframe — and `Presenter::draw` still places it by `Fit::surface_rect` and nothing else.

**5. The attach control landed in Task 1 rather than Task 2**

Task 1's acceptance criteria require `cargo clippy --all-targets -- -D warnings` to exit 0, and a
`Presenter` with no way to attach is a `Presenter` whose outbound half is dead code. Rather than
commit a task that could not pass its own gate, Task 1 carries the per-row Watch control and the
attachment indication (the presentation half: *a picture, and a way to ask for one*), and Task 2
carries the input capture and `Outbound::input` (the driving half). Both tasks touch `chrome.rs` and
`main.rs`, which the plan's own `files_modified` already lists.

**6. The empty-state and connection assertions are made through the client's interface, not a
control-socket view-connection count**

The plan's Task 3 step 1 asks the suite to "assert the shell's control socket reports a view
connection". **No such command exists**, and adding one would be shell source this plan may not
touch. What is asserted instead is stronger in one respect and weaker in none that matters: the
client's `tabs.empty` label is drawn **only** while the connection state is `Connected`, so finding
it proves the connection reached the server *and* that the server reported zero agent tabs. The
server-side evidence for the attachment itself is `view_holds`, read over the control socket, which
is the same hook 05-08 added for exactly this reason.

## Issues Encountered

- **A real bug in the shell, found by a test written to pass.** `Shared::capture_now` takes a
  `hide_after` flag and `process_pending_captures` passes `true` for a background tab — and that
  path **does not consult `Tab::held_for_view`**. So `screenshot` on a tab a remote viewer is
  attached to re-hides the webview, and a hidden webview answers no hit test. Every visible symptom
  points elsewhere: the hold count still reads 1, and frames keep arriving with an advancing
  sequence because `view::capture` paints the tab's own offscreen context and never asks whether the
  webview is shown. Only input stops working, and it stops silently. This is 05-06's blocker
  returning through a second door.

  **Not fixed here** — this plan's criteria require `git status --porcelain crates/talaria-shell/src/`
  to be empty, and it is. Logged to `deferred-items.md` with the reproduction, the mechanism and the
  fix (`hide_after` should be `view_holds(tab) == 0`, or the re-hide should go through
  `visibility_of`, which already knows about holds). The suite routes around it: `tab_viewport` reads
  the tab's size out of the page with `evaluate`, and its doc comment says why.

- **The pump ticks when the loop wakes.** A shell sitting with no control-socket traffic and no
  window events did not deliver frames on its own timer under Xvfb; one `evaluate` was enough to
  restart the flow. This is 05-08's shape, not a regression, and 05-08's own e2e already works this
  way — its Issues section records the same lesson from the other side ("a test that waits for a
  frame must first make one exist"). Section 47 forces a repaint before each read for exactly this
  reason. Worth a look on real hardware during 05-11's manual pass, since a viewer watching a page
  nobody is touching is the normal state.

- **Two windows, one origin.** There is no window manager on the test display, so the shell's window
  and the client's both sit at (0, 0) and overlap. The client is mapped second and is therefore on
  top, which is what makes a real pointer click reach it; `click_client_point` calls
  `xdotool windowfocus --sync` first to make that explicit rather than rely on it. Every section
  that clicks the **shell's** chrome runs before the client starts, and none runs after.

- **`KeyEvent` is not constructible in a test.** `winit::event::KeyEvent` carries a
  `platform_specific` field whose Linux type implements no `Default`, so a test cannot build one.
  The seam moved to the two values that actually travel — the logical key and the state — which is
  also what the server's own key module says should cross this boundary, so the constraint pushed the
  design in the right direction rather than around it.

## Verification

| Check | Result |
|-------|--------|
| `cargo build --release --locked` | exit 0, no warnings |
| `cargo clippy --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --locked` | **486 passed**, 0 failed (401 + 59 + 24 + 2) |
| `python3 tests/e2e/remote_view_test.py` | exit 0, `REMOTE VIEW CHECKS PASSED` |
| `python3 tests/e2e/run_all.py` (`:95`) | **23/23 PASS**, `failed: none`, exit 0 |
| `takeover_test` / `panel_click_test`, unmodified | both PASS; `git diff --stat` on both is empty |
| `git status --porcelain crates/talaria-shell/src/` | empty |
| `git diff Cargo.toml` | empty |
| `git diff --stat Cargo.lock` | `1 +` — one dependency edge, no package added |
| `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'` | **1** |
| `grep -c 'fn fit' …/present.rs` / `…/input.rs` | **1** / **0** |
| `grep -ci 'scale_denominator\|denominator' …/present.rs` | 16 |
| `grep -ci 'resize' …/present.rs` | 5 |
| `grep -rn 'Viewport' crates/talaria-client/src/` | **0** — no server-side resize request exists |
| `grep -ci 'floor' …/input.rs` | 5 |
| `grep -ci 'clamp' …/input.rs` | **0** |
| `grep -c 'seq' …/input.rs` | 26 |
| `grep -rq 'unwrap()' crates/talaria-client/src/` | no match in any of the five files |
| `grep -rEci 'danger\|no_verify\|noverify\|accept_invalid\|insecure_skip' crates/talaria-client/src/` | 0 in every file |
| `grep -c '#[test]' …/present.rs` / `…/input.rs` | **16** / **20** |
| `grep -c 'start_client' tests/e2e/remote_view_test.py` | 1 |
| `grep -ci 'xdotool' tests/e2e/remote_view_test.py` | 8 |
| `grep -ci 'margin\|letterbox' tests/e2e/remote_view_test.py` | 12 |
| `grep -ci 'upper\|lower' tests/e2e/remote_view_test.py` | 61 |
| `grep -ci 'viewport' tests/e2e/remote_view_test.py` | 24 |

Display: `TALARIA_E2E_DISPLAY=:95` throughout, per the phase's `deferred-items.md` entry — `:98`
belongs to this machine's self-hosted CI runner. One display activity at a time; no ad-hoc run was
started while the full suite was in flight.

## Commits

| Task | Commit | What |
|------|--------|------|
| 1 | `2dcf02e` | `present.rs` — the fit transform, keyframe and delta application, ordering, the surface clear, the denominator; `net.rs` — the frame channel decoded and the outbound half; `chrome.rs` — the page area, the fitted rectangle, the Watch control, the readings; the `png` edge |
| 2 | `a8a1e66` | `input.rs` — the inverse transform, the floor rule, the outside-the-surface refusal, the monotonic sequence, the three refusals, the key-name agreement test; the loop wiring |
| 3 | `1e0f72f` | `remote_view_test.py` sections 35–48, the `CHANGELOG.md` entry, and the two `deferred-items.md` entries |

## What the next plans inherit

- **05-10 — the rate controller.** `SurfaceSize::denominator` is honoured end to end already: a
  frame at *d* > 1 is decoded at the texture's own size and fitted to the page's, so the ladder can
  step down without a client change. The client's own half of the latency estimate is
  `Presenter::last_applied_input` beside `Capture::last_seq` — both read off one clock, nothing
  synchronised. **`ClientView::Cadence` is not sent by the client**: `Outbound::control` will carry
  it, but nothing constructs one yet, and that is 05-10's. And the fourth client surface (link
  quality) is the one 05-07's `deferred-items.md` entry says triggers a design contract.
- **05-11 — `VERIFICATION.md`.** One manual item is **removed**: remote keyboard to a
  held-but-not-focused background tab is now proven (D13). One is **unchanged**: the hardware
  readback re-measure. One is **added**: Success Criterion 1's two-machine run over Tailscale
  (D15) — the suite proves the protocol, the authorisation, the transform and the takeover over
  loopback, and nothing about TLS termination, the tailnet `Host` or relay behaviour. And one
  **bug** is waiting: the `screenshot`-hides-a-held-tab entry in `deferred-items.md`, which is a
  small shell fix plus a regression test.

## User Setup Required

None. No new configuration file, no new environment variable, and no new package in the dependency
tree — one existing crate gained one consumer.

## Self-Check: PASSED

- `.planning/phases/05-distributed-mode/05-09-SUMMARY.md` — FOUND
- `crates/talaria-client/src/present.rs` — FOUND
- `crates/talaria-client/src/input.rs` — FOUND
- `tests/e2e/remote_view_test.py` — FOUND
- `.planning/phases/05-distributed-mode/deferred-items.md` — FOUND (two entries appended)
- `CHANGELOG.md` — FOUND (one `### Added` entry)
- commit `2dcf02e` (Task 1) — FOUND
- commit `a8a1e66` (Task 2) — FOUND
- commit `1e0f72f` (Task 3) — FOUND

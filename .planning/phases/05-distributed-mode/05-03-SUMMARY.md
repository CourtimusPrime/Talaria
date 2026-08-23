---
phase: 05-distributed-mode
plan: 03
subsystem: api
tags: [protocol, wire-format, websocket, multiplexing, serde, frame-header, input, refactor]

# Dependency graph
requires:
  - phase: 02-mcp-agent-control
    provides: "`Command`, `Outcome`, `ResultPayload`, `TabInfo` and `Event` — the vocabulary that goes onto the view wire unchanged, and `ChromeRect`'s refusal, whose argument the view channel reuses verbatim"
  - phase: 04-authenticated-remote-transport-v2
    provides: "the HTTP listener the view route will mount on, and `http.rs`'s habit of citing a decision by name inline"
  - phase: 05-distributed-mode
    plan: 01
    provides: "`axum`'s `ws` feature and `tokio-tungstenite 0.29.0` already in the lockfile, so this plan needed no dependency change at all"
provides:
  - "`crates/talaria-protocol/src/wire.rs` — the multiplexed view envelope: five channel tags, a fixed 51-byte little-endian frame header, the input message and the two view control messages, with 21 tests"
  - "The final channel-tag assignment: `0x01` control, `0x02` tabs, `0x03` event, `0x04` input, `0x10` frame — a low JSON block and a high binary block, with the gap between them load-bearing"
  - "Ten structural refusals, each with its own test, none of which substitutes a default"
  - "`FrameHeader::last_applied_input` — the field that makes input-to-photon latency measurable with no clock shared between the two machines, and lets a client discard a frame that predates its own most recent click"
  - "`crates/talaria-protocol/src/local.rs` — the Unix socket path, directory and `getuid` shim, moved verbatim behind `#[cfg(unix)]` and deliberately not re-exported"
  - "A corrected `lib.rs` header that says the crate is the shared vocabulary and names the three wires that carry it"
  - "A corrected `control.rs` header that records *why* the control socket stayed local: `SO_PEERCRED` has no TCP equivalent"
affects: [05-05, 05-06, 05-07, 05-08, 05-09, 05-10, 05-11]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "A design decision expressed as an absence in a type rather than as a runtime check: the view channel has no variant that can carry the agent tool vocabulary, so a remote click is unreachable from the MCP tool surface by construction and a `grep` returning 0 is the assertion"
    - "Structural refusal and semantic refusal split by *what state the check needs*: anything decidable from the bytes alone lives in the protocol crate; anything needing a particular tab's viewport, a particular connection's high-water mark or a particular tab's owner is left to the shell, and the module header says so rather than leaving it as an omission"
    - "A fixed binary layout documented as a table in the type's own doc comment, with `to_bytes` and `from_bytes` adjacent, so a field added to one is visibly missing from the other — the same discipline `settings.rs` applies to its JSON pair"
    - "A refusal with no field to be informative with: `ServerView::Refused` is a unit variant, so 'that tab is the human's' and 'there is no such tab' are byte-identical answers"
    - "Tests named for the property they pin rather than for the function they call — `a_tile_that_exceeds_the_frame_it_declares_yields_no_frame_header`, not `test_from_bytes`"

key-files:
  created:
    - crates/talaria-protocol/src/wire.rs
    - crates/talaria-protocol/src/local.rs
  modified:
    - crates/talaria-protocol/src/lib.rs
    - crates/talaria-shell/src/control.rs
    - crates/talaria-mcp/src/socket.rs
    - CHANGELOG.md

key-decisions:
  - "The channel tags are `0x01` control, `0x02` tabs, `0x03` event, `0x04` input, `0x10` frame. The gap between `0x04` and `0x10` is the design: the low block is human-readable JSON and the high block is binary, so one byte tells a reader which world it is in before it decodes anything. New JSON channels take the next low tag; new binary channels the next high one"
  - "The frame header is exactly 51 bytes, little-endian, named once as `FRAME_HEADER_LEN`: version(1) kind(1) scale_denominator(1) tab_id(8) frame_seq(8) last_applied_input(8) tile_x(4) tile_y(4) tile_width(4) tile_height(4) frame_width(4) frame_height(4). The version byte is first so a decoder can refuse before it reads anything else"
  - "The view channel carries no agent tool vocabulary, per D-05-02 read literally — a viewer lists, watches, clicks, scrolls and types, and cannot open, navigate, evaluate, close or download. Expressed as an absence in the enum rather than as a check, because an absence survives a later reader and a check invites an exception"
  - "`ServerView::Refused` is a unit variant with no reason field at all. A refused attach must not distinguish 'that tab belongs to the human' from 'no such tab', or the channel becomes an enumeration oracle for the human's own browsing"
  - "`TabList.tabs` carries no `#[serde(default)]`. An empty list is a legitimate state and an absent field is a malformed message; a default would collapse the two, which is the exact class of silent substitution this wire refuses"
  - "The Unix helpers were **moved**, not re-exported. A `pub use local::*` would have left the crate root's public surface exactly as it was, which is the thing being corrected. Both callers — and there are exactly two — follow the move by name"
  - "`control.rs`'s new header records the *reason* the control socket did not become the distributed transport rather than only deleting the wrong claim: the `Hello` line is self-asserted and is safe only because the kernel already vouched that the peer runs as the same OS user, and `SO_PEERCRED` has no TCP equivalent"
  - "The `///` doc on the `pub mod local;` declaration was demoted to a `//` comment: rustdoc resolves an outer module doc's intra-doc links in the *declaring* file's scope, so `[`current_uid`]` written there is unresolvable. The prose lives in `local.rs`'s own `//!` header, where its links resolve"

patterns-established:
  - "Pattern: a refusal-per-row test suite — every structural refusal the module makes gets its own named test, so a later relaxation of one check cannot hide inside another test's assertion"
  - "Pattern: test the largest value every field can hold in the same round trip, which catches a width mistake in the byte layout that a plausible-looking sample value would not"
  - "Pattern: test the overflow case of a bounds check explicitly (`tile_x = u32::MAX`, `tile_width = 2`), because a `checked_add` and a wrapping add pass the ordinary case identically"

requirements-completed: [DIST-01]

coverage:
  - id: D1
    description: "The multiplexed view wire has a named vocabulary in the crate that owns Talaria's vocabulary: five channel tags, a fixed-layout frame header, an input message and two view control messages, reusing `TabInfo`, `Outcome` and `Event` unchanged"
    requirement: "DIST-01"
    verification:
      - kind: unit
        ref: "cargo test -p talaria-protocol --locked — wire::tests::every_channel_tag_round_trips_through_its_meaning, client_view_messages_round_trip, server_view_messages_round_trip, every_input_kind_round_trips, a_frame_header_round_trips_every_field, a_frame_header_round_trips_the_largest_value_each_field_can_hold, a_tab_list_round_trips_preserving_order"
        status: pass
    human_judgment: false
  - id: D2
    description: "A structurally malformed wire message is refused, never defaulted — ten separate refusals with a test each"
    requirement: "DIST-01"
    verification:
      - kind: unit
        ref: "cargo test -p talaria-protocol --locked — a_byte_that_names_no_channel_is_refused, a_slice_shorter_than_the_layout_yields_no_frame_header, an_unrecognised_format_version_yields_no_frame_header, an_unrecognised_frame_kind_yields_no_frame_header, a_zero_scale_denominator_yields_no_frame_header, a_zero_sized_tile_yields_no_frame_header, a_tile_that_exceeds_the_frame_it_declares_yields_no_frame_header, an_input_message_missing_a_field_is_refused, a_coordinate_that_is_not_a_finite_number_is_refused_rather_than_clamped, a_key_naming_both_a_character_and_a_named_key_is_refused, a_key_naming_neither_a_character_nor_a_named_key_is_refused, an_absent_tab_list_field_is_refused_rather_than_defaulted"
        status: pass
    human_judgment: false
  - id: D3
    description: "The agent tool vocabulary is unreachable from the view channel and no wire input type is reachable from the MCP tool surface — both by absence rather than by check (T-05-04, high)"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "grep -c 'Command' crates/talaria-protocol/src/wire.rs is 0; grep -c 'wire::' crates/talaria-mcp/src/tools.rs is 0; grep -c 'MouseMove\\|MouseButton\\|WheelEvent\\|KeyboardEvent' crates/talaria-protocol/src/lib.rs is 0"
        status: pass
    human_judgment: false
  - id: D4
    description: "The server's refusal carries no discriminating reason, so the view channel is not an enumeration oracle for the human's tabs (T-05-05, high)"
    requirement: "DIST-01"
    verification:
      - kind: unit
        ref: "cargo test -p talaria-protocol --locked — wire::tests::a_refusal_carries_no_discriminating_reason asserts the whole serialised form is {\"view\":\"refused\"}"
        status: pass
    human_judgment: false
  - id: D5
    description: "The Unix socket path, directory and getuid shim moved out of the crate root into a `#[cfg(unix)]` local module, not re-exported, with both callers following by name"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "grep -c 'pub fn socket_path' local.rs is 1 and lib.rs is 0; grep -c 'pub use local' lib.rs is 0; grep -c 'cfg(unix)' lib.rs is 1; grep -c 'local::' in both talaria-mcp/src/socket.rs and talaria-shell/src/control.rs is 1"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/run_all.py on TALARIA_E2E_DISPLAY=:95 — control_socket_test, mcp_client_test and single_instance_test all PASS; 22/22, failed: none"
        status: pass
    human_judgment: false
  - id: D6
    description: "Both module headers that misdescribed the crate as the distributed wire now say what it actually is, with reasons rather than deletions"
    requirement: "DIST-01"
    verification:
      - kind: other
        ref: "! grep -qi 'distributed-mode client later\\|precursor to the distributed' lib.rs control.rs; grep -ci 'shared vocabulary' lib.rs is 1; grep -ci 'peer' control.rs is 16; cargo doc --no-deps -p talaria-protocol clean"
        status: pass
    human_judgment: false
  - id: D7
    description: "Nothing existing changed shape: all three transports, the three pre-existing round-trip tests and the whole e2e suite are untouched, and the lockfile is unmoved"
    verification:
      - kind: unit
        ref: "cargo test --locked — 291 + 24 + 2 = 317 passed (baseline 296), 0 failed; tests::round_trip_request, tests::chrome_rects_round_trip, tests::reply_shapes all unmodified and passing"
        status: pass
      - kind: e2e
        ref: "python3 tests/e2e/run_all.py — 22/22, failed: none, exit 0"
        status: pass
      - kind: other
        ref: "git diff --stat Cargo.toml Cargo.lock crates/talaria-protocol/Cargo.toml empty; grep -A1 'name = \"primeorder\"' Cargo.lock | grep -c '0.14.0-rc.14' is 1; cargo clippy --all-targets --locked -- -D warnings and cargo build --release --locked both exit 0"
        status: pass
    human_judgment: false

# Metrics
duration: 30min
completed: 2026-08-23
status: complete
---

# Phase 5 Plan 03: The `wire` and `local` Modules Summary

**The view channel now has a named, tested vocabulary — five channel tags, a fixed 51-byte frame
header carrying the last-applied-input sequence, and ten structural refusals that produce nothing
rather than a plausible default — and the crate that owns it stopped claiming a Unix filesystem path
was inherent to it, in the two module headers and in its own public surface.**

## Performance

- **Duration:** 30 min
- **Started:** 2026-08-23T11:50:00Z
- **Completed:** 2026-08-23T12:19:00Z
- **Tasks:** 2 of 2
- **Files modified:** 6 (2 created, 4 modified)

## Accomplishments

- **The view wire has a vocabulary, and two of its properties are absences.**
  `crates/talaria-protocol/src/wire.rs` defines the multiplexed envelope: `Channel` with its five
  tags, `TabList`, `FrameHeader` with `FrameKind`, `InputMessage` with `PointerButton` /
  `ButtonAction` / `WheelMode`, and `ClientView` / `ServerView`. `grep -c 'Command' wire.rs` is **0**
  and `grep -c 'winit' wire.rs` is **0** — the view channel carries no agent tool vocabulary and no
  window-system type reaches the wire. Neither is enforced by a check; there is simply no type that
  connects them.

- **Ten structural refusals, one test each, none of which defaults.** A tag naming no channel; a
  slice shorter than the layout (asserted for *every* length from 0 to 50); an unknown format
  version; an unknown frame kind; a zero scale denominator; a zero-area tile; a tile that does not
  fit inside the frame it declares — including the `checked_add` overflow case, which a wrapping add
  would have passed; a missing JSON field; a coordinate that is not a finite number, tested both as a
  constructed `NAN`/`INFINITY` and as the `1e400` literal serde_json decodes to an infinity; and a
  key naming both or neither a character and a named key.

- **`last_applied_input` is in the header, with its purpose written down.** It is the one field
  whose job is not obvious from its name and the one a later reader would delete as redundant with
  `frame_seq`, so its doc comment states both jobs: input-to-photon latency measurable off the
  client's own clock alone, and dropping a frame that predates the viewer's most recent click.

- **The move is a move.** `socket_dir`, `socket_path`, `ensure_socket_dir`, `current_uid` and the raw
  `extern "C" getuid` shim now live in `local.rs` behind `#[cfg(unix)]`, verbatim, with their doc
  comments intact. `grep -c 'pub use local' lib.rs` is **0** — a re-export would have left the root's
  public surface exactly as it was, which is the thing being corrected. Both callers (there are
  exactly two) name the module.

- **Both misdescribing headers now say what the thing is, and why.** `lib.rs` says the crate is the
  shared vocabulary, names the three wires that carry it, and keeps the request/reply correlation
  paragraph that has held. `control.rs` stops calling itself "the in-process precursor to the
  distributed protocol" and records the reason it stayed local rather than only the correction:
  `SO_PEERCRED` has no TCP equivalent, so the same handshake cannot admit a peer on another machine.

- **Nothing existing moved.** `cargo test --locked` is **317 passed** against a 296 baseline, with
  the three pre-existing round-trip tests unmodified. e2e is **22/22, `failed: none`**. The lockfile
  is byte-unchanged and `primeorder` is still pinned at `0.14.0-rc.14`.

## The wire, as landed

### Channel tags

| Tag | Channel | Payload |
|-----|---------|---------|
| `0x01` | control | JSON `ClientView` / `ServerView` |
| `0x02` | tabs | JSON `TabList` (a `Vec<TabInfo>` behind a required field) |
| `0x03` | event | JSON `talaria_protocol::Event` |
| `0x04` | input | JSON `InputMessage` |
| `0x10` | frame | `FrameHeader` (51 bytes) then raw PNG bytes |

The gap between `0x04` and `0x10` is deliberate and is documented on `Channel`: the low block is
human-readable JSON and the high block is binary, so a tag byte tells a reader which world it is in
before it decodes anything. New JSON channels take the next low tag; new binary channels take the
next high one.

### `FrameHeader` byte layout — `FRAME_HEADER_LEN` = 51, little-endian

| Offset | Width | Field |
|--------|-------|-------|
| 0 | 1 | format version (`FRAME_FORMAT_VERSION` = 1) |
| 1 | 1 | kind (0 keyframe, 1 tile) |
| 2 | 1 | scale denominator (1 full, 2 half; never 0) |
| 3 | 8 | `tab_id` |
| 11 | 8 | `frame_seq` |
| 19 | 8 | `last_applied_input` |
| 27 | 4 | `tile_x` |
| 31 | 4 | `tile_y` |
| 35 | 4 | `tile_width` |
| 39 | 4 | `tile_height` |
| 43 | 4 | `frame_width` |
| 47 | 4 | `frame_height` |

`to_bytes` and `from_bytes` sit adjacent, so a field added to one is visibly missing from the other.
`FRAME_HEADER_LEN` is public so the shell's writer and the client's reader size their buffers from
one place rather than two literals.

### Sequence semantics, stated once

Both contracts are documented in this module and enforced elsewhere, so the two ends cannot disagree
about them: `InputMessage`'s `seq` is strictly increasing within one connection and a non-increasing
value is dropped by the server (05-06 enforces); `FrameHeader::frame_seq` is strictly increasing
within one connection and tab so a client can discard a frame it has already superseded.

### The structural/semantic split

Written into `wire.rs`'s module header rather than left as an omission. Structural refusal lives
here because it needs nothing but the bytes. Semantic refusal — a coordinate outside a *particular
tab's* viewport, a sequence that did not increase on *this* connection, a key name that maps to no
key, a tab that is not agent-owned — lives in the shell's `remote_input` module (05-06), because
every one needs state this crate does not have and must not acquire.

## Task Commits

1. **Task 1: The wire module — envelope, frame header, input and view control messages** —
   `2cc3d33` (feat)
2. **Task 2: Move the Unix-only helpers into a local module, and correct the two headers that
   misdescribe the crate** — `adb4155` (refactor)
3. **CLAUDE.md-mandated changelog entry** — `012c442` (docs)

## Files Created/Modified

- `crates/talaria-protocol/src/wire.rs` — **created.** The multiplexed view envelope: channel tags
  and `Channel::from_tag`, `TabList`, `FrameKind` + `FrameHeader` with its adjacent byte
  encoder/decoder, `InputMessage` with its adjacent `to_json`/`from_json` and `is_well_formed`,
  `ClientView` / `ServerView`, `PROTOCOL_VERSION`, and 21 tests.
- `crates/talaria-protocol/src/local.rs` — **created.** The five Unix-only items, moved verbatim,
  with a header saying what they are, what they are *not* (part of the wire), and the one security
  fact that keeps them together.
- `crates/talaria-protocol/src/lib.rs` — header rewritten to "shared vocabulary"; the two new module
  declarations; the moved items removed along with the now-unused `PathBuf` import.
- `crates/talaria-shell/src/control.rs` — header rewritten; import follows the move.
- `crates/talaria-mcp/src/socket.rs` — import follows the move.
- `CHANGELOG.md` — Added and Changed entries for this plan.

## Decisions Made

Recorded in full in the frontmatter's `key-decisions`. The three that a later reader most needs:

1. **The view channel carries no agent tool vocabulary, and that is a shape, not a rule.** D-05-02
   read literally against Success Criterion 1 says a viewer lists, watches and drives: it clicks,
   scrolls and types into a page, and it does not open tabs, navigate them, evaluate script in them,
   close them or download through them. Putting that vocabulary on the human input channel would
   create a second tool surface reachable by a party whose only qualification is holding a token, and
   `ChromeRect`'s own doc comment already made this argument once, for agents. `grep -c 'Command'`
   returning 0 is the assertion.

2. **`ServerView::Refused` has no field.** Two refusals that differ are two bits an attacker did not
   have. A refused attach must not distinguish "that tab belongs to the human" from "there is no such
   tab", or the channel becomes an enumeration oracle for the human's own browsing. Its whole
   serialised form is `{"view":"refused"}`, and a test asserts exactly that string.

3. **`TabList.tabs` has no `serde(default)`.** An empty tab list is a legitimate state; an absent
   field is a malformed message. A default would collapse the two into one answer, which is the
   precise failure mode this whole wire refuses.

## Deviations from Plan

### Auto-fixed Issues

**1. [CLAUDE.md directive] `CHANGELOG.md` entry added, which the plan's `files_modified` did not list**

- **Found during:** plan close-out
- **Issue:** the global CLAUDE.md directive "Log all changes in a project CHANGELOG.md file" is a
  hard constraint and takes precedence over the plan's file list. `05-01` set the precedent within
  this same phase by recording its dependency landing (`44397c8`).
- **Fix:** an `### Added` entry for the wire module and a `### Changed` entry for the crate-root
  correction, in the existing Keep-a-Changelog Unreleased section and in the file's established
  prose-with-reasoning style.
- **Files modified:** `CHANGELOG.md`
- **Verification:** `git diff --stat` limited to `CHANGELOG.md`; no source or manifest touched
- **Committed in:** `012c442` (separate from both task commits, so neither task's diff carries it)
- **Note:** this does **not** encroach on `05-11`, which owns the phase's user-facing closeout entry
  plus the `.claude/CLAUDE.md` and `PROJECT.md` claim corrections. The validation map's
  `grep -c 'DIST-0[12]' CHANGELOG.md` assertion remains 05-11's to satisfy.

**2. [Rule 1 - Bug] A broken rustdoc intra-doc link introduced by the module declaration's doc comment**

- **Found during:** Task 2
- **Issue:** `local.rs`'s `//!` header links `[`current_uid`]`, an item defined in that same file.
  With a `///` doc comment also attached to `pub mod local;` in `lib.rs`, rustdoc merged the two and
  resolved the merged block's links in **`lib.rs`'s** scope, where `current_uid` no longer exists —
  `warning: unresolved link to 'current_uid'`. `cargo clippy` does not run rustdoc lints, so this
  would have shipped silently.
- **Fix:** demoted the declaration-site `///` to a plain `//` comment. The prose already lived, in
  fuller form, in `local.rs`'s own header, where its links resolve.
- **Files modified:** `crates/talaria-protocol/src/lib.rs`
- **Verification:** `cargo doc --no-deps -p talaria-protocol --locked` now finishes with no warnings
- **Committed in:** `adb4155` (part of the task commit)

### Not deviations, recorded for the reader

- **Zero fix attempts were spent on the wire module.** It compiled, passed clippy and passed all 21
  of its tests on the first run.
- **`grep -c 'cfg(unix)' lib.rs` is exactly 1.** The plan asked for "at least 1". The second
  `#[cfg(unix)]` that used to sit inside `ensure_socket_dir` travelled with the function into
  `local.rs`, where it is now technically redundant under the module's own gate — it was left in
  place because the plan says verbatim, and a diff that changed it would be a different plan.

## Verification Evidence

| Check | Result |
|-------|--------|
| `cargo test -p talaria-protocol --locked` | 24 passed, 0 failed — 21 new, 3 pre-existing unmodified |
| `cargo test --locked` | **317 passed**, 0 failed (2 + 24 + 291), 9 ignored — baseline was 296 |
| `cargo clippy --all-targets --locked -- -D warnings` | exit 0 |
| `cargo build --release --locked` | exit 0 |
| `cargo doc --no-deps -p talaria-protocol --locked` | exit 0, no warnings |
| `python3 tests/e2e/run_all.py` (`TALARIA_E2E_DISPLAY=:95`) | **22/22 PASS, `failed: none`**, exit 0 |
| `grep -c 'Command' crates/talaria-protocol/src/wire.rs` | 0 |
| `grep -c 'winit' crates/talaria-protocol/src/wire.rs` | 0 |
| `grep -c 'wire::' crates/talaria-mcp/src/tools.rs` | 0 |
| `grep -c 'MouseMove\|MouseButton\|WheelEvent\|KeyboardEvent' crates/talaria-protocol/src/lib.rs` | 0 |
| `grep -c '#[test]' crates/talaria-protocol/src/wire.rs` | 21 (plan asked ≥ 14) |
| `grep -c 'FrameHeader' .../wire.rs` / `last_applied_input` / `D-05-02` | 23 / 6 / 4 |
| `grep -c 'pub fn socket_path'` local.rs / lib.rs | 1 / 0 |
| `grep -c 'pub use local' lib.rs` | 0 |
| `grep -c 'local::'` socket.rs / control.rs | 1 / 1 |
| stale claims (`distributed-mode client later`, `precursor to the distributed`) | both gone |
| `grep -ci 'shared vocabulary' lib.rs` / `grep -ci 'peer' control.rs` | 1 / 16 |
| `git diff --stat Cargo.toml Cargo.lock crates/talaria-protocol/Cargo.toml` | empty |
| `grep -A1 'name = "primeorder"' Cargo.lock \| grep -c '0.14.0-rc.14'` | 1 |

The e2e suite was run on display `:95`, not `:98` — `:98` belongs to this machine's self-hosted CI
runner and `harness.start_xvfb()` `pkill`s stale Xvfb on its display without a lock, so two
concurrent runs SIGKILL each other. Recorded in the phase's `deferred-items.md` by 05-02.

## Threat Mitigations Landed

| Threat ID | Severity | How it was mitigated here |
|-----------|----------|---------------------------|
| T-05-04 | high | The agent tool vocabulary is unreachable from the human input channel because no type connects them. `grep` returns 0 in both directions |
| T-05-05 | high | `ServerView::Refused` is a unit variant with no field to be informative with |
| T-05-11 | medium | Ten structural refusals with ten tests; every decode returns an `Option` and every JSON field is required |
| T-05-15 | low | The strictly-increasing-per-connection sequence contract is documented here in one place; 05-06 enforces it, TLS (05-04) covers the cross-connection case |
| T-05-10 | medium | Accepted as planned. The header names a tab id, a geometry and two sequence numbers — no URL, no title, no owner |
| T-05-SC | n/a | Accepted. No dependency change; the lockfile is byte-unchanged |

## Threat Flags

None. This plan adds no network endpoint, no auth path, no file access and no schema at a trust
boundary — the wire types have no listener yet. The `/view` route that gives them one is 05-05's.

## Known Stubs

None. Every type this plan defines is complete, tested and consumed by name in a later plan; nothing
returns a hardcoded empty value or a placeholder.

## What the Next Plans Inherit

- **05-05** (the `/view` route) dispatches on `Channel::from_tag`, and an unknown byte is a refusal
  rather than a fallthrough.
- **05-06** (`remote_input`) consumes `InputMessage::from_json` and owns every semantic check this
  module deliberately declined: the viewport bound, the per-connection sequence high-water mark, the
  named-key table lookup, and the agent-owned-tab filter.
- **05-07** (the client) reads `FRAME_HEADER_LEN` and `PROTOCOL_VERSION` from here rather than from
  its own literals.
- **05-08** (the frame pump) fills `FrameHeader` and must carry `last_applied_input` forward from the
  input path, or 05-10's rate controller has no latency estimate.
- **05-10** (the degrade ladder) sets `scale_denominator`, which exists so a half-resolution frame is
  self-describing rather than inferred from a size ratio that would be wrong on an odd-sized
  viewport.
- **05-11** owns the matching claim in `.claude/CLAUDE.md` and `PROJECT.md` — this plan corrected the
  two Rust module headers only, so the documentation closeout has one owner.

## Self-Check: PASSED

- `crates/talaria-protocol/src/wire.rs` — FOUND
- `crates/talaria-protocol/src/local.rs` — FOUND
- `crates/talaria-protocol/src/lib.rs` — FOUND (modified)
- `crates/talaria-shell/src/control.rs` — FOUND (modified)
- `crates/talaria-mcp/src/socket.rs` — FOUND (modified)
- `CHANGELOG.md` — FOUND (modified)
- Commit `2cc3d33` — FOUND
- Commit `adb4155` — FOUND
- Commit `012c442` — FOUND

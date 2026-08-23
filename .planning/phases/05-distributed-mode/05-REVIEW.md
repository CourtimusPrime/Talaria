---
phase: 05-distributed-mode
reviewed: 2026-08-24T00:00:00Z
depth: deep
files_reviewed: 22
files_reviewed_list:
  - crates/talaria-client/Cargo.toml
  - crates/talaria-client/src/chrome.rs
  - crates/talaria-client/src/input.rs
  - crates/talaria-client/src/main.rs
  - crates/talaria-client/src/net.rs
  - crates/talaria-client/src/present.rs
  - crates/talaria-client/src/rate.rs
  - crates/talaria-protocol/src/lib.rs
  - crates/talaria-protocol/src/local.rs
  - crates/talaria-protocol/src/wire.rs
  - crates/talaria-shell/src/app.rs
  - crates/talaria-shell/src/control.rs
  - crates/talaria-shell/src/http.rs
  - crates/talaria-shell/src/keyutils.rs
  - crates/talaria-shell/src/oauth.rs
  - crates/talaria-shell/src/remote_input.rs
  - crates/talaria-shell/src/settings.rs
  - crates/talaria-shell/src/tabs.rs
  - crates/talaria-shell/src/view.rs
  - crates/talaria-mcp/src/socket.rs
  - scripts/tailscale-serve.sh
  - scripts/two-machine-check.sh
findings:
  critical: 4
  warning: 10
  info: 3
  total: 17
status: issues_found
---

# Phase 5: Code Review Report

**Reviewed:** 2026-08-24
**Depth:** deep
**Files Reviewed:** 22
**Status:** issues_found

## Summary

The authentication and containment work is genuinely strong and I could not break it. `/view`
verifies in its own right against the same `Arc<dyn AuthProvider>` (`http.rs:1570-1589`), the route
is merged **inside** both outer layers so it inherits the origin/host refusal
(`http.rs:1367-1398`), `WebSocketUpgrade` carries no `.protocols()` call so a subprotocol cannot
smuggle a credential, there is no query-parameter path, and every refusal is one byte-identical
expression (`http.rs:1756-1759`). The four claimed absences in `remote_input.rs` are true *at the
call level* (see WR-07 for the caveat), tab ids are never recycled, the agent-only lookup is a
lookup rather than a check, `Rect::contains` is half-open so a coordinate at `width` is refused,
`page_position` really is the algebraic inverse of `fit`, and the wire decoder substitutes no
defaults and uses `checked_add` in the one place header arithmetic can wrap. TLS has no escape
hatch in any spelling and `http://` is refused for anything but a loopback literal. No token,
credential or `Authorization` value reaches a log line in either crate.

What survives is a different class of problem. **The availability story does not hold.** The rung
ladder — the mechanism `ClientView::Cadence` was reshaped to create, explicitly to close T-05-12-D —
is bypassed by the input channel, which resets the pump deadline to *now* on every accepted message
(CR-01). The concurrent-attachment cap is per *connection* with no cap on connections, so the
number it bounds is not the number that costs anything (CR-02). The outbound frame queue is
unbounded with no drop policy, in the process that holds the vault (CR-03). Each of these is
reachable by a party whose only qualification is holding a token, and the first two end with a
winit loop the local human cannot get a click into — including the click that would revoke.

Separately, remote scrolling is quantitatively wrong by a factor of 76 and is not covered by any
test (CR-04), and three of the module doc comments this phase leans on make claims the code does
not keep (WR-04, WR-05, WR-07).

## Critical Issues

### CR-01: The input channel resets the frame-pump deadline, so the rung ladder bounds nothing

**File:** `crates/talaria-shell/src/view.rs:1098-1113`

**Issue:** `admit_input` ends with `attachment.due = attachment.due.min(now)`. `take_due`
(`view.rs:1400-1435`) sets `due = now + interval` after each tick, but any accepted input pulls it
straight back to the present. The rung's interval therefore bounds the pump only while the viewer
is *silent*; while it is sending, the pump ticks once per inbound message, at whatever rate the
socket delivers them.

That is exactly the property `ClientView::Cadence`'s own doc says was designed out
(`wire.rs:701-718`): *"an interval a client invents … is also a way for one viewer to pin the
engine's loop at whatever rate it liked (T-05-12-D). … The decision is removed rather than
bounded."* It was not removed; it moved to the input channel, where there is no bound at all.

**Attack path.** A party holding an accepted token (the only qualification — the register accepts
that party is bounded "by a revoke") opens `/view`, attaches to one agent tab, then writes
`{"kind":"mouse_move","tab":N,"seq":k,"x":1,"y":1}` in a tight loop with `k` increasing. Every
message passes the sequence rule, resolves through `agent_tab`, lands inside the viewport, and is
delivered — and every one of them makes the attachment due. `next_capture_deadline`
(`app.rs:781-788`) then returns an instant already past, `ControlFlow::WaitUntil` wakes
immediately, and `process_view_frames` (`app.rs:892-935`) performs a full `paint()` +
`read_to_image()` on the winit main thread per turn — 05-02 measured that pair at a fifth of a
30 ms tick on its own. The local human's window stops drawing at interactive rates. With one
attachment this is a heavy tax; at the cap (eight) it is a freeze, and the Access panel's Revoke
control is inside the loop that is frozen.

**Fix:** an input may move the deadline *forward in the schedule* but must never place two ticks
closer than the rung's own interval. Record when the attachment last ticked, and clamp:

```rust
struct Attachment {
    // …
    /// When this attachment last had a tick taken. `None` before the first.
    last_tick: Option<Instant>,
}

pub fn admit_input(&mut self, connection: u64, tab: u64, seq: u64) -> bool {
    let Some(session) = self.session_mut(connection) else { return false };
    if seq <= session.last_applied_input {
        return false;
    }
    session.last_applied_input = seq;
    let rung = session.rung;
    let Some(attachment) = session.attachment_mut(tab) else { return false };
    let now = Instant::now();
    attachment.last_input = Some(now);
    // The earliest a tick may be taken is one rung interval after the last one.
    // Without this floor the ladder bounds only an idle viewer, and a driving
    // one sets the loop's rate (T-05-12-D).
    let floor = attachment
        .last_tick
        .map_or(now, |last| (last + rung.interval()).max(now));
    attachment.due = attachment.due.min(floor);
    true
}
```

and set `attachment.last_tick = Some(now)` alongside `attachment.due = now + interval` in
`take_due`. Add a unit test that feeds a thousand inputs inside one interval and asserts
`take_due` yields at most one tick.

---

### CR-02: The attachment cap is per connection, and nothing caps connections

**File:** `crates/talaria-shell/src/view.rs:110-131`, `crates/talaria-shell/src/view.rs:982-999`,
`crates/talaria-shell/src/http.rs:684-693`

**Issue:** `DEFAULT_MAX_ATTACH` is documented as the answer to T-05-12 — *"an uncapped attachment
count is a way for one client — holding nothing but a token — to make the local browser unusable
for the human sitting at it."* But the cap is counted against a `ViewSession`, and
`ViewSessions::opened` pushes unconditionally; `ViewSockets::register` pushes unconditionally; no
code anywhere bounds how many `/view` sockets one `client_id` — or all clients together — may hold
open. The doc acknowledges this in one clause (*"the number of connections is bounded by the token
holder, and by a revoke"*) and that clause is the whole hole: the token holder **is** the attacker
in this threat, so the cap bounds a number the attacker does not have to respect.

**Attack path.** One token, K WebSocket upgrades (each independently verified, each accepted), 8
attachments apiece. `take_due` walks every session and every attachment, so the pump performs
`K × 8` paints and framebuffer readbacks per interval **on the winit main thread**, plus `K × 8`
full-surface buffers held on the encoder thread (one per live attachment, per `encode_frames`'
own accounting at `view.rs:583-586`). At K = 8 that is 64 readbacks per 30 ms. The browser stops
responding to the human, and — the part that makes it more than a nuisance — the consent/Access
panel that would revoke the token is drawn by the loop that is now saturated. Combine with CR-01
and the rate is not even bounded by the interval.

**Fix:** put a ceiling on live view connections, refuse the upgrade above it with the same
`view_refusal()` body, and make the resource cap total rather than per connection:

```rust
/// The most view sockets this process serves at once, whoever owns them.
const DEFAULT_MAX_VIEW_CONNECTIONS: usize = 4;

// in ViewSockets::register, answering whether the socket was accepted:
fn register(&self, connection: u64, client_id: &str, close: oneshot::Sender<()>) -> bool {
    let Ok(mut open) = self.open.lock() else { return false };
    if open.len() >= max_view_connections() {
        return false;
    }
    open.push(ViewSocketHandle { connection, client_id: client_id.to_owned(), close });
    true
}
```

with `ViewRoute::run` returning `view_refusal()`-equivalent closure behaviour when `register`
answers `false`, and `ViewSessions::attach` additionally refusing once the *total* attachment count
across all sessions reaches a global ceiling.

---

### CR-03: The outbound frame channel is unbounded, in the process that holds the vault

**File:** `crates/talaria-shell/src/view.rs:886-890`, `crates/talaria-shell/src/view.rs:940-946`,
`crates/talaria-shell/src/http.rs:1610-1620`

**Issue:** each view connection's `out` is a `tokio::sync::mpsc::UnboundedSender<Vec<u8>>`, written
by the encoder thread and drained by the socket task. `ViewSession::send` is
`let _ = self.out.send(frame);` with no depth check anywhere, and the pump's cadence is driven by
the server's own clock — `take_due` never asks whether the previous frame was actually written.
The doc justifies the unbounded channel (*"a bounded send would mean the event loop waiting on a
socket"*) but never supplies the other half: a policy for what happens when the reader falls
behind. There is none.

**Attack path.** A viewer completes the upgrade, attaches, and then simply stops reading from its
socket (a hostile client, or an ordinary one on a stalled relayed path — 05-RESEARCH measured
13 Mbit/s on DERP, where a single 522 KB keyframe takes ~300 ms to transmit). The peer's TCP
receive window closes, `socket.send(...)` in `ViewRoute::pump` blocks, and `out_rx` grows without
bound while the pump keeps producing. On a busy page at the fastest rung this is hundreds of
kilobytes every 30 ms per attachment; at the attachment cap, tens of megabytes per second. The
process that runs out of memory is the one holding the encrypted credential vault and the human's
logged-in sessions, and everything open in it is lost when it is OOM-killed.

The same absence covers the inbound direction: `axum`'s WebSocket default `max_message_size` is
64 MiB, every inbound frame becomes an owned `Vec<u8>` shipped to the main thread on the winit
event queue (`http.rs:1683-1690`), and nothing bounds how many are in flight.

**Fix:** bound the queue and make overflow a *drop*, not a wait — a superseded frame is exactly the
thing the ordering rule already lets a client discard. Either keep the unbounded channel and gate
production on depth:

```rust
/// The most frames that may be queued for one viewer before the pump stops
/// producing for it. A viewer that is not reading gets stale pixels, never a
/// browser that runs out of memory.
const MAX_QUEUED_FRAMES: usize = 4;

// in take_due, before pushing a DueTick:
if session.out.len() >= MAX_QUEUED_FRAMES {
    // Skip this tick entirely; the deadline still advances, so the pump does
    // not spin, and the next keyframe will resync whatever was missed.
    attachment.due = now + interval;
    attachment.keyframe = true;
    continue;
}
```

or switch `out` to a bounded `mpsc::channel(N)` and use `try_send`, treating `Err(Full)` the same
way. Also set an explicit `max_message_size` / `max_frame_size` on the upgrade
(`WebSocketUpgrade::max_message_size`) sized to the largest legitimate client message, which is a
few hundred bytes of JSON.

---

### CR-04: A remote viewer's scroll travels 1/76th of the local human's

**File:** `crates/talaria-shell/src/remote_input.rs:128-132`,
`crates/talaria-shell/src/remote_input.rs:236-247`, `crates/talaria-shell/src/app.rs:1586-1592`,
`crates/talaria-shell/src/app.rs:1721-1736`

**Issue:** the local path converts a winit `LineDelta(dx, dy)` by multiplying by
`WHEEL_LINE_PIXELS` (76.0) and *still* labelling the result `WheelMode::DeltaLine` — which is what
Servo's own embedding example does (`servo-0.4.0/examples/winit_minimal.rs:136-137`), so 76 units
per notch is the convention this engine is fed. The remote path takes the client's raw line count
— the client sends `MouseScrollDelta::LineDelta` through unchanged as `WheelMode::Line`
(`talaria-client/src/input.rs:280-284`) — and hands the engine `WheelDelta { y: 1.0, mode:
DeltaLine }`. One notch of a remote viewer's wheel is 1/76th of one notch of the local human's.

`WHEEL_LINE_PIXELS`' own doc comment asserts the opposite in bold: *"Named once and read by **both**
input paths — the local one converting winit's `LineDelta`, and [`crate::remote_input`] converting
a wire wheel in `WheelMode::Line` units — so a viewer's scroll covers the same distance the human's
does."* `grep -rn WHEEL_LINE_PIXELS crates/` returns three hits, all in `app.rs`; `remote_input.rs`
never names it. And `grep -n "wheel\|scroll" tests/e2e/remote_view_test.py` returns one comment and
no assertion — the wheel is the one input verb with no end-to-end coverage at all, which is why a
76× error shipped.

"Scroll" is named in the phase's own success criterion. On a page whose content is a screenful,
a remote viewer turning the wheel produces no visible movement.

**Fix:** convert in the same units the local path does, at the one place the wire mode is
translated, and make the constant's claim true:

```rust
// remote_input.rs — the wheel arm
InputMessage::Wheel { x, y, dx, dy, mode, .. } => {
    // Line deltas are scaled to the engine's own line size here, exactly as
    // the local path scales winit's `LineDelta`, so a viewer's notch and the
    // human's notch travel the same distance.
    let scale = match mode {
        WheelMode::Line => f64::from(app::WHEEL_LINE_PIXELS),
        WheelMode::Pixel => 1.0,
    };
    app::deliver_wheel(
        &webview,
        point(*x, *y),
        WheelDelta { x: dx * scale, y: dy * scale, z: 0.0, mode: wheel_mode(*mode) },
    )
},
```

and add a `tests/e2e/remote_view_test.py` case that scrolls through the client and asserts
`window.scrollY` moved by a comparable amount to the local path's, not merely that it is non-zero.

## Warnings

### WR-01: The capture drain asks `visibility_of` the wrong question

**File:** `crates/talaria-shell/src/app.rs:518-531`

**Issue:** the fix in `2dd72b5` correctly routes the re-hide decision through `visibility_of`, but
passes `Some(id) == tabs.active_id(tab.owner.view())` for the `displayed` argument. That is "active
in its own view", not "displayed" — `TabManager::displayed()` additionally requires the tab's view
to be the current `mode`. So a tab that is `active_me` while the human is in Agents mode is scored
`DisplayedAndFocused`, `hide_after` is `false`, and the webview is left shown when
`sync_visibility` would hide it. Reachable by an agent taking a routine screenshot of the human's
active Me tab while the human is looking at the Agents view. It self-heals on the next
`sync_visibility`, but until then two webviews are shown and painting, which is the exact
invariant the surrounding comment claims `visibility_of` is the single source of truth for.

**Fix:** ask the same question `sync_visibility` asks.

```rust
let displayed_id = tabs.displayed().map(|tab| tab.id);
tabs.find_by_webview(&capture.webview).and_then(|id| {
    tabs.get(id).map(|tab| crate::tabs::visibility_of(Some(id) == displayed_id, tab.held_for_view))
})
```

---

### WR-02: A dead encoder thread is undetectable, and every viewer silently gets nothing

**File:** `crates/talaria-shell/src/view.rs:556-570`, `crates/talaria-shell/src/view.rs:686-694`

**Issue:** `FrameEncoder::running()` reports only whether the *spawn* succeeded. If the thread ever
exits — a panic anywhere in `encode_frames`, `reduce`, `crop` or the png encoder — `jobs.send()`
starts returning `Err(SendError)` and the code discards it: `let _ = jobs.send(job);`. That is the
one signal available and it is thrown away. Consequences: `running()` keeps answering `true`, so
`attach` keeps accepting leases; `take_encoder_failure()` keeps answering `None`, so nothing is
reported; the pump keeps paying a full readback per tick per attachment and posting the buffers into
a channel with no receiver; and every viewer sits on a stream that will never produce another frame.
This is the same failure shape as the bug fixed in `2dd72b5` (frames appear healthy while the thing
they depend on is gone), one layer down.

**Fix:** make the send's failure the death signal.

```rust
struct FrameEncoder {
    jobs: Option<std::sync::mpsc::Sender<Job>>,
    failure: Option<String>,
}

fn send(&mut self, job: Job) {
    let Some(jobs) = self.jobs.as_ref() else { return };
    if jobs.send(job).is_err() {
        // The thread is gone. Record it once, so `running()` refuses the next
        // attach and the loop reports the reason exactly as a failed spawn is.
        log::error!("the frame encoder thread has ended; no viewer can be sent frames");
        self.failure = Some("the frame encoder thread ended".to_owned());
        self.jobs = None;
    }
}
```

(`ViewSessions::frame_captured` and `release_frames` then take `&mut self`, which they already do
and already have.) Consider also wrapping the thread body so an exit is logged with its cause.

---

### WR-03: A stalled viewer can survive listener shutdown, leaking its holds for the life of the process

**File:** `crates/talaria-shell/src/http.rs:1185-1220`, `crates/talaria-shell/src/http.rs:1600-1636`

**Issue:** the hold and the session are released only by `AppEvent::ViewClosed`, which `run` sends
in its tail (`http.rs:1626-1631`). On the ordinary paths — client close, revoke, `Handled::Close` —
the tail is reached. It is not reached on one path: `close_all()` fires each `close_tx`, but the
socket task only observes it inside the `select!`. A task currently awaiting
`socket.send(Message::Binary(..))` on a peer whose receive window is closed is not in the select,
does not see the close, and after `SHUTDOWN_GRACE` (3 s) the runtime is dropped and the task with
it — before `send_event(ViewClosed)` runs.

The main thread then keeps a `ViewSession` with live attachments forever: `Tab::held_for_view`
never decrements, so those tabs stay `show()`n for a viewer that is gone; `next_tick()` keeps
returning deadlines, so the loop keeps waking every 30 ms and paying a paint + readback for frames
posted into a channel nobody drains — permanently, and *after* the operator turned remote access
off. Turning it back on adds a second set. This is CR-03's leak with no way out short of restart.

**Fix:** make the closure of the socket table also tell the main thread, so the release does not
depend on the task getting to run:

```rust
// in close_matching, after taking the handles:
for handle in taken {
    let _ = handle.close.send(());
    // Also raised here, because a task that is about to be dropped by a runtime
    // shutdown never reaches its own tail. `ViewClosed` is idempotent: the main
    // thread's `closed` is a no-op for a connection it no longer holds.
    let _ = self.proxy.send_event(AppEvent::ViewClosed { connection: handle.connection });
}
```

(`ViewSessions::closed` already returns early for an unknown connection, so the double delivery is
free.) Alternatively, wrap the outbound write in `tokio::time::timeout` so a stalled peer cannot
hold the task outside the select indefinitely.

---

### WR-04: `WebView::blur()` is global, so the `Visibility` table cannot mean what it says

**File:** `crates/talaria-shell/src/tabs.rs:262-291`

**Issue:** `Visibility::HeldForViewing` is documented as "shown and deliberately **not** focused",
and `visibility_of`'s doc says "Displayed wins over held … a viewer attached to the tab the human
happens to be looking at must not downgrade it out of focus, which is the one way this table could
have let a remote party change something local." Neither is achievable with the call being made.
`servo::WebView::blur()` sends `EmbedderToConstellationMessage::BlurWebView`, which **carries no
webview id** (`servo-0.4.0/webview.rs:416-422`), and the servo façade handles the reply by clearing
focus on *every* webview it holds (`servo-0.4.0/servo.rs:786-791`). `sync_visibility` walks
`self.tabs` in insertion order and fires `focus()` for the displayed tab and `blur()` for every
held or hidden one — so whenever the displayed tab is not last in the vector, the focus it was just
given is immediately cleared for the whole instance.

The global blur predates this phase (the `Hidden` arm did it too, and local keyboard events are
delivered straight to the displayed webview via `notify_input_event`, which is why nobody noticed),
but Phase 5 makes it a *remote-triggered* effect — `hold_for_view` / `release_view_hold` both call
`sync_visibility` — and adds a third arm whose documented semantics are unachievable. Anything that
reads focus in the page (`document.hasFocus()`, `:focus-within`, `focus`/`blur` handlers, caret
rendering) sees the local human's tab lose focus, order-dependently, when a remote viewer attaches.

**Fix:** stop expressing "not focused" with a call that means "nothing is focused". Blur once, at
most, and only when nothing should be focused; focus the displayed tab last.

```rust
pub fn sync_visibility(&self) {
    let displayed_id = self.displayed().map(|tab| tab.id);
    for tab in &self.tabs {
        match visibility_of(Some(tab.id) == displayed_id, tab.held_for_view) {
            Visibility::DisplayedAndFocused | Visibility::HeldForViewing => tab.webview.show(),
            Visibility::Hidden => tab.webview.hide(),
        }
    }
    // `blur()` is a *global* BlurWebView with no webview id, so it may be sent
    // only when no tab should hold focus at all — and the focus must be sent
    // after it, never before.
    match displayed_id.and_then(|id| self.get(id)) {
        Some(tab) => tab.webview.focus(),
        None => {
            if let Some(tab) = self.tabs.first() {
                tab.webview.blur();
            }
        },
    }
}
```

and correct the two doc comments to say that "held" means shown and *not given* focus, which is a
different statement from "blurred".

---

### WR-05: `ClientView::Viewport`'s `width` and `height` are ignored, and no client sends it

**File:** `crates/talaria-shell/src/view.rs:1123-1131`, `crates/talaria-protocol/src/wire.rs:699-700`

**Issue:** the wire variant is documented as *"The viewer's window changed size; paint `tab` at this
size from now on."* The server matches `ClientView::Viewport { tab, .. }` and does nothing with the
size at all — it forces a keyframe and answers. That is the *correct* behaviour per
`present.rs`'s module header (*"nothing on the wire asks the server to make the page a different
size"* — reflowing an agent's page because a human started watching is the failure being avoided),
but the wire type still advertises the opposite, and `grep -rn 'ClientView::Viewport'
crates/talaria-client/` returns nothing: the shipped client never sends it. So this is a protocol
surface a client author will read, implement against, and find silently ignored — and the next
person to "finish" it will implement the reflow the design rejected.

**Fix:** either drop `width`/`height` from the variant (`Viewport { tab: u64 }`, keeping it as the
"my surface is gone, resync me" message it actually is) or rename it `ClientView::Resync { tab }`,
and rewrite the doc to say what the server does: force a keyframe, refuse when the connection holds
no lease on the tab, and never change the page's size.

---

### WR-06: At `scale_denominator > 1` the client believes the page is up to `denominator - 1` pixels wider than it is

**File:** `crates/talaria-client/src/present.rs:82-90`, `crates/talaria-shell/src/view.rs:280-300`,
`crates/talaria-shell/src/app.rs:1594-1602`

**Issue:** `reduce()` rounds the destination up (`width.div_ceil(divisor)`), so for a source width
`W` the header carries `frame_width = ceil(W / d)`. The client recovers the page size as
`page_width = frame_width * d`, which is `W` rounded *up* to the next multiple of `d` — up to
`d - 1` pixels larger than the real viewport. `page_position` accepts any `x < page_width`, so at
`d = 2` on an odd-width viewport the client will happily emit `x = W`, and the server's containment
test against `webview.size()` (`app.rs:1596-1602`, half-open) refuses it. Same in `y`.

Nothing errors on either side: the client sends, the server drops silently, and a click on the last
column or row of the page does nothing. It is one pixel today because the ladder only ever uses
`d ∈ {1, 2}`, but it is one pixel at the *edge of the page* — the review brief's own reason for
refusing rather than clamping — and it scales with any future denominator.

**Fix:** carry the page's true size rather than letting the client reconstruct it by multiplication.
Either round the reduction down for the purpose of the declared size (losing the edge strip, which
`reduce`'s comment argues against), or add the source dimensions to the frame header so
`SurfaceSize::page_width` is a read rather than a derivation. The cheapest correct change is the
latter, and the header already has the shape for it. Failing that, have `page_position` refuse
`x >= page_width - (denominator - 1)`, and say why in the comment.

---

### WR-07: The four "structural refusals" in `remote_input` are conventions, not structure

**File:** `crates/talaria-shell/src/remote_input.rs:11-18`, `crates/talaria-shell/src/app.rs:188`,
`crates/talaria-shell/src/app.rs:272`, `crates/talaria-shell/src/app.rs:291`

**Issue:** the module header claims the strongest available guarantee: *"None of those is refused by
a check here — there is simply no type in this module that connects to one, which is what makes the
guarantee survive a later reader (T-05-04)."* But `apply` takes `&Shared`, and on `Shared` the
fields `tabs`, `toolbar_height` and `webview_point` are all `pub` (`app.rs:188`, `:272`, `:291`).
`state.webview_point.set(..)`, `state.toolbar_height.get()` and `state.tabs.borrow().displayed()`
are each one expression away inside this module. The guarantee that is actually in force is "nobody
has written those three lines yet" — which is the same class of protection the file elsewhere
argues against (`agent_tab`'s "a `get` followed by an owner test would satisfy that today and stop
satisfying it the first time somebody adds a second call site").

`admit` already demonstrates the right shape by taking its two tables as narrow parameters.

**Fix:** give `apply` the same treatment, so the claim is true by construction:

```rust
/// Everything the remote input path may reach, and nothing else.
pub struct RemoteInputContext<'a> {
    pub views: &'a RefCell<ViewSessions>,
    pub tabs: &'a RefCell<TabManager>,
}

pub fn apply(context: RemoteInputContext<'_>, connection: u64, message: &InputMessage) -> bool
```

with `app::view_message` building the context at the one call site. The chrome, the toolbar height
and the local cursor cache are then genuinely unreachable from this module, which is what the
header says.

---

### WR-08: Nothing bounds the length of a wire key, so one message types an arbitrary string

**File:** `crates/talaria-protocol/src/wire.rs:626-634`,
`crates/talaria-shell/src/keyutils.rs:151-165`

**Issue:** `InputMessage::Key { key: Option<String>, .. }` is validated for "exactly one of `key`
and `named`" and for non-emptiness (`keyutils.rs:158`), and for nothing else.
`keyboard_event_from_wire` builds `Key::Character(text.to_owned())` from whatever arrived, so a
single input message can carry a megabyte-long "character" and inject it into a focused field in
one keystroke. With `axum`'s default 64 MiB WebSocket message limit, that is a 64 MiB allocation
per message on the main thread's path as well.

The channel exists so a human can type. A human types one grapheme cluster.

**Fix:** bound it in the wire's own well-formedness gate, where every other structural refusal
already lives, so both ends enforce one rule:

```rust
/// The most characters one key message may carry. A keystroke is one grapheme
/// cluster; a longer value is a bulk-text injection wearing a keystroke's shape.
pub const MAX_KEY_CHARS: usize = 8;

// in is_well_formed:
InputMessage::Key { key, named, .. } => {
    key.is_some() != named.is_some()
        && key.as_deref().is_none_or(|text| {
            !text.is_empty() && text.chars().count() <= MAX_KEY_CHARS
        })
},
```

and set `WebSocketUpgrade::max_message_size` on the `/view` route to something proportionate.

---

### WR-09: `tailscale-serve.sh down` removes a mapping it may not have created

**File:** `scripts/tailscale-serve.sh:230-245`

**Issue:** the teardown guard is `if [ -n "$existing" ] && [ "$existing" != "$TARGET" ]; then fail`.
`claimed_target` reads the `Web` section and prints only a handler's `Proxy` value; a handler with
no `Proxy` (a `tailscale serve --https=PORT /some/path` static mount, or any shape the parser does
not recognise) prints nothing, `existing` is empty, the guard is skipped, and
`tailscale serve --bg --https=$SERVE_PORT off` removes it. `port_is_claimed` returning true only
proves the `TCP` section has the port, which a static mount also satisfies. That contradicts the
script's own stated rule — *"this script does not remove what it did not create"* — on a host the
header says already runs several unrelated services behind Serve.

**Fix:** fail closed when the target cannot be established.

```bash
existing="$(claimed_target)"
if [ -z "$existing" ]; then
  fail "port ${SERVE_PORT} carries a mapping whose target could not be read — refusing to remove a mapping this script cannot confirm it created"
fi
if [ "$existing" != "$TARGET" ]; then
  fail "port ${SERVE_PORT} proxies to ${existing}, not ${TARGET} — that mapping is not this script's to remove"
fi
```

---

### WR-10: The frame header's latency echo counts inputs that were never applied

**File:** `crates/talaria-shell/src/view.rs:1098-1113`, `crates/talaria-shell/src/view.rs:918-931`

**Issue:** `admit_input` records `session.last_applied_input = seq` **before** it has established
that the connection holds a lease on the tab, and `remote_input::apply` can still refuse afterwards
on four further grounds (the agent-only lookup, a crashed tab, an unmapped key name, a coordinate
outside the viewport). All of those consume the number — which is correct and deliberate for replay
resistance (T-05-15). But the *same* field is what `FrameHeader::last_applied_input` echoes
(`view.rs:918-931`), and the client turns that echo into `reading.input_to_photon_ms`, which
`two-machine-check.sh` names as ">>> THE EVIDENCE for Success Criterion 2".

So the phase's headline measurement is computed from a counter that advances on refused inputs. A
viewer clicking in the letterboxed margin, or on a crashed tab, or on a tab it has not attached to,
produces a latency figure for a round trip that never included a hit test or a repaint — biased
*low*, in the direction that makes the number look better.

**Fix:** keep the two facts apart. `last_applied_input` (replay/ordering high-water mark) stays
where it is; add a separate `last_delivered_input`, written only by `remote_input::apply` on the
`true` return, and stamp *that* into the header.

```rust
/// The highest input sequence that actually reached a page on this connection.
/// Distinct from `last_applied_input`, which advances on refusals too so a
/// number cannot be reused (T-05-15) — echoing that one would report a latency
/// for a round trip that never included a hit test.
pub last_delivered_input: u64,
```

## Info

### IN-01: `Link::probing` executes `tailscale` resolved through `PATH`

**File:** `crates/talaria-client/src/rate.rs:198-214`

**Issue:** `std::process::Command::new("tailscale")` searches `PATH`. No shell is involved and no
attacker-controlled string reaches the argument list, so this is not injection — but the client
binary will execute whatever `tailscale` a user's `PATH` resolves to, purely to decorate one
status line.

**Fix:** either accept it explicitly in the doc comment, or resolve an absolute path
(`/usr/bin/tailscale`, falling back to `PATH`) so the ordinary case does not depend on the
environment.

### IN-02: The client's credential is not scrubbed after use

**File:** `crates/talaria-client/src/net.rs:497`

**Issue:** `drop(credential)` frees the `String` without overwriting it, and the same bytes are
already copied into the `HeaderValue` and (in the common path) into the process environment. The
`set_sensitive(true)` marking prevents printing, not residency. Low value given the token is also
in `TALARIA_CLIENT_TOKEN` for the process's life, but the `drop` reads as a scrub and is not one.

**Fix:** drop the misleading `drop(credential)` line, or zeroize before dropping and say which one
is intended.

### IN-03: `attach` starts the encoder thread before confirming the connection exists

**File:** `crates/talaria-shell/src/view.rs:1155-1163`

**Issue:** `self.encoder()` (which lazily spawns `talaria-frames`) is called before
`self.session_mut(connection)`, so an attach naming a connection the table has never heard of still
pays for the thread. Harmless in practice — the connection is always in the table by then — but it
undercuts the stated property that "a browser nobody has attached to spawns no thread".

**Fix:** move the `encoder().running()` check below the session lookup, or check
`self.encoder.is_some()` first and only construct on a path that has a real session.

---

_Reviewed: 2026-08-24_
_Reviewer: Claude (gsd-code-reviewer)_
_Depth: deep_

//! The remote view sessions: who is watching, what they are attached to, and
//! the snapshot of agent tabs they are given.
//!
//! **What this module reaches, and what it deliberately does not.** A view
//! session reaches webviews and nothing else. It never touches the chrome, it
//! never reads or writes the tab table's active-tab state, and it never
//! consults or changes the human's own view mode. Both directions of that
//! coupling are forbidden, and they are forbidden separately because they fail
//! differently:
//!
//! - **A viewer attaching does not change what the human is looking at.** A
//!   remote party that could move the local window is a remote party that can
//!   put a page in front of somebody sitting at the machine.
//! - **A human switching tabs does not redirect a viewer.** A viewer whose
//!   target followed [`crate::tabs::TabManager::displayed`] would be watching
//!   "whatever is on screen right now", which is a Me tab the moment the human
//!   flips the toggle — the exact leak `D-05-02` exists to prevent, arriving
//!   through the back door.
//!
//! So a viewer's target is always a tab id it named and this module resolved
//! through the agent-only lookup, and never a tab the local state happens to
//! be pointing at.
//!
//! The wire this speaks is [`talaria_protocol::wire`]; the transport is
//! [`crate::http`]'s `/view` route, which owns the credential and the socket.
//! Everything here runs on the winit main thread, because everything here
//! reads the tab table.

use std::time::{Duration, Instant};

use servo::{OffscreenRenderingContext, RenderingContext, WebView};
use tokio::sync::mpsc::UnboundedSender;

use talaria_protocol::wire::{Channel, ClientView, InputMessage, ServerView, TabList};
use talaria_protocol::TabInfo;

use crate::tabs::TabManager;

/// One frame of the view wire: a channel tag byte, then the channel's payload.
///
/// The envelope is composed here rather than in [`talaria_protocol::wire`]
/// because that crate owns what a *channel* means and this side owns what goes
/// on one. `Channel::tag` is still the only place a tag number is spelled.
pub fn encode(channel: Channel, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + 1);
    frame.push(channel.tag());
    frame.extend_from_slice(payload);
    frame
}

/// One control-channel message, framed. `None` if it could not be serialised.
///
/// Serialising a [`ServerView`] cannot actually fail — every field is a number
/// or a string — but this returns an `Option` rather than using `expect`,
/// because the caller is a socket task and this module's degrade direction is
/// "send nothing", never "abort the browser".
///
/// Not generic over the message type, deliberately: `talaria-shell` carries no
/// `serde` dependency of its own and names no derive, so the two shapes that
/// go on this wire each get their own builder rather than one function with a
/// `Serialize` bound this crate cannot spell.
pub fn control_frame(message: &ServerView) -> Option<Vec<u8>> {
    serde_json::to_vec(message).ok().map(|payload| encode(Channel::Control, &payload))
}

/// The server's first message on every connection: the wire version.
///
/// Built here so [`crate::http`] does not have to know the envelope's shape to
/// send it, and so there is one expression producing the hello rather than two
/// that could drift.
pub fn hello_frame() -> Option<Vec<u8>> {
    control_frame(&ServerView::Hello { protocol: talaria_protocol::wire::PROTOCOL_VERSION })
}

/// The refusal frame, produced by one expression for the reason
/// [`ServerView::Refused`] carries no field: two refusals that differ are two
/// bits a viewer did not have.
pub fn refused_frame() -> Option<Vec<u8>> {
    control_frame(&ServerView::Refused)
}

/// The tabs-channel payload for one snapshot.
pub fn tab_list_frame(tabs: Vec<TabInfo>) -> Option<Vec<u8>> {
    serde_json::to_vec(&TabList { tabs }).ok().map(|payload| encode(Channel::Tabs, &payload))
}

/// Split an inbound frame into its channel and its payload.
///
/// `None` for an empty frame or a tag byte that names no channel — a tag this
/// does not recognise is a refusal and never a fallthrough to a default
/// channel, which is [`Channel::from_tag`]'s rule and the reason it is used
/// here rather than a match of this module's own.
pub fn split_channel(frame: &[u8]) -> Option<(Channel, &[u8])> {
    let (tag, payload) = frame.split_first()?;
    Channel::from_tag(*tag).map(|channel| (channel, payload))
}

/// Decode a control-channel payload into the message a viewer sent.
///
/// Structural refusal only: a payload that is not this vocabulary yields
/// `None`, and deciding whether the *named tab* may be attached to is not a
/// question this function is allowed to have an opinion about.
pub fn decode_control(payload: &[u8]) -> Option<ClientView> {
    serde_json::from_slice(payload).ok()
}

/// The default ceiling on how many tabs one view connection may hold at once.
///
/// **A cap rather than a courtesy.** From the frame plan onward an attachment
/// holds a webview shown and a frame pump ticking, so an uncapped attachment
/// count is a way for one client — holding nothing but a token — to make the
/// local browser unusable for the human sitting at it (T-05-12). The cap lands
/// here, with the attachment bookkeeping, rather than beside the pump, because
/// this is the one place that can refuse before anything is leased.
///
/// Eight is a viewer watching several agents at once and nothing like a load
/// generator. It is a per-connection number, not a per-client one; the number
/// of connections is bounded by the token holder, and by a revoke.
const DEFAULT_MAX_ATTACH: usize = 8;

/// The cap's environment override, `TALARIA_VIEW_MAX_ATTACH`.
///
/// The same shape `TALARIA_COMMAND_TIMEOUT_SECS` uses: an unset variable, a
/// value that is not a number, and a value of zero all fall back to the
/// default. **Never to zero and never to unbounded** — a typo that disabled
/// the view channel entirely and a typo that removed the ceiling are both
/// worse answers than ignoring the typo.
pub fn max_attachments() -> usize {
    parse_max_attachments(std::env::var("TALARIA_VIEW_MAX_ATTACH").ok().as_deref())
}

/// The cap the override spells, or the default.
///
/// Split out from [`max_attachments`] with the raw value as a parameter for
/// the same reason every function in [`crate::agents`] takes its clock as one:
/// the interesting property is what a bad value does, and asserting that
/// through the process environment would make one test's setting another
/// test's answer.
fn parse_max_attachments(raw: Option<&str>) -> usize {
    raw.and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_ATTACH)
}

/// The interval between frames for an attachment whose viewer is **driving**
/// the tab, in whole milliseconds.
///
/// Serves DIST-02's ~30–60 ms takeover half, and it is the value the pump
/// requests **unconditionally, on every machine** — there is deliberately no
/// environment check here, no software-rendering special case and no knob that
/// lowers it. 05-02 measured this tree's real engine sustaining a 30 ms cadence
/// in three page shapes with one overrun per three hundred ticks, and that one
/// overrun was the first tick after a lease was taken rather than a steady
/// state. If a cadence ever cannot be sustained, the *requested* rate does not
/// move: 05-10's degrade ladder lowers the *delivered* one, and the ladder's
/// fastest rung is this number.
const DRIVEN_TICK_MS: u64 = 30;

/// The interval between frames for an attachment nobody is driving, in whole
/// milliseconds.
///
/// Serves DIST-02's ~200–500 ms passive half and sits inside that band rather
/// than at either end of it. A passive viewer is watching an agent work, so
/// the frames still have to be timely enough to follow what it is doing; they
/// do not have to be timely enough to aim a click with.
const PASSIVE_TICK_MS: u64 = 250;

/// How long after the last accepted input an attachment falls back from the
/// driven cadence to the passive one, in whole milliseconds.
///
/// One second, which is longer than the gap between two keystrokes and shorter
/// than a pause for thought — a viewer typing must not drop to a quarter-second
/// cadence between characters, and a viewer who has stopped must not hold the
/// fast rate indefinitely.
const DEFAULT_VIEW_IDLE_MS: u64 = 1000;

/// The idle threshold, overridable through `TALARIA_VIEW_IDLE_MS`.
///
/// The same shape [`max_attachments`] and `TALARIA_COMMAND_TIMEOUT_SECS` use:
/// an unset variable, a value that is not a number and a value of zero all
/// fall back to the default — **never to zero and never to unbounded**. Zero
/// would mean every attachment was permanently driven, which is the whole
/// browser paying takeover cost for a viewer nobody is touching.
pub fn view_idle() -> Duration {
    parse_view_idle(std::env::var("TALARIA_VIEW_IDLE_MS").ok().as_deref())
}

/// The threshold the override spells, or the default. Split out with the raw
/// value as a parameter so a bad value is assertable without one test's
/// environment becoming another's answer.
fn parse_view_idle(raw: Option<&str>) -> Duration {
    let millis = raw
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_VIEW_IDLE_MS);
    Duration::from_millis(millis)
}

/// The interval one attachment should tick at, given how long ago its viewer's
/// last input was accepted.
///
/// **The transition, stated once so nobody has to infer it from two clocks.**
/// An attachment enters the driven cadence on the first accepted input for
/// that tab on that connection, and returns to the passive cadence once the
/// elapsed time since the last accepted input *reaches* the threshold. An
/// input arriving exactly at the threshold re-enters the driven cadence,
/// because a new input always does — the comparison is on elapsed-since-last
/// rather than on a countdown, so there is one rule and no race between a
/// timer and a timestamp.
fn tick_interval(since_last_input: Option<Duration>, idle: Duration) -> Duration {
    match since_last_input {
        Some(elapsed) if elapsed < idle => Duration::from_millis(DRIVEN_TICK_MS),
        _ => Duration::from_millis(PASSIVE_TICK_MS),
    }
}

/// One tab's pixels, off the engine and safe to move to another thread.
///
/// A plain buffer rather than the engine's own image type, and that is the
/// point: it is what makes the tile comparison and the encode a pure function
/// of two byte slices, testable without a live engine and — because it is
/// `Send` — movable off the winit loop, which is the whole of the design 05-02
/// confirmed.
///
/// `pixels` is RGBA8, row-major, exactly `width * height * 4` bytes.
pub struct Surface {
    pub width: u32,
    pub height: u32,
    #[expect(dead_code, reason = "read by the tile comparison and the frame encoder")]
    pub pixels: Vec<u8>,
}

impl Surface {
    /// The engine's readback, taken apart into a plain buffer.
    fn from_image(image: servo::RgbaImage) -> Self {
        let (width, height) = (image.width(), image.height());
        Self { width, height, pixels: image.into_raw() }
    }
}

/// What one tick of the pump got from the engine.
///
/// Two answers rather than an `Option`, so a skipped tick cannot be read as an
/// empty frame at the call site: a failed framebuffer read leaves the lease
/// perfectly good and the next tick tries again. 05-02 saw this outcome zero
/// times in nine hundred sustained ticks, which is why it is worth a line in
/// the log and nothing more drastic.
pub enum CaptureOutcome {
    /// The pixels this tick painted.
    Painted(Surface),
    /// The framebuffer read did not answer.
    ReadFailed,
}

/// Paint a tab's webview and read its framebuffer back.
///
/// **Unconditionally, without waiting for a frame-ready notification, and
/// without the screenshot path's queue.** The engine's own documentation says
/// an embedder may paint and present without one and that Servo repaints even
/// when it is not first told to (`servo-0.4.0/webview.rs:77`), which is what
/// licenses this. Waiting would be wrong twice over: a static page never
/// produces such a notification at all, and the screenshot queue answers the
/// wait with a one-and-a-half-second deadline, which is a screenshot-shaped
/// answer to a frame-shaped question — at thirty-three ticks a second it would
/// be thirty-three dead deadlines and a static page delivering nothing until
/// each one expired. Painting unconditionally and letting the tile comparison
/// decide whether anything is worth sending costs one readback and one
/// comparison on a page nobody is touching, and sends nothing.
///
/// Takes the two handles rather than the tab table, so the caller can clone
/// them out under a scoped borrow and release it **before** the slowest step
/// in the tick. The engine's cells are borrowed from its own callbacks too,
/// and a borrow held across a readback is a borrow held across the slowest
/// thing in the loop.
pub fn capture(webview: &WebView, context: &OffscreenRenderingContext) -> CaptureOutcome {
    webview.paint();
    let size = context.size2d().to_i32();
    let rect = euclid::Box2D::from_origin_and_size(
        euclid::Point2D::origin(),
        euclid::Size2D::new(size.width, size.height),
    );
    match context.read_to_image(rect) {
        Some(image) => CaptureOutcome::Painted(Surface::from_image(image)),
        None => CaptureOutcome::ReadFailed,
    }
}

/// What the view sessions need from the tab table, and nothing more.
///
/// A trait for one reason: every `Tab` owns a live `WebView`, so a real tab
/// table cannot exist without an engine — and the properties worth asserting
/// here (the filter, the ordering, the cap, the identical refusals, the
/// attachment bookkeeping) are all decidable without one. The real
/// implementation below is four lines; a fake in the tests is what makes those
/// four lines' *consequences* testable.
///
/// Note what is **not** on this trait: there is no way to ask for a tab by id
/// without the agent-only filter, no way to ask which tab is displayed, and no
/// way to change anything. A viewer's whole vocabulary against the tab table
/// is these two questions.
pub trait ViewTabs {
    /// The agent tabs, in the tab table's own order.
    fn agent_snapshot(&self) -> Vec<TabInfo>;
    /// The viewport of an agent-owned tab in device pixels, or `None`.
    fn agent_viewport(&self, tab: u64) -> Option<(u32, u32)>;
    /// Hold an agent-owned tab shown for a viewer, answering whether the hold
    /// was taken. A tab the human owns takes none.
    fn hold_for_view(&mut self, tab: u64) -> bool;
    /// Release one hold. Releasing one that was never taken, or one on a tab
    /// that has since closed, is not an error.
    fn release_view_hold(&mut self, tab: u64);
}

impl ViewTabs for TabManager {
    /// **What a viewer sees: every agent's tabs, not only its own client's.**
    ///
    /// Decided rather than defaulted (T-05-10). The human is the trust root,
    /// and a remote human is the trust root at a distance; what they see
    /// should match what the local Agents view shows, which is not
    /// session-scoped either. A viewer scoped to one client would be a
    /// different product — a per-agent monitor — and would still not be a
    /// smaller grant, because the party holding the token is the same party.
    /// The acceptance is bounded by who holds a token, and by a revoke.
    ///
    /// **Nothing here sorts.** The tab table's insertion order *is* the order,
    /// which is what makes two tabs registered in the same millisecond keep a
    /// stable relative order across repeated reads — a sort on any field these
    /// two tabs share would be free to swap them between one read and the
    /// next. "We did not sort" is invisible in a diff, so it is written down.
    fn agent_snapshot(&self) -> Vec<TabInfo> {
        self.agent_tabs().map(|tab| crate::app::tab_info(self, tab)).collect()
    }

    /// Through [`TabManager::agent_tab`], never through `get`: a tab the human
    /// owns yields `None` here, so remoting one is unrepresentable rather than
    /// refused downstream (`D-05-02`).
    fn agent_viewport(&self, tab: u64) -> Option<(u32, u32)> {
        self.agent_tab(tab).map(|tab| {
            let size = tab.webview.size();
            (size.width as u32, size.height as u32)
        })
    }

    /// Straight through to the tab table's own hold, which is agent-only for
    /// the same reason [`TabManager::agent_tab`] is: the filter is a lookup
    /// rather than a check, so a tab the human owns cannot be held at all.
    fn hold_for_view(&mut self, tab: u64) -> bool {
        TabManager::hold_for_view(self, tab)
    }

    fn release_view_hold(&mut self, tab: u64) {
        TabManager::release_view_hold(self, tab)
    }
}

/// What one inbound frame asks of [`ViewSessions::message`]'s caller.
///
/// A three-way answer rather than a `bool`, because the input channel is the
/// one thing this module deliberately cannot finish: delivering a keystroke
/// needs a webview, and handing this module a webview would make it the second
/// path from the wire to the engine. It hands the message back instead, and
/// [`crate::remote_input`] stays the only one.
pub enum Handled {
    /// Answered here; keep the connection.
    Done,
    /// Nothing this module can answer at all: close the connection (T-05-11).
    Close,
    /// A structurally sound input message, **delivered nowhere yet**. The
    /// caller hands it to [`crate::remote_input::apply`].
    Input(InputMessage),
}

/// One tab a connection holds a view lease on, and everything the pump needs
/// to decide when to paint it next.
///
/// Per attachment and never per tab: two viewers on one tab have their own
/// sequence spaces, their own keyframes and their own cadences, so a record
/// shared between them would let either one's scroll reset the other's frame
/// numbering.
struct Attachment {
    /// The agent-owned tab this lease is on.
    tab: u64,
    /// The sequence the next frame for this attachment will carry. Starts at
    /// one and only ever increases; a tick whose readback failed still
    /// consumes its number, because the contract is *strictly increasing* and
    /// not *contiguous*, and a client that re-used a number could not tell a
    /// superseded frame from a fresh one.
    next_frame_seq: u64,
    /// When this attachment is next due to be painted.
    due: Instant,
    /// When this connection's last input aimed at this tab was accepted, if
    /// any. `None` is a viewer that has only ever watched — see
    /// [`tick_interval`] for the transition this drives.
    last_input: Option<Instant>,
    /// Whether the next frame must be a whole keyframe regardless of what
    /// changed: set on attach, on a re-attach that lost its acknowledgement,
    /// and on a viewport change.
    keyframe: bool,
}

impl Attachment {
    fn new(tab: u64, now: Instant) -> Self {
        Self {
            tab,
            next_frame_seq: 1,
            // Due immediately: the first frame is the one the viewer is
            // waiting on before it can draw anything at all.
            due: now,
            last_input: None,
            keyframe: true,
        }
    }

    /// The interval this attachment is currently ticking at.
    fn interval(&self, now: Instant, idle: Duration) -> Duration {
        tick_interval(self.last_input.map(|at| now.saturating_duration_since(at)), idle)
    }
}

/// One attachment's tick, taken out of the session table so the work can be
/// done with the borrow released.
pub struct DueTick {
    pub connection: u64,
    pub tab: u64,
    /// The sequence this frame will carry, already consumed.
    pub frame_seq: u64,
    /// The input sequence this connection had applied when the tick was taken
    /// — stamped into the header the encoder assembles, and the whole of the
    /// latency instrumentation.
    #[expect(dead_code, reason = "stamped into the frame header by the encoder")]
    pub last_applied_input: u64,
    /// Whether this frame must be a keyframe whatever the comparison says.
    pub keyframe: bool,
    /// This connection's outbound frames, cloned so the encoder thread can
    /// write onto it without reaching back into the main thread's tables.
    #[expect(dead_code, reason = "written by the encoder thread")]
    pub out: UnboundedSender<Vec<u8>>,
}

/// One remote viewer's connection, as the main thread sees it.
pub struct ViewSession {
    /// The connection's own id, minted by the listener thread. Never a tab id
    /// and never anything the viewer chose.
    pub connection: u64,
    /// The **verified** client the viewer's token names. Kept so a session can
    /// be attributed in a log line and so the table can be reasoned about
    /// alongside the socket registry a revoke closes; nothing here re-checks
    /// it, because a socket that reached this table was already authenticated
    /// by the route that accepted it.
    pub client_id: String,
    /// This connection's outbound frames. Unbounded because every producer is
    /// the main thread itself: a bounded send would mean the event loop
    /// waiting on a socket, which is the one thing it must never do.
    out: UnboundedSender<Vec<u8>>,
    /// The tabs this connection holds a view lease on, in the order it
    /// attached to them. Bounded by [`max_attachments`].
    attached: Vec<Attachment>,
    /// Whether this connection has been sent a snapshot yet. A connection that
    /// has not must get one even when the snapshot has not changed, or a
    /// viewer joining a quiet browser would wait for a tab to open before
    /// learning there are none.
    snapshotted: bool,
    /// The highest input sequence number this connection has had accepted.
    ///
    /// **One field, three jobs**, which is why it is one field and not three
    /// mechanisms: it is the replay resistance inside a connection (a number
    /// cannot be used twice), it is the ordering rule under coalescing (a late
    /// message that lost a race is dropped rather than applied out of order),
    /// and it is what [`talaria_protocol::wire::FrameHeader::last_applied_input`]
    /// echoes so a client can measure input-to-photon latency off its own
    /// clock alone, with no clock shared between the two machines.
    ///
    /// *Accepted* means the message passed the sequence rule and was handed on
    /// to [`crate::remote_input`]. A message refused further down — an
    /// unattached tab, a tab the human owns, a coordinate outside the target's
    /// viewport — still consumes its number, because a number that could be
    /// reused is a message that could be replayed later, once the state it was
    /// refused for has changed (T-05-15).
    ///
    /// The field is on the *session* rather than on the attachment because the
    /// sequence space is per connection: two viewers on one tab must not share
    /// one, or either could replay or reorder the other's input by choosing
    /// numbers.
    pub last_applied_input: u64,
}

impl ViewSession {
    /// Push one frame toward this viewer. A closed channel is not an error:
    /// the socket task has ended and the session is about to be removed.
    fn send(&self, frame: Option<Vec<u8>>) {
        if let Some(frame) = frame {
            let _ = self.out.send(frame);
        }
    }

    /// How many tabs this connection is attached to. Test-only: nothing on a
    /// shipped path needs the number.
    #[cfg(test)]
    fn attachment_count(&self) -> usize {
        self.attached.len()
    }

    /// Whether this connection holds a lease on `tab`.
    #[cfg(test)]
    fn holds(&self, tab: u64) -> bool {
        self.attached.iter().any(|attachment| attachment.tab == tab)
    }

    fn attachment_mut(&mut self, tab: u64) -> Option<&mut Attachment> {
        self.attached.iter_mut().find(|attachment| attachment.tab == tab)
    }
}

/// Every open view connection, on the main thread.
///
/// Insertion-ordered, and nothing here ever sorts it — see
/// [`ViewSessions::snapshot`] for why that matters on the tab side and why the
/// same reasoning is applied to this table for free.
#[derive(Default)]
pub struct ViewSessions {
    sessions: Vec<ViewSession>,
    /// The last snapshot published, encoded. Kept so an unchanged tab table
    /// costs nothing per turn; see [`ViewSessions::publish`].
    published: Option<Vec<u8>>,
}

impl ViewSessions {
    /// Record a newly accepted connection.
    pub fn opened(&mut self, connection: u64, client_id: String, out: UnboundedSender<Vec<u8>>) {
        self.sessions.push(ViewSession {
            connection,
            client_id,
            out,
            attached: Vec::new(),
            snapshotted: false,
            last_applied_input: 0,
        });
        if let Some(session) = self.sessions.last() {
            log::debug!(
                "view connection {} is client {}",
                session.connection,
                session.client_id
            );
        }
    }

    /// Whether any connection is open. The event loop asks before doing any
    /// snapshot work at all, so a browser with no viewer pays nothing.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Forget a connection that ended, releasing every attachment it held.
    ///
    /// Releasing is the point: an attachment is a lease on a tab, and a lease
    /// held by a connection that is gone is a webview this browser keeps shown
    /// and repaints for nobody.
    pub fn closed(&mut self, connection: u64, tabs: &mut dyn ViewTabs) {
        let Some(index) = self.sessions.iter().position(|s| s.connection == connection) else {
            return;
        };
        let session = self.sessions.remove(index);
        for attachment in &session.attached {
            tabs.release_view_hold(attachment.tab);
        }
    }

    fn session_mut(&mut self, connection: u64) -> Option<&mut ViewSession> {
        self.sessions.iter_mut().find(|session| session.connection == connection)
    }

    /// Hand one inbound frame to the connection that sent it, and answer.
    ///
    /// **Structural refusal has already happened** — [`split_channel`] and
    /// [`decode_control`] both yield nothing for anything malformed, and a
    /// frame that did not decode ends the connection rather than becoming a
    /// value with a default in it (T-05-11). What is left here is semantic,
    /// and every semantic refusal is [`ServerView::Refused`], which carries no
    /// field to differ in.
    ///
    /// Returns what the caller must do next — see [`Handled`].
    pub fn message(&mut self, connection: u64, frame: &[u8], tabs: &mut dyn ViewTabs) -> Handled {
        let Some((channel, payload)) = split_channel(frame) else {
            return Handled::Close;
        };
        match channel {
            Channel::Control => match decode_control(payload) {
                Some(message) => {
                    self.control(connection, message, tabs);
                    Handled::Done
                },
                None => Handled::Close,
            },
            // A viewer sends on the control and input channels and on no
            // other. The tabs, event and frame channels are the server's own
            // direction, and a client writing on one is a client this server
            // does not understand — closed rather than ignored, because
            // ignoring it would leave the two ends disagreeing about what the
            // connection is for.
            Channel::Input => self.input(payload),
            Channel::Tabs | Channel::Event | Channel::Frame => Handled::Close,
        }
    }

    /// Decode one input-channel message. **Structural decoding only, and
    /// nothing is delivered anywhere from here.**
    ///
    /// [`InputMessage::from_json`] refuses a missing field, an unknown kind, a
    /// coordinate that is not a finite number and a key naming both or neither
    /// — all decidable from the bytes — and a payload it refuses ends the
    /// connection rather than becoming a message with a substituted zero in it
    /// (T-05-11).
    ///
    /// Everything left is semantic and belongs to [`crate::remote_input`],
    /// which is why the message goes back to the caller rather than onward
    /// from here.
    fn input(&mut self, payload: &[u8]) -> Handled {
        match std::str::from_utf8(payload).ok().and_then(InputMessage::from_json) {
            Some(message) => Handled::Input(message),
            None => Handled::Close,
        }
    }

    /// Whether `connection` may deliver an input message naming `tab` with
    /// sequence `seq` — recording the sequence when it may.
    ///
    /// The three questions that need this table and no engine, answered in the
    /// order [`crate::remote_input::apply`] documents:
    ///
    /// 1. **The connection exists.** An input message on a connection this
    ///    table has never heard of answers nothing.
    /// 2. **The sequence strictly increased.** Equal or lower is dropped — the
    ///    wire's own word for it — and the accepted value is recorded on
    ///    *this* connection, never globally (T-05-15).
    /// 3. **The connection holds a lease on the tab.** An unattached tab is
    ///    refused even when it is agent-owned, because attachment is what the
    ///    concurrent-attachment cap is counted against; input that bypassed it
    ///    would bypass the cap.
    ///
    /// Ownership is *not* asked here. It is asked through the agent-only
    /// lookup, which is the tab table's question rather than this table's.
    pub fn admit_input(&mut self, connection: u64, tab: u64, seq: u64) -> bool {
        let Some(session) = self.session_mut(connection) else { return false };
        if seq <= session.last_applied_input {
            return false;
        }
        session.last_applied_input = seq;
        let Some(attachment) = session.attachment_mut(tab) else { return false };
        // The driven cadence starts here, and it starts on *this* tick rather
        // than at the end of the passive interval already in flight: the input
        // has just landed and the next photon is the one being measured. See
        // [`tick_interval`] for the rule that takes it back down again.
        let now = Instant::now();
        attachment.last_input = Some(now);
        attachment.due = attachment.due.min(now);
        true
    }

    /// Answer one control-channel message.
    fn control(&mut self, connection: u64, message: ClientView, tabs: &mut dyn ViewTabs) {
        match message {
            ClientView::Attach { tab } => self.attach(connection, tab, tabs),
            ClientView::Detach { tab } => self.detach(connection, tab, tabs),
            // A resize is a hint about how the viewer wants `tab` painted.
            // What it costs the server is one keyframe: the client is about to
            // reallocate its surface, so every tile it holds is about to be
            // meaningless and a delta composited onto a resized texture would
            // be a stripe of stale pixels. It is still answered on the one
            // question this module can decide first: whether this connection
            // holds a lease on the tab it named. A viewer that could resize a
            // tab it never attached to would be reaching a tab it was refused.
            ClientView::Viewport { tab, .. } => match self.require_keyframe(connection, tab) {
                true => {},
                false => self.refuse(connection),
            },
            // A request, not a promise, and deliberately unanswered: the
            // server delivers at whatever rate the link can carry, and a
            // reply here would be a commitment this side cannot keep.
            ClientView::Cadence { .. } => {},
        }
    }

    /// Begin a view lease on `tab` for `connection`.
    ///
    /// **Every refusal below is the same refusal**, with no field to differ in
    /// ([`ServerView::Refused`]). A tab the human owns, a tab that was never
    /// created, and a tab beyond this connection's cap all produce identical
    /// bytes — because a viewer that could tell them apart could enumerate the
    /// human's own browsing without ever reaching a page of it (T-05-05-A).
    fn attach(&mut self, connection: u64, tab: u64, tabs: &mut dyn ViewTabs) {
        // Through the agent-only lookup, so a human-owned tab yields nothing
        // rather than yielding a tab that is then rejected (`D-05-02`).
        let Some((width, height)) = tabs.agent_viewport(tab) else {
            self.refuse(connection);
            return;
        };
        let cap = max_attachments();
        let now = Instant::now();
        let mut take_hold = false;
        {
            let Some(session) = self.session_mut(connection) else { return };
            match session.attachment_mut(tab) {
                // A client that lost its own acknowledgement and asked again
                // is starting from nothing, so it gets a fresh keyframe as
                // well as the same answer — a delta against a surface it no
                // longer holds would composite onto an empty texture.
                Some(attachment) => {
                    attachment.keyframe = true;
                    attachment.due = attachment.due.min(now);
                },
                None => {
                    if session.attached.len() >= cap {
                        session.send(refused_frame());
                        return;
                    }
                    session.attached.push(Attachment::new(tab, now));
                    take_hold = true;
                },
            }
        }
        // **The hold, and the reason an attachment is worth anything at all.**
        // Servo answers a hit test only for a shown webview, so before this
        // line an attachment neither showed a webview nor ticked a pump and a
        // remote click on a tab the local human was not looking at reached
        // nothing. Taken once for the life of the lease rather than per frame:
        // a show-and-hide cycle thirty-three times a second is a different
        // cost profile from showing once, and it is one the local loop pays.
        //
        // It changes nothing on this window. The tab is shown and **not**
        // focused, nothing here touches which tab is displayed or which view
        // the human is in, and every tab renders into its own framebuffer — so
        // the tab the human is looking at keeps its own pixels.
        if take_hold && !tabs.hold_for_view(tab) {
            // Unreachable in one turn — the viewport lookup above just
            // resolved this tab through the same agent-only filter — but a
            // lease on a tab that took no hold would be a pump painting
            // something nobody is showing, so it is refused rather than
            // recorded.
            self.drop_attachment(connection, tab);
            self.refuse(connection);
            return;
        }
        // Acknowledged either way: a client that lost its own answer and asked
        // again gets the same one rather than a refusal it cannot act on.
        if let Some(session) = self.session_mut(connection) {
            session.send(control_frame(&ServerView::Attached { tab, width, height }));
        }
    }

    /// Forget one attachment without answering the viewer. The bookkeeping
    /// half of every release path, so the three of them cannot drift.
    fn drop_attachment(&mut self, connection: u64, tab: u64) -> bool {
        let Some(session) = self.session_mut(connection) else { return false };
        let before = session.attached.len();
        session.attached.retain(|attachment| attachment.tab != tab);
        before != session.attached.len()
    }

    /// Require the next frame for `connection`'s lease on `tab` to be a
    /// keyframe. Answers whether the lease exists, which is the same question
    /// the resize refusal asks.
    fn require_keyframe(&mut self, connection: u64, tab: u64) -> bool {
        let Some(session) = self.session_mut(connection) else { return false };
        let Some(attachment) = session.attachment_mut(tab) else { return false };
        attachment.keyframe = true;
        true
    }

    /// Release `connection`'s lease on `tab`.
    ///
    /// Releasing something that was never held is not an error, and the answer
    /// is the same either way: "you are not attached to this tab" is the state
    /// afterwards in both cases, and two different answers would tell a caller
    /// which tabs it had — cheap to keep uniform, so kept uniform.
    fn detach(&mut self, connection: u64, tab: u64, tabs: &mut dyn ViewTabs) {
        if self.drop_attachment(connection, tab) {
            // Only when a lease was actually held: a release for a hold that
            // was never taken would decrement a count two other viewers are
            // relying on.
            tabs.release_view_hold(tab);
        }
        if let Some(session) = self.session_mut(connection) {
            session.send(control_frame(&ServerView::Detached { tab }));
        }
    }

    /// Whether `connection` currently holds a lease on `tab`.
    #[cfg(test)]
    fn holds(&mut self, connection: u64, tab: u64) -> bool {
        self.session_mut(connection).is_some_and(|session| session.holds(tab))
    }

    /// When the pump is next due to paint anything, or `None` when no
    /// attachment exists at all.
    ///
    /// **Joined into the loop's existing wait computation and never installed
    /// as a second control-flow source** — two things deciding when the loop
    /// wakes is how a loop stops waking. `None` is the ordinary case and it
    /// costs the browser nothing: with no viewer attached there is no tick, no
    /// readback and no tab held shown.
    pub fn next_tick(&self) -> Option<Instant> {
        self.sessions
            .iter()
            .flat_map(|session| session.attached.iter().map(|attachment| attachment.due))
            .min()
    }

    /// Every attachment due to be painted at `now`, with its frame sequence
    /// consumed and its cadence advanced.
    ///
    /// Taken out of the table so the caller can drop the borrow **before** the
    /// paint and the readback, which is the deferred-drain idiom this file's
    /// neighbours use and which matters more here than anywhere else: the
    /// engine borrows these same cells from its own callbacks, and a borrow
    /// held across a readback is a borrow held across the slowest thing in the
    /// loop.
    pub fn take_due(&mut self, now: Instant, idle: Duration) -> Vec<DueTick> {
        let mut due = Vec::new();
        for session in &mut self.sessions {
            let last_applied_input = session.last_applied_input;
            for attachment in &mut session.attached {
                if attachment.due > now {
                    continue;
                }
                let interval = attachment.interval(now, idle);
                // From `now` rather than from the old deadline: a loop that
                // ran late must not then try to catch up by ticking twice in
                // a row, which is how a slow machine turns a missed frame
                // into a burst.
                attachment.due = now + interval;
                let frame_seq = attachment.next_frame_seq;
                attachment.next_frame_seq += 1;
                due.push(DueTick {
                    connection: session.connection,
                    tab: attachment.tab,
                    frame_seq,
                    last_applied_input,
                    keyframe: std::mem::take(&mut attachment.keyframe),
                    out: session.out.clone(),
                });
            }
        }
        due
    }

    /// Send every connection the current agent-tab snapshot, when it has
    /// changed or when a connection has not had one yet.
    ///
    /// Called from the event loop rather than from each place the tab table is
    /// mutated: a snapshot that had to be published by hand at every mutation
    /// site is a snapshot that goes stale the first time somebody adds a site
    /// and forgets. The comparison is on the encoded bytes, so an unchanged
    /// table sends nothing at all and this can safely run every turn.
    ///
    /// Zero agent tabs publishes an **empty list**, which is a legitimate
    /// state and not an error, not an absent field and not a closed
    /// connection — see [`TabList`], whose `tabs` field deliberately carries
    /// no serde default so the two cannot collapse into one.
    /// **A tab that closed underneath a viewer is detached here, and here
    /// only.** A tab can go away four ways — the page closed itself, its agent
    /// closed it, the human closed it, or it crashed and was cleared — and
    /// hooking each of those would be four places to forget. Comparing the
    /// leases against the snapshot is one place that cannot be forgotten,
    /// because the snapshot is the same value the viewer is about to be sent.
    /// The notice goes out *before* the new snapshot, so a viewer learns why a
    /// tab left rather than inferring it from an absence.
    pub fn publish(&mut self, tabs: &dyn ViewTabs) {
        if self.sessions.is_empty() {
            return;
        }
        let snapshot = tabs.agent_snapshot();
        let live: Vec<u64> = snapshot.iter().map(|tab| tab.tab_id).collect();
        let Some(frame) = tab_list_frame(snapshot) else { return };
        let changed = self.published.as_deref() != Some(frame.as_slice());
        for session in &mut self.sessions {
            let gone: Vec<u64> = session
                .attached
                .iter()
                .map(|attachment| attachment.tab)
                .filter(|tab| !live.contains(tab))
                .collect();
            for tab in gone {
                // Nothing is released back to the tab table here, and that is
                // not an omission: a tab absent from the snapshot has been
                // removed from the table entirely, so its hold count went with
                // it and there is no visibility left to restore.
                session.attached.retain(|attachment| attachment.tab != tab);
                session.send(control_frame(&ServerView::Detached { tab }));
            }
            if changed || !session.snapshotted {
                session.send(Some(frame.clone()));
                session.snapshotted = true;
            }
        }
        self.published = Some(frame);
    }

    /// One tick's pixels, off the loop.
    pub fn frame_captured(&mut self, tick: &DueTick, surface: Surface) {
        log::trace!(
            "view connection {} tab {} frame {} painted {}x{}",
            tick.connection,
            tick.tab,
            tick.frame_seq,
            surface.width,
            surface.height
        );
    }

    /// Put back a keyframe a tick consumed but could not deliver.
    ///
    /// A forced keyframe is the only part of a tick that must survive a failed
    /// readback: the sequence number is spent either way (strictly increasing,
    /// not contiguous), and a delta is correct against an unchanged previous
    /// frame — but a client that was promised a whole surface and got nothing
    /// would composite the next delta onto a texture it has not been given.
    pub fn require_keyframe_again(&mut self, tick: &DueTick) {
        self.require_keyframe(tick.connection, tick.tab);
    }

    /// Send this connection the one refusal.
    fn refuse(&mut self, connection: u64) {
        if let Some(session) = self.session_mut(connection) {
            session.send(refused_frame());
        }
    }
}

/// Test scaffolding, shared with [`crate::remote_input`]'s own suite.
///
/// It lives at module level rather than inside `mod tests` because
/// `remote_input` decides the *same* questions against the *same* two tables,
/// and a second fake tab table is a second thing to keep in agreement with the
/// real one. One fake, read by both suites, is the same argument
/// [`crate::keyutils`]'s shared key table makes.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    use tokio::sync::mpsc::UnboundedReceiver;

    /// A tab table with no engine behind it.
    ///
    /// The point of [`ViewTabs`]: every `Tab` owns a live `WebView`, so a real
    /// table cannot exist in a unit test — and every property this module owns
    /// is decidable without one. What the real implementation adds is two
    /// lines of Servo call, exercised end to end by
    /// `tests/e2e/remote_view_test.py`.
    ///
    /// It carries Me tabs as well as agent ones **on purpose**: a fake that
    /// only held agent tabs could not fail the filter test, and a test that
    /// cannot fail is not evidence.
    pub(crate) struct FakeTabs {
        /// `(tab_id, is_agent)`, in insertion order.
        pub(crate) tabs: Vec<(u64, bool)>,
        /// How many viewers hold each tab shown, keyed by tab id. The same
        /// count [`crate::tabs::Tab::held_for_view`] keeps, so the arithmetic
        /// the visibility synchronisation consults is assertable without an
        /// engine.
        pub(crate) holds: std::collections::BTreeMap<u64, usize>,
    }

    impl FakeTabs {
        pub(crate) fn with(tabs: &[(u64, bool)]) -> Self {
            Self { tabs: tabs.to_vec(), holds: std::collections::BTreeMap::new() }
        }

        pub(crate) fn none() -> Self {
            Self { tabs: Vec::new(), holds: std::collections::BTreeMap::new() }
        }

        /// How many viewers hold `tab` shown.
        pub(crate) fn held(&self, tab: u64) -> usize {
            self.holds.get(&tab).copied().unwrap_or(0)
        }
    }

    impl ViewTabs for FakeTabs {
        fn agent_snapshot(&self) -> Vec<TabInfo> {
            self.tabs
                .iter()
                .filter(|(_, agent)| *agent)
                .map(|(id, _)| TabInfo {
                    tab_id: *id,
                    url: format!("https://example.com/{id}"),
                    title: format!("tab {id}"),
                    owner: "client-a".into(),
                    focused: false,
                    crashed: false,
                    loading: false,
                })
                .collect()
        }

        fn agent_viewport(&self, tab: u64) -> Option<(u32, u32)> {
            self.tabs
                .iter()
                .find(|(id, agent)| *id == tab && *agent)
                .map(|_| (1280, 736))
        }

        /// Agent-only, exactly as the real table's is — a fake that held the
        /// human's tabs too could not fail the filter test.
        fn hold_for_view(&mut self, tab: u64) -> bool {
            if !self.tabs.iter().any(|(id, agent)| *id == tab && *agent) {
                return false;
            }
            *self.holds.entry(tab).or_insert(0) += 1;
            true
        }

        fn release_view_hold(&mut self, tab: u64) {
            if let Some(count) = self.holds.get_mut(&tab) {
                *count = count.saturating_sub(1);
            }
        }
    }

    /// One connected viewer, plus the read end of its outbound channel.
    pub(crate) struct Viewer {
        pub(crate) connection: u64,
        frames: UnboundedReceiver<Vec<u8>>,
    }

    impl Viewer {
        /// Every frame written to this viewer since the last drain.
        pub(crate) fn drain(&mut self) -> Vec<Vec<u8>> {
            let mut frames = Vec::new();
            while let Ok(frame) = self.frames.try_recv() {
                frames.push(frame);
            }
            frames
        }

        /// The control-channel messages among them.
        pub(crate) fn control(&mut self) -> Vec<ServerView> {
            self.drain()
                .iter()
                .filter_map(|frame| match split_channel(frame) {
                    Some((Channel::Control, payload)) => serde_json::from_slice(payload).ok(),
                    _ => None,
                })
                .collect()
        }

        /// The tab lists among them.
        pub(crate) fn snapshots(&mut self) -> Vec<Vec<u64>> {
            self.drain()
                .iter()
                .filter_map(|frame| match split_channel(frame) {
                    Some((Channel::Tabs, payload)) => {
                        serde_json::from_slice::<TabList>(payload).ok()
                    },
                    _ => None,
                })
                .map(|list| list.tabs.iter().map(|tab| tab.tab_id).collect())
                .collect()
        }
    }

    pub(crate) fn connect(
        sessions: &mut ViewSessions,
        connection: u64,
        client_id: &str,
    ) -> Viewer {
        let (out, frames) = tokio::sync::mpsc::unbounded_channel();
        sessions.opened(connection, client_id.to_owned(), out);
        Viewer { connection, frames }
    }

    pub(crate) fn attach(
        sessions: &mut ViewSessions,
        viewer: &Viewer,
        tab: u64,
        tabs: &mut dyn ViewTabs,
    ) {
        let frame = control_request(&ClientView::Attach { tab });
        assert!(
            matches!(sessions.message(viewer.connection, &frame, tabs), Handled::Done),
            "the connection was closed",
        );
    }

    pub(crate) fn detach(
        sessions: &mut ViewSessions,
        viewer: &Viewer,
        tab: u64,
        tabs: &mut dyn ViewTabs,
    ) {
        let frame = control_request(&ClientView::Detach { tab });
        assert!(
            matches!(sessions.message(viewer.connection, &frame, tabs), Handled::Done),
            "the connection was closed",
        );
    }

    /// A client-side control frame, composed the way a real viewer composes
    /// one.
    pub(crate) fn control_request(message: &ClientView) -> Vec<u8> {
        encode(Channel::Control, &serde_json::to_vec(message).expect("serializable"))
    }

}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    /// `D-05-02`: a tab the human owns never appears in a snapshot, in any
    /// state.
    #[test]
    fn a_snapshot_lists_agent_tabs_and_never_a_tab_the_human_owns() {
        let tabs = FakeTabs::with(&[(1, false), (2, true), (3, false), (4, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        sessions.publish(&tabs);
        assert_eq!(viewer.snapshots(), vec![vec![2, 4]]);
    }

    /// Zero agent tabs is a legitimate state: an empty list, not an error, not
    /// an absent field, and not a closed connection.
    #[test]
    fn no_agent_tabs_publishes_an_empty_list_rather_than_nothing() {
        let tabs = FakeTabs::none();
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        sessions.publish(&tabs);
        let frames = viewer.drain();
        assert_eq!(frames.len(), 1, "a viewer of a browser with no agent tabs was told nothing");
        let Some((Channel::Tabs, payload)) = split_channel(&frames[0]) else {
            panic!("the snapshot did not arrive on the tabs channel");
        };
        let list: TabList = serde_json::from_slice(payload).expect("a tab list");
        assert!(list.tabs.is_empty());
        // The field is present and empty, which is what keeps "no agent tabs"
        // distinguishable from "this message forgot to say".
        assert!(
            String::from_utf8_lossy(payload).contains("\"tabs\":[]"),
            "{}",
            String::from_utf8_lossy(payload)
        );
    }

    /// The snapshot preserves the tab table's own order and is never sorted,
    /// so two tabs registered in the same millisecond keep a stable relative
    /// order across repeated reads.
    #[test]
    fn a_snapshot_preserves_insertion_order_across_repeated_reads() {
        let tabs = FakeTabs::with(&[(9, true), (2, true), (7, true)]);
        let mut sessions = ViewSessions::default();
        let mut first = connect(&mut sessions, 1, "client-a");
        sessions.publish(&tabs);
        assert_eq!(first.snapshots(), vec![vec![9, 2, 7]]);
        // A second viewer reads the same table and gets the same order — a
        // sort on any field these tabs share would be free to disagree.
        let mut second = connect(&mut sessions, 2, "client-a");
        sessions.publish(&tabs);
        assert_eq!(second.snapshots(), vec![vec![9, 2, 7]]);
    }

    /// An attach to an agent tab is acknowledged with the tab's viewport, so a
    /// client can size its surface before the first frame rather than after.
    #[test]
    fn attaching_to_an_agent_tab_is_acknowledged_with_its_viewport() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        let _ = viewer.drain();
        attach(&mut sessions, &viewer, 1, &mut tabs);
        assert_eq!(
            viewer.control(),
            vec![ServerView::Attached { tab: 1, width: 1280, height: 736 }]
        );
    }

    /// T-05-05: a tab the human owns is refused, and it is refused by the
    /// lookup yielding nothing rather than by a check after the fact.
    #[test]
    fn attaching_to_a_tab_the_human_owns_is_refused() {
        let mut tabs = FakeTabs::with(&[(1, false), (2, true)]);
        assert!(tabs.agent_viewport(1).is_none(), "a human-owned tab resolved on the view path");
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        let _ = viewer.drain();
        attach(&mut sessions, &viewer, 1, &mut tabs);
        assert_eq!(viewer.control(), vec![ServerView::Refused]);
    }

    /// T-05-05-A: the two refusals are **byte-identical**, so a viewer cannot
    /// enumerate the human's tabs by watching which ids refuse differently.
    #[test]
    fn a_human_owned_tab_and_a_tab_that_does_not_exist_refuse_identically() {
        let mut tabs = FakeTabs::with(&[(1, false)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        let _ = viewer.drain();

        attach(&mut sessions, &viewer, 1, &mut tabs);
        let owned = viewer.drain();
        attach(&mut sessions, &viewer, 999, &mut tabs);
        let absent = viewer.drain();

        assert_eq!(owned.len(), 1, "the refusal was not one frame");
        assert_eq!(
            owned, absent,
            "a refused attach distinguishes a tab the human owns from a tab that does not \
             exist, which makes the channel an enumeration oracle for the human's browsing"
        );
    }

    /// Attaching twice from one connection is acknowledged and does not
    /// duplicate the attachment.
    #[test]
    fn attaching_twice_to_one_tab_does_not_duplicate_the_attachment() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        attach(&mut sessions, &viewer, 1, &mut tabs);
        assert_eq!(
            sessions.session_mut(1).expect("the session").attachment_count(),
            1,
            "one tab was leased twice by one connection"
        );
    }

    /// Two viewers may watch one tab. Neither displaces the other, and each
    /// keeps its own sequence space.
    #[test]
    fn two_connections_may_attach_to_the_same_tab() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut first = connect(&mut sessions, 1, "client-a");
        let mut second = connect(&mut sessions, 2, "client-b");
        let _ = first.drain();
        let _ = second.drain();

        attach(&mut sessions, &first, 1, &mut tabs);
        attach(&mut sessions, &second, 1, &mut tabs);
        let acknowledged = ServerView::Attached { tab: 1, width: 1280, height: 736 };
        assert_eq!(first.control(), vec![acknowledged.clone()]);
        assert_eq!(second.control(), vec![acknowledged]);
        assert_eq!(sessions.session_mut(1).expect("first").attachment_count(), 1);
        assert_eq!(sessions.session_mut(2).expect("second").attachment_count(), 1);
    }

    /// T-05-12: a client cannot pin an unbounded number of the engine's tabs.
    #[test]
    fn attaching_beyond_the_cap_is_refused_with_the_same_refusal() {
        // Every tab agent-owned, so the only thing that can refuse is the cap.
        let all: Vec<(u64, bool)> = (1..=(DEFAULT_MAX_ATTACH as u64 + 2))
            .map(|id| (id, true))
            .collect();
        let mut tabs = FakeTabs::with(&all);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        let _ = viewer.drain();

        for id in 1..=DEFAULT_MAX_ATTACH as u64 {
            attach(&mut sessions, &viewer, id, &mut tabs);
        }
        assert_eq!(
            viewer.control().len(),
            DEFAULT_MAX_ATTACH,
            "an attach inside the cap was refused"
        );
        attach(&mut sessions, &viewer, DEFAULT_MAX_ATTACH as u64 + 1, &mut tabs);
        assert_eq!(viewer.control(), vec![ServerView::Refused]);
        assert_eq!(
            sessions.session_mut(1).expect("the session").attachment_count(),
            DEFAULT_MAX_ATTACH,
            "a refused attach was recorded anyway"
        );
    }

    /// The cap's override lands on the default for anything it cannot read,
    /// and never on zero and never on unbounded.
    #[test]
    fn the_attachment_cap_falls_back_to_its_default_and_never_to_zero() {
        assert_eq!(parse_max_attachments(Some("3")), 3);
        assert_eq!(parse_max_attachments(Some("  12  ")), 12);
        for bad in [None, Some(""), Some("0"), Some("-1"), Some("lots"), Some("2.5")] {
            assert_eq!(
                parse_max_attachments(bad),
                DEFAULT_MAX_ATTACH,
                "{bad:?} did not fall back to the default"
            );
        }
    }

    /// Detaching releases the lease; detaching one that was never held is not
    /// an error and is answered the same way.
    #[test]
    fn detaching_releases_a_lease_and_detaching_nothing_is_not_an_error() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let _ = viewer.drain();

        let frame = control_request(&ClientView::Detach { tab: 1 });
        assert!(matches!(sessions.message(1, &frame, &mut tabs), Handled::Done));
        assert_eq!(viewer.control(), vec![ServerView::Detached { tab: 1 }]);
        assert_eq!(sessions.session_mut(1).expect("the session").attachment_count(), 0);

        // And again, holding nothing.
        assert!(
            matches!(sessions.message(1, &frame, &mut tabs), Handled::Done),
            "a redundant detach closed the connection",
        );
        assert_eq!(viewer.control(), vec![ServerView::Detached { tab: 1 }]);
    }

    /// A tab that closes underneath a viewer produces a detach notice and is
    /// absent from the next snapshot — for **every** viewer holding it.
    #[test]
    fn a_tab_closing_detaches_every_viewer_holding_it() {
        let mut open = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let mut first = connect(&mut sessions, 1, "client-a");
        let mut second = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &first, 1, &mut open);
        attach(&mut sessions, &second, 1, &mut open);
        sessions.publish(&open);
        let _ = first.drain();
        let _ = second.drain();

        let closed = FakeTabs::with(&[(2, true)]);
        sessions.publish(&closed);
        assert_eq!(first.control(), vec![ServerView::Detached { tab: 1 }]);
        assert_eq!(sessions.session_mut(1).expect("first").attachment_count(), 0);
        assert_eq!(second.control(), vec![ServerView::Detached { tab: 1 }]);
        assert_eq!(sessions.session_mut(2).expect("second").attachment_count(), 0);

        let mut third = connect(&mut sessions, 3, "client-c");
        sessions.publish(&closed);
        assert_eq!(third.snapshots(), vec![vec![2]], "the closed tab is still in the snapshot");
    }

    /// A connection ending releases every attachment it held.
    #[test]
    fn a_connection_ending_releases_every_attachment_it_held() {
        let mut tabs = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        let survivor = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        attach(&mut sessions, &viewer, 2, &mut tabs);
        attach(&mut sessions, &survivor, 1, &mut tabs);

        sessions.closed(1, &mut tabs);
        assert!(sessions.session_mut(1).is_none(), "the session outlived its connection");
        assert_eq!(
            sessions.session_mut(2).expect("the survivor").attachment_count(),
            1,
            "one connection ending took another's attachment"
        );
    }

    /// A frame this server does not understand ends the connection rather than
    /// being guessed at (T-05-11).
    #[test]
    fn a_frame_that_does_not_decode_ends_the_connection() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        // Empty, an unknown tag, a control payload that is not the vocabulary,
        // and a write on a channel that is the server's own direction.
        for frame in [
            Vec::new(),
            vec![0x7f, b'{', b'}'],
            encode(Channel::Control, br#"{"view":"launch_missiles"}"#),
            encode(Channel::Control, b"not json at all"),
            encode(Channel::Tabs, br#"{"tabs":[]}"#),
            encode(Channel::Frame, &[0u8; 8]),
            encode(Channel::Input, br#"{"kind":"mouse_move","tab":1,"seq":1,"x":null,"y":2}"#),
        ] {
            assert!(
                matches!(sessions.message(viewer.connection, &frame, &mut tabs), Handled::Close),
                "a frame this server cannot answer left the connection open: {frame:?}"
            );
        }
    }

    /// A well-formed input message is handed **back** to the caller rather
    /// than acted on here — this module owns no path to a webview, and that
    /// absence is the reason `remote_input` can be the only one.
    #[test]
    fn a_well_formed_input_message_is_handed_back_and_delivered_nowhere() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let _ = viewer.drain();

        let frame = encode(
            Channel::Input,
            InputMessage::MouseMove { tab: 1, seq: 7, x: 4.0, y: 5.0 }
                .to_json()
                .expect("well formed")
                .as_bytes(),
        );
        let handled = sessions.message(viewer.connection, &frame, &mut tabs);
        assert!(
            matches!(handled, Handled::Input(InputMessage::MouseMove { seq: 7, .. })),
            "a decoded input message was not handed back",
        );
        // Nothing was sent and nothing was recorded: admitting the message is
        // `remote_input`'s call, made through `admit_input`.
        assert!(viewer.drain().is_empty(), "decoding an input message answered the viewer");
        assert_eq!(sessions.session_mut(1).expect("the session").last_applied_input, 0);
    }

    /// The input channel's sequence high-water mark is per connection, and it
    /// only ever advances.
    #[test]
    fn the_input_sequence_mark_is_per_connection_and_only_advances() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let first = connect(&mut sessions, 1, "client-a");
        let second = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &first, 1, &mut tabs);
        attach(&mut sessions, &second, 1, &mut tabs);

        assert!(sessions.admit_input(first.connection, 1, 7));
        assert_eq!(sessions.session_mut(1).expect("first").last_applied_input, 7);
        // A replay does not move the mark backwards, and is dropped: the
        // wire's own word for it.
        assert!(!sessions.admit_input(first.connection, 1, 3));
        assert!(!sessions.admit_input(first.connection, 1, 7));
        assert_eq!(sessions.session_mut(1).expect("first").last_applied_input, 7);
        // And the other connection's space is its own: two viewers on one tab
        // neither share a mark nor starve each other.
        assert_eq!(sessions.session_mut(2).expect("second").last_applied_input, 0);
        assert!(sessions.admit_input(second.connection, 1, 1));
        assert_eq!(sessions.session_mut(2).expect("second").last_applied_input, 1);
    }

    /// A resize naming a tab this connection never attached to is refused, and
    /// with the same refusal as everything else.
    #[test]
    fn a_viewport_for_an_unattached_tab_is_refused() {
        let mut tabs = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let _ = viewer.drain();

        let held = control_request(&ClientView::Viewport { tab: 1, width: 800, height: 600 });
        assert!(matches!(sessions.message(1, &held, &mut tabs), Handled::Done));
        assert!(viewer.control().is_empty(), "a resize of an attached tab was answered");

        let other = control_request(&ClientView::Viewport { tab: 2, width: 800, height: 600 });
        assert!(matches!(sessions.message(1, &other, &mut tabs), Handled::Done));
        assert_eq!(viewer.control(), vec![ServerView::Refused]);
    }

    /// An unchanged tab table costs a connected viewer nothing, so this can
    /// safely run on every turn of the event loop.
    #[test]
    fn an_unchanged_tab_table_publishes_nothing_after_the_first_time() {
        let tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        sessions.publish(&tabs);
        assert_eq!(viewer.snapshots().len(), 1);
        sessions.publish(&tabs);
        sessions.publish(&tabs);
        assert!(viewer.drain().is_empty(), "an unchanged tab table was republished");
        // But a change is published.
        sessions.publish(&FakeTabs::with(&[(1, true), (2, true)]));
        assert_eq!(viewer.snapshots(), vec![vec![1, 2]]);
    }

    /// A browser with no viewer does no snapshot work at all.
    #[test]
    fn publishing_with_no_viewer_is_a_no_op() {
        let mut sessions = ViewSessions::default();
        assert!(sessions.is_empty());
        sessions.publish(&FakeTabs::with(&[(1, true)]));
        assert!(sessions.is_empty());
    }

    // ---- the visibility hold -------------------------------------------

    /// An attachment holds its tab shown. Without this a remote click reaches
    /// nothing, because Servo answers a hit test only for a shown webview.
    #[test]
    fn attaching_holds_the_tab_shown() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        assert_eq!(tabs.held(1), 0, "a tab was held before anyone attached");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        assert_eq!(tabs.held(1), 1);
    }

    /// The two-viewer arithmetic, which is the whole reason the marker is a
    /// count and not a flag: the first detach must not release the tab out
    /// from under the second viewer.
    #[test]
    fn two_viewers_hold_one_tab_and_the_first_detach_does_not_release_it() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let first = connect(&mut sessions, 1, "client-a");
        let second = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &first, 1, &mut tabs);
        attach(&mut sessions, &second, 1, &mut tabs);
        assert_eq!(tabs.held(1), 2);

        detach(&mut sessions, &first, 1, &mut tabs);
        assert_eq!(tabs.held(1), 1, "the first detach released a tab the second was watching");
        detach(&mut sessions, &second, 1, &mut tabs);
        assert_eq!(tabs.held(1), 0, "the last detach did not release the tab");
    }

    /// Attaching twice from **one** connection holds once. The count follows
    /// leases, not messages, or a client that resent an attach would pin a tab
    /// shown forever.
    #[test]
    fn attaching_twice_from_one_connection_holds_the_tab_once() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        attach(&mut sessions, &viewer, 1, &mut tabs);
        assert_eq!(tabs.held(1), 1);
        detach(&mut sessions, &viewer, 1, &mut tabs);
        assert_eq!(tabs.held(1), 0);
    }

    /// Detaching a tab that was never attached releases nothing — a release
    /// for a hold that was never taken would decrement a count another viewer
    /// is relying on.
    #[test]
    fn detaching_a_tab_that_was_never_held_releases_nothing() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let watcher = connect(&mut sessions, 1, "client-a");
        let meddler = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &watcher, 1, &mut tabs);
        assert_eq!(tabs.held(1), 1);

        detach(&mut sessions, &meddler, 1, &mut tabs);
        detach(&mut sessions, &meddler, 1, &mut tabs);
        assert_eq!(tabs.held(1), 1, "a viewer released a hold it never took");
    }

    /// `D-05-02`: a tab the human owns takes no hold, because the hold goes
    /// through the same agent-only lookup the viewport does.
    #[test]
    fn a_tab_the_human_owns_is_never_held() {
        let mut tabs = FakeTabs::with(&[(1, false)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        assert_eq!(tabs.held(1), 0);
        assert!(!sessions.holds(1, 1), "a refused attach recorded a lease anyway");
    }

    /// A connection ending releases every hold it had, and only its own.
    #[test]
    fn a_connection_ending_releases_every_hold_it_had() {
        let mut tabs = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let leaving = connect(&mut sessions, 1, "client-a");
        let staying = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &leaving, 1, &mut tabs);
        attach(&mut sessions, &leaving, 2, &mut tabs);
        attach(&mut sessions, &staying, 1, &mut tabs);
        assert_eq!((tabs.held(1), tabs.held(2)), (2, 1));

        sessions.closed(1, &mut tabs);
        assert_eq!(
            (tabs.held(1), tabs.held(2)),
            (1, 0),
            "a connection ending released the wrong holds"
        );
        assert!(sessions.holds(2, 1), "one connection ending took another's lease");
    }

    /// A tab closing removes the attachment. Nothing is released back to the
    /// table, because a tab absent from the snapshot has been removed from it
    /// entirely and took its count with it.
    #[test]
    fn a_tab_closing_removes_the_attachment_that_held_it() {
        let mut open = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut open);
        attach(&mut sessions, &viewer, 2, &mut open);
        sessions.publish(&open);

        sessions.publish(&FakeTabs::with(&[(2, true)]));
        assert!(!sessions.holds(1, 1), "the lease on a closed tab survived it");
        assert!(sessions.holds(1, 2), "closing one tab dropped the lease on another");
        assert!(sessions.next_tick().is_some(), "the surviving lease stopped ticking");
    }

    // ---- the tick ------------------------------------------------------

    /// A browser with no viewer has no tick at all, so it never wakes the loop
    /// and never reads a framebuffer back.
    #[test]
    fn with_no_attachment_there_is_no_tick_and_nothing_is_due() {
        let mut sessions = ViewSessions::default();
        assert_eq!(sessions.next_tick(), None);
        assert!(sessions.take_due(Instant::now(), view_idle()).is_empty());

        // And a connection with no attachment is still no tick: it is the
        // lease that costs something, not the socket.
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let _viewer = connect(&mut sessions, 1, "client-a");
        assert_eq!(sessions.next_tick(), None);
        assert!(sessions.take_due(Instant::now(), view_idle()).is_empty());
        let _ = &mut tabs;
    }

    /// An attachment is due immediately — the first frame is the one the
    /// viewer cannot draw anything without — and not due again until its
    /// interval has passed.
    #[test]
    fn an_attachment_is_due_at_once_and_then_not_until_its_interval_passes() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);

        let now = Instant::now();
        assert!(sessions.next_tick().is_some_and(|due| due <= now));
        assert_eq!(sessions.take_due(now, view_idle()).len(), 1);
        assert!(sessions.take_due(now, view_idle()).is_empty(), "one tick painted twice");

        // The passive interval, since nothing has been driven.
        let passive = Duration::from_millis(PASSIVE_TICK_MS);
        assert!(sessions.take_due(now + passive - Duration::from_millis(1), view_idle()).is_empty());
        assert_eq!(sessions.take_due(now + passive, view_idle()).len(), 1);
    }

    /// The first frame of an attachment is a keyframe and the next is not: a
    /// client holds nothing to composite a delta onto until it has one.
    #[test]
    fn the_first_frame_of_an_attachment_is_a_keyframe_and_the_second_is_not() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);

        let now = Instant::now();
        let first = sessions.take_due(now, view_idle());
        assert!(first[0].keyframe, "the first frame after an attach was not a keyframe");
        let later = now + Duration::from_millis(PASSIVE_TICK_MS);
        let second = sessions.take_due(later, view_idle());
        assert!(!second[0].keyframe, "every frame is a keyframe, so nothing is a delta");
    }

    /// A viewport change and a re-attach both require the next frame to be a
    /// whole keyframe — the client's surface is about to be, or has already
    /// been, thrown away.
    #[test]
    fn a_viewport_change_and_a_reattach_each_require_a_keyframe() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let now = Instant::now();
        assert!(sessions.take_due(now, view_idle())[0].keyframe);

        let resize = control_request(&ClientView::Viewport { tab: 1, width: 640, height: 480 });
        assert!(matches!(sessions.message(1, &resize, &mut tabs), Handled::Done));
        let after_resize = now + Duration::from_millis(PASSIVE_TICK_MS);
        assert!(
            sessions.take_due(after_resize, view_idle())[0].keyframe,
            "a viewport change did not produce a keyframe"
        );

        attach(&mut sessions, &viewer, 1, &mut tabs);
        let after_reattach = after_resize + Duration::from_millis(PASSIVE_TICK_MS);
        assert!(
            sessions.take_due(after_reattach, view_idle())[0].keyframe,
            "a re-attach did not produce a keyframe"
        );
    }

    /// A tick whose readback failed puts its forced keyframe back. The
    /// sequence number is spent either way; the promise of a whole surface is
    /// not.
    #[test]
    fn a_keyframe_a_failed_tick_consumed_is_put_back() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);

        let now = Instant::now();
        let lost = sessions.take_due(now, view_idle()).remove(0);
        assert!(lost.keyframe);
        sessions.require_keyframe_again(&lost);
        let next = sessions.take_due(now + Duration::from_millis(PASSIVE_TICK_MS), view_idle());
        assert!(next[0].keyframe, "a keyframe lost to a failed readback was never re-sent");
        assert!(
            next[0].frame_seq > lost.frame_seq,
            "a failed tick's sequence number was re-used"
        );
    }

    /// Frame sequences are strictly increasing per attachment, and the two
    /// attachments on one tab have their own spaces.
    #[test]
    fn frame_sequences_increase_per_attachment_and_never_across_them() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let first = connect(&mut sessions, 1, "client-a");
        let second = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &first, 1, &mut tabs);
        attach(&mut sessions, &second, 1, &mut tabs);

        let mut now = Instant::now();
        let mut seen: Vec<(u64, u64)> = Vec::new();
        for _ in 0..3 {
            for tick in sessions.take_due(now, view_idle()) {
                seen.push((tick.connection, tick.frame_seq));
            }
            now += Duration::from_millis(PASSIVE_TICK_MS);
        }
        let sequences = |connection: u64| -> Vec<u64> {
            seen.iter().filter(|(c, _)| *c == connection).map(|(_, s)| *s).collect()
        };
        assert_eq!(sequences(1), vec![1, 2, 3]);
        assert_eq!(sequences(2), vec![1, 2, 3], "two viewers shared one sequence space");
    }

    /// A tick that ran late does not then tick twice to catch up: the next
    /// deadline is measured from when the frame was actually taken.
    #[test]
    fn a_late_tick_does_not_burst_to_catch_up() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);

        let very_late = Instant::now() + Duration::from_millis(PASSIVE_TICK_MS * 10);
        assert_eq!(sessions.take_due(very_late, view_idle()).len(), 1);
        assert!(
            sessions.take_due(very_late, view_idle()).is_empty(),
            "a loop that ran late produced a burst of frames instead of one"
        );
    }

    // ---- the cadence ---------------------------------------------------

    /// The transition, at the threshold and one millisecond either side of it.
    /// The boundary is the part somebody will test, so it is pinned here.
    #[test]
    fn the_cadence_falls_back_to_passive_exactly_at_the_idle_threshold() {
        let idle = Duration::from_millis(1000);
        let driven = Duration::from_millis(DRIVEN_TICK_MS);
        let passive = Duration::from_millis(PASSIVE_TICK_MS);

        assert_eq!(tick_interval(None, idle), passive, "a viewer that never drove was driven");
        assert_eq!(tick_interval(Some(Duration::ZERO), idle), driven);
        assert_eq!(tick_interval(Some(idle - Duration::from_millis(1)), idle), driven);
        assert_eq!(
            tick_interval(Some(idle), idle),
            passive,
            "the threshold itself was still driven, so 'reaches' meant 'exceeds'"
        );
        assert_eq!(tick_interval(Some(idle + Duration::from_millis(1)), idle), passive);
    }

    /// The requested driven cadence is 30 ms and the passive one is inside the
    /// 200–500 ms band the requirement names — asserted rather than left to a
    /// comment, because these are the two numbers a later edit would move.
    #[test]
    fn the_two_cadences_are_the_measured_ones() {
        assert_eq!(DRIVEN_TICK_MS, 30);
        assert!(
            (200..=500).contains(&PASSIVE_TICK_MS),
            "the passive cadence left the band DIST-02 names: {PASSIVE_TICK_MS}"
        );
    }

    /// An accepted input puts the attachment on the driven cadence, and puts
    /// it there *now* rather than at the end of the passive interval already
    /// in flight.
    #[test]
    fn an_accepted_input_moves_the_attachment_to_the_driven_cadence() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let now = Instant::now();
        assert_eq!(sessions.take_due(now, view_idle()).len(), 1, "the first frame was not due");

        assert!(sessions.admit_input(viewer.connection, 1, 1));
        let after_input = Instant::now();
        assert!(
            sessions.next_tick().is_some_and(|due| due <= after_input),
            "an input did not bring the next frame forward"
        );
        let taken = sessions.take_due(after_input, view_idle());
        assert_eq!(taken.len(), 1);
        // And the interval that follows is the driven one, well inside the
        // passive interval that was in flight a moment ago.
        assert!(sessions
            .next_tick()
            .is_some_and(|due| due <= after_input + Duration::from_millis(DRIVEN_TICK_MS)));
    }

    /// The idle threshold's override lands on the default for anything it
    /// cannot read, and never on zero and never on unbounded.
    #[test]
    fn the_idle_threshold_falls_back_to_its_default_and_never_to_zero() {
        assert_eq!(parse_view_idle(Some("250")), Duration::from_millis(250));
        assert_eq!(parse_view_idle(Some("  4000 ")), Duration::from_millis(4000));
        let default = Duration::from_millis(DEFAULT_VIEW_IDLE_MS);
        for bad in [None, Some(""), Some("0"), Some("-1"), Some("ages"), Some("2.5")] {
            assert_eq!(parse_view_idle(bad), default, "{bad:?} did not fall back to the default");
        }
    }
}

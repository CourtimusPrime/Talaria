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
    attached: Vec<u64>,
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
    /// held by a connection that is gone is a tab this browser keeps working
    /// for nobody.
    pub fn closed(&mut self, connection: u64) {
        self.sessions.retain(|session| session.connection != connection);
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
    pub fn message(&mut self, connection: u64, frame: &[u8], tabs: &dyn ViewTabs) -> Handled {
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
        session.attached.contains(&tab)
    }

    /// Answer one control-channel message.
    fn control(&mut self, connection: u64, message: ClientView, tabs: &dyn ViewTabs) {
        match message {
            ClientView::Attach { tab } => self.attach(connection, tab, tabs),
            ClientView::Detach { tab } => self.detach(connection, tab),
            // A resize is a hint about how the viewer wants `tab` painted, and
            // painting is not this plan's. It is still answered on the one
            // question this module can decide: whether this connection holds a
            // lease on the tab it named. A viewer that could resize a tab it
            // never attached to would be reaching a tab it was refused.
            ClientView::Viewport { tab, .. } => match self.holds(connection, tab) {
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
    fn attach(&mut self, connection: u64, tab: u64, tabs: &dyn ViewTabs) {
        // Through the agent-only lookup, so a human-owned tab yields nothing
        // rather than yielding a tab that is then rejected (`D-05-02`).
        let Some((width, height)) = tabs.agent_viewport(tab) else {
            self.refuse(connection);
            return;
        };
        let cap = max_attachments();
        let Some(session) = self.session_mut(connection) else { return };
        let already = session.attached.contains(&tab);
        if !already {
            if session.attached.len() >= cap {
                session.send(refused_frame());
                return;
            }
            session.attached.push(tab);
        }
        // Acknowledged either way, and the second attach adds nothing: a
        // client that lost its own answer and asked again gets the same one
        // rather than a refusal it cannot act on.
        session.send(control_frame(&ServerView::Attached { tab, width, height }));
    }

    /// Release `connection`'s lease on `tab`.
    ///
    /// Releasing something that was never held is not an error, and the answer
    /// is the same either way: "you are not attached to this tab" is the state
    /// afterwards in both cases, and two different answers would tell a caller
    /// which tabs it had — cheap to keep uniform, so kept uniform.
    fn detach(&mut self, connection: u64, tab: u64) {
        let Some(session) = self.session_mut(connection) else { return };
        session.attached.retain(|held| *held != tab);
        session.send(control_frame(&ServerView::Detached { tab }));
    }

    /// Whether `connection` currently holds a lease on `tab`.
    fn holds(&mut self, connection: u64, tab: u64) -> bool {
        self.session_mut(connection).is_some_and(|session| session.attached.contains(&tab))
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
                .copied()
                .filter(|held| !live.contains(held))
                .collect();
            for tab in gone {
                session.attached.retain(|held| *held != tab);
                session.send(control_frame(&ServerView::Detached { tab }));
            }
            if changed || !session.snapshotted {
                session.send(Some(frame.clone()));
                session.snapshotted = true;
            }
        }
        self.published = Some(frame);
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
    }

    impl FakeTabs {
        pub(crate) fn with(tabs: &[(u64, bool)]) -> Self {
            Self { tabs: tabs.to_vec() }
        }

        pub(crate) fn none() -> Self {
            Self { tabs: Vec::new() }
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
        tabs: &dyn ViewTabs,
    ) {
        let frame = control_request(&ClientView::Attach { tab });
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
        let tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        let _ = viewer.drain();
        attach(&mut sessions, &viewer, 1, &tabs);
        assert_eq!(
            viewer.control(),
            vec![ServerView::Attached { tab: 1, width: 1280, height: 736 }]
        );
    }

    /// T-05-05: a tab the human owns is refused, and it is refused by the
    /// lookup yielding nothing rather than by a check after the fact.
    #[test]
    fn attaching_to_a_tab_the_human_owns_is_refused() {
        let tabs = FakeTabs::with(&[(1, false), (2, true)]);
        assert!(tabs.agent_viewport(1).is_none(), "a human-owned tab resolved on the view path");
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        let _ = viewer.drain();
        attach(&mut sessions, &viewer, 1, &tabs);
        assert_eq!(viewer.control(), vec![ServerView::Refused]);
    }

    /// T-05-05-A: the two refusals are **byte-identical**, so a viewer cannot
    /// enumerate the human's tabs by watching which ids refuse differently.
    #[test]
    fn a_human_owned_tab_and_a_tab_that_does_not_exist_refuse_identically() {
        let tabs = FakeTabs::with(&[(1, false)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        let _ = viewer.drain();

        attach(&mut sessions, &viewer, 1, &tabs);
        let owned = viewer.drain();
        attach(&mut sessions, &viewer, 999, &tabs);
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
        let tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &tabs);
        attach(&mut sessions, &viewer, 1, &tabs);
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
        let tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut first = connect(&mut sessions, 1, "client-a");
        let mut second = connect(&mut sessions, 2, "client-b");
        let _ = first.drain();
        let _ = second.drain();

        attach(&mut sessions, &first, 1, &tabs);
        attach(&mut sessions, &second, 1, &tabs);
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
        let tabs = FakeTabs::with(&all);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        let _ = viewer.drain();

        for id in 1..=DEFAULT_MAX_ATTACH as u64 {
            attach(&mut sessions, &viewer, id, &tabs);
        }
        assert_eq!(
            viewer.control().len(),
            DEFAULT_MAX_ATTACH,
            "an attach inside the cap was refused"
        );
        attach(&mut sessions, &viewer, DEFAULT_MAX_ATTACH as u64 + 1, &tabs);
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
        let tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &tabs);
        let _ = viewer.drain();

        let frame = control_request(&ClientView::Detach { tab: 1 });
        assert!(matches!(sessions.message(1, &frame, &tabs), Handled::Done));
        assert_eq!(viewer.control(), vec![ServerView::Detached { tab: 1 }]);
        assert_eq!(sessions.session_mut(1).expect("the session").attachment_count(), 0);

        // And again, holding nothing.
        assert!(
            matches!(sessions.message(1, &frame, &tabs), Handled::Done),
            "a redundant detach closed the connection",
        );
        assert_eq!(viewer.control(), vec![ServerView::Detached { tab: 1 }]);
    }

    /// A tab that closes underneath a viewer produces a detach notice and is
    /// absent from the next snapshot — for **every** viewer holding it.
    #[test]
    fn a_tab_closing_detaches_every_viewer_holding_it() {
        let open = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let mut first = connect(&mut sessions, 1, "client-a");
        let mut second = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &first, 1, &open);
        attach(&mut sessions, &second, 1, &open);
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
        let tabs = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        let survivor = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &viewer, 1, &tabs);
        attach(&mut sessions, &viewer, 2, &tabs);
        attach(&mut sessions, &survivor, 1, &tabs);

        sessions.closed(1);
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
        let tabs = FakeTabs::with(&[(1, true)]);
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
                matches!(sessions.message(viewer.connection, &frame, &tabs), Handled::Close),
                "a frame this server cannot answer left the connection open: {frame:?}"
            );
        }
    }

    /// A well-formed input message is handed **back** to the caller rather
    /// than acted on here — this module owns no path to a webview, and that
    /// absence is the reason `remote_input` can be the only one.
    #[test]
    fn a_well_formed_input_message_is_handed_back_and_delivered_nowhere() {
        let tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &tabs);
        let _ = viewer.drain();

        let frame = encode(
            Channel::Input,
            InputMessage::MouseMove { tab: 1, seq: 7, x: 4.0, y: 5.0 }
                .to_json()
                .expect("well formed")
                .as_bytes(),
        );
        let handled = sessions.message(viewer.connection, &frame, &tabs);
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
        let tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let first = connect(&mut sessions, 1, "client-a");
        let second = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &first, 1, &tabs);
        attach(&mut sessions, &second, 1, &tabs);

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
        let tabs = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &tabs);
        let _ = viewer.drain();

        let held = control_request(&ClientView::Viewport { tab: 1, width: 800, height: 600 });
        assert!(matches!(sessions.message(1, &held, &tabs), Handled::Done));
        assert!(viewer.control().is_empty(), "a resize of an attached tab was answered");

        let other = control_request(&ClientView::Viewport { tab: 2, width: 800, height: 600 });
        assert!(matches!(sessions.message(1, &other, &tabs), Handled::Done));
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
}

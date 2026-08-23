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

use talaria_protocol::wire::{Channel, ClientView, ServerView};

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
}

impl ViewSession {
    /// Push one frame toward this viewer. A closed channel is not an error:
    /// the socket task has ended and the session is about to be removed.
    fn send(&self, frame: Option<Vec<u8>>) {
        if let Some(frame) = frame {
            let _ = self.out.send(frame);
        }
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
}

impl ViewSessions {
    /// Record a newly accepted connection.
    pub fn opened(&mut self, connection: u64, client_id: String, out: UnboundedSender<Vec<u8>>) {
        self.sessions.push(ViewSession { connection, client_id, out });
        if let Some(session) = self.sessions.last() {
            log::debug!(
                "view connection {} is client {}",
                session.connection,
                session.client_id
            );
        }
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
    /// Returns `false` when the connection should be closed.
    pub fn message(&mut self, connection: u64, frame: &[u8]) -> bool {
        let Some((channel, payload)) = split_channel(frame) else {
            return false;
        };
        match channel {
            Channel::Control => match decode_control(payload) {
                Some(message) => {
                    self.control(connection, message);
                    true
                },
                None => false,
            },
            // A viewer sends on the control and input channels and on no
            // other. The tabs, event and frame channels are the server's own
            // direction, and a client writing on one is a client this server
            // does not understand — closed rather than ignored, because
            // ignoring it would leave the two ends disagreeing about what the
            // connection is for.
            Channel::Input => true,
            Channel::Tabs | Channel::Event | Channel::Frame => false,
        }
    }

    /// Answer one control-channel message.
    fn control(&mut self, connection: u64, message: ClientView) {
        match message {
            ClientView::Attach { .. }
            | ClientView::Detach { .. }
            | ClientView::Viewport { .. }
            | ClientView::Cadence { .. } => self.refuse(connection),
        }
    }

    /// Send this connection the one refusal.
    fn refuse(&mut self, connection: u64) {
        if let Some(session) = self.session_mut(connection) {
            session.send(refused_frame());
        }
    }
}

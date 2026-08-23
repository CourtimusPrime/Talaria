/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 *
 * Portions derived from Servo's servoshell and winit_minimal embedding
 * example (servo v0.4.0); this file keeps the MPL 2.0 header per its
 * file-level copyleft. Talaria's original code is MIT OR Apache-2.0. */

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use base64::Engine;
use servo::{
    InputEvent, MouseButton as ServoMouseButton, MouseButtonAction, MouseButtonEvent,
    MouseLeftViewportEvent, MouseMoveEvent, OffscreenRenderingContext, RenderingContext, Servo,
    ServoBuilder, WebView, WheelDelta, WheelEvent, WheelMode, WindowRenderingContext,
};
use url::Url;
use webrender_api::units::DevicePixel;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;

use talaria_protocol::{
    Command, Outcome, ResultPayload, TabInfo,
};

use crate::agents::Agents;
use crate::bookmarks::Bookmarks;
use crate::control::AgentRequest;
use crate::downloads::Downloads;
use crate::gui::{ChromePanel, Gui, UiAction};
use crate::history::History;
use crate::http::{RemoteAccess, ShutdownHandle, StreamRegistry};
use crate::keyutils;
use crate::oauth::SharedAgents;
use crate::settings::{SearchEngine, Settings};
use crate::tabs::{TabManager, TabOwner, ViewMode};
use crate::vault::Vault;

#[derive(Debug)]
pub enum AppEvent {
    Wake,
    SessionStarted {
        session_id: u64,
        client: String,
        events: tokio::sync::mpsc::UnboundedSender<talaria_protocol::ServerMessage>,
    },
    SessionEnded { session_id: u64 },
    Agent(AgentRequest),
    /// One download finished successfully, raised from the background thread
    /// `download()` runs on so the main loop can record it.
    ///
    /// `path` is the path that was **actually written** — `create_unique`'s
    /// return value, which carries a ` (1)` suffix whenever the requested
    /// name collided — and `filename` is the name that was requested. They
    /// are two different strings on purpose, and only the first may ever
    /// reach the Downloads panel's Open button.
    ///
    /// `requested_by_agent` is provenance, not a filter. Pitfall 5 requires
    /// the store to record *every* completed download regardless of who asked
    /// for it — the opposite of history's Me-only filter, because a download
    /// is about what landed on the human's filesystem, which matters whoever
    /// caused it. That still holds, and nothing here drops a row.
    ///
    /// What `03-04` got wrong was dropping the fact as well as the filter.
    /// The Downloads panel hangs a one-click handoff to the OS's default
    /// application off each row, and an agent chose the bytes *and* the
    /// extension that decides which application that is; the human pressing
    /// the button is choosing with that fact or without it. It is a bool
    /// rather than the requesting session's label because the label is a
    /// string the agent wrote — see [`crate::downloads::DownloadEntry`].
    DownloadCompleted {
        path: String,
        filename: String,
        url: String,
        bytes: u64,
        requested_by_agent: bool,
    },
    /// The remote MCP listener bound, raised from the `talaria-http` thread.
    ///
    /// `addr` is the address that was **actually bound**, read back off the
    /// socket — never the configured one. Everything the human is shown, and
    /// the origin [`parse_agent_url`] refuses, comes from this string, which
    /// is what stops the toolbar and the Access panel disagreeing.
    RemoteListenerBound { addr: String },
    /// The remote MCP listener could not bind, or stopped without being asked
    /// to. Remote access goes back to off: the chrome never claims a listener
    /// that is not there.
    RemoteListenerFailed { addr: String, error: String },
    /// An agent is asking to drive this browser, and a human has to answer.
    ///
    /// Raised from the `talaria-http` thread once `/authorize` has checked
    /// every parameter the caller supplied, which is why the panel this opens
    /// has no error state. The request carries the channel the decision goes
    /// back down and **nothing else** — no authorization code, no token and no
    /// PKCE verifier ever crosses into the chrome.
    ///
    /// This is the only [`AppEvent`] that opens a panel. It goes through
    /// [`crate::gui::Gui::raise_consent`] rather than through
    /// [`UiAction::SetPanel`], which refuses this panel outright — see
    /// `apply_ui_actions`.
    ConsentRequested(crate::oauth::ConsentRequest),
    /// A remote viewer's WebSocket was accepted, raised from the
    /// `talaria-http` thread once its bearer token verified.
    ///
    /// `client_id` is the **verified** client the token names — minted by this
    /// browser at registration and approved by a human — never a name the peer
    /// asserted, and it is what a revoke matches on. `out` is this
    /// connection's outbound frame channel: the main thread pushes onto it and
    /// the socket task writes it out, which is the whole of what the listener
    /// thread may hold, because `Shared` is `Rc`-based and cannot cross the
    /// boundary. It must **never** be used to reach any tab the human owns —
    /// what may travel down it is decided by [`crate::view`], not here.
    ViewOpened {
        connection: u64,
        client_id: String,
        out: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    },
    /// One frame arrived from a viewer, still exactly as it was on the wire.
    ///
    /// Undecoded on purpose: structural decoding is
    /// [`talaria_protocol::wire`]'s, and doing it on the listener thread would
    /// put a second decoder on a hostile peer's path. `connection` is the
    /// session's own id and is never a tab id — a viewer does not get to name
    /// which connection it is.
    ViewMessage { connection: u64, frame: Vec<u8> },
    /// A viewer's WebSocket ended, however it ended: the client closed it, a
    /// revoke closed it, or the listener shut down. Every attachment the
    /// connection held is released here and nowhere else.
    ViewClosed { connection: u64 },
    /// The frame encoder thread could not be started, so no viewer can be
    /// given frames on this run.
    ///
    /// Reported rather than fatal, which is the same posture the listener
    /// takes when it cannot bind: a browser whose frame encoder could not
    /// start is still a browser, and the local human loses nothing at all.
    /// The affected viewers already have their answer — an attach is refused
    /// with the one refusal rather than accepted into a stream that would
    /// never produce a frame — and this carries the *reason* to the one thread
    /// that can log it with the rest of the browser's lifecycle.
    ///
    /// Raised at most once per failed encoder: the reason is taken out of the
    /// handle rather than read from it.
    FrameEncoderFailed { error: String },
}

/// A connected control-socket client: display label + its event channel.
pub struct Session {
    pub client: String,
    pub events: tokio::sync::mpsc::UnboundedSender<talaria_protocol::ServerMessage>,
}

impl std::fmt::Debug for AgentRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentRequest")
            .field("session_id", &self.session_id)
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

pub struct Shared {
    pub window: Window,
    pub servo: Servo,
    pub window_rendering_context: Rc<WindowRenderingContext>,
    /// The way back onto the main thread from work that is not on it.
    ///
    /// The **fourth** holder of an [`EventLoopProxy`] in this codebase, after
    /// [`Waker`] (servo's own wake mechanism), the control thread's
    /// independently-created one, and the short-lived thread each download
    /// runs on — and the first that lives on `Shared`, because until downloads
    /// there was nothing on `Shared` a background thread needed to tell it
    /// about. The fourth is [`crate::http`]'s listener, which unlike the
    /// download threads is long-lived and holds its clone for as long as
    /// remote access is on. `Shared` is `Rc`-based and therefore not `Send`,
    /// so a spawned thread cannot touch any of it; cloning this proxy *before*
    /// the spawn and sending an [`AppEvent`] back is the only route, and
    /// `user_event` is the only place that route lands.
    pub event_proxy: EventLoopProxy<AppEvent>,
    pub tabs: RefCell<TabManager>,
    pub sessions: RefCell<BTreeMap<u64, Session>>,
    pub vault: RefCell<Vault>,
    /// The human's browsing history. Plaintext, Me-tab-only, and written one
    /// line at a time — see [`crate::history`] for why it diverges from the
    /// vault's whole-array shape.
    pub history: RefCell<History>,
    /// The pages the human chose to keep. A small whole-array store written
    /// only when the star or a bookmark row's Trash button is pressed — see
    /// [`crate::bookmarks`] for why every one of those writes is atomic.
    pub bookmarks: RefCell<Bookmarks>,
    /// The human's preferences — today, only the search engine the address
    /// bar falls back to. Read at navigation time through a shared borrow,
    /// never re-read from disk per keystroke; see [`crate::settings`].
    pub settings: RefCell<Settings>,
    /// Every download that completed, whoever asked for it. Appended from
    /// `user_event`'s [`AppEvent::DownloadCompleted`] arm and from nowhere
    /// else — see [`crate::downloads`] for why this store, unlike history,
    /// has no owner filter.
    pub downloads: RefCell<Downloads>,
    /// What the remote MCP listener is doing. The one source of truth the
    /// Access panel's status block, the toolbar's connection glyph and
    /// [`parse_agent_url`]'s own-origin refusal all read, written only from
    /// the listener's own [`AppEvent::RemoteListenerBound`] /
    /// [`AppEvent::RemoteListenerFailed`] — see [`crate::http`].
    pub remote: RefCell<RemoteAccess>,
    /// The handle that stops the listener thread, present exactly while one
    /// is running. Taken (not cloned) when remote access is switched off, so
    /// the type itself makes a second shutdown of the same listener
    /// unrepresentable.
    pub remote_shutdown: RefCell<Option<ShutdownHandle>>,
    /// The live response streams the listener is serving, addressable by the
    /// client that owns them — see [`crate::http::StreamRegistry`].
    ///
    /// Empty whenever nothing is listening, and empty *while* a listener is
    /// binding: a revoke in that window has no stream to close because none
    /// can exist yet. It is the second half of `UiAction::RevokeClient`, and
    /// the half without which "revoked" would be true of the store and false
    /// of an agent still receiving frames.
    pub remote_streams: RefCell<StreamRegistry>,
    /// The remote viewers currently connected, and what each is attached to —
    /// see [`crate::view`].
    ///
    /// Written only from the three `AppEvent::View*` arms, which are the only
    /// place the listener thread's route lands. **It must never be used to
    /// decide what the human sees**: a viewer's presence changes nothing about
    /// this window, and the tab a viewer is watching is a tab id it named, not
    /// [`TabManager::displayed`]. Empty whenever remote access is off, because
    /// then there is no listener and therefore no `/view` route at all.
    pub views: RefCell<crate::view::ViewSessions>,
    /// The registered agent clients and the digests of the tokens they hold —
    /// see [`crate::agents`].
    ///
    /// **The one field on `Shared` that is not a `RefCell`, and the reason is
    /// the whole point of it.** The four Phase 3 stores are read and written
    /// only on the winit main thread, so a `RefCell` is exactly right for
    /// them. This one is also read by the `talaria-http` thread, on every
    /// single request, by [`crate::oauth::TalariaAuth`] — and it must be the
    /// *same* store, not a second one loaded from the same file. Two
    /// independently-loaded values would give the browser chrome and the token
    /// verifier different answers, so a revocation a human just performed
    /// would keep working over HTTP until the next restart. An
    /// [`std::sync::Arc`] over a [`std::sync::Mutex`] is what makes that
    /// unrepresentable rather than merely avoided.
    ///
    /// Every critical section on either side is one synchronous operation over
    /// a `Vec` of a handful of records, with no `await` inside it, so the
    /// listener thread cannot block the event loop.
    pub agents: SharedAgents,
    /// The one authorization request currently waiting for a human, if any.
    ///
    /// **An `Option`, and never a queue.** "At most one consent request is on
    /// screen at a time" is the anti-harassment cap `04-UI-SPEC.md` fixes, and
    /// parking at most one makes it structural rather than something a counter
    /// has to keep right. The HTTP side refuses a second request outright, so
    /// nothing is dropped by this shape — the caller is told no.
    ///
    /// Written from [`AppEvent::ConsentRequested`] and taken by the Approve
    /// and Deny handlers in `apply_ui_actions`. The consent panel reads it
    /// each frame the way every other panel reads its store, which is also
    /// where the arm delay's `raised_at_ms` comes from — no timing state lives
    /// on the chrome.
    pub pending_consent: RefCell<Option<crate::oauth::ConsentRequest>>,
    /// Bottom of the chrome strip, in logical points.
    pub toolbar_height: Cell<f32>,
    /// Last cursor position, physical pixels.
    pub last_cursor: Cell<Option<PhysicalPosition<f64>>>,
    /// The **local** cursor's last position relative to the webview viewport,
    /// in device pixels.
    ///
    /// Written by [`forward_mouse_move`] and read by [`forward_mouse_button`]
    /// and [`forward_wheel`], and **deliberately not shared with any other
    /// input source**. This one `Cell` is the mechanism by which the toolbar
    /// subtraction reaches all three local forwarders while only one of them
    /// names it, so a second input source writing here would inherit a local
    /// window's chrome offset without any line saying so — and a remote client
    /// draws no server toolbar (T-05-04-A). It would contaminate in the other
    /// direction too: the human moving the mouse would re-aim a remote
    /// viewer's next click (T-05-04-B).
    ///
    /// [`crate::remote_input`] therefore never touches this field; the wire
    /// carries coordinates on every pointer message precisely so it needs no
    /// cached previous position.
    pub webview_point: Cell<euclid::Point2D<f32, DevicePixel>>,
    pub modifiers: Cell<ModifiersState>,
    /// Last window title we set, so the per-frame refresh only touches the
    /// window when the displayed tab's title actually changes.
    pub window_title: RefCell<String>,
    /// Screenshot requests for tabs that are not currently displayed: the
    /// target must be shown and produce a fresh frame before its pixels exist
    /// in the shared framebuffer. Serviced from the event loop (never inside
    /// servo callbacks, which run under a painter borrow).
    pub pending_captures: RefCell<Vec<PendingCapture>>,
    /// `tabs_open` / `navigate` replies waiting for the new page to finish
    /// loading (Playwright `goto` semantics), so an agent's next `evaluate`
    /// runs in the page it asked for rather than the previous one — or in
    /// no document at all, which Servo reports as an opaque InternalError.
    pub pending_loads: RefCell<Vec<PendingLoad>>,
    /// `evaluate` follow-ups that must be issued from the event loop: a
    /// promise result being polled until it settles, or a raw re-run when
    /// the page's CSP forbids the eval-based wrapper. Servo invokes evaluate
    /// callbacks while its evaluator is mutably borrowed, so a follow-up
    /// evaluate can never be started from inside a callback.
    pub pending_evals: RefCell<Vec<PendingEval>>,
    /// One entry per outstanding `evaluate`, so a second script command on a
    /// tab whose script thread is already wedged is refused immediately
    /// instead of burning the whole command timeout. Maintained only from
    /// the event loop; see [`EvalGuard`] for why the completion signal
    /// itself is not.
    pub evaluating: RefCell<Vec<EvalGuard>>,
    /// Unsolicited lifecycle events waiting to be handed to the one session
    /// that owns the tab each describes. Queued rather than sent directly
    /// because a crash or a close is raised from inside a servo delegate
    /// callback, where the session map may be momentarily borrowed — and an
    /// event dropped there is a notification the agent never learns it
    /// missed. Drained from the event loop; see [`Shared::queue_event`].
    pub pending_events: RefCell<Vec<PendingEvent>>,
    /// Tab-table changes a delegate callback could not apply because the
    /// table was busy. Skipping one loses a crash mark, a close, a URL
    /// update or a whole popup, so the work is deferred to the loop instead
    /// (where the table is never contended) rather than discarded.
    pub pending_tab_work: RefCell<Vec<TabWork>>,
    /// Completed navigations waiting to become history rows. Queued rather
    /// than written where they are raised, because they are raised from
    /// inside a servo delegate callback, where the tab table and the store
    /// may both be momentarily borrowed — and a visit dropped there is a page
    /// the user never learns their history missed. Drained by
    /// `process_pending_history_writes()` on the event loop, which reads the
    /// tab's URL and title fresh and applies the Me-only filter.
    pub pending_history_writes: RefCell<Vec<HistoryWrite>>,
}

/// One completed navigation, deferred out of the delegate callback that saw it.
///
/// Only the tab id travels: the URL and the title are read at drain time, not
/// captured here, so a page whose title arrived a moment after `Complete` is
/// recorded with the title it actually ended up with.
pub struct HistoryWrite {
    pub tab_id: u64,
}

/// Wall-clock milliseconds since the unix epoch.
///
/// A clock reporting a time before the epoch yields 0 rather than an error:
/// this is a timestamp on a history row, so a nonsensical one is a cosmetic
/// problem and refusing to record the visit would be a worse answer to it.
/// That is a degrade, not the genuine invariant `expect()` is reserved for.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}

/// One unsolicited event, addressed to exactly one session.
///
/// The addressee is a plain session id rather than an `Option`: a human-owned
/// tab has no agent addressee at all, so its lifecycle produces no entry —
/// the owner filter is expressed in the type rather than re-checked at the
/// send.
pub struct PendingEvent {
    pub session_id: u64,
    pub event: talaria_protocol::Event,
}

/// A tab-table mutation a servo delegate callback had to defer.
///
/// Everything here is keyed by [`WebView`] rather than by tab id, because
/// resolving a webview to its tab is itself a tab-table read — the very thing
/// the callback could not do.
pub enum TabWork {
    /// Adopt an already-built popup webview under its parent's tab. Building
    /// the webview and its framebuffer needs no tab-table borrow, so a
    /// callback can always get that far before deciding to defer.
    AdoptPopup {
        parent: WebView,
        webview: WebView,
        rendering_context: Rc<OffscreenRenderingContext>,
    },
    /// Mark the webview's tab crashed and notify its owner.
    MarkCrashed { webview: WebView },
    /// Close the webview's tab and notify its owner.
    Close { webview: WebView },
    /// Apply a location change to the webview's tab.
    UrlChanged { webview: WebView, url: Url },
}

/// One outstanding `evaluate` on a tab.
///
/// `done` is a reference-counted `Cell`, deliberately — not a `RefCell` and
/// not a field on `Tab`. Every way an evaluate can end is reachable from
/// inside a servo evaluate callback, which runs while the engine holds its
/// own borrows; setting a `Cell` neither borrows nor can fail, so a terminus
/// records completion without the `try_borrow` that silently drops work
/// elsewhere in this file. A missed flag here would leave a healthy tab
/// permanently refused, so it must not be droppable.
pub struct EvalGuard {
    pub tab_id: u64,
    /// Set by whichever terminus ends the evaluate; swept from the loop.
    pub done: Rc<Cell<bool>>,
    /// Expire the entry even if the callback never arrives at all — the
    /// engine can drop one, and a lost callback must self-heal rather than
    /// wedge the tab forever. Same instant the evaluate itself is bounded by.
    pub deadline: std::time::Instant,
}

pub struct PendingEval {
    pub webview: WebView,
    pub step: EvalStep,
    pub reply: tokio::sync::oneshot::Sender<Outcome>,
    /// Give up polling a promise after this (just under the command timeout).
    pub deadline: std::time::Instant,
    /// Earliest time to issue the next evaluate.
    pub next: std::time::Instant,
    /// This evaluate's [`EvalGuard::done`] flag, carried forward so whichever
    /// branch finally replies clears the tab's in-flight entry.
    pub done: Rc<Cell<bool>>,
}

pub enum EvalStep {
    /// Poll `window.__talaria_async[slot]` until the promise settles.
    Poll { slot: u64 },
    /// Run the agent's script unwrapped (CSP blocked eval): promises are not
    /// awaited on such pages.
    RunRaw { script: String },
}

/// How long `tabs_open` / `navigate` wait for LoadStatus::Complete before
/// replying anyway with `loading: true`. Well under the 30s command timeout.
pub const LOAD_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

pub struct PendingLoad {
    pub tab_id: u64,
    pub reply: tokio::sync::oneshot::Sender<Outcome>,
    pub deadline: std::time::Instant,
    /// The page was already Complete when the navigation was requested, so
    /// a Complete notification only counts after a Started/HeadParsed one
    /// (otherwise it is the *old* page's).
    pub needs_start: bool,
    pub started: bool,
    /// Set by notify_load_status_changed; serviced from the event loop.
    pub ready: bool,
}

pub struct PendingCapture {
    pub webview: WebView,
    pub context: Rc<OffscreenRenderingContext>,
    pub reply: tokio::sync::oneshot::Sender<Outcome>,
    /// Set by notify_new_frame_ready once the target has painted a frame.
    pub ready: bool,
    pub deadline: std::time::Instant,
}

impl Shared {
    /// Paint a webview into its own framebuffer and read the pixels back.
    /// `hide_after` re-hides a background tab that was shown just to produce
    /// a frame; the displayed tab is never touched (its framebuffer is its
    /// own), so captures cause no visible flicker.
    pub fn capture_now(
        &self,
        webview: &WebView,
        context: &OffscreenRenderingContext,
        hide_after: bool,
    ) -> Outcome {
        webview.paint();
        let size = context.size2d().to_i32();
        let rect = euclid::Box2D::from_origin_and_size(
            euclid::Point2D::origin(),
            euclid::Size2D::new(size.width, size.height),
        );
        let image = context.read_to_image(rect);
        if hide_after {
            webview.hide();
        }
        match image {
            Some(image) => encode_screenshot(image),
            None => Outcome::Error { message: "framebuffer read failed".into() },
        }
    }

    /// Service queued background-tab captures whose frame is ready (or whose
    /// wait timed out — a static page may never produce a new frame).
    pub fn process_pending_captures(&self) {
        let mut due = Vec::new();
        {
            let mut pending = self.pending_captures.borrow_mut();
            let now = std::time::Instant::now();
            let mut index = 0;
            while index < pending.len() {
                if pending[index].ready || pending[index].deadline <= now {
                    due.push(pending.remove(index));
                } else {
                    index += 1;
                }
            }
        }
        for capture in due {
            // A tab a viewer is watching must stay shown. `capture_now`'s
            // re-hide exists to undo the show this queue performed for a
            // background tab, and it used to fire unconditionally — which
            // silently broke remote takeover, because a hidden webview answers
            // no hit test. The viewer's frames kept arriving (they are read
            // from the offscreen buffer, which does not need the webview shown)
            // while its clicks stopped landing, and nothing reported it: the
            // hold count still read 1 and only `last_applied_input` stopped
            // advancing. An agent taking a routine screenshot was enough.
            //
            // `visibility_of` is the single source of truth for whether a
            // webview should be shown, so ask it rather than re-deriving the
            // rule here.
            let still_wanted = self
                .tabs
                .try_borrow()
                .ok()
                .and_then(|tabs| {
                    tabs.find_by_webview(&capture.webview).and_then(|id| {
                        tabs.get(id).map(|tab| {
                            crate::tabs::visibility_of(Some(id) == tabs.active_id(tab.owner.view()), tab.held_for_view)
                        })
                    })
                })
                .unwrap_or(crate::tabs::Visibility::Hidden);
            let hide_after = matches!(still_wanted, crate::tabs::Visibility::Hidden);
            let outcome = self.capture_now(&capture.webview, &capture.context, hide_after);
            let _ = capture.reply.send(outcome);
        }
        self.process_pending_loads();
        // Chained here rather than added to each winit handler: all three
        // handlers already call this drain, so both queues are reached from
        // every one of them. Tab work runs first because applying it is what
        // raises the events the second drain then delivers.
        self.process_pending_tab_work();
        self.process_pending_events();
        // And the frame pump, for the same reason and gated by its own
        // deadlines rather than by which handler happened to run: an
        // attachment that is not due costs the comparison of one instant.
        self.process_view_frames();
    }

    /// Turn queued completed navigations into history rows.
    ///
    /// Runs on the event loop, so borrowing normally is safe here — and it is
    /// the only place the store is written, which is what keeps file I/O out
    /// of the delegate callback that raised the visit.
    ///
    /// Four things are silently skipped, all of them routine: a tab that was
    /// closed between the callback and this drain, a tab an agent owns, a
    /// load an agent started in one of the human's own tabs, and a tab still
    /// sitting on the blank page.
    ///
    /// Borrows the tab table mutably rather than sharing it, because the
    /// provenance flag [`belongs_in_history`] reads is taken as it is read —
    /// see that function for why the read and the clear are one step.
    pub fn process_pending_history_writes(&self) {
        let writes: Vec<HistoryWrite> =
            self.pending_history_writes.borrow_mut().drain(..).collect();
        if writes.is_empty() {
            return;
        }
        let mut tabs = self.tabs.borrow_mut();
        let mut history = self.history.borrow_mut();
        for write in writes {
            let Some(tab) = tabs.get_mut(write.tab_id) else { continue };
            // The whole of the filter, and it asks who *caused* this load
            // rather than who owns the tab — the two are not the same
            // question, and answering the second one let an agent write rows
            // the human would read as their own.
            if !belongs_in_history(&tab.owner, &mut tab.load_started_by_agent) {
                continue;
            }
            let Some(url) = tab.webview.url() else { continue };
            if url.as_str() == "about:blank" {
                continue;
            }
            let title = tab.webview.page_title().unwrap_or_default();
            history.append(url.to_string(), title, now_ms());
        }
    }

    /// Drop in-flight entries that have completed or expired. Runs on the
    /// event loop, so borrowing normally is safe here.
    pub fn sweep_evaluating(&self) {
        let now = std::time::Instant::now();
        self.evaluating
            .borrow_mut()
            .retain(|guard| !guard.done.get() && guard.deadline > now);
    }

    /// Whether `tab_id` already has an evaluate in flight.
    pub fn tab_evaluating(&self, tab_id: u64) -> bool {
        self.sweep_evaluating();
        self.evaluating.borrow().iter().any(|guard| guard.tab_id == tab_id)
    }

    /// Record a new outstanding evaluate on `tab_id` and hand back its
    /// completion flag for the evaluate machinery to set when it ends.
    pub fn begin_evaluating(&self, tab_id: u64) -> Rc<Cell<bool>> {
        let done = Rc::new(Cell::new(false));
        self.evaluating.borrow_mut().push(EvalGuard {
            tab_id,
            done: done.clone(),
            deadline: std::time::Instant::now() + promise_wait(),
        });
        done
    }

    /// Issue due evaluate follow-ups (see [`PendingEval`]).
    pub fn process_pending_evals(self: &Rc<Self>) {
        self.sweep_evaluating();
        let now = std::time::Instant::now();
        let due: Vec<PendingEval> = {
            let mut pending = self.pending_evals.borrow_mut();
            let mut due = Vec::new();
            let mut index = 0;
            while index < pending.len() {
                if pending[index].next <= now {
                    due.push(pending.remove(index));
                } else {
                    index += 1;
                }
            }
            due
        };
        for eval in due {
            if eval.deadline <= now {
                eval.done.set(true);
                let _ = eval.reply.send(Outcome::Error {
                    message: format!(
                        "promise did not settle within {}s",
                        promise_wait().as_secs()
                    ),
                });
                continue;
            }
            let state = self.clone();
            let PendingEval { webview, step, reply, deadline, done, .. } = eval;
            match step {
                EvalStep::Poll { slot } => {
                    let poll_webview = webview.clone();
                    webview.evaluate_javascript(poll_script(slot), move |result| {
                        match result {
                            Ok(servo::JSValue::Object(mut map)) => {
                                if map.contains_key("__talaria_pending") {
                                    // Still in flight: carry the flag forward
                                    // rather than clearing it.
                                    state.pending_evals.borrow_mut().push(PendingEval {
                                        webview: poll_webview,
                                        step: EvalStep::Poll { slot },
                                        reply,
                                        deadline,
                                        next: std::time::Instant::now()
                                            + std::time::Duration::from_millis(50),
                                        done,
                                    });
                                    state.window.request_redraw();
                                    return;
                                }
                                let settled = match map.remove("state") {
                                    Some(servo::JSValue::String(s)) => s,
                                    _ => String::new(),
                                };
                                let outcome = if settled == "ok" {
                                    Outcome::Ok {
                                        result: ResultPayload::Value {
                                            value: map
                                                .remove("value")
                                                .map(js_value_to_json)
                                                .unwrap_or(serde_json::Value::Null),
                                        },
                                    }
                                } else {
                                    let message = match map.remove("message") {
                                        Some(servo::JSValue::String(m)) => m,
                                        _ => "unknown".into(),
                                    };
                                    Outcome::Error { message: format!("promise rejected: {message}") }
                                };
                                done.set(true);
                                let _ = reply.send(outcome);
                            },
                            Ok(_) => {
                                done.set(true);
                                let _ = reply.send(Outcome::Error {
                                    message: "promise poll returned an unexpected shape".into(),
                                });
                            },
                            Err(error) => {
                                done.set(true);
                                let _ = reply.send(Outcome::Error {
                                    message: format!(
                                        "page went away before the promise settled ({})",
                                        describe_js_error(error)
                                    ),
                                });
                            },
                        }
                    });
                },
                EvalStep::RunRaw { script } => {
                    webview.evaluate_javascript(script, move |result| {
                        done.set(true);
                        let _ = reply.send(match result {
                            Ok(value) => Outcome::Ok {
                                result: ResultPayload::Value { value: js_value_to_json(value) },
                            },
                            Err(error) => Outcome::Error { message: describe_js_error(error) },
                        });
                    });
                },
            }
        }
    }

    /// Reply to `tabs_open` / `navigate` requests whose page finished loading,
    /// timed out waiting, or whose tab went away meanwhile.
    fn process_pending_loads(&self) {
        let mut due = Vec::new();
        {
            let mut pending = self.pending_loads.borrow_mut();
            let tabs = self.tabs.borrow();
            let now = std::time::Instant::now();
            let mut index = 0;
            while index < pending.len() {
                let load = &pending[index];
                if load.ready || load.deadline <= now || tabs.get(load.tab_id).is_none() {
                    due.push(pending.remove(index));
                } else {
                    index += 1;
                }
            }
        }
        for load in due {
            let tabs = self.tabs.borrow();
            let outcome = match tabs.get(load.tab_id) {
                Some(tab) => Outcome::Ok {
                    result: ResultPayload::Tab { tab: tab_info(&tabs, tab) },
                },
                None => Outcome::Error {
                    message: format!("tab {} was closed while loading", load.tab_id),
                },
            };
            let _ = load.reply.send(outcome);
        }
    }

    /// Queue a reply for when `tab_id`'s current navigation completes.
    pub fn reply_after_load(
        &self,
        tab_id: u64,
        needs_start: bool,
        reply: tokio::sync::oneshot::Sender<Outcome>,
    ) {
        self.pending_loads.borrow_mut().push(PendingLoad {
            tab_id,
            reply,
            deadline: std::time::Instant::now() + LOAD_WAIT,
            needs_start,
            started: false,
            ready: false,
        });
    }

    /// Earliest deadline among queued captures/loads/evals, outstanding
    /// evaluates and the frame pump's next tick, for WaitUntil scheduling.
    /// The in-flight registry is a source here because a deadline nothing
    /// wakes for is not a deadline: an entry whose callback was lost must
    /// expire on time, not whenever the loop happens to turn next.
    ///
    /// **The frame pump joins this computation rather than installing a
    /// second control-flow source**, which is the whole of how its cadence is
    /// scheduled: two things deciding when the loop wakes is how a loop stops
    /// waking. With no viewer attached the pump contributes `None` and the
    /// loop waits exactly as it did before.
    pub fn next_capture_deadline(&self) -> Option<std::time::Instant> {
        let captures = self.pending_captures.borrow().iter().map(|c| c.deadline).min();
        let loads = self.pending_loads.borrow().iter().map(|l| l.deadline).min();
        let evals = self.pending_evals.borrow().iter().map(|e| e.next).min();
        let evaluating = self.evaluating.borrow().iter().map(|g| g.deadline).min();
        let frames = self.views.borrow().next_tick();
        [captures, loads, evals, evaluating, frames].into_iter().flatten().min()
    }

    /// Keep the window title in step with whatever tab is displayed (tab
    /// switches, closes and view toggles included — not just page-title
    /// events). Called from the chrome update; cheap when nothing changed.
    pub fn refresh_window_title(&self, title: Option<String>) {
        let title = match title {
            Some(title) if !title.is_empty() => format!("{title} — Talaria"),
            _ => "Talaria".to_owned(),
        };
        if *self.window_title.borrow() != title {
            self.window.set_title(&title);
            *self.window_title.borrow_mut() = title;
        }
    }

    /// A remote viewer's socket was accepted. Record the connection.
    ///
    /// Nothing about the local window changes: no redraw is requested, no tab
    /// is shown, focused or switched to, and the view mode is untouched. A
    /// remote party connecting is not an event the human is supposed to notice
    /// on screen, and making it one would be the first half of letting a
    /// viewer move what somebody sitting here is looking at.
    pub fn view_opened(
        &self,
        connection: u64,
        client_id: String,
        out: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    ) {
        self.views.borrow_mut().opened(connection, client_id, out);
        self.publish_views();
    }

    /// One frame from a viewer. A frame that could not be answered ends the
    /// connection — the socket task learns that when its channel drops.
    ///
    /// The input channel lands in [`crate::remote_input`] and nowhere else:
    /// this is the one dispatch site, and it deliberately routes *nothing*
    /// through the window-event arms, the browser-shortcut handler or the
    /// interface-action queue that a local click travels through (T-05-04).
    pub fn view_message(&self, connection: u64, frame: &[u8]) {
        // Scoped, as every borrow here is: the tab table is read through a
        // shared borrow that is released before the session table is touched
        // again, so a message can never wedge the loop it reports into.
        let handled = {
            // Mutable now that an attach takes a visibility hold on the tab
            // table, and still scoped for the same reason it always was.
            let mut tabs = self.tabs.borrow_mut();
            self.views.borrow_mut().message(connection, frame, &mut *tabs)
        };
        // Raised here because this is where an attach happens, and an attach
        // is what starts the encoder. Taken rather than read, so a failure is
        // reported once and not on every message that follows it.
        if let Some(error) = self.views.borrow_mut().take_encoder_failure() {
            let _ = self.event_proxy.send_event(AppEvent::FrameEncoderFailed { error });
        }
        match handled {
            crate::view::Handled::Done => {},
            crate::view::Handled::Close => self.view_closed(connection),
            crate::view::Handled::Input(message) => {
                // Whether it was applied is not reported to the peer: every
                // refusal is silent beyond the fact of not happening, and none
                // of them says which rule was broken.
                let _ = crate::remote_input::apply(self, connection, &message);
            },
        }
    }

    /// Publish the agent-tab snapshot to every viewer whose copy is stale.
    ///
    /// Run from the event loop rather than from each tab-table mutation, and
    /// a no-op when nothing changed — see [`crate::view::ViewSessions::publish`].
    pub fn publish_views(&self) {
        if self.views.borrow().is_empty() {
            return;
        }
        let tabs = self.tabs.borrow();
        self.views.borrow_mut().publish(&*tabs);
    }

    /// A viewer's socket ended. Release everything it held — the attachment
    /// records here, and the visibility holds those records took on the tab
    /// table, which is what returns each tab to whatever the local view state
    /// says it should be.
    pub fn view_closed(&self, connection: u64) {
        let mut tabs = self.tabs.borrow_mut();
        self.views.borrow_mut().closed(connection, &mut *tabs);
    }

    /// One turn of the frame pump: paint and read back every attachment that
    /// is due, and hand each buffer onward.
    ///
    /// **The whole of what the winit loop spends on a viewer.** The tile
    /// comparison and the encode are not here — they are the encoder thread's,
    /// because the readback buffer is `Send` and moving them off the loop is
    /// the difference between the pump costing about a twentieth of a tick and
    /// about a quarter of it (05-02 measured 5.8 % on a settled page and 21 %
    /// while scrolling, with the comparison and the encode already excluded).
    ///
    /// The borrows are both scoped and both released before the paint: the due
    /// list is taken out of the session table first, and the two engine
    /// handles are cloned out of the tab table second, so neither cell is held
    /// across the slowest step. Servo borrows the same tab table from its own
    /// delegate callbacks.
    pub fn process_view_frames(&self) {
        let due = {
            let mut views = self.views.borrow_mut();
            if views.is_empty() {
                return;
            }
            views.take_due(std::time::Instant::now(), crate::view::view_idle())
        };
        for tick in due {
            let handles = {
                // Through the agent-only lookup, so a tab the human owns is
                // not merely refused here — it cannot be named.
                let tabs = self.tabs.borrow();
                tabs.agent_tab(tick.tab)
                    .map(|tab| (tab.webview.clone(), tab.rendering_context.clone()))
            };
            // The lease outlived its tab. `publish_views` detaches it on this
            // same turn, so nothing is said here.
            let Some((webview, context)) = handles else {
                continue;
            };
            match crate::view::capture(&webview, &context) {
                crate::view::CaptureOutcome::Painted(surface) => {
                    self.views.borrow_mut().frame_captured(&tick, surface);
                },
                // A skipped tick and nothing more: the lease is still good and
                // the next tick will try again. Warn rather than error,
                // because this is a degraded fallback the user should know
                // about and not a subsystem failing.
                crate::view::CaptureOutcome::ReadFailed => {
                    log::warn!(
                        "framebuffer read failed for tab {} on view connection {}; \
                         skipping this frame",
                        tick.tab,
                        tick.connection
                    );
                    if tick.keyframe {
                        self.views.borrow_mut().require_keyframe_again(&tick);
                    }
                },
            }
        }
    }

    /// Queue an unsolicited event for the session that owns the tab it
    /// describes — and for no other.
    ///
    /// A human-owned tab has no agent addressee, so its lifecycle produces
    /// nothing at all. This is *addressing*, not policy: Talaria does not
    /// gate what an agent may do, it only decides who a message was for.
    /// Telling agent A about agent B's tab ids is a leak, and it is free to
    /// avoid here while the delivery path is being built.
    ///
    /// Safe to call from inside a servo delegate callback: pushing onto the
    /// queue touches neither the session map nor the tab table, and the drain
    /// holds this borrow only long enough to take the entries out.
    pub fn queue_event(&self, owner: &TabOwner, event: talaria_protocol::Event) {
        let TabOwner::Agent { session_id, .. } = owner else {
            return;
        };
        self.pending_events
            .borrow_mut()
            .push(PendingEvent { session_id: *session_id, event });
        // A queued event that nothing wakes for is a dropped event.
        self.window.request_redraw();
    }

    /// Hand each queued event to its addressee's channel.
    ///
    /// Split scope, as with every other queue here: the entries are taken out
    /// under a borrow that makes no call into servo and no call into the
    /// session map, which is what leaves a producing callback unable to
    /// contend for it in the first place.
    pub fn process_pending_events(&self) {
        let due: Vec<PendingEvent> = std::mem::take(&mut self.pending_events.borrow_mut());
        for entry in due {
            // A session that disconnected between the event being queued and
            // this drain is simply absent from the map. Discarding its entry
            // is correct addressing, not a lost notification: there is no
            // longer anyone the event was for.
            if let Some(session) = self.sessions.borrow().get(&entry.session_id) {
                let _ = session
                    .events
                    .send(talaria_protocol::ServerMessage::Event { event: entry.event });
            }
        }
    }

    /// Apply tab-table changes a delegate callback had to defer.
    ///
    /// Same split scope: the queue borrow is released before anything touches
    /// the tab table or calls into servo, so a callback pushing onto it can
    /// never find it busy.
    pub fn process_pending_tab_work(&self) {
        let due: Vec<TabWork> = std::mem::take(&mut self.pending_tab_work.borrow_mut());
        for work in due {
            match work {
                TabWork::AdoptPopup { parent, webview, rendering_context } => {
                    let mut tabs = self.tabs.borrow_mut();
                    self.adopt_popup(&mut tabs, &parent, webview, rendering_context);
                },
                TabWork::MarkCrashed { webview } => {
                    let marked = {
                        let mut tabs = self.tabs.borrow_mut();
                        tabs.find_by_webview_mut(&webview).map(|tab| {
                            tab.crashed = true;
                            (tab.id, tab.owner.clone())
                        })
                    };
                    if let Some((tab_id, owner)) = marked {
                        self.queue_event(&owner, talaria_protocol::Event::TabCrashed { tab_id });
                    }
                },
                TabWork::Close { webview } => {
                    let mut closed = None;
                    {
                        let mut tabs = self.tabs.borrow_mut();
                        // Owner first: the close removes the tab, and an event
                        // raised afterwards would have nothing to address.
                        if let Some(id) = tabs.find_by_webview(&webview) {
                            let owner = tabs.get(id).map(|tab| tab.owner.clone());
                            if let (Some(owner), true) = (owner, tabs.close(id)) {
                                closed = Some((id, owner));
                            }
                        }
                    }
                    if let Some((tab_id, owner)) = closed {
                        self.queue_event(&owner, talaria_protocol::Event::TabClosed { tab_id });
                    }
                },
                TabWork::UrlChanged { webview, url } => {
                    apply_url_change(&mut self.tabs.borrow_mut(), &webview, &url);
                },
            }
        }
    }

    /// Adopt an already-built popup webview as a tab under its opener: the
    /// popup takes the parent's owner (an agent's popups stay that agent's)
    /// and fronts its own view only if the opener was that view's active tab.
    ///
    /// Shared by the inline delegate path and the deferred queue so both
    /// produce the same tab, including the same activation rule.
    fn adopt_popup(
        &self,
        tabs: &mut TabManager,
        parent_webview: &WebView,
        webview: WebView,
        rendering_context: Rc<OffscreenRenderingContext>,
    ) {
        let Some(parent_id) = tabs.find_by_webview(parent_webview) else {
            return;
        };
        let Some(owner) = tabs.get(parent_id).map(|tab| tab.owner.clone()) else {
            return;
        };
        // A popup fronts its own view when its opener was the active tab
        // there (browser behaviour), and stays behind an opener that was
        // already in the background. Which view is *displayed* is irrelevant:
        // an agent's popup must not drag the human out of the Me view.
        let parent_active = tabs.active_id(owner.view()) == Some(parent_id);
        // The popup's real URL arrives via notify_url_changed; until then the
        // URL bar shows what a `window.open()` with no argument keeps forever.
        let id = tabs.register(
            webview,
            rendering_context,
            owner.clone(),
            "about:blank".into(),
            parent_active,
            true,
        );
        log::info!("popup from tab {parent_id} opened as tab {id}");
        // Adoption is the one way a tab joins an agent's session without the
        // agent asking, so it is the one that needs announcing; everything
        // else the agent opened, it already knows about from its own reply.
        self.queue_event(&owner, talaria_protocol::Event::TabOpened {
            tab_id: id,
            opener_tab_id: parent_id,
        });
    }
}

/// Apply a page-driven location change to the webview's tab. Shared by
/// `notify_url_changed` and the deferred queue so a deferred update is
/// byte-for-byte the update the callback would have made.
fn apply_url_change(tabs: &mut TabManager, webview: &WebView, url: &Url) {
    let Some(tab) = tabs.find_by_webview_mut(webview) else {
        return;
    };
    if !tab.location_dirty {
        tab.location = url.to_string();
    }
    // The navigation an adopted popup was waiting for has begun, so its
    // blank grace window is over and `load_status` alone tells the truth
    // from here. Servo runs a whole Started→Complete cycle on the popup's
    // *own* blank document first, which is why load status can't be that
    // signal.
    if url.as_str() != "about:blank" {
        tab.initial_blank_until = None;
    }
}

impl Shared {
    pub fn hidpi_scale(&self) -> f32 {
        self.window.scale_factor() as f32
    }

    fn toolbar_height_device(&self) -> f32 {
        self.toolbar_height.get() * self.hidpi_scale()
    }

    pub fn open_tab(self: &Rc<Self>, url: Url, owner: TabOwner) -> u64 {
        let delegate: Rc<dyn servo::WebViewDelegate> = self.clone();
        self.tabs.borrow_mut().open(
            &self.servo,
            &self.window_rendering_context,
            self.window.inner_size(),
            delegate,
            self.hidpi_scale(),
            url,
            owner,
        )
    }
}

pub enum App {
    Initial { waker: Waker, initial_url: Url },
    Running(Rc<Shared>),
}

impl App {
    pub fn new(event_loop: &EventLoop<AppEvent>, initial_url: Url) -> Self {
        App::Initial {
            waker: Waker(event_loop.create_proxy()),
            initial_url,
        }
    }

    fn state(&self) -> Option<&Rc<Shared>> {
        match self {
            App::Running(state) => Some(state),
            App::Initial { .. } => None,
        }
    }
}

thread_local! {
    static GUI: RefCell<Option<Gui>> = const { RefCell::new(None) };
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let App::Initial { waker, initial_url } = self else {
            return;
        };
        let display_handle = event_loop.display_handle().expect("display handle");
        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("Talaria")
                    .with_inner_size(winit::dpi::LogicalSize::new(1200.0, 800.0)),
            )
            .expect("create window");
        let window_handle = window.window_handle().expect("window handle");

        let window_rendering_context = Rc::new(
            WindowRenderingContext::new(display_handle, window_handle, window.inner_size())
                .expect("create window rendering context"),
        );
        let _ = window_rendering_context.make_current();

        // Persistent engine profile (localStorage, indexeddb, cookies…).
        // The default is a temp dir, which both discards session state on
        // exit and failed to initialize ClientStorage's sqlite at all.
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("talaria")
            .join("servo");
        let _ = std::fs::create_dir_all(&config_dir);
        let opts = servo::Opts {
            config_dir: Some(config_dir),
            ..Default::default()
        };

        let servo = ServoBuilder::default()
            .opts(opts)
            .event_loop_waker(Box::new(waker.clone()))
            .build();
        servo.setup_logging();

        GUI.with_borrow_mut(|gui| {
            *gui = Some(Gui::new(event_loop, window_rendering_context.clone()));
        });

        let state = Rc::new(Shared {
            window,
            servo,
            window_rendering_context,
            // `waker` is the destructured `&Waker` from `App::Initial`,
            // already in scope here; its proxy is cloned rather than moved
            // because servo keeps the `Waker` itself.
            event_proxy: waker.0.clone(),
            tabs: RefCell::new(TabManager::new()),
            sessions: RefCell::new(BTreeMap::new()),
            vault: RefCell::new(Vault::load()),
            history: RefCell::new(History::load()),
            bookmarks: RefCell::new(Bookmarks::load()),
            settings: RefCell::new(Settings::load()),
            downloads: RefCell::new(Downloads::load()),
            remote: RefCell::new(RemoteAccess::default()),
            remote_shutdown: RefCell::new(None),
            remote_streams: RefCell::new(StreamRegistry::default()),
            views: RefCell::new(crate::view::ViewSessions::default()),
            // Loaded once, here, and shared by clone with the listener thread
            // — never loaded a second time anywhere. See the field's comment.
            agents: Arc::new(Mutex::new(Agents::load())),
            pending_consent: RefCell::new(None),
            toolbar_height: Cell::new(0.0),
            last_cursor: Cell::new(None),
            webview_point: Cell::new(euclid::Point2D::zero()),
            modifiers: Cell::new(ModifiersState::empty()),
            window_title: RefCell::new(String::new()),
            pending_captures: RefCell::new(Vec::new()),
            pending_loads: RefCell::new(Vec::new()),
            pending_evals: RefCell::new(Vec::new()),
            evaluating: RefCell::new(Vec::new()),
            pending_events: RefCell::new(Vec::new()),
            pending_tab_work: RefCell::new(Vec::new()),
            pending_history_writes: RefCell::new(Vec::new()),
        });

        // The switch survives a restart: a `config.json` that says remote
        // access is on starts the listener here, before the first tab, and a
        // fresh install (no file) starts nothing at all — D-04-04's
        // default-off is this `if` and no other check.
        // Cloned rather than copied: `RemoteAccessConfig` now carries the
        // advertised origin, which is a `String`, so the type is no longer
        // `Copy`. See `settings::RemoteAccessConfig::advertised_url`.
        let configured = state.settings.borrow().remote_access.clone();
        if configured.enabled {
            start_remote_listener(&state, &configured);
        }

        state.open_tab(initial_url.clone(), TabOwner::Me);
        *self = App::Running(state);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, _cause: winit::event::StartCause) {
        if let Some(state) = self.state() {
            state.process_pending_captures();
            state.process_pending_evals();
            state.process_pending_history_writes();
            set_wait(event_loop, state);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        if let Some(state) = self.state() {
            state.process_pending_captures();
            state.process_pending_evals();
            state.process_pending_history_writes();
            match event {
                AppEvent::Wake => {},
                AppEvent::SessionStarted { session_id, client, events } => {
                    state
                        .sessions
                        .borrow_mut()
                        .insert(session_id, Session { client, events });
                    state.window.request_redraw();
                },
                AppEvent::SessionEnded { session_id } => {
                    state.sessions.borrow_mut().remove(&session_id);
                    state.window.request_redraw();
                },
                AppEvent::Agent(request) => {
                    execute_agent_command(state, request);
                    state.window.request_redraw();
                },
                // The one arm raised from off the main thread. The store is
                // touched here and only here, because `Shared` is `Rc`-based
                // and the thread that learned about the download cannot hold
                // any of it.
                AppEvent::DownloadCompleted { path, filename, url, bytes, requested_by_agent } => {
                    state.downloads.borrow_mut().append(
                        path,
                        filename,
                        url,
                        bytes,
                        now_ms(),
                        requested_by_agent,
                    );
                    state.window.request_redraw();
                },
                // Raised from the `talaria-http` thread, which cannot touch
                // any of `Shared`. The address written here is the one the
                // socket reported, so the two surfaces that render it and the
                // refusal that keys on it read the same string.
                AppEvent::RemoteListenerBound { addr } => {
                    *state.remote.borrow_mut() = RemoteAccess::Bound { addr };
                    state.window.request_redraw();
                },
                AppEvent::RemoteListenerFailed { addr, error } => {
                    // The shutdown handle goes with it: the thread it would
                    // have stopped is already gone.
                    state.remote_shutdown.borrow_mut().take();
                    *state.remote.borrow_mut() = RemoteAccess::Failed { addr, error };
                    state.window.request_redraw();
                },
                // Also raised from the `talaria-http` thread. The request is
                // parked here — at most one, ever — and the panel reads it
                // from there each frame, so no timing state and no secret
                // travels into the chrome.
                AppEvent::ConsentRequested(request) => {
                    log::info!("consent requested by client {}", request.client_id);
                    *state.pending_consent.borrow_mut() = Some(request);
                    // Through `raise_consent` and never through
                    // `UiAction::SetPanel`, which refuses this panel: the two
                    // entry points are disjoint by construction, which is what
                    // makes "no control can open the Approve button" a
                    // property a reviewer confirms in one place.
                    GUI.with_borrow_mut(|gui| {
                        if let Some(gui) = gui.as_mut() {
                            gui.raise_consent();
                        }
                    });
                    // The flow starts in an *external* browser (RFC 8252), so
                    // the human is by definition looking somewhere else when
                    // this arrives. Ask for their attention — and deliberately
                    // do **not** take focus: pulling focus toward a screen
                    // whose primary button grants full browser control is the
                    // exact opposite of the arm delay that button carries.
                    state.window.request_user_attention(Some(
                        winit::window::UserAttentionType::Informational,
                    ));
                    state.window.request_redraw();
                },
                // The three view arms. Everything they do happens here, on the
                // main thread, because everything they do reads the tab table
                // — the listener thread holds none of `Shared`.
                AppEvent::ViewOpened { connection, client_id, out } => {
                    state.view_opened(connection, client_id, out);
                },
                AppEvent::ViewMessage { connection, frame } => {
                    state.view_message(connection, &frame);
                },
                AppEvent::ViewClosed { connection } => {
                    state.view_closed(connection);
                },
                // Nothing local changes. The chrome is not told, no panel
                // opens and no tab moves: this is a remote capability that is
                // unavailable, which is a fact about the viewers and not about
                // the browser the human is using.
                AppEvent::FrameEncoderFailed { error } => {
                    log::error!(
                        "the frame encoder is unavailable, so remote viewers cannot be \
                         sent frames: {error}"
                    );
                },
            }
            // *After* the arm, not before it: an agent command that opened or
            // closed a tab has to reach a viewer on this turn rather than
            // waiting for whatever happens next.
            state.publish_views();
            state.servo.spin_event_loop();
            set_wait(event_loop, state);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = self.state().cloned() else {
            return;
        };
        state.process_pending_captures();
        state.process_pending_evals();
        state.process_pending_history_writes();
        // Alongside the other per-turn drains, and for the same reason: a tab
        // opened, closed or crashed by any path at all reaches a viewer from
        // one place rather than from every mutation site.
        state.publish_views();

        let over_toolbar = |state: &Shared| {
            state
                .last_cursor
                .get()
                .is_none_or(|p| (p.y / state.window.scale_factor()) < state.toolbar_height.get() as f64)
        };

        // When the chrome has replaced the page — any panel, or a crashed
        // tab's recovery page — the area below the toolbar belongs to egui,
        // not to a webview. Without this, a click down there is forwarded to
        // a page nobody can see and the panel's own buttons are unreachable
        // by mouse.
        let chrome_replaces_page = |state: &Shared| {
            GUI.with_borrow(|gui| gui.as_ref().is_some_and(|gui| gui.panel_open()))
                || state.tabs.borrow().displayed().is_some_and(|tab| tab.crashed)
        };

        let mut repaint_now = false;
        match &event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            },
            WindowEvent::Resized(new_size) => {
                state.window_rendering_context.resize(*new_size);
                repaint_now = true;
            },
            WindowEvent::RedrawRequested => {
                repaint_now = true;
            },
            WindowEvent::ModifiersChanged(modifiers) => {
                state.modifiers.set(modifiers.state());
                // egui tracks modifiers from this event too.
                GUI.with_borrow_mut(|gui| {
                    if let Some(gui) = gui.as_mut() {
                        let _ = gui.on_window_event(&state.window, &event);
                    }
                });
            },
            WindowEvent::CursorMoved { position, .. } => {
                state.last_cursor.set(Some(*position));
                // egui always sees cursor movement (tooltips); the page also
                // sees it when the cursor is over the viewport.
                let response = GUI.with_borrow_mut(|gui| {
                    gui.as_mut().map(|gui| gui.on_window_event(&state.window, &event))
                });
                if response.is_some_and(|r| r.repaint) {
                    state.window.request_redraw();
                }
                if !over_toolbar(&state) {
                    forward_mouse_move(&state, *position);
                }
            },
            WindowEvent::MouseInput { button, state: element_state, .. }
                if !over_toolbar(&state) && !chrome_replaces_page(&state) =>
            {
                GUI.with_borrow_mut(|gui| {
                    if let Some(gui) = gui.as_mut() {
                        gui.surrender_focus();
                    }
                });
                forward_mouse_button(&state, *button, *element_state);
            },
            WindowEvent::MouseWheel { delta, .. }
                if !over_toolbar(&state) && !chrome_replaces_page(&state) =>
            {
                forward_wheel(&state, *delta);
            },
            WindowEvent::KeyboardInput { event: key_event, .. }
                if handle_browser_shortcut(&state, key_event) => {},
            WindowEvent::KeyboardInput { event: key_event, .. }
                if !GUI.with_borrow(|gui| {
                    gui.as_ref().is_some_and(|gui| gui.has_keyboard_focus())
                }) =>
            {
                if let Some(keyboard_event) = keyutils::keyboard_event_from_winit(key_event) {
                    let webview = state.tabs.borrow().displayed().map(|t| t.webview.clone());
                    if let Some(webview) = webview {
                        webview.notify_input_event(InputEvent::Keyboard(keyboard_event));
                    }
                }
            },
            _ => {
                let response = GUI.with_borrow_mut(|gui| {
                    gui.as_mut().map(|gui| gui.on_window_event(&state.window, &event))
                });
                if response.is_some_and(|r| r.repaint) && !matches!(event, WindowEvent::RedrawRequested) {
                    state.window.request_redraw();
                }
            },
        }

        if repaint_now {
            let actions = GUI.with_borrow_mut(|gui| {
                gui.as_mut().map(|gui| gui.update(&state)).unwrap_or_default()
            });
            apply_ui_actions(&state, actions);
            GUI.with_borrow_mut(|gui| {
                if let Some(gui) = gui.as_mut() {
                    gui.paint(&state.window);
                }
            });
        }

        state.servo.spin_event_loop();
        set_wait(event_loop, &state);
    }
}

/// Sleep until the next queued capture deadline, or indefinitely if none.
fn set_wait(event_loop: &ActiveEventLoop, state: &Rc<Shared>) {
    let flow = match state.next_capture_deadline() {
        Some(deadline) => ControlFlow::WaitUntil(deadline),
        None => ControlFlow::Wait,
    };
    event_loop.set_control_flow(flow);
}

/// The intent a panel's keyboard shortcut emits: show `panel`, or go back to
/// the page when it is the one already showing.
///
/// Human-only surface, and deliberately so: every panel toggle is a keystroke
/// or a toolbar button, never a command that arrives over the control socket.
fn toggle_panel(panel: ChromePanel) -> UiAction {
    let current = GUI.with_borrow(|gui| {
        gui.as_ref().map_or(ChromePanel::None, |gui| gui.panel())
    });
    UiAction::SetPanel(match current == panel {
        true => ChromePanel::None,
        false => panel,
    })
}

/// Standard browser keyboard shortcuts, intercepted before both egui and the
/// page: Ctrl+L (focus URL bar), Ctrl+T (new tab), Ctrl+W (close tab),
/// Ctrl+R / F5 (reload), Alt+Left / Alt+Right (back / forward),
/// Ctrl+Tab / Ctrl+Shift+Tab (cycle tabs), Ctrl+K (credentials panel),
/// Ctrl+H (history panel), Ctrl+B (bookmarks panel), Ctrl+D (bookmark or
/// un-bookmark the displayed page).
/// Returns true when consumed.
fn handle_browser_shortcut(state: &Rc<Shared>, key_event: &winit::event::KeyEvent) -> bool {
    use winit::keyboard::{Key as WinitKey, NamedKey as WinitNamedKey};

    if key_event.state != ElementState::Pressed {
        return false;
    }
    let ctrl = state.modifiers.get().control_key();
    let alt = state.modifiers.get().alt_key();
    let action = match &key_event.logical_key {
        WinitKey::Character(c) if ctrl => match c.to_lowercase().as_str() {
            "l" => {
                GUI.with_borrow_mut(|gui| {
                    if let Some(gui) = gui.as_mut() {
                        gui.focus_location_bar();
                    }
                });
                state.window.request_redraw();
                return true;
            },
            "k" => Some(toggle_panel(ChromePanel::Credentials)),
            "h" => Some(toggle_panel(ChromePanel::History)),
            "b" => Some(toggle_panel(ChromePanel::Bookmarks)),
            "j" => Some(toggle_panel(ChromePanel::Downloads)),
            // Scoped to the displayed page, so with no tab open there is
            // nothing to bookmark and the key does nothing — the same shape
            // Ctrl+W uses for the tab it would have closed.
            "d" => state.tabs.borrow().displayed().map(|tab| {
                let url = tab
                    .webview
                    .url()
                    .map(|url| url.to_string())
                    .filter(|url| !url.is_empty())
                    .unwrap_or_else(|| tab.location.clone());
                let title = tab
                    .webview
                    .page_title()
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| url.clone());
                UiAction::ToggleBookmark(url, title)
            }),
            "t" => Some(UiAction::NewTab),
            "w" => state
                .tabs
                .borrow()
                .displayed()
                .map(|tab| UiAction::CloseTab(tab.id)),
            "r" => Some(UiAction::Reload),
            _ => None,
        },
        WinitKey::Named(WinitNamedKey::F5) => Some(UiAction::Reload),
        WinitKey::Named(WinitNamedKey::ArrowLeft) if alt => Some(UiAction::Back),
        WinitKey::Named(WinitNamedKey::ArrowRight) if alt => Some(UiAction::Forward),
        WinitKey::Named(WinitNamedKey::Tab) if ctrl => {
            let forward = !state.modifiers.get().shift_key();
            state.tabs.borrow_mut().cycle(forward);
            state.window.request_redraw();
            return true;
        },
        _ => None,
    };
    match action {
        Some(action) => {
            apply_ui_actions(state, vec![action]);
            state.window.request_redraw();
            true
        },
        None => false,
    }
}

/// How far one line of wheel scroll travels, in pixels.
///
/// Named once and read by **both** input paths — the local one converting
/// winit's `LineDelta` in [`forward_wheel`], and [`crate::remote_input`]
/// converting a wire wheel in [`talaria_protocol::wire::WheelMode::Line`]
/// units in `wheel_scale` — so a viewer's scroll covers the same distance the
/// human's does. `grep -rn WHEEL_LINE_PIXELS crates/` naming only this file is
/// how the 76× remote-scroll defect shipped, so the second reader is worth
/// checking for rather than assuming.
///
/// **The scaled value is still handed to the engine as `DeltaLine` and that is
/// not a mistake**: it is what Servo's own embedding example does
/// (`servo-0.4.0/examples/winit_minimal.rs:136-137`), so 76 units per notch is
/// the convention this engine is fed. Both paths therefore multiply *and* keep
/// the mode, because a path that did only one of the two would differ from the
/// other in a way no test of a single path could see.
pub(crate) const WHEEL_LINE_PIXELS: f32 = 76.0;

/// The rectangle a webview's own input coordinates live in: its size, with its
/// top-left as the origin.
///
/// Always the **given** webview's, never the displayed tab's. A containment
/// test run against a different tab's size is exactly how a remote click would
/// end up bounded by whatever page the local human happened to switch to.
fn viewport_of(webview: &WebView) -> euclid::Rect<f32, DevicePixel> {
    let size = webview.size();
    euclid::Rect::new(
        euclid::Point2D::zero(),
        euclid::Size2D::new(size.width, size.height),
    )
}

/// Deliver one pointer motion to `webview`.
///
/// `point` is **already relative to `webview`'s own viewport**; applying any
/// window-chrome offset in here would be applying it twice, because the one
/// caller that has a window subtracts it before calling. Returns whether the
/// point was inside the viewport and the event was delivered — a point outside
/// is a refusal and never a clamp to the nearest edge, since clamping turns
/// "aim at nothing" into "aim at the nearest thing".
pub(crate) fn deliver_mouse_move(
    webview: &WebView,
    point: euclid::Point2D<f32, DevicePixel>,
) -> bool {
    if !viewport_of(webview).contains(point) {
        return false;
    }
    webview.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(point.into())));
    true
}

/// Deliver one pointer button event to `webview`.
///
/// `point` is **already relative to `webview`'s own viewport**; applying any
/// window-chrome offset in here would be applying it twice. Returns whether it
/// was delivered.
pub(crate) fn deliver_mouse_button(
    webview: &WebView,
    point: euclid::Point2D<f32, DevicePixel>,
    button: ServoMouseButton,
    action: MouseButtonAction,
) -> bool {
    if !viewport_of(webview).contains(point) {
        return false;
    }
    webview.notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
        action,
        button,
        point.into(),
    )));
    true
}

/// Deliver one wheel event to `webview`.
///
/// `point` is **already relative to `webview`'s own viewport**; applying any
/// window-chrome offset in here would be applying it twice. Returns whether it
/// was delivered.
pub(crate) fn deliver_wheel(
    webview: &WebView,
    point: euclid::Point2D<f32, DevicePixel>,
    delta: WheelDelta,
) -> bool {
    if !viewport_of(webview).contains(point) {
        return false;
    }
    webview.notify_input_event(InputEvent::Wheel(WheelEvent::new(delta, point.into())));
    true
}

fn forward_mouse_move(state: &Shared, position: PhysicalPosition<f64>) {
    let webview = state.tabs.borrow().displayed().map(|t| t.webview.clone());
    let Some(webview) = webview else { return };

    let mut point = euclid::Point2D::<f32, DevicePixel>::new(position.x as f32, position.y as f32);
    // The toolbar subtraction lives at the **call site** rather than inside
    // `deliver_mouse_move`, and that placement is the input half of `D-05-01`.
    // A window coordinate has a chrome strip above the page; a remote viewer's
    // coordinate does not, because a client draws no server toolbar. Shared,
    // this line would put a silent toolbar-height error on every remote click:
    // nothing errors, links near the top of a page still work, and links below
    // a control look "flaky".
    point.y -= state.toolbar_height_device();

    let previous = state.webview_point.get();
    state.webview_point.set(point);

    // The left-viewport transition is the local path's alone: it needs a
    // previous position, and the remote path deliberately keeps none — every
    // wire pointer message carries its own coordinates instead.
    if !deliver_mouse_move(&webview, point) && viewport_of(&webview).contains(previous) {
        webview.notify_input_event(InputEvent::MouseLeftViewport(
            MouseLeftViewportEvent::default(),
        ));
    }
}

fn forward_mouse_button(
    state: &Shared,
    button: winit::event::MouseButton,
    element_state: ElementState,
) {
    let webview = state.tabs.borrow().displayed().map(|t| t.webview.clone());
    let Some(webview) = webview else { return };

    // Read back out of the **local** cursor cache, which is the mechanism by
    // which the toolbar subtraction above reaches this function and
    // `forward_wheel` without either of them naming it. Deliberately not
    // shared with any other input source — see the field's own doc comment.
    let point = state.webview_point.get();

    let mouse_button = match button {
        winit::event::MouseButton::Left => ServoMouseButton::Left,
        winit::event::MouseButton::Right => ServoMouseButton::Right,
        winit::event::MouseButton::Middle => ServoMouseButton::Middle,
        winit::event::MouseButton::Back => ServoMouseButton::Back,
        winit::event::MouseButton::Forward => ServoMouseButton::Forward,
        winit::event::MouseButton::Other(value) => ServoMouseButton::Other(value),
    };
    let action = match element_state {
        ElementState::Pressed => MouseButtonAction::Down,
        ElementState::Released => MouseButtonAction::Up,
    };
    deliver_mouse_button(&webview, point, mouse_button, action);
}

fn forward_wheel(state: &Shared, delta: MouseScrollDelta) {
    let webview = state.tabs.borrow().displayed().map(|t| t.webview.clone());
    let Some(webview) = webview else { return };

    let (dx, dy, mode) = match delta {
        MouseScrollDelta::LineDelta(x, y) => (
            (x * WHEEL_LINE_PIXELS) as f64,
            (y * WHEEL_LINE_PIXELS) as f64,
            WheelMode::DeltaLine,
        ),
        MouseScrollDelta::PixelDelta(delta) => (delta.x, delta.y, WheelMode::DeltaPixel),
    };
    // The local cursor cache again, for the same reason it is read above.
    let point = state.webview_point.get();
    deliver_wheel(&webview, point, WheelDelta { x: dx, y: dy, z: 0.0, mode });
}

/// Start the remote MCP listener, and record that it is starting.
///
/// A no-op when one is already bound or on its way to being bound, so a second
/// request while a bind is in flight cannot produce a second listener
/// (T-04-03-03). The state goes to `Starting` here rather than to `Bound`:
/// only the listener's own event may claim an address, because only the
/// listener knows one.
fn start_remote_listener(state: &Rc<Shared>, remote: &crate::settings::RemoteAccessConfig) {
    if state.remote.borrow().is_live() {
        return;
    }
    // Cloned out here, before the spawn: `state` is an `Rc<Shared>` and `Rc`
    // is not `Send`, so the closure inside `http::spawn` can never borrow it.
    // The proxy is the whole of what crosses the thread boundary.
    let proxy = state.event_proxy.clone();
    // The store handle is cloned, not the store: the `Arc` is what makes the
    // listener's verifier and the chrome's revoke one store rather than two.
    let agents = Arc::clone(&state.agents);
    *state.remote.borrow_mut() = RemoteAccess::Starting;
    // The registry comes back empty and fills itself in when the bind lands —
    // it is the listener that knows what state a revoke has to reach, and it
    // does not know until it has an address.
    // The advertised origin travels with the port because both are facts the
    // listener needs before it binds, and both come from configuration — never
    // from a request. See `settings::RemoteAccessConfig::advertised_url`.
    let (shutdown, streams) =
        crate::http::spawn(proxy, agents, remote.port, remote.advertised_url.clone());
    *state.remote_shutdown.borrow_mut() = Some(shutdown);
    *state.remote_streams.borrow_mut() = streams;
    state.window.request_redraw();
}

/// Answer the parked consent request and close the panel.
///
/// `id` is checked against the parked request rather than trusted: the two can
/// only disagree if a decision was applied a frame after the request it named
/// was replaced, and answering the *wrong* request in that window would be a
/// human approving one agent and granting another. A mismatch drops the taken
/// request, which closes its channel, which the HTTP side reads as a refusal —
/// the degrade direction is deny.
///
/// The panel closes through `set_panel(ChromePanel::None)`, which is allowed:
/// only the `Consent` variant is refused there, and only as a *destination*.
/// It closes to no panel rather than restoring whatever was open before,
/// because panel state is a single enum and a restore stack for one screen
/// would be new machinery for no benefit.
fn resolve_consent(state: &Rc<Shared>, id: u64, decision: crate::oauth::ConsentDecision) {
    match state.pending_consent.borrow_mut().take() {
        Some(request) if request.id == id => request.resolve(decision),
        Some(request) => log::warn!(
            "a consent decision named request {id} but {} was parked; refusing both",
            request.id
        ),
        None => log::warn!("a consent decision arrived for request {id}, which is not parked"),
    }
    GUI.with_borrow_mut(|gui| {
        if let Some(gui) = gui.as_mut() {
            gui.set_panel(ChromePanel::None);
        }
    });
    state.window.request_redraw();
}

fn apply_ui_actions(state: &Rc<Shared>, actions: Vec<UiAction>) {
    for action in actions {
        match action {
            UiAction::SwitchMode(mode) => {
                let mut tabs = state.tabs.borrow_mut();
                tabs.mode = mode;
                tabs.sync_visibility();
                drop(tabs);
                state.window.request_redraw();
            },
            UiAction::Go(input) => {
                // Cloned rather than held as a `Ref`: the arm below takes a
                // mutable borrow of `tabs`, and a live immutable borrow of
                // `settings` across it is a borrow-checker fight for nothing.
                // A `SearchEngine` is two short strings.
                let engine = state.settings.borrow().search_engine.clone();
                let url = resolve_location(&input, &engine);
                let mut tabs = state.tabs.borrow_mut();
                if let Some(tab) = tabs.displayed_mut() {
                    tab.location = url.to_string();
                    tab.location_dirty = false;
                    tab.webview.load(url);
                } else {
                    drop(tabs);
                    state.open_tab(url, TabOwner::Me);
                }
            },
            UiAction::NewTab => {
                let url = Url::parse("https://servo.org").expect("static url");
                state.open_tab(url, TabOwner::Me);
            },
            UiAction::CloseTab(id) => {
                // Owner before close: the close removes the tab, so an event
                // raised afterwards has nothing left to key its addressee on.
                let owner = state.tabs.borrow().get(id).map(|tab| tab.owner.clone());
                let closed = state.tabs.borrow_mut().close(id);
                if let (true, Some(owner)) = (closed, owner) {
                    state.queue_event(&owner, talaria_protocol::Event::TabClosed { tab_id: id });
                }
            },
            UiAction::SelectTab(id) => {
                state.tabs.borrow_mut().set_active(id);
            },
            UiAction::Back => {
                if let Some(tab) = state.tabs.borrow().displayed() {
                    if tab.webview.can_go_back() {
                        tab.webview.go_back(1);
                    }
                }
            },
            UiAction::Forward => {
                if let Some(tab) = state.tabs.borrow().displayed() {
                    if tab.webview.can_go_forward() {
                        tab.webview.go_forward(1);
                    }
                }
            },
            UiAction::Reload => {
                if let Some(tab) = state.tabs.borrow().displayed() {
                    tab.webview.reload();
                }
            },
            UiAction::ReloadCrashed(id) => {
                if let Some(tab) = state.tabs.borrow_mut().get_mut(id) {
                    tab.crashed = false;
                    tab.webview.reload();
                }
            },
            UiAction::SaveCredential(entry) => {
                let mut vault = state.vault.borrow_mut();
                vault.upsert(entry);
                drop(vault);
                state.window.request_redraw();
            },
            UiAction::DeleteCredential { host, username } => {
                let mut vault = state.vault.borrow_mut();
                let removed = vault.delete(&host, &username);
                drop(vault);
                if !removed {
                    // Not fatal, but the user just pressed a delete and
                    // nothing went — say so rather than looking like it did.
                    log::warn!("no stored credential for {username} at {host}");
                }
                state.window.request_redraw();
            },
            // The refusal. Adding `Consent` to `ChromePanel` is what made
            // `SetPanel(Consent)` *representable*, and "no control constructs
            // it" would otherwise be an invariant every future panel button
            // had to maintain by discipline — one toolbar button written
            // without thinking would open the screen whose primary control
            // grants an agent the human's logged-in session. With the refusal
            // here and the matching one in `Gui::set_panel`, "no control can
            // open the Approve button" is a property a reviewer confirms in
            // two functions rather than by auditing every call site that will
            // ever exist. Dropped with a warning because it can only mean a
            // bug: the consent panel is raised by `Gui::raise_consent` from
            // `user_event`, and by nothing else.
            UiAction::SetPanel(ChromePanel::Consent) => {
                log::warn!("refusing a set-panel action naming the consent panel");
            },
            UiAction::SetPanel(panel) => {
                GUI.with_borrow_mut(|gui| {
                    if let Some(gui) = gui.as_mut() {
                        gui.set_panel(panel);
                    }
                });
                state.window.request_redraw();
            },
            // The two arms that answer a human. Each takes the parked request
            // — there is at most one — and sends the decision down the channel
            // the HTTP thread is waiting on. The *code* is minted over there,
            // after the decision arrives, which is why neither of these arms
            // touches a token, a code or a verifier.
            UiAction::ApproveConsent(id) => {
                resolve_consent(state, id, crate::oauth::ConsentDecision::Approve);
            },
            UiAction::DenyConsent(id) => {
                resolve_consent(state, id, crate::oauth::ConsentDecision::Deny);
            },
            UiAction::ClearHistory => {
                state.history.borrow_mut().clear();
                state.window.request_redraw();
            },
            UiAction::ToggleBookmark(url, title) => {
                // The whole of the toggle lives here, against the store the
                // star drew from — never inside the egui closure that raised
                // the intent. `Bookmarks::upsert` deliberately refuses to be
                // the toggle itself: an add that silently overwrote would
                // rewrite the title of a bookmark the user meant to remove.
                let mut bookmarks = state.bookmarks.borrow_mut();
                match bookmarks.is_bookmarked(&url) {
                    true => {
                        bookmarks.remove(&url);
                    },
                    false => bookmarks.upsert(url, title, now_ms()),
                }
                drop(bookmarks);
                state.window.request_redraw();
            },
            UiAction::RemoveBookmark(url) => {
                let removed = state.bookmarks.borrow_mut().remove(&url);
                if !removed {
                    // Not fatal, but the user just pressed a remove and
                    // nothing went — say so rather than looking like it did.
                    log::warn!("no bookmark for {url}");
                }
                state.window.request_redraw();
            },
            UiAction::SaveSearchEngine(engine) => {
                // The panel's Save button is disabled until the template is
                // valid, so nothing is re-checked here; `Settings::save`
                // updates the running session whether or not the write lands.
                log::info!("search engine set to {}", engine.name);
                state.settings.borrow_mut().save(engine);
                state.window.request_redraw();
            },
            // The one action in this function that starts an external
            // process, and the only place in the codebase that does.
            //
            // T-03-04-01/T-03-04-02: `path` arrived here from the Downloads
            // panel, which cloned it out of `DownloadEntry::path`, which was
            // written by `AppEvent::DownloadCompleted`, which carried
            // `create_unique`'s returned path — the file this shell actually
            // wrote. No step in that chain re-derives it from the requested
            // filename, and no step in it is reachable from the control
            // socket: the whole chain is entered only by a human pressing a
            // button in a human-only panel.
            UiAction::OpenDownload(path) => {
                match std::process::Command::new("xdg-open").arg(&path).spawn() {
                    // Deliberately not waited on. The handler is a whole
                    // application (a PDF viewer, a file manager) whose
                    // lifetime is nothing to do with this event loop, and
                    // blocking here would freeze the browser behind it.
                    Ok(_) => {},
                    Err(error) => {
                        // The name to apologise with, taken from the path we
                        // tried rather than from the entry's requested
                        // filename — this reports what was actually attempted.
                        let filename = std::path::Path::new(&path)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.clone());
                        log::warn!("could not open {path}: {error}");
                        GUI.with_borrow_mut(|gui| {
                            if let Some(gui) = gui.as_mut() {
                                gui.set_download_open_error(filename, error.to_string());
                            }
                        });
                    },
                }
                state.window.request_redraw();
            },
            UiAction::RemoveDownload(path) => {
                // A list row, never the file: `Downloads::remove` touches
                // nothing on disk, which is what the row's hover text
                // promises.
                let removed = state.downloads.borrow_mut().remove(&path);
                if !removed {
                    // Not fatal, but the user just pressed a remove and
                    // nothing went — say so rather than looking like it did.
                    log::warn!("no downloads entry for {path}");
                }
                state.window.request_redraw();
            },
            UiAction::DismissDownloadError => {
                GUI.with_borrow_mut(|gui| {
                    if let Some(gui) = gui.as_mut() {
                        gui.clear_download_open_error();
                    }
                });
                state.window.request_redraw();
            },
            // The one action that opens or closes a network port. Reached
            // from the Access panel's button and from nowhere else — see
            // `UiAction::SetRemoteAccess` for why that is a security property
            // rather than a layout decision.
            UiAction::SetRemoteAccess(enabled) => {
                let mut configured = state.settings.borrow().remote_access.clone();
                configured.enabled = enabled;
                // Persisted first, in both directions, so the choice survives
                // a restart. The write applies to this session whether or not
                // it reaches disk, which is what makes the off direction below
                // reliable even on a read-only config directory.
                state.settings.borrow_mut().save_remote_access(configured.clone());
                match enabled {
                    // `start_remote_listener` is a no-op while one is already
                    // bound or starting, so a second click during a bind in
                    // flight cannot produce a second listener (T-04-03-03).
                    true => start_remote_listener(state, &configured),
                    false => {
                        // Taken, not cloned: the handle is consumed, so the
                        // same listener cannot be shut down twice. In-flight
                        // requests are resolved rather than stranded — each
                        // awaiting sink holds a `oneshot` receiver, and the
                        // runtime going away closes them, so every pending
                        // request comes back to its caller as an error. That
                        // is the cancellation mechanism 02-04 established,
                        // reused here rather than reinvented.
                        if let Some(handle) = state.remote_shutdown.borrow_mut().take() {
                            handle.shutdown();
                        }
                        // Off immediately, without waiting for the thread to
                        // finish draining: the human asked for it, and a
                        // status block that kept saying "on" for three seconds
                        // afterwards would be the wrong kind of honest.
                        *state.remote.borrow_mut() = RemoteAccess::Off;
                        state.window.request_redraw();
                    },
                }
            },
            // The other action that changes who may drive this browser, and
            // the only one that takes access away. Reached from one Access
            // panel row's second, confirming click and from nowhere else —
            // see `UiAction::RevokeClient` for why that is a security property
            // rather than a layout decision.
            UiAction::RevokeClient(client_id) => {
                // **Both halves, in one arm, because either alone is a lie.**
                // The store mutation is what refuses the client's *next*
                // request: verification is a live read of this same store, so
                // a record dropped here is not there to be found a microsecond
                // later. It does nothing whatever to a response stream the
                // client already holds open — that stream has no next request
                // to fail — so the registry's termination is the other half.
                // A revoke that removed the row and left a stream delivering
                // would be exactly the false completion this is written to
                // prevent (T-7).
                let changed = match state.agents.lock() {
                    Ok(mut store) => store.revoke_client(&client_id),
                    Err(_) => {
                        // Degrade, never abort — and note that this degrades
                        // toward *not* revoking, which is the one direction
                        // here that is not safe. It is also unreachable
                        // outside a panic inside the store, and the honest
                        // answer is to say so rather than to invent a
                        // recovery: a poisoned store already refuses every
                        // credential (`TalariaAuth::verify`), so the client
                        // has lost access anyway, by a worse route.
                        log::error!(
                            "agent store lock is poisoned; {client_id} was not revoked"
                        );
                        false
                    },
                };
                // Attempted whichever way the store went. A stream can only
                // exist for a client that had a token, and if the store
                // somehow disagrees, closing the stream is still the answer.
                state.remote_streams.borrow().terminate_client(&client_id);
                match changed {
                    true => log::info!("revoked agent client {client_id}"),
                    // Not an error: revoking something already gone is a
                    // no-op the store reports rather than refuses.
                    false => log::debug!("nothing to revoke for agent client {client_id}"),
                }
                // What deliberately does **not** happen, so nobody reads the
                // absence as an oversight: the client's existing tabs stay
                // open. They are visible in the Agents view and the human can
                // take any of them over, and closing a person's tabs because a
                // credential was withdrawn would destroy state they may want.
                // The agent cannot drive them any more, which is the whole of
                // what a revoke is for.
                //
                // And the honest boundary on timing: a request that passed
                // verification a moment before this ran will finish. That is
                // correct — verification is per request, and a command already
                // executing against the engine is not interruptible. What is
                // guaranteed is the next request and the open stream.
                state.window.request_redraw();
            },
            // The recovery half of CR-04. Not a revoke: it takes only
            // registrations no human ever approved, so it cannot cost anybody
            // access — which is why it needs no confirming click and why the
            // panel offers it plainly.
            UiAction::ForgetUnapprovedClients => {
                let forgotten = match state.agents.lock() {
                    Ok(mut store) => store.forget_unapproved(),
                    Err(_) => {
                        // Degrade, never abort. This one degrades toward *not*
                        // forgetting, which is the safe direction here: a
                        // registration that stays is inert, and a poisoned
                        // store already refuses every credential.
                        log::error!(
                            "agent store lock is poisoned; no registration was forgotten"
                        );
                        0
                    },
                };
                match forgotten {
                    0 => log::debug!("no unapproved registrations to forget"),
                    count => log::info!("forgot {count} unapproved registration(s)"),
                }
                // A parked consent request for one of them is deliberately not
                // cancelled here: it resolves as it always would, and the
                // approval then finds no registration and refuses the grant.
                // The failure lands on a refused grant, never a granted one.
                state.window.request_redraw();
            },
            UiAction::DismissVaultNotice => {
                let mut vault = state.vault.borrow_mut();
                vault.clear_notices();
                drop(vault);
                state.window.request_redraw();
            },
        }
    }
}

/// Which URLs an agent may hand to `tabs_open` / `navigate`.
///
/// `http` and `https` are the web, which is what an agent is here to drive.
/// `data` is admitted because it grants an agent nothing it cannot already
/// produce through `evaluate` on a page it already controls — refusing it
/// would remove capability without removing risk.
///
/// The blank page is admitted by an exact match on the whole URL string
/// rather than by scheme, because popup adoption registers and compares that
/// exact literal (`register(..., "about:blank")`, `notify_url_changed`), and
/// admitting the whole `about` scheme would widen the surface to the engine's
/// internal pages.
///
/// Everything else is refused: `file` (the user's disk, readable straight back
/// out through `evaluate`), the `javascript` pseudo-scheme (script in a
/// document the agent holds no evaluate handle on), `blob` object URLs, and any
/// custom scheme Servo may register. The human's own URL bar is deliberately
/// unaffected — see `resolve_location`.
fn agent_scheme_allowed(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https" | "data") || url.as_str() == "about:blank"
}

/// Whether `url` names the address the remote MCP listener is bound to.
///
/// Host and port, compared against the string the listener itself reported.
/// `localhost` is folded in when the bound host is loopback, because it
/// resolves there and an agent typing it would otherwise walk straight past a
/// refusal that only knew one spelling of the same machine.
fn is_listener_origin(url: &Url, bound: &str) -> bool {
    let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) else {
        return false;
    };
    if format!("{host}:{port}").eq_ignore_ascii_case(bound) {
        return true;
    }
    let loopback_alias = host.eq_ignore_ascii_case("localhost")
        && bound.starts_with(&format!("{}:", crate::settings::LOOPBACK_BIND));
    loopback_alias && bound.ends_with(&format!(":{port}"))
}

/// Agent-supplied URLs: accept scheme-less hosts ("example.com") by assuming
/// https, but never fall back to a search query — an agent that meant to
/// search should do so explicitly. Anything that parses is then held to
/// `agent_scheme_allowed`; the refusal names the rejected scheme so the agent
/// can tell policy apart from a typo.
///
/// `bound_origin` carries the `host:port` the remote MCP listener is bound to,
/// or nothing when nothing is bound, and a URL matching it is refused (T-3).
/// An agent that can `evaluate` in a tab it owns and can point that tab at
/// Talaria's own endpoints can at minimum enumerate authorization metadata and
/// self-register, and from 04-06 could try to drive its own consent screen
/// from inside a page. There is no legitimate reason for an agent to browse
/// the transport it is already speaking.
///
/// **This is the command half of that refusal and not the whole of it.** It
/// guards `Command::TabsOpen` and `Command::Navigate` — the two places an
/// agent hands this browser a URL — and it says nothing about a load a *page*
/// starts, which is one `evaluate` away. `Shared::request_navigation` is where
/// the invariant is actually enforced, on every navigation however it began;
/// this stays because it refuses earlier and with a message an agent can read,
/// not because it is the boundary (WR-03).
///
/// Keyed on what is **actually bound** rather than on the configured port, so
/// it invents no rule while the listener is off: with remote access disabled
/// there is no origin to protect and `http://127.0.0.1:8779/` is an ordinary
/// address like any other. The refusal names the reason, in the same style as
/// the scheme refusal above it and `validate_download_filename` below, so an
/// agent can tell policy from a typo.
fn parse_agent_url(input: &str, bound_origin: Option<&str>) -> Result<Url, String> {
    let parsed = match Url::parse(input) {
        Ok(url) => Ok(url),
        Err(url::ParseError::RelativeUrlWithoutBase) if !input.contains(' ') => {
            Url::parse(&format!("https://{input}"))
        },
        Err(error) => Err(error),
    };
    match parsed {
        Ok(url) if !agent_scheme_allowed(&url) => Err(format!(
            "scheme {} is not allowed for agents — use http, https, data:, or about:blank",
            url.scheme()
        )),
        Ok(url) if bound_origin.is_some_and(|bound| is_listener_origin(&url, bound)) => {
            Err("that address is Talaria's own remote-access listener — agents may not \
                 browse the transport they are speaking"
                .to_owned())
        },
        Ok(url) => Ok(url),
        Err(error) => Err(format!("bad url: {error}")),
    }
}

/// The name a `download` command may write its file under.
///
/// Two separate jobs, which is why the refusals are worded separately. `/`
/// and `..` keep the write inside the downloads directory — that half is
/// unchanged and its message is unchanged with it. The character classes
/// [`crate::gui::is_display_unsafe`] names are about something else
/// entirely: the requested name is also the row's entire visible label in
/// the Downloads panel, and a newline makes one row render as two while a
/// bidi override makes `report.fdp.exe` render as `report.exe.pdf`. Neither
/// is a path problem, so neither was caught by the two checks above, and a
/// label an agent can make lie sits directly above a button that hands the
/// file to the OS.
///
/// Refused here, at the boundary, so a name that could do it never reaches
/// the store. The panel sanitizes as well, for rows a build without this
/// check already wrote.
fn validate_download_filename(filename: &str) -> Result<(), String> {
    if filename.contains('/') || filename.contains("..") {
        // Verbatim: this is the refusal `download_bounds_test.py` pins.
        return Err("bad filename".into());
    }
    match filename.chars().find(|character| crate::gui::is_display_unsafe(*character)) {
        Some(character) => Err(format!(
            "bad filename: U+{:04X} would let the name misrepresent itself in the \
             downloads list",
            character as u32
        )),
        None => Ok(()),
    }
}

/// Omnibox behavior: URL if it parses (or looks like a host), search query
/// otherwise.
///
/// `about:` and `file:` are named explicitly because neither carries a host,
/// so the host test alone would send both to the search engine. The human is
/// the trust root here: unlike `parse_agent_url`, this path allowlists
/// nothing — a user who types a local path gets the local file.
///
/// The search engine is a parameter rather than a hardcoded literal as of
/// BROWSE-03; it affects only the final fallback, never any of the
/// URL-detection branches above it. Passing [`SearchEngine::default`] gives
/// exactly the behaviour this function had before that parameter existed.
pub fn resolve_location(input: &str, engine: &SearchEngine) -> Url {
    let input = input.trim();
    if let Ok(url) = Url::parse(input) {
        if !url.scheme().is_empty() && url.host().is_some()
            || url.scheme() == "about"
            || url.scheme() == "file"
        {
            return url;
        }
    }
    if !input.contains(' ') && input.contains('.') {
        if let Ok(url) = Url::parse(&format!("https://{input}")) {
            return url;
        }
    }
    let query: String =
        url::form_urlencoded::byte_serialize(input.as_bytes()).collect();
    // `replacen(.., 1)` substitutes the first placeholder only, which is why
    // `settings::is_valid_template` insists on exactly one. The fallback
    // below is the second layer of that same defense: a template that never
    // passed through `Settings::load`'s validation — built in code, or
    // hand-edited into a file this process already had open — must degrade to
    // a working search rather than take the navigation down with it.
    let resolved = engine.url_template.replacen("{query}", &query, 1);
    Url::parse(&resolved).unwrap_or_else(|_| {
        log::warn!("search template {:?} does not parse; searching DuckDuckGo", engine.url_template);
        Url::parse(&format!("https://duckduckgo.com/?q={query}")).expect("static url")
    })
}


/// Whether one completed load belongs in the human's browsing history — and,
/// in the same step, the taking of the provenance flag that decided it.
///
/// Owning the tab is necessary but not sufficient. An agent's `evaluate` can
/// navigate one of the human's own tabs by assigning `location.href`, and the
/// page that lands is not a page the human went to; filtering on the owner
/// alone let exactly that row through, indistinguishable from a real visit.
///
/// The flag is cleared here rather than at the next navigation so that a
/// human-initiated load on the same tab afterwards *is* recorded. That makes
/// the failure direction "miss a row" — never "forge one", which is the
/// direction that matters for a record a person reasons about.
fn belongs_in_history(owner: &TabOwner, load_started_by_agent: &mut bool) -> bool {
    let started_by_agent = std::mem::replace(load_started_by_agent, false);
    matches!(owner, TabOwner::Me) && !started_by_agent
}

/// Agent-facing snapshot of one tab.
///
/// `pub(crate)` so [`crate::view`]'s snapshot builds the *same* shape an agent
/// gets over the control socket rather than a parallel one — a second
/// spelling of "what a tab looks like" is a second thing to keep in step.
pub(crate) fn tab_info(tabs: &TabManager, tab: &crate::tabs::Tab) -> TabInfo {
    let focused = tabs.active_id(ViewMode::Me) == Some(tab.id)
        || tabs.active_id(ViewMode::Agents) == Some(tab.id);
    TabInfo {
        tab_id: tab.id,
        url: tab.webview.url().map(|u| u.to_string()).unwrap_or_else(|| tab.location.clone()),
        title: tab.webview.page_title().unwrap_or_default(),
        owner: tab.owner.label(),
        focused,
        crashed: tab.crashed,
        // An adopted popup's blank starting document reports Complete before
        // the navigation its opener asked for begins; keep it "loading" over
        // that gap so agents don't script the document about to be replaced.
        loading: tab.webview.load_status() != servo::LoadStatus::Complete
            || tab.initial_blank_until.is_some_and(|until| std::time::Instant::now() < until),
    }
}

fn execute_agent_command(state: &Rc<Shared>, request: AgentRequest) {
    let AgentRequest { session_id, client, command, reply } = request;

    let webview_for = |tab_id: u64| -> Result<WebView, Outcome> {
        state
            .tabs
            .borrow()
            .get(tab_id)
            .map(|t| t.webview.clone())
            .ok_or(Outcome::Error { message: format!("no tab {tab_id}") })
    };

    // Crashed tabs reject page-level commands with a tool error (per SPEC's
    // crash-recovery decision); `navigate` recovers the tab instead.
    let crashed = |tab_id: u64| -> bool {
        state.tabs.borrow().get(tab_id).is_some_and(|t| t.crashed)
    };

    // For `evaluate` ONLY. A page that wedges its own script thread is exactly
    // when the human needs to get back into the tab, so screenshot, tabs_close,
    // tabs_focus, tabs_list, navigate, cookies_read, open_for_user and download
    // must keep answering on a busy tab — that is the product's reactive
    // backstop, not an oversight. Do not generalise this across the dispatch.
    let busy = |tab_id: u64| -> bool { state.tab_evaluating(tab_id) };

    // Read once, as an owned string, rather than holding a `Ref` across the
    // whole dispatch: the arms below take borrows of their own and a live
    // immutable borrow of `remote` across them is a borrow-checker fight for
    // nothing. `None` while nothing is bound, which is what keeps the refusal
    // in `parse_agent_url` from inventing a rule the listener does not have.
    let listener_origin: Option<String> =
        state.remote.borrow().bound_addr().map(str::to_owned);

    match command {
        Command::TabsList => {
            let tabs = state.tabs.borrow();
            let infos: Vec<TabInfo> = tabs.iter().map(|t| tab_info(&tabs, t)).collect();
            let _ = reply.send(Outcome::Ok { result: ResultPayload::Tabs { tabs: infos } });
        },
        Command::TabsOpen { url } => {
            match parse_agent_url(&url, listener_origin.as_deref()) {
                Ok(url) => {
                    let id = state.open_tab(url, TabOwner::Agent { session_id, client });
                    // A fresh webview has no document yet: reply once the
                    // page has loaded so the agent's next call has something
                    // to act on.
                    state.reply_after_load(id, false, reply);
                },
                Err(message) => {
                    let _ = reply.send(Outcome::Error { message });
                },
            }
        },
        Command::TabsClose { tab_id } => {
            // Owner before close, for the same reason as the UI close path.
            // Note this is the tab's owner, not the requesting session: an
            // agent may close a tab it does not own, and the notification
            // still belongs to whoever owned it.
            let owner = state.tabs.borrow().get(tab_id).map(|tab| tab.owner.clone());
            let closed = state.tabs.borrow_mut().close(tab_id);
            if let (true, Some(owner)) = (closed, owner) {
                state.queue_event(&owner, talaria_protocol::Event::TabClosed { tab_id });
            }
            let _ = reply.send(if closed {
                Outcome::Ok { result: ResultPayload::Empty {} }
            } else {
                Outcome::Error { message: format!("no tab {tab_id}") }
            });
        },
        Command::TabsFocus { tab_id } => {
            let exists = state.tabs.borrow().get(tab_id).is_some();
            if exists {
                state.tabs.borrow_mut().set_active(tab_id);
                let _ = reply.send(Outcome::Ok { result: ResultPayload::Empty {} });
            } else {
                let _ = reply.send(Outcome::Error { message: format!("no tab {tab_id}") });
            }
        },
        Command::Navigate { tab_id, url } => {
            // Refused before the URL is even looked at, because this is the
            // boundary rather than a detail of it: an agent may drive the
            // tabs it opened, and navigating one of the human's own tabs out
            // from under them is a hijack, not a feature. `tabs_open` gives
            // an agent a tab it owns, which is the whole of its legitimate
            // need. Deliberately narrower than `Command::TabsClose`, which
            // permits acting on a tab the requester does not own and says so
            // in its own comment — that decision stands and this one does not
            // touch it.
            let is_the_humans_tab = state
                .tabs
                .borrow()
                .get(tab_id)
                .is_some_and(|tab| matches!(tab.owner, TabOwner::Me));
            if is_the_humans_tab {
                let _ = reply.send(Outcome::Error {
                    message: format!(
                        "tab {tab_id} is one of the human's own — agents may only \
                         navigate tabs they opened with tabs_open"
                    ),
                });
                return;
            }
            match (webview_for(tab_id), parse_agent_url(&url, listener_origin.as_deref())) {
                (Ok(webview), Ok(url)) => {
                    if let Some(tab) = state.tabs.borrow_mut().get_mut(tab_id) {
                        tab.location = url.to_string();
                        tab.location_dirty = false;
                        tab.crashed = false;
                    }
                    let needs_start = webview.load_status() == servo::LoadStatus::Complete;
                    webview.load(url);
                    state.reply_after_load(tab_id, needs_start, reply);
                },
                (Err(outcome), _) => {
                    let _ = reply.send(outcome);
                },
                (_, Err(message)) => {
                    let _ = reply.send(Outcome::Error { message });
                },
            }
        },
        Command::Evaluate { tab_id, script } => {
            // Test hook: lets e2e tests exercise the crash-recovery path
            // without needing a real WebContent crash.
            if script == "__talaria_sim_crash__"
                && std::env::var("TALARIA_TEST_HOOKS").as_deref() == Ok("1")
            {
                // Read the owner from the tab it marks, exactly as the real
                // notify_crashed does: the hook is only useful to the suite
                // if it exercises the same addressed path.
                let marked = state.tabs.borrow_mut().get_mut(tab_id).map(|tab| {
                    tab.crashed = true;
                    tab.owner.clone()
                });
                match marked {
                    Some(owner) => {
                        state.window.request_redraw();
                        state.queue_event(&owner, talaria_protocol::Event::TabCrashed { tab_id });
                        let _ = reply.send(Outcome::Ok { result: ResultPayload::Empty {} });
                    },
                    None => {
                        let _ = reply.send(Outcome::Error { message: format!("no tab {tab_id}") });
                    },
                }
                return;
            }
            if crashed(tab_id) {
                let _ = reply.send(Outcome::Error {
                    message: format!("tab {tab_id} crashed — navigate it to recover"),
                });
                return;
            }
            // Fail fast in front of the command timeout, never instead of it:
            // the evaluate already in flight still runs until that outer bound.
            // The wording says a previous evaluate is still running so the
            // agent and the user read this as a wedged page, not a flaky tool.
            if busy(tab_id) {
                let _ = reply.send(Outcome::Error {
                    message: format!("tab {tab_id} busy — a previous evaluate is still running"),
                });
                return;
            }
            match webview_for(tab_id) {
                Ok(webview) => {
                    // Provenance, not permission — the script is allowed to
                    // run, including on one of the human's own tabs, because
                    // takeover works in both directions. What it may not do
                    // is have `location.href = ...` filed as a page the human
                    // visited, so the load it may be about to start is marked
                    // here and the history drain reads that mark. This is the
                    // live case: `Command::Navigate` above refuses a Me tab
                    // outright, and every other agent command is incapable of
                    // navigating one.
                    if let Some(tab) = state.tabs.borrow_mut().get_mut(tab_id) {
                        tab.load_started_by_agent = true;
                    }
                    // Only after the lookup succeeds: a request naming a tab
                    // that does not exist must leave no entry behind.
                    let done = state.begin_evaluating(tab_id);
                    start_evaluate(state, webview, script, reply, done);
                },
                Err(outcome) => {
                    let _ = reply.send(outcome);
                },
            }
        },
        Command::Screenshot { tab_id } => {
            if crashed(tab_id) {
                let _ = reply.send(Outcome::Error {
                    message: format!("tab {tab_id} crashed — navigate it to recover"),
                });
                return;
            }
            match webview_for(tab_id) {
                Ok(webview) => {
                    // Each tab has its own framebuffer. The displayed tab can
                    // be captured immediately; a background tab is briefly
                    // shown (into its OWN framebuffer — the displayed tab is
                    // untouched, so nothing flickers) and captured once it
                    // paints a fresh frame, or at the timeout for pages that
                    // never produce one.
                    let tabs = state.tabs.borrow();
                    let context = tabs
                        .get(tab_id)
                        .expect("checked by webview_for")
                        .rendering_context
                        .clone();
                    let is_displayed = tabs.displayed().is_some_and(|t| t.webview == webview);
                    drop(tabs);
                    if is_displayed {
                        let outcome = state.capture_now(&webview, &context, false);
                        let _ = reply.send(outcome);
                    } else {
                        webview.show();
                        state.pending_captures.borrow_mut().push(PendingCapture {
                            webview,
                            context,
                            reply,
                            ready: false,
                            deadline: std::time::Instant::now()
                                + std::time::Duration::from_millis(1500),
                        });
                        state.window.request_redraw();
                    }
                },
                Err(outcome) => {
                    let _ = reply.send(outcome);
                },
            }
        },
        Command::CookiesRead { domain } => {
            let entries = state.vault.borrow().matching(&domain);
            let _ = reply.send(Outcome::Ok { result: ResultPayload::Credentials { entries } });
        },
        Command::OpenForUser { url } => {
            // `OpenForUser` is the single-instance forward — a second launch
            // handing this one the URL a *human* typed on a command line, so
            // it resolves through the human path with the human's own engine.
            let engine = state.settings.borrow().search_engine.clone();
            let url = resolve_location(&url, &engine);
            let id = state.open_tab(url, TabOwner::Me);
            {
                let mut tabs = state.tabs.borrow_mut();
                tabs.mode = ViewMode::Me;
                tabs.sync_visibility();
            }
            state.window.focus_window();
            state.window.request_user_attention(Some(winit::window::UserAttentionType::Informational));
            state.window.request_redraw();
            state.reply_after_load(id, false, reply);
        },
        Command::Download { url, filename } => {
            if let Err(message) = validate_download_filename(&filename) {
                let _ = reply.send(Outcome::Error { message });
                return;
            }
            // Cloned out here, before the spawn: `state` is an `Rc<Shared>`
            // and `Rc` is not `Send`, so the closure below can never borrow
            // it. The proxy is the whole of what crosses the thread boundary.
            let proxy = state.event_proxy.clone();
            std::thread::spawn(move || {
                // `reply` doubles as the cancellation handle: `download` polls
                // its closed state, which becomes true the moment the control
                // socket's command timeout drops the receiving half.
                let outcome = download(&url, &filename, &reply);
                // Matched by reference so `outcome` is still intact for the
                // reply below. Gated on `Outcome::Ok` specifically: a download
                // that was capped, cancelled, timed out or failed to connect
                // took an `Outcome::Error` path, left nothing at the
                // destination, and must produce no list row claiming it did.
                //
                // `path` here is `create_unique`'s returned path — the file
                // that was actually written, which is not `filename` whenever
                // a name collided. Nothing downstream re-derives it.
                if let Outcome::Ok { result: ResultPayload::Download { path, bytes } } = &outcome {
                    // A closed receiver means the event loop is already gone,
                    // which is shutdown rather than an error worth reporting
                    // from a background thread — the same convention
                    // `control.rs`'s own `send_event` call sites use.
                    let _ = proxy.send_event(AppEvent::DownloadCompleted {
                        path: path.clone(),
                        filename: filename.clone(),
                        url: url.clone(),
                        bytes: *bytes,
                        // This function *is* the agent path — every command
                        // reaching it arrived on the control socket. A
                        // human-initiated download, if one is ever added,
                        // raises the same event with `false`, which is why
                        // the event carries the fact rather than inferring it
                        // from where it was raised.
                        requested_by_agent: true,
                    });
                }
                let _ = reply.send(outcome);
            });
        },
        Command::ChromeRects => {
            // Test hook: lets the e2e suites click a real chrome widget by
            // name instead of hardcoding a coordinate that moves every time
            // the toolbar gains a button. Gated at the point of use on the
            // same variable the `evaluate` crash hook checks.
            //
            // Refused as an unrecognised command rather than as a forbidden
            // one, because that is what it is outside a test run — a build
            // without the hook has nothing to answer with. Serde's own
            // unknown-variant text is not reproduced verbatim (it would go
            // stale the moment a command is added, and it names every command
            // that does exist); what matters is that the refusal says nothing
            // about a feature being withheld. See `Command::ChromeRects` for
            // why chrome geometry is not an agent's to read.
            if std::env::var("TALARIA_TEST_HOOKS").as_deref() != Ok("1") {
                let _ = reply.send(Outcome::Error { message: "unknown command".into() });
                return;
            }
            // The last frame that was drawn, which is the only frame whose
            // layout exists. The caller in `user_event` asks for a redraw as
            // soon as this returns, so a client that polls gets a fresh
            // answer on its next read.
            let rects = GUI.with_borrow(|gui| {
                gui.as_ref().map(|gui| gui.chrome_rects().to_vec()).unwrap_or_default()
            });
            let _ = reply.send(Outcome::Ok {
                result: ResultPayload::ChromeRects {
                    rects,
                    scale: state.window.scale_factor(),
                },
            });
        },
        Command::ViewHolds { tab_id } => {
            // Gated and refused identically to `Command::ChromeRects` — see
            // that arm for why a test hook is refused as *unrecognised* rather
            // than as forbidden, and `Command::ViewHolds` for why a count of
            // who is watching is not an agent's to read.
            if std::env::var("TALARIA_TEST_HOOKS").as_deref() != Ok("1") {
                let _ = reply.send(Outcome::Error { message: "unknown command".into() });
                return;
            }
            let holds = state.tabs.borrow().view_holds(tab_id);
            let _ = reply.send(Outcome::Ok {
                result: ResultPayload::Value {
                    value: match holds {
                        Some(count) => serde_json::Value::from(count),
                        None => serde_json::Value::Null,
                    },
                },
            });
        },
    }
}

/// How long a returned promise may take to settle: just under the control
/// socket's command timeout, so the agent gets our message rather than a
/// generic timeout.
fn promise_wait() -> std::time::Duration {
    let timeout_secs: u64 = std::env::var("TALARIA_COMMAND_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    std::time::Duration::from_secs(timeout_secs.saturating_sub(2).max(1))
}

/// Wrap an agent script so that (a) a returned promise/thenable is parked in
/// `window.__talaria_async[slot]` and polled to settlement, (b) top-level
/// `await` works: a single expression is returned as-is, a multi-statement
/// script runs inside an async function and needs `return`,
/// (c) a page whose CSP forbids eval reports that so we can fall back to a
/// raw run. Indirect eval keeps ordinary global-script semantics.
fn wrap_script(script: &str) -> String {
    let source = serde_json::to_string(script).expect("string is serializable");
    format!(
        "(function(){{var s={source};var v;\
         try{{v=(0,eval)(s)}}catch(e){{\
           if(e instanceof EvalError)return{{__talaria_csp:1}};\
           if(e instanceof SyntaxError&&/await/.test(String(e.message))){{\
             try{{v=(0,eval)('(async function(){{return(\\n'+s+'\\n)}})()')}}\
             catch(e2){{if(!(e2 instanceof SyntaxError))throw e2;\
               v=(0,eval)('(async function(){{'+s+'\\n}})()')}}}}\
           else throw e}}\
         if(v&&typeof v.then==='function'){{\
           var a=window.__talaria_async=window.__talaria_async||{{}};\
           var id=a.__next=(a.__next||0)+1;a[id]={{state:'pending'}};\
           v.then(function(x){{a[id]={{state:'ok',value:x}}}},\
                  function(e){{a[id]={{state:'error',message:String(e&&e.message||e)}}}});\
           return{{__talaria_async:id}}}}\
         return{{__talaria_value:v}}}})()"
    )
}

fn poll_script(slot: u64) -> String {
    format!(
        "(function(){{var a=window.__talaria_async;var s=a&&a[{slot}];\
         if(!s||s.state==='pending')return{{__talaria_pending:1}};\
         delete a[{slot}];return s}})()"
    )
}

/// Run an agent script with promise support (see [`wrap_script`]).
fn start_evaluate(
    state: &Rc<Shared>,
    webview: WebView,
    script: String,
    reply: tokio::sync::oneshot::Sender<Outcome>,
    done: Rc<Cell<bool>>,
) {
    let state = state.clone();
    let deadline = std::time::Instant::now() + promise_wait();
    let target = webview.clone();
    webview.evaluate_javascript(wrap_script(&script), move |result| {
        let outcome = match result {
            Ok(servo::JSValue::Object(mut map)) => {
                // Both parking branches are still in flight, so neither
                // clears the tab's entry — the follow-up carries the flag.
                if let Some(servo::JSValue::Number(slot)) = map.get("__talaria_async") {
                    state.pending_evals.borrow_mut().push(PendingEval {
                        webview: target,
                        step: EvalStep::Poll { slot: *slot as u64 },
                        reply,
                        deadline,
                        next: std::time::Instant::now() + std::time::Duration::from_millis(20),
                        done,
                    });
                    state.window.request_redraw();
                    return;
                }
                if map.contains_key("__talaria_csp") {
                    state.pending_evals.borrow_mut().push(PendingEval {
                        webview: target,
                        step: EvalStep::RunRaw { script },
                        reply,
                        deadline,
                        next: std::time::Instant::now(),
                        done,
                    });
                    state.window.request_redraw();
                    return;
                }
                Outcome::Ok {
                    result: ResultPayload::Value {
                        value: map
                            .remove("__talaria_value")
                            .map(js_value_to_json)
                            .unwrap_or(serde_json::Value::Null),
                    },
                }
            },
            Ok(value) => Outcome::Ok {
                result: ResultPayload::Value { value: js_value_to_json(value) },
            },
            Err(error) => Outcome::Error { message: describe_js_error(error) },
        };
        done.set(true);
        let _ = reply.send(outcome);
    });
}

/// Agent-readable evaluate failures: the thrown error's message for script
/// errors, and a "not ready, retry" hint for the engine-side states that
/// only mean the page has no usable document yet.
fn describe_js_error(error: servo::JavaScriptEvaluationError) -> String {
    use servo::JavaScriptEvaluationError as E;
    match error {
        E::EvaluationFailure(Some(info)) => {
            let mut message = format!("script threw: {}", info.message);
            if info.line_number > 0 {
                message.push_str(&format!(" (line {}:{})", info.line_number, info.column));
            }
            message
        },
        E::EvaluationFailure(None) => "script threw".into(),
        E::CompilationFailure => "script failed to compile".into(),
        E::DocumentNotFound | E::WebViewNotReady | E::InternalError => format!(
            "page not ready ({error:?}) — the tab is still loading or navigating; \
             wait for tabs_list to report loading:false and retry"
        ),
        E::SerializationError(inner) => {
            format!("result could not be serialized: {inner:?} — return plain data (strings, numbers, arrays, objects)")
        },
    }
}

/// Flatten servo's JSValue into plain JSON — agents should see `42`, not
/// the Rust enum encoding `{"Number": 42.0}`. Non-data handles (elements,
/// frames, windows) become descriptive strings.
fn js_value_to_json(value: servo::JSValue) -> serde_json::Value {
    use serde_json::Value;
    match value {
        servo::JSValue::Undefined | servo::JSValue::Null => Value::Null,
        servo::JSValue::Boolean(b) => Value::Bool(b),
        servo::JSValue::Number(n) => {
            // Integral doubles serialize as integers ("42", not "42.0") so
            // agents comparing against JSON integers aren't surprised.
            if n.fract() == 0.0 && n.abs() < i64::MAX as f64 {
                Value::Number(serde_json::Number::from(n as i64))
            } else {
                serde_json::Number::from_f64(n).map(Value::Number).unwrap_or(Value::Null)
            }
        },
        servo::JSValue::String(s) => Value::String(s),
        servo::JSValue::Element(s) => Value::String(format!("[element {s}]")),
        servo::JSValue::ShadowRoot(s) => Value::String(format!("[shadow-root {s}]")),
        servo::JSValue::Frame(s) => Value::String(format!("[frame {s}]")),
        servo::JSValue::Window(s) => Value::String(format!("[window {s}]")),
        servo::JSValue::Array(items) => {
            Value::Array(items.into_iter().map(js_value_to_json).collect())
        },
        servo::JSValue::Object(map) => Value::Object(
            map.into_iter().map(|(k, v)| (k, js_value_to_json(v))).collect(),
        ),
    }
}

fn encode_screenshot(image: servo::RgbaImage) -> Outcome {
    let (width, height) = (image.width(), image.height());
    let raw = image.into_raw();
    let mut png_data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_data, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let Ok(mut writer) = encoder.write_header() else {
            return Outcome::Error { message: "png header".into() };
        };
        if writer.write_image_data(&raw).is_err() {
            return Outcome::Error { message: "png encode".into() };
        }
    }
    Outcome::Ok {
        result: ResultPayload::Screenshot {
            png_base64: base64::engine::general_purpose::STANDARD.encode(&png_data),
            width,
            height,
        },
    }
}

/// Ceiling on a downloaded body when nothing overrides it: 2 GiB. A download is
/// an agent-initiated write to the user's disk, so it needs a bound the remote
/// server cannot raise.
const DEFAULT_MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Ceiling on a downloaded body, tunable through `TALARIA_MAX_DOWNLOAD_BYTES`.
/// A value that does not parse as an unsigned 64-bit integer falls back to the
/// default rather than to zero — a typo must not silently disable downloads —
/// and never to unbounded, so a typo cannot silently disable the cap either.
fn max_download_bytes() -> u64 {
    std::env::var("TALARIA_MAX_DOWNLOAD_BYTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_MAX_DOWNLOAD_BYTES)
}

/// How long a download may take, for the connect, for any single read, and
/// overall: just under the control socket's command timeout, so the agent gets
/// this path's specific message rather than a generic timeout. Derived from
/// `TALARIA_COMMAND_TIMEOUT_SECS` exactly the way `promise_wait` is.
fn download_timeout() -> std::time::Duration {
    let timeout_secs: u64 = std::env::var("TALARIA_COMMAND_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    std::time::Duration::from_secs(timeout_secs.saturating_sub(2).max(1))
}

/// Create the destination file without ever clobbering one the user already
/// has: try the plain name, then `stem (1).ext`, `stem (2).ext`, and so on.
/// Creation is exclusive rather than truncating, which is what makes the
/// guarantee hold under two concurrent downloads asking for the same name — the
/// loser of the race sees `AlreadyExists` and advances to the next counter
/// instead of opening the winner's file. A collision is resolved here, never by
/// prompting: an agent cannot see a dialog.
fn create_unique(
    dir: &std::path::Path,
    filename: &str,
) -> std::io::Result<(std::fs::File, std::path::PathBuf)> {
    // A leading dot is part of the name, not an extension separator, so
    // `.bashrc` uniquifies as `.bashrc (1)` rather than ` (1).bashrc`.
    let (stem, extension) = match filename.rfind('.') {
        Some(dot) if dot > 0 => (&filename[..dot], &filename[dot..]),
        _ => (filename, ""),
    };
    for counter in 0..1000 {
        let candidate = if counter == 0 {
            dir.join(filename)
        } else {
            dir.join(format!("{stem} ({counter}){extension}"))
        };
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&candidate) {
            Ok(file) => return Ok((file, candidate)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "too many files already share this name",
    ))
}

/// Give up on a download in progress: drop the writer, remove the partial file
/// so nothing half-written survives at the destination, and report why.
fn abandon_download(
    writer: std::io::BufWriter<std::fs::File>,
    path: &std::path::Path,
    message: String,
) -> Outcome {
    drop(writer);
    let _ = std::fs::remove_file(path);
    Outcome::Error { message }
}

/// Where a download is allowed to land.
///
/// `dirs::download_dir()` returns `None` whenever XDG user directories are not
/// configured — a headless server, a container, a minimal install, a CI job:
/// which is to say, exactly the environments this browser is most often run in.
/// The previous fallback was the process's current working directory, so an
/// agent that chooses a filename also chose to write it into whatever directory
/// the user happened to launch Talaria from. That is a poor place to put a file
/// nobody asked for and a worse one to put a file an agent named.
///
/// So: the configured directory, else `~/Downloads` if it already exists, else
/// nothing — and the caller refuses rather than inventing a location. This does
/// not create `~/Downloads`; a browser silently creating directories in a home
/// it was not asked to touch is its own surprise.
fn downloads_dir() -> Option<std::path::PathBuf> {
    if let Some(dir) = dirs::download_dir() {
        return Some(dir);
    }
    let fallback = dirs::home_dir()?.join("Downloads");
    fallback.is_dir().then_some(fallback)
}

/// Fetch `url` into the user's downloads directory under `filename`: bounded in
/// bytes, bounded in time, never clobbering an existing file, and abandoned the
/// moment nobody is waiting for the result. `reply` is borrowed purely as a
/// cancellation handle — the control socket drops its receiving half when the
/// command timeout fires, so a closed sender means the agent already got an
/// error and these bytes are unwanted.
fn download(
    url: &str,
    filename: &str,
    reply: &tokio::sync::oneshot::Sender<Outcome>,
) -> Outcome {
    let dir = match downloads_dir() {
        Some(dir) => dir,
        None => {
            return Outcome::Error {
                message: "no downloads directory: set XDG_DOWNLOAD_DIR or create ~/Downloads".into(),
            };
        },
    };
    let max_bytes = max_download_bytes();
    let timeout = download_timeout();
    // A bare `ureq::get` has no timeouts at all, so a server that accepts and
    // then stalls would hold this thread open long past the command timeout.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(timeout)
        .timeout_read(timeout)
        .build();
    let response = match agent.get(url).call() {
        Ok(response) => response,
        Err(error) => return Outcome::Error { message: format!("request failed: {error}") },
    };
    let (file, path) = match create_unique(&dir, filename) {
        Ok(created) => created,
        Err(error) => return Outcome::Error { message: format!("create failed: {error}") },
    };
    let mut reader = response.into_reader();
    let mut writer = std::io::BufWriter::new(file);
    let mut buffer = vec![0_u8; 64 * 1024];
    // Counted from the bytes we actually read and write, never from
    // content-length: the header is attacker-controlled and need not match the
    // body it describes.
    let mut written: u64 = 0;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if reply.is_closed() {
            let message = "download cancelled: nothing is waiting for the result".to_owned();
            return abandon_download(writer, &path, message);
        }
        if std::time::Instant::now() >= deadline {
            // The per-read timeout above cannot catch a peer that drips one
            // byte at a time; this overall deadline can.
            let seconds = timeout.as_secs();
            let message = format!("download timed out after {seconds}s");
            return abandon_download(writer, &path, message);
        }
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) => {
                return abandon_download(writer, &path, format!("read failed: {error}"));
            },
        };
        written += read as u64;
        if written > max_bytes {
            let message = format!(
                "download exceeds the {max_bytes} byte cap \
                 (raise TALARIA_MAX_DOWNLOAD_BYTES to allow more)"
            );
            return abandon_download(writer, &path, message);
        }
        let write_result = writer.write_all(&buffer[..read]);
        if let Err(error) = write_result {
            return abandon_download(writer, &path, format!("write failed: {error}"));
        }
    }
    let flush_result = writer.flush();
    if let Err(error) = flush_result {
        return abandon_download(writer, &path, format!("write failed: {error}"));
    }
    Outcome::Ok {
        result: ResultPayload::Download {
            path: path.display().to_string(),
            bytes: written,
        },
    }
}

impl servo::WebViewDelegate for Shared {
    /// Allow every navigation but one: an agent-owned tab reaching the
    /// listener's own origin.
    ///
    /// **Talaria is not a policy layer, and this is not policy.** The
    /// listener's own address is the transport the agent is already speaking;
    /// there is no legitimate reason for an agent to browse it, and
    /// `parse_agent_url`'s refusal on `tabs_open` and `navigate` was worth
    /// nothing while a page-driven navigation reached the same address in one
    /// call — `evaluate(tab, "location.href = 'http://127.0.0.1:PORT/register'")`,
    /// or a `window.open`, or a `<meta refresh>`, or a link click (WR-03).
    /// A mitigation keyed on where a URL is *parsed* rather than on where a
    /// load *happens* is the Phase 3 history-filter mistake again; this makes
    /// the invariant structural.
    ///
    /// The human's own tabs are untouched, including a human's own tab
    /// pointed at the listener — the refusal is keyed on the tab's owner, and
    /// nothing here narrows what a person may browse.
    ///
    /// **Refusing means `deny()`, never `return`.** `NavigationRequest`'s
    /// `Drop` sends *allow* when no response was sent, so dropping the value
    /// is the opposite of a refusal.
    fn request_navigation(
        &self,
        webview: WebView,
        navigation_request: servo::NavigationRequest,
    ) {
        // A delegate callback, so every borrow is a `try_borrow` and every
        // failure is answered rather than panicked on — and answered in the
        // direction that refuses, per this module's degrade rule. Both
        // failure arms are unreachable in practice: neither `remote` nor
        // `tabs` is held across a call into the engine.
        let Ok(remote) = self.remote.try_borrow() else {
            log::warn!(
                "could not read the listener's address while deciding a navigation; \
                 refusing it"
            );
            navigation_request.deny();
            return;
        };
        let bound = remote.bound_addr().map(str::to_owned);
        drop(remote);
        // Cheapest question first, and the one that needs no tab table: with
        // nothing bound there is no origin to protect, and a URL that is not
        // the listener's is nobody's business here.
        if !bound.as_deref().is_some_and(|bound| is_listener_origin(&navigation_request.url, bound))
        {
            navigation_request.allow();
            return;
        }
        // It *is* this browser's own listener, so whose tab it is now decides.
        let agent_owned = match self.tabs.try_borrow() {
            Ok(tabs) => tabs
                .find_by_webview(&webview)
                .and_then(|id| tabs.get(id))
                .map(|tab| tab.owner.is_agent()),
            Err(_) => None,
        };
        match agent_owned {
            // A human's tab, and a person may browse their own machine.
            Some(false) => navigation_request.allow(),
            // An agent's tab, or a tab table that could not say. Unanswered
            // resolves to a refusal *at this one address*: a denied
            // navigation is recoverable and a permitted one is not.
            _ => {
                log::warn!(
                    "refused a navigation to Talaria's own remote-access listener from a \
                     tab that is not the human's"
                );
                navigation_request.deny();
            },
        }
    }

    fn request_create_new(
        &self,
        parent_webview: WebView,
        request: servo::CreateNewWebViewRequest,
    ) {
        // window.open / target=_blank: a new tab in the parent's view with the
        // parent's owner (an agent's popups stay that agent's). No popup
        // blocking — Talaria isn't a policy layer.
        //
        // Build the framebuffer and the webview first: neither needs the tab
        // table, so the callback always gets this far and a busy table costs
        // a deferral rather than the whole popup.
        let rendering_context =
            Rc::new(self.window_rendering_context.offscreen_context(self.window.inner_size()));
        let webview = request
            .builder(rendering_context.clone())
            .hidpi_scale_factor(euclid::Scale::new(self.hidpi_scale()))
            .delegate(parent_webview.delegate())
            .build();
        match self.tabs.try_borrow_mut() {
            Ok(mut tabs) => {
                self.adopt_popup(&mut tabs, &parent_webview, webview, rendering_context)
            },
            Err(_) => {
                // Routine churn, not a degraded condition: the adoption
                // happens on the next loop turn instead of never.
                log::debug!("popup adoption deferred: tab table busy");
                self.pending_tab_work.borrow_mut().push(TabWork::AdoptPopup {
                    parent: parent_webview,
                    webview,
                    rendering_context,
                });
            },
        }
        self.window.request_redraw();
    }

    fn notify_new_frame_ready(&self, webview: WebView) {
        // Runs inside servo's painter borrow: only mark state, never paint
        // or toggle visibility here.
        match self.pending_captures.try_borrow_mut() {
            Ok(mut pending) => {
                for capture in pending.iter_mut().filter(|c| c.webview == webview) {
                    capture.ready = true;
                }
            },
            // This queue is only ever borrowed inside a scope that makes no
            // call into servo, so contention here is an invariant violation
            // rather than routine churn — and the cost is a capture that
            // waits out its deadline and returns a stale frame. Loud, so it
            // is never invisible again.
            Err(_) => log::error!("capture-ready mark lost: pending_captures busy"),
        }
        self.window.request_redraw();
    }

    fn notify_load_status_changed(&self, webview: WebView, status: servo::LoadStatus) {
        // Only mark state here; replies are built from the event loop.
        let tab_id = match self.tabs.try_borrow() {
            Ok(tabs) => tabs.find_by_webview(&webview),
            Err(_) => {
                log::error!("load-status mark lost: tab table busy");
                None
            },
        };
        if let Some(tab_id) = tab_id {
            match self.pending_loads.try_borrow_mut() {
                Ok(mut pending) => {
                    for load in pending.iter_mut().filter(|l| l.tab_id == tab_id) {
                        match status {
                            servo::LoadStatus::Started | servo::LoadStatus::HeadParsed => {
                                load.started = true;
                            },
                            servo::LoadStatus::Complete => {
                                if load.started || !load.needs_start {
                                    load.ready = true;
                                }
                            },
                        }
                    }
                },
                // Same invariant as above: this queue is never borrowed
                // across a call into servo, so a failure here means a
                // tabs_open / navigate reply waits out LOAD_WAIT instead of
                // answering when the page actually finished.
                Err(_) => log::error!("load-status mark lost for tab {tab_id}: pending_loads busy"),
            }
            // History capture is deliberately independent of the queue above:
            // a page the human opened from the URL bar has no pending_loads
            // entry at all, and it is still a visit. Gated on Complete rather
            // than on the URL changing, so a page that never finished loading
            // does not enter the record as though it had.
            if matches!(status, servo::LoadStatus::Complete) {
                match self.pending_history_writes.try_borrow_mut() {
                    Ok(mut pending) => pending.push(HistoryWrite { tab_id }),
                    Err(_) => log::error!("history write lost for tab {tab_id}: queue busy"),
                }
            }
        }
        // Wakes the loop so pending loads get serviced; also refreshes the
        // spinner.
        self.window.request_redraw();
    }

    fn notify_page_title_changed(&self, _webview: WebView, _title: Option<String>) {
        // The chrome refresh picks the title up from the displayed tab.
        self.window.request_redraw();
    }

    fn notify_url_changed(&self, webview: WebView, url: Url) {
        match self.tabs.try_borrow_mut() {
            Ok(mut tabs) => apply_url_change(&mut tabs, &webview, &url),
            // Skipping would leave the URL bar showing the previous page and,
            // for an adopted popup, leave the blank grace window open so the
            // tab keeps reporting as loading.
            Err(_) => {
                self.pending_tab_work
                    .borrow_mut()
                    .push(TabWork::UrlChanged { webview, url });
            },
        }
        self.window.request_redraw();
    }

    fn notify_crashed(&self, webview: WebView, reason: String, backtrace: Option<String>) {
        log::error!("tab crashed: {reason} {backtrace:?}");
        let crashed_tab = match self.tabs.try_borrow_mut() {
            Ok(mut tabs) => tabs.find_by_webview_mut(&webview).map(|tab| {
                tab.crashed = true;
                (tab.id, tab.owner.clone())
            }),
            // Deferring keeps both halves. Skipping the mark would leave a
            // dead tab looking healthy to tabs_list and to the crash page,
            // and skipping the event would leave the agent to discover the
            // crash from its next tool call.
            Err(_) => {
                self.pending_tab_work
                    .borrow_mut()
                    .push(TabWork::MarkCrashed { webview });
                None
            },
        };
        if let Some((tab_id, owner)) = crashed_tab {
            self.queue_event(&owner, talaria_protocol::Event::TabCrashed { tab_id });
        }
        self.window.request_redraw();
    }

    fn notify_closed(&self, webview: WebView) {
        let mut closed = None;
        match self.tabs.try_borrow_mut() {
            Ok(mut tabs) => {
                // Owner before close, as on every other close path.
                if let Some(id) = tabs.find_by_webview(&webview) {
                    let owner = tabs.get(id).map(|tab| tab.owner.clone());
                    if let (Some(owner), true) = (owner, tabs.close(id)) {
                        closed = Some((id, owner));
                    }
                }
            },
            // The drain resolves the owner and raises the event from the
            // loop, where the table is never contended. Skipping would leave
            // a closed webview listed as a live tab forever.
            Err(_) => {
                self.pending_tab_work
                    .borrow_mut()
                    .push(TabWork::Close { webview });
            },
        }
        if let Some((tab_id, owner)) = closed {
            self.queue_event(&owner, talaria_protocol::Event::TabClosed { tab_id });
        }
        self.window.request_redraw();
    }
}

#[derive(Clone)]
pub struct Waker(pub EventLoopProxy<AppEvent>);

impl embedder_traits::EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn embedder_traits::EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        let _ = self.0.send_event(AppEvent::Wake);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An engine that is not the default, so "did the parameter reach the
    /// fallback" is answerable without reading DuckDuckGo out of a URL.
    fn custom() -> SearchEngine {
        SearchEngine {
            name: "Example".to_owned(),
            url_template: "https://example.com/search?q={query}&lang=en".to_owned(),
        }
    }

    /// The zero-regression guard. Whatever else this function grows, the
    /// default engine must keep producing exactly the URL the hardcoded
    /// literal produced before BROWSE-03 existed.
    /// The whole point of `downloads_dir`: it may decline to name a
    /// directory, but it must never name a relative one. The previous
    /// behaviour resolved to `"."`, which put an agent-named file in whatever
    /// directory Talaria was launched from.
    #[test]
    fn the_downloads_directory_is_never_relative() {
        if let Some(dir) = downloads_dir() {
            assert!(
                dir.is_absolute(),
                "downloads_dir returned the relative path {}",
                dir.display()
            );
        }
    }

    #[test]
    fn the_default_engine_reproduces_the_old_hardcoded_search_url() {
        let url = resolve_location("hello world", &SearchEngine::default());
        assert_eq!(url.as_str(), "https://duckduckgo.com/?q=hello+world");
    }

    #[test]
    fn a_configured_engine_is_what_a_search_actually_uses() {
        let url = resolve_location("hello world", &custom());
        assert_eq!(url.as_str(), "https://example.com/search?q=hello+world&lang=en");
    }

    /// The engine parameter touches the fallback and nothing above it: every
    /// URL-shaped input resolves identically no matter which engine is
    /// passed. This is the guard against a future edit sliding the engine up
    /// into one of the URL-detection branches.
    #[test]
    fn url_shaped_input_is_unaffected_by_the_engine() {
        for input in [
            "https://example.org/path",
            "example.org",
            "about:blank",
            "file:///tmp/page.html",
            "  https://example.org/  ",
        ] {
            let with_default = resolve_location(input, &SearchEngine::default());
            let with_custom = resolve_location(input, &custom());
            assert_eq!(
                with_default, with_custom,
                "{input:?} resolved differently depending on the search engine"
            );
            assert!(
                !with_default.as_str().contains("example.com/search"),
                "{input:?} was treated as a search query"
            );
        }
    }

    /// Defense in depth, one layer below `Settings::load`'s own validation: a
    /// `SearchEngine` built in code (a test, a future caller) can carry a
    /// template that never reaches the load-time check, and this function
    /// must degrade rather than panic on any of them.
    #[test]
    fn a_malformed_template_still_yields_a_url_rather_than_a_panic() {
        for template in [
            // No placeholder: nothing to substitute.
            "https://example.com/search",
            // Two placeholders: `replacen(.., 1)` leaves the second behind.
            "https://example.com/{query}?q={query}",
            // Not a URL at all once substituted.
            "not a url {query}",
            "",
        ] {
            let engine = SearchEngine {
                name: "Broken".to_owned(),
                url_template: template.to_owned(),
            };
            let url = resolve_location("hello world", &engine);
            assert!(
                !url.as_str().is_empty(),
                "template {template:?} produced no usable url"
            );
        }
    }

    /// `data:` in the human's address bar is *deliberately* a search, not a
    /// navigation, and this test exists so that stays a decision rather than
    /// an accident.
    ///
    /// `parse_agent_url` admits `data:`; this path does not, and the
    /// asymmetry runs the right way. "The human typed it, so trust it" is
    /// weakest exactly here: the paste-this-into-your-address-bar attack
    /// makes the human a courier for someone else's payload, which is why
    /// this function's own doc comment calls the human the trust *root*
    /// rather than the trust *source*. An agent, by contrast, builds its own
    /// `data:` URLs, and SEC-01 hardened that path on its own terms. Widening
    /// this one to match would be a trust-model change with no requirement
    /// asking for it — see `03-03-SUMMARY.md`.
    #[test]
    fn a_data_url_is_searched_for_rather_than_opened() {
        let url = resolve_location("data:text/html,<h1>hi</h1>", &SearchEngine::default());
        assert_eq!(url.scheme(), "https", "a data: url was navigated to from the address bar");
        assert!(url.as_str().starts_with("https://duckduckgo.com/?q="));
    }

    /// CR-02's exact shape, in the one function that decides it. An agent
    /// reaching one of the human's own tabs — through `evaluate`, the only
    /// command that still can — must not put a row in front of the human
    /// that reads as somewhere they went. The second half is the half that
    /// keeps the failure direction right: the very next load on that same
    /// tab is the human's again, and it is recorded.
    #[test]
    fn an_agent_caused_load_on_a_me_tab_is_skipped_and_the_next_human_one_is_recorded() {
        let mut load_started_by_agent = true;

        assert!(
            !belongs_in_history(&TabOwner::Me, &mut load_started_by_agent),
            "an agent-caused load on the human's own tab reached their history",
        );
        assert!(
            !load_started_by_agent,
            "the mark outlived the load that set it, so the human's next visit is lost too",
        );
        assert!(
            belongs_in_history(&TabOwner::Me, &mut load_started_by_agent),
            "a human navigation after an agent-caused one was not recorded",
        );
    }

    /// The original owner filter, still doing its own job: an agent's own tab
    /// is out regardless of what marked it.
    #[test]
    fn an_agent_owned_tab_is_out_however_its_load_started() {
        let owner = TabOwner::Agent { session_id: 7, client: "some-agent".to_owned() };
        for mut load_started_by_agent in [false, true] {
            assert!(!belongs_in_history(&owner, &mut load_started_by_agent));
        }
    }

    /// The ordinary case, asserted so the two above cannot pass by refusing
    /// everything.
    #[test]
    fn an_unmarked_load_on_a_me_tab_is_recorded() {
        let mut load_started_by_agent = false;
        assert!(belongs_in_history(&TabOwner::Me, &mut load_started_by_agent));
    }

    /// The refusal `download_bounds_test.py` already pins, kept verbatim so
    /// the two checks below cannot be added by changing it.
    #[test]
    fn a_filename_that_is_a_path_is_still_refused_with_the_same_message() {
        assert_eq!(validate_download_filename("../escape.bin"), Err("bad filename".into()));
        assert_eq!(validate_download_filename("sub/escape.bin"), Err("bad filename".into()));
    }

    /// CR-04(c). A newline is not a path problem, so neither of the two
    /// checks above sees it — and the requested name is the row's entire
    /// visible label, so a name with one in it renders as two lines and the
    /// second reads as copy rather than as a filename.
    #[test]
    fn a_filename_carrying_a_newline_is_refused_at_the_command_boundary() {
        let refusal = validate_download_filename("invoice.pdf\n\nSafe — from your bank");
        assert!(refusal.is_err(), "a filename that renders as two rows was accepted");
        let message = refusal.unwrap_err();
        assert!(message.starts_with("bad filename"), "{message}");
        assert!(message.contains("U+000A"), "the refusal did not name the character: {message}");
    }

    /// The other half: a right-to-left override reorders what follows it, so
    /// the extension the human reads is not the extension `xdg-open` will
    /// dispatch on. Every character here is printable, so a control-character
    /// check alone would let it through.
    #[test]
    fn a_filename_carrying_a_bidi_override_is_refused_at_the_command_boundary() {
        for name in [
            "report\u{202E}fdp.exe",
            "report\u{202A}.pdf",
            "report\u{2066}.pdf",
            "report\u{2069}.pdf",
        ] {
            let refusal = validate_download_filename(name);
            assert!(refusal.is_err(), "a bidi override survived to the store: {name:?}");
            assert!(refusal.unwrap_err().starts_with("bad filename"));
        }
    }

    /// And the names that must keep working, including the one shape
    /// `create_unique` produces on a collision.
    #[test]
    fn an_ordinary_filename_is_accepted() {
        for name in ["report.pdf", "report (1).pdf", "\u{65E5}\u{672C}\u{8A9E}.txt", "a b c.bin"] {
            assert_eq!(validate_download_filename(name), Ok(()), "{name:?} was refused");
        }
    }

    // ---------------------------------------------------------------------
    // parse_agent_url's own-origin refusal (T-3)
    // ---------------------------------------------------------------------

    /// The refusal itself: while something is bound, an agent may not point a
    /// tab at it.
    #[test]
    fn the_listeners_own_origin_is_refused_while_it_is_bound() {
        let bound = Some("127.0.0.1:8779");
        for input in [
            "http://127.0.0.1:8779/",
            "http://127.0.0.1:8779/mcp",
            "http://127.0.0.1:8779/.well-known/oauth-authorization-server",
            "http://127.0.0.1:8779",
        ] {
            let refusal = parse_agent_url(input, bound);
            assert!(refusal.is_err(), "{input} reached the listener");
        }
    }

    /// `localhost` resolves to the same place, so it is the same refusal — a
    /// gate that knew only one spelling of one machine would not be a gate.
    #[test]
    fn the_localhost_spelling_of_the_listener_is_refused_too() {
        assert!(parse_agent_url("http://localhost:8779/mcp", Some("127.0.0.1:8779")).is_err());
    }

    /// Keyed on what is actually bound: with the listener off there is no
    /// origin to protect, and the same URL is an ordinary address.
    #[test]
    fn the_same_address_is_allowed_when_nothing_is_bound() {
        let allowed = parse_agent_url("http://127.0.0.1:8779/", None);
        assert_eq!(allowed.map(|url| url.to_string()), Ok("http://127.0.0.1:8779/".to_owned()));
    }

    /// A neighbouring port on the same host is somebody else's server — very
    /// often the thing an agent legitimately wants to look at.
    #[test]
    fn a_different_port_on_the_same_host_is_allowed() {
        let bound = Some("127.0.0.1:8779");
        for input in ["http://127.0.0.1:8080/", "http://localhost:3000/", "http://127.0.0.1/"] {
            assert!(parse_agent_url(input, bound).is_ok(), "{input} was refused");
        }
    }

    /// The refusal names the reason, so an agent can tell policy from a typo —
    /// the same contract the scheme refusal beside it keeps.
    #[test]
    fn the_listener_refusal_names_the_reason() {
        let refusal = parse_agent_url("http://127.0.0.1:8779/mcp", Some("127.0.0.1:8779"))
            .expect_err("the listener's own origin must be refused");
        assert!(refusal.contains("remote-access listener"), "{refusal}");
        assert!(!refusal.starts_with("bad url"), "{refusal}");
    }

    /// The scheme gate still runs first: a refused scheme must not be reported
    /// as a listener-origin problem, and must stay refused with nothing bound.
    #[test]
    fn the_scheme_refusal_still_comes_first() {
        let refusal = parse_agent_url("file:///etc/passwd", Some("127.0.0.1:8779"))
            .expect_err("file: is not an agent scheme");
        assert!(refusal.starts_with("scheme file"), "{refusal}");
    }

    /// And every ordinary URL is untouched by the new parameter.
    #[test]
    fn ordinary_urls_are_unaffected_by_the_bound_origin() {
        for bound in [None, Some("127.0.0.1:8779")] {
            assert!(parse_agent_url("https://example.com/", bound).is_ok());
            assert!(parse_agent_url("example.com", bound).is_ok());
            assert!(parse_agent_url("about:blank", bound).is_ok());
        }
    }
}

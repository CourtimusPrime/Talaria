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

use crate::control::AgentRequest;
use crate::gui::{Gui, UiAction};
use crate::keyutils;
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
    pub tabs: RefCell<TabManager>,
    pub sessions: RefCell<BTreeMap<u64, Session>>,
    pub vault: RefCell<Vault>,
    /// Bottom of the chrome strip, in logical points.
    pub toolbar_height: Cell<f32>,
    /// Last cursor position, physical pixels.
    pub last_cursor: Cell<Option<PhysicalPosition<f64>>>,
    /// Last cursor position relative to the webview viewport, device pixels.
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
            let outcome = self.capture_now(&capture.webview, &capture.context, true);
            let _ = capture.reply.send(outcome);
        }
        self.process_pending_loads();
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

    /// Earliest deadline among queued captures/loads/evals and outstanding
    /// evaluates, for WaitUntil scheduling. The in-flight registry is a
    /// source here because a deadline nothing wakes for is not a deadline:
    /// an entry whose callback was lost must expire on time, not whenever
    /// the loop happens to turn next.
    pub fn next_capture_deadline(&self) -> Option<std::time::Instant> {
        let captures = self.pending_captures.borrow().iter().map(|c| c.deadline).min();
        let loads = self.pending_loads.borrow().iter().map(|l| l.deadline).min();
        let evals = self.pending_evals.borrow().iter().map(|e| e.next).min();
        let evaluating = self.evaluating.borrow().iter().map(|g| g.deadline).min();
        [captures, loads, evals, evaluating].into_iter().flatten().min()
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

    /// Push an unsolicited event to every connected control client.
    /// Non-blocking (unbounded channel), so this is safe to call from servo
    /// delegate callbacks.
    pub fn broadcast_event(&self, event: talaria_protocol::Event) {
        if let Ok(sessions) = self.sessions.try_borrow() {
            for session in sessions.values() {
                let _ = session
                    .events
                    .send(talaria_protocol::ServerMessage::Event { event: event.clone() });
            }
        }
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
            tabs: RefCell::new(TabManager::new()),
            sessions: RefCell::new(BTreeMap::new()),
            vault: RefCell::new(Vault::load()),
            toolbar_height: Cell::new(0.0),
            last_cursor: Cell::new(None),
            webview_point: Cell::new(euclid::Point2D::zero()),
            modifiers: Cell::new(ModifiersState::empty()),
            window_title: RefCell::new(String::new()),
            pending_captures: RefCell::new(Vec::new()),
            pending_loads: RefCell::new(Vec::new()),
            pending_evals: RefCell::new(Vec::new()),
            evaluating: RefCell::new(Vec::new()),
        });

        state.open_tab(initial_url.clone(), TabOwner::Me);
        *self = App::Running(state);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, _cause: winit::event::StartCause) {
        if let Some(state) = self.state() {
            state.process_pending_captures();
            state.process_pending_evals();
            set_wait(event_loop, state);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        if let Some(state) = self.state() {
            state.process_pending_captures();
            state.process_pending_evals();
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
            }
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

        let over_toolbar = |state: &Shared| {
            state
                .last_cursor
                .get()
                .is_none_or(|p| (p.y / state.window.scale_factor()) < state.toolbar_height.get() as f64)
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
            WindowEvent::MouseInput { button, state: element_state, .. } if !over_toolbar(&state) => {
                GUI.with_borrow_mut(|gui| {
                    if let Some(gui) = gui.as_mut() {
                        gui.surrender_focus();
                    }
                });
                forward_mouse_button(&state, *button, *element_state);
            },
            WindowEvent::MouseWheel { delta, .. } if !over_toolbar(&state) => {
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

/// Standard browser keyboard shortcuts, intercepted before both egui and the
/// page: Ctrl+L (focus URL bar), Ctrl+T (new tab), Ctrl+W (close tab),
/// Ctrl+R / F5 (reload), Alt+Left / Alt+Right (back / forward),
/// Ctrl+Tab / Ctrl+Shift+Tab (cycle tabs). Returns true when consumed.
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

fn forward_mouse_move(state: &Shared, position: PhysicalPosition<f64>) {
    let webview = state.tabs.borrow().displayed().map(|t| t.webview.clone());
    let Some(webview) = webview else { return };

    let mut point = euclid::Point2D::<f32, DevicePixel>::new(position.x as f32, position.y as f32);
    point.y -= state.toolbar_height_device();

    let previous = state.webview_point.get();
    state.webview_point.set(point);

    let size = webview.size();
    let viewport = euclid::Rect::new(
        euclid::Point2D::zero(),
        euclid::Size2D::new(size.width as f32, size.height as f32),
    );
    if !viewport.contains(point) {
        if viewport.contains(previous) {
            webview.notify_input_event(InputEvent::MouseLeftViewport(
                MouseLeftViewportEvent::default(),
            ));
        }
        return;
    }
    webview.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(point.into())));
}

fn forward_mouse_button(
    state: &Shared,
    button: winit::event::MouseButton,
    element_state: ElementState,
) {
    let webview = state.tabs.borrow().displayed().map(|t| t.webview.clone());
    let Some(webview) = webview else { return };

    let point = state.webview_point.get();
    let size = webview.size();
    let viewport = euclid::Rect::new(
        euclid::Point2D::zero(),
        euclid::Size2D::new(size.width as f32, size.height as f32),
    );
    if !viewport.contains(point) {
        return;
    }

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
    webview.notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
        action,
        mouse_button,
        point.into(),
    )));
}

fn forward_wheel(state: &Shared, delta: MouseScrollDelta) {
    let webview = state.tabs.borrow().displayed().map(|t| t.webview.clone());
    let Some(webview) = webview else { return };

    let (dx, dy, mode) = match delta {
        MouseScrollDelta::LineDelta(x, y) => {
            ((x * 76.0) as f64, (y * 76.0) as f64, WheelMode::DeltaLine)
        },
        MouseScrollDelta::PixelDelta(delta) => (delta.x, delta.y, WheelMode::DeltaPixel),
    };
    let point = state.webview_point.get();
    webview.notify_input_event(InputEvent::Wheel(WheelEvent::new(
        WheelDelta { x: dx, y: dy, z: 0.0, mode },
        point.into(),
    )));
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
                let url = resolve_location(&input);
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
                if state.tabs.borrow_mut().close(id) {
                    state.broadcast_event(talaria_protocol::Event::TabClosed { tab_id: id });
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

/// Agent-supplied URLs: accept scheme-less hosts ("example.com") by assuming
/// https, but never fall back to a search query — an agent that meant to
/// search should do so explicitly. Anything that parses is then held to
/// `agent_scheme_allowed`; the refusal names the rejected scheme so the agent
/// can tell policy apart from a typo.
fn parse_agent_url(input: &str) -> Result<Url, String> {
    let parsed = match Url::parse(input) {
        Ok(url) => Ok(url),
        Err(url::ParseError::RelativeUrlWithoutBase) if !input.contains(' ') => {
            Url::parse(&format!("https://{input}"))
        },
        Err(error) => Err(error),
    };
    match parsed {
        Ok(url) if agent_scheme_allowed(&url) => Ok(url),
        Ok(url) => Err(format!(
            "scheme {} is not allowed for agents — use http, https, data:, or about:blank",
            url.scheme()
        )),
        Err(error) => Err(format!("bad url: {error}")),
    }
}

/// Omnibox behavior: URL if it parses (or looks like a host), search query
/// otherwise.
///
/// `about:` and `file:` are named explicitly because neither carries a host,
/// so the host test alone would send both to the search engine. The human is
/// the trust root here: unlike `parse_agent_url`, this path allowlists
/// nothing — a user who types a local path gets the local file.
pub fn resolve_location(input: &str) -> Url {
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
    Url::parse(&format!("https://duckduckgo.com/?q={query}")).expect("static url")
}


/// Agent-facing snapshot of one tab.
fn tab_info(tabs: &TabManager, tab: &crate::tabs::Tab) -> TabInfo {
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

    match command {
        Command::TabsList => {
            let tabs = state.tabs.borrow();
            let infos: Vec<TabInfo> = tabs.iter().map(|t| tab_info(&tabs, t)).collect();
            let _ = reply.send(Outcome::Ok { result: ResultPayload::Tabs { tabs: infos } });
        },
        Command::TabsOpen { url } => {
            match parse_agent_url(&url) {
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
            let closed = state.tabs.borrow_mut().close(tab_id);
            if closed {
                state.broadcast_event(talaria_protocol::Event::TabClosed { tab_id });
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
            match (webview_for(tab_id), parse_agent_url(&url)) {
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
                if let Some(tab) = state.tabs.borrow_mut().get_mut(tab_id) {
                    tab.crashed = true;
                    state.window.request_redraw();
                    state.broadcast_event(talaria_protocol::Event::TabCrashed { tab_id });
                    let _ = reply.send(Outcome::Ok { result: ResultPayload::Empty {} });
                } else {
                    let _ = reply.send(Outcome::Error { message: format!("no tab {tab_id}") });
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
            let url = resolve_location(&url);
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
            if filename.contains('/') || filename.contains("..") {
                let _ = reply.send(Outcome::Error { message: "bad filename".into() });
                return;
            }
            std::thread::spawn(move || {
                // `reply` doubles as the cancellation handle: `download` polls
                // its closed state, which becomes true the moment the control
                // socket's command timeout drops the receiving half.
                let outcome = download(&url, &filename, &reply);
                let _ = reply.send(outcome);
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
    let dir = dirs::download_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
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
    fn request_navigation(
        &self,
        _webview: WebView,
        navigation_request: servo::NavigationRequest,
    ) {
        // The default delegate drops the request, which blocks link-click
        // navigation entirely. Talaria is not a policy layer: allow all.
        navigation_request.allow();
    }

    fn request_create_new(
        &self,
        parent_webview: WebView,
        request: servo::CreateNewWebViewRequest,
    ) {
        // window.open / target=_blank: a new tab in the parent's view with the
        // parent's owner (an agent's popups stay that agent's). No popup
        // blocking — Talaria isn't a policy layer.
        let Ok(mut tabs) = self.tabs.try_borrow_mut() else {
            log::warn!("dropping popup request: tab table busy");
            return;
        };
        let Some(parent_id) = tabs.find_by_webview(&parent_webview) else {
            return;
        };
        let owner = tabs.get(parent_id).map(|t| t.owner.clone()).expect("just found");
        // A popup fronts its own view when its opener was the active tab
        // there (browser behaviour), and stays behind an opener that was
        // already in the background. Which view is *displayed* is irrelevant:
        // an agent's popup must not drag the human out of the Me view.
        let parent_active = tabs.active_id(owner.view()) == Some(parent_id);
        let rendering_context =
            Rc::new(self.window_rendering_context.offscreen_context(self.window.inner_size()));
        let webview = request
            .builder(rendering_context.clone())
            .hidpi_scale_factor(euclid::Scale::new(self.hidpi_scale()))
            .delegate(parent_webview.delegate())
            .build();
        // The popup's real URL arrives via notify_url_changed; until then the
        // URL bar shows what a `window.open()` with no argument keeps forever.
        let id = tabs.register(
            webview,
            rendering_context,
            owner,
            "about:blank".into(),
            parent_active,
            true,
        );
        drop(tabs);
        log::info!("popup from tab {parent_id} opened as tab {id}");
        self.window.request_redraw();
    }

    fn notify_new_frame_ready(&self, webview: WebView) {
        // Runs inside servo's painter borrow: only mark state, never paint
        // or toggle visibility here.
        if let Ok(mut pending) = self.pending_captures.try_borrow_mut() {
            for capture in pending.iter_mut().filter(|c| c.webview == webview) {
                capture.ready = true;
            }
        }
        self.window.request_redraw();
    }

    fn notify_load_status_changed(&self, webview: WebView, status: servo::LoadStatus) {
        // Only mark state here; replies are built from the event loop.
        let tab_id = self.tabs.try_borrow().ok().and_then(|tabs| tabs.find_by_webview(&webview));
        if let (Some(tab_id), Ok(mut pending)) = (tab_id, self.pending_loads.try_borrow_mut()) {
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
        if let Ok(mut tabs) = self.tabs.try_borrow_mut() {
            if let Some(tab) = tabs.find_by_webview_mut(&webview) {
                if !tab.location_dirty {
                    tab.location = url.to_string();
                }
                // The navigation an adopted popup was waiting for has begun,
                // so its blank grace window is over and `load_status` alone
                // tells the truth from here. Servo runs a whole
                // Started→Complete cycle on the popup's *own* blank document
                // first, which is why load status can't be that signal.
                if url.as_str() != "about:blank" {
                    tab.initial_blank_until = None;
                }
            }
        }
        self.window.request_redraw();
    }

    fn notify_crashed(&self, webview: WebView, reason: String, backtrace: Option<String>) {
        log::error!("tab crashed: {reason} {backtrace:?}");
        let mut crashed_tab = None;
        if let Ok(mut tabs) = self.tabs.try_borrow_mut() {
            if let Some(tab) = tabs.find_by_webview_mut(&webview) {
                tab.crashed = true;
                crashed_tab = Some(tab.id);
            }
        }
        if let Some(tab_id) = crashed_tab {
            self.broadcast_event(talaria_protocol::Event::TabCrashed { tab_id });
        }
        self.window.request_redraw();
    }

    fn notify_closed(&self, webview: WebView) {
        if let Ok(tabs) = self.tabs.try_borrow() {
            if let Some(id) = tabs.find_by_webview(&webview) {
                drop(tabs);
                if let Ok(mut tabs) = self.tabs.try_borrow_mut() {
                    if tabs.close(id) {
                        drop(tabs);
                        self.broadcast_event(talaria_protocol::Event::TabClosed { tab_id: id });
                    }
                }
            }
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


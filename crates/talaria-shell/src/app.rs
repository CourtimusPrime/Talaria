/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 *
 * Portions derived from Servo's servoshell and winit_minimal embedding
 * example (servo v0.4.0); this file keeps the MPL 2.0 header per its
 * file-level copyleft. Talaria's original code is MIT OR Apache-2.0. */

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
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
    /// Screenshot requests for tabs that are not currently displayed: the
    /// target must be shown and produce a fresh frame before its pixels exist
    /// in the shared framebuffer. Serviced from the event loop (never inside
    /// servo callbacks, which run under a painter borrow).
    pub pending_captures: RefCell<Vec<PendingCapture>>,
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
    }

    /// Earliest deadline among queued captures, for WaitUntil scheduling.
    pub fn next_capture_deadline(&self) -> Option<std::time::Instant> {
        self.pending_captures.borrow().iter().map(|c| c.deadline).min()
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
            pending_captures: RefCell::new(Vec::new()),
        });

        state.open_tab(initial_url.clone(), TabOwner::Me);
        *self = App::Running(state);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, _cause: winit::event::StartCause) {
        if let Some(state) = self.state() {
            state.process_pending_captures();
            set_wait(event_loop, state);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        if let Some(state) = self.state() {
            state.process_pending_captures();
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
                        gui.on_window_event(&state.window, &event);
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
/// Ctrl+R / F5 (reload). Returns true when the event was consumed.
fn handle_browser_shortcut(state: &Rc<Shared>, key_event: &winit::event::KeyEvent) -> bool {
    use winit::keyboard::{Key as WinitKey, NamedKey as WinitNamedKey};

    if key_event.state != ElementState::Pressed {
        return false;
    }
    let ctrl = state.modifiers.get().control_key();
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

/// Agent-supplied URLs: accept scheme-less hosts ("example.com") by assuming
/// https, but never fall back to a search query — an agent that meant to
/// search should do so explicitly.
fn parse_agent_url(input: &str) -> Result<Url, url::ParseError> {
    match Url::parse(input) {
        Ok(url) => Ok(url),
        Err(url::ParseError::RelativeUrlWithoutBase) if !input.contains(' ') => {
            Url::parse(&format!("https://{input}"))
        },
        Err(error) => Err(error),
    }
}

/// Omnibox behavior: URL if it parses (or looks like a host), search query
/// otherwise.
pub fn resolve_location(input: &str) -> Url {
    let input = input.trim();
    if let Ok(url) = Url::parse(input) {
        if !url.scheme().is_empty() && url.host().is_some() || url.scheme() == "about" {
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

    match command {
        Command::TabsList => {
            let tabs = state.tabs.borrow();
            let infos: Vec<TabInfo> = tabs.iter().map(|t| {
                let focused = tabs.active_id(ViewMode::Me) == Some(t.id)
                    || tabs.active_id(ViewMode::Agents) == Some(t.id);
                TabInfo {
                    tab_id: t.id,
                    url: t.webview.url().map(|u| u.to_string()).unwrap_or_else(|| t.location.clone()),
                    title: t.webview.page_title().unwrap_or_default(),
                    owner: t.owner.label(),
                    focused,
                    crashed: t.crashed,
                }
            }).collect();
            let _ = reply.send(Outcome::Ok { result: ResultPayload::Tabs { tabs: infos } });
        },
        Command::TabsOpen { url } => {
            match parse_agent_url(&url) {
                Ok(url) => {
                    let id = state.open_tab(url, TabOwner::Agent { session_id, client });
                    let tabs = state.tabs.borrow();
                    let tab = tabs.get(id).expect("just opened");
                    let info = TabInfo {
                        tab_id: tab.id,
                        url: tab.location.clone(),
                        title: String::new(),
                        owner: tab.owner.label(),
                        focused: tabs.active_id(ViewMode::Agents) == Some(id),
                        crashed: false,
                    };
                    let _ = reply.send(Outcome::Ok { result: ResultPayload::Tab { tab: info } });
                },
                Err(error) => {
                    let _ = reply.send(Outcome::Error { message: format!("bad url: {error}") });
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
                    webview.load(url);
                    let _ = reply.send(Outcome::Ok { result: ResultPayload::Empty {} });
                },
                (Err(outcome), _) => {
                    let _ = reply.send(outcome);
                },
                (_, Err(error)) => {
                    let _ = reply.send(Outcome::Error { message: format!("bad url: {error}") });
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
            match webview_for(tab_id) {
                Ok(webview) => {
                    webview.evaluate_javascript(script, move |result| {
                        let outcome = match result {
                            Ok(value) => Outcome::Ok {
                                result: ResultPayload::Value { value: js_value_to_json(value) },
                            },
                            Err(error) => Outcome::Error { message: format!("{error:?}") },
                        };
                        let _ = reply.send(outcome);
                    });
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
        Command::Download { url, filename } => {
            if filename.contains('/') || filename.contains("..") {
                let _ = reply.send(Outcome::Error { message: "bad filename".into() });
                return;
            }
            std::thread::spawn(move || {
                let outcome = download(&url, &filename);
                let _ = reply.send(outcome);
            });
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

fn download(url: &str, filename: &str) -> Outcome {
    let dir = dirs::download_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let path = dir.join(filename);
    let response = match ureq::get(url).call() {
        Ok(response) => response,
        Err(error) => return Outcome::Error { message: format!("request failed: {error}") },
    };
    let mut reader = response.into_reader();
    let file = match std::fs::File::create(&path) {
        Ok(file) => file,
        Err(error) => return Outcome::Error { message: format!("create failed: {error}") },
    };
    let mut writer = std::io::BufWriter::new(file);
    match std::io::copy(&mut reader, &mut writer) {
        Ok(bytes) => Outcome::Ok {
            result: ResultPayload::Download { path: path.display().to_string(), bytes },
        },
        Err(error) => Outcome::Error { message: format!("write failed: {error}") },
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

    fn notify_page_title_changed(&self, webview: WebView, title: Option<String>) {
        let is_displayed = self
            .tabs
            .try_borrow()
            .ok()
            .and_then(|tabs| tabs.displayed().map(|t| t.webview == webview))
            .unwrap_or(false);
        if is_displayed {
            let title = match title {
                Some(title) if !title.is_empty() => format!("{title} — Talaria"),
                _ => "Talaria".to_owned(),
            };
            self.window.set_title(&title);
        }
        self.window.request_redraw();
    }

    fn notify_url_changed(&self, webview: WebView, url: Url) {
        if let Ok(mut tabs) = self.tabs.try_borrow_mut() {
            if let Some(tab) = tabs.find_by_webview_mut(&webview) {
                if !tab.location_dirty {
                    tab.location = url.to_string();
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


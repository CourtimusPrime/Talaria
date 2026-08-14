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
    SessionStarted { session_id: u64, client: String },
    SessionEnded { session_id: u64 },
    Agent(AgentRequest),
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
    pub rendering_context: Rc<OffscreenRenderingContext>,
    pub tabs: RefCell<TabManager>,
    pub sessions: RefCell<BTreeMap<u64, String>>,
    pub vault: RefCell<Vault>,
    /// Bottom of the chrome strip, in logical points.
    pub toolbar_height: Cell<f32>,
    /// Last cursor position, physical pixels.
    pub last_cursor: Cell<Option<PhysicalPosition<f64>>>,
    /// Last cursor position relative to the webview viewport, device pixels.
    pub webview_point: Cell<euclid::Point2D<f32, DevicePixel>>,
    pub modifiers: Cell<ModifiersState>,
    /// (hide, show) pairs queued by screenshot captures; applied on the next
    /// event-loop turn because servo callbacks run inside a painter borrow
    /// where hide()/show() would re-enter and panic.
    pub pending_visibility: RefCell<Vec<(WebView, WebView)>>,
}

impl Shared {
    pub fn apply_pending_visibility(&self) {
        for (hide, show) in self.pending_visibility.borrow_mut().drain(..) {
            hide.hide();
            show.show();
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
            self.rendering_context.clone(),
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
        let rendering_context = Rc::new(
            window_rendering_context.offscreen_context(window.inner_size()),
        );

        let servo = ServoBuilder::default()
            .event_loop_waker(Box::new(waker.clone()))
            .build();
        servo.setup_logging();

        GUI.with_borrow_mut(|gui| {
            *gui = Some(Gui::new(event_loop, rendering_context.clone()));
        });

        let state = Rc::new(Shared {
            window,
            servo,
            window_rendering_context,
            rendering_context,
            tabs: RefCell::new(TabManager::new()),
            sessions: RefCell::new(BTreeMap::new()),
            vault: RefCell::new(Vault::load()),
            toolbar_height: Cell::new(0.0),
            last_cursor: Cell::new(None),
            webview_point: Cell::new(euclid::Point2D::zero()),
            modifiers: Cell::new(ModifiersState::empty()),
            pending_visibility: RefCell::new(Vec::new()),
        });

        state.open_tab(initial_url.clone(), TabOwner::Me);
        *self = App::Running(state);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        if let Some(state) = self.state() {
            state.apply_pending_visibility();
            match event {
                AppEvent::Wake => {},
                AppEvent::SessionStarted { session_id, client } => {
                    state.sessions.borrow_mut().insert(session_id, client);
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
        }
        event_loop.set_control_flow(ControlFlow::Wait);
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
        state.apply_pending_visibility();

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
        event_loop.set_control_flow(ControlFlow::Wait);
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
                let shown = tabs.active_id(mode);
                drop(tabs);
                if let Some(id) = shown {
                    state.tabs.borrow_mut().set_active(id);
                }
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
                state.tabs.borrow_mut().close(id);
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
        }
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
            match Url::parse(&url) {
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
            match (webview_for(tab_id), Url::parse(&url)) {
                (Ok(webview), Ok(url)) => {
                    if let Some(tab) = state.tabs.borrow_mut().get_mut(tab_id) {
                        tab.location = url.to_string();
                        tab.location_dirty = false;
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
            match webview_for(tab_id) {
                Ok(webview) => {
                    // All webviews share one offscreen framebuffer, and
                    // `WebView::take_screenshot` reads whatever was composited
                    // last (i.e. the displayed tab). Instead: un-hide the
                    // target, paint it into the framebuffer, read the pixels
                    // back synchronously, then restore the displayed tab.
                    // No servo callback involved, so no painter re-entrancy.
                    let restore = state
                        .tabs
                        .borrow()
                        .displayed()
                        .map(|t| t.webview.clone())
                        .filter(|displayed| *displayed != webview);
                    if restore.is_some() {
                        webview.show();
                    }
                    webview.paint();
                    let size = state.rendering_context.size2d().to_i32();
                    let rect = euclid::Box2D::from_origin_and_size(
                        euclid::Point2D::origin(),
                        euclid::Size2D::new(size.width, size.height),
                    );
                    let image = state.rendering_context.read_to_image(rect);
                    // KNOWN ISSUE: for a tab that is not currently displayed,
                    // this still captures the displayed tab's pixels — a
                    // hidden webview's paint() appears not to reach the shared
                    // framebuffer. Screenshots of the *displayed* tab are
                    // correct. Tracked in OVERNIGHT_LOG.md.
                    if let Some(displayed) = restore {
                        webview.hide();
                        displayed.show();
                    }
                    let outcome = match image {
                        Some(image) => encode_screenshot(image),
                        None => Outcome::Error { message: "framebuffer read failed".into() },
                    };
                    let _ = reply.send(outcome);
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
        servo::JSValue::Number(n) => serde_json::Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::Null),
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
    fn notify_new_frame_ready(&self, _webview: WebView) {
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
        if let Ok(mut tabs) = self.tabs.try_borrow_mut() {
            if let Some(tab) = tabs.find_by_webview_mut(&webview) {
                tab.crashed = true;
            }
        }
        self.window.request_redraw();
    }

    fn notify_closed(&self, webview: WebView) {
        if let Ok(tabs) = self.tabs.try_borrow() {
            if let Some(id) = tabs.find_by_webview(&webview) {
                drop(tabs);
                if let Ok(mut tabs) = self.tabs.try_borrow_mut() {
                    tabs.close(id);
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


//! Talaria's remote view client — the second front end, and the thin one.
//!
//! **What this binary is.** A viewer for a *Talaria server's* agent tabs. It
//! opens a window, connects to one server over a WebSocket, authenticates with
//! a bearer credential, and shows that server's agent tabs. From `05-09` it
//! will also present their frames and send pointer and keyboard input back.
//!
//! **What it deliberately is not.** It links no web engine. There is no
//! `servo` line in this crate's manifest and none in its dependency tree, and
//! that is the whole reason this binary exists rather than the shell being made
//! headless (`D-05-01`): a client with no engine builds in seconds and runs on
//! a machine carrying none of the engine's native build prerequisites, and the
//! local path the shell measured at 4–36 ms is untouched because no local code
//! was changed to make this possible.
//!
//! It also owns no tabs and can open, navigate, evaluate, close or download
//! nothing. That is not an unfinished feature: per `D-05-02` the view channel
//! carries no command vocabulary at all — see
//! [`talaria_protocol::wire::ClientView`], whose own doc comment makes the
//! argument. A viewer lists, watches and drives; it does not manage.
//!
//! **Two clients on one machine is a legitimate state.** A reader who knows
//! `crates/talaria-shell/src/main.rs` will look here for the forward-to-the-
//! running-copy-and-exit dance it does at startup, and not find it. That dance
//! exists in the shell because one process owns the control socket *and* the
//! engine's profile directory, and a second launch stealing either would make
//! the first unreachable to agents. This binary owns neither, so there is
//! nothing to contend for and nothing to forward — launching a second copy
//! pointed at a second server is an ordinary thing to want.
//!
//! **The window's graphics context is this crate's own.** The shell builds its
//! chrome on the engine's `WindowRenderingContext`; there is no engine here, so
//! [`WindowSurface`] below builds an equivalent directly on `surfman` — the
//! same crate, at the same version, with the same features that
//! `servo-paint-api` itself uses. Everything downstream of that one line is the
//! shell's arrangement verbatim, including the icon font, so the two front ends
//! do not drift apart visually.

mod chrome;
mod net;

use std::cell::{Cell, RefCell};
use std::error::Error;
use std::sync::Arc;

use chrome::{Chrome, UiAction, View};
use egui_glow::EguiGlow;
use glow::HasContext as _;
use talaria_protocol::TabInfo;
use surfman::{
    Connection, Context, ContextAttributeFlags, ContextAttributes, Device, GLApi, SurfaceAccess,
    SurfaceType,
};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::raw_window_handle::{HasDisplayHandle as _, HasWindowHandle as _};
use winit::window::{Window, WindowId};

/// The window's initial size, matching the shell's so a human running both
/// side by side sees one product rather than two.
const INITIAL_SIZE: (f64, f64) = (1200.0, 800.0);

/// The one thing this binary is told on the command line: which server to view.
///
/// **A credential is never an argument.** See `net.rs` for where it does come
/// from and why.
/// What a geometry line on standard output is prefixed with, so a reader can
/// tell it from anything else a library might print.
const GEOMETRY_PREFIX: &str = "talaria-client-rects ";

const USAGE: &str = "usage: talaria-client <server-base-url>\n\
                     \n\
                     Example: talaria-client https://thinkpad.tailnet.ts.net\n\
                     \n\
                     The bearer credential is read from TALARIA_CLIENT_TOKEN, or from an\n\
                     owner-only token file in this client's configuration directory. It is\n\
                     never taken from the command line: on this platform every account can\n\
                     read another process's arguments.";

/// Anything the network thread has to tell the event loop.
///
/// One user-event channel, the same one-way discipline the server keeps between
/// its listener thread and its engine thread: the network thread builds one of
/// these and sends it down a cloned [`winit::event_loop::EventLoopProxy`]. It
/// never touches interface state, because a thread that did would reintroduce
/// exactly the borrow hazards the intent convention exists to prevent.
///
/// One variant, because there is one thread and it has one thing to say. What
/// varies is [`net::Update`], which is that module's vocabulary rather than
/// this one's — the loop forwards it and does not interpret it.
#[derive(Debug)]
enum ClientEvent {
    /// The connection thread reported.
    Net(net::Update),
}

/// A window-sized OpenGL surface and the `glow` context that draws into it.
///
/// This is the one construction step the shell has no analog for.
/// `crates/talaria-shell/src/gui.rs` builds `EguiGlow` from
/// `WindowRenderingContext::glow_gl_api()`, which is the engine's own context;
/// with no engine there is no such object, so this type does what
/// `servo-paint-api`'s `WindowRenderingContext` does and nothing more: make a
/// `surfman` connection from the display handle, create a device and a context,
/// bind a widget surface to it, and load `glow` through the context's own
/// address loader.
///
/// It is deliberately *smaller* than the engine's version. There is no swap
/// chain, no offscreen variant, no surface-texture path and no `gleam`
/// alongside `glow` — the client draws one interface into one window, and every
/// one of those exists to serve a compositor this binary does not have.
struct WindowSurface {
    /// Shared with `egui_glow`'s painter, which is why it is an `Arc` and not
    /// an `Rc` — the painter's own signature asks for one.
    gl: Arc<glow::Context>,
    device: RefCell<Device>,
    context: RefCell<Context>,
}

impl WindowSurface {
    /// Build a context for `window_handle` at `size`, in device pixels.
    ///
    /// Returns the `surfman` error rather than panicking: a machine with no
    /// usable GL is a real state, and the caller is the entry point, which is
    /// the one place allowed to turn that into an exit with a message.
    fn new(
        display_handle: winit::raw_window_handle::DisplayHandle<'_>,
        window_handle: winit::raw_window_handle::WindowHandle<'_>,
        size: winit::dpi::PhysicalSize<u32>,
    ) -> Result<Self, surfman::Error> {
        if size.width == 0 || size.height == 0 {
            return Err(surfman::Error::Failed);
        }
        let connection = Connection::from_display_handle(display_handle)?;
        let adapter = connection.create_adapter()?;
        let device = connection.create_device(&adapter)?;

        let flags = ContextAttributeFlags::ALPHA
            | ContextAttributeFlags::DEPTH
            | ContextAttributeFlags::STENCIL;
        // The same two version floors `servo-paint-api` picks, for the same
        // reason: they are the lowest that egui's shaders compile against on
        // each API, and asking for more narrows the set of machines this runs
        // on for nothing.
        let version = match connection.gl_api() {
            GLApi::GLES => surfman::GLVersion { major: 3, minor: 0 },
            GLApi::GL => surfman::GLVersion { major: 3, minor: 2 },
        };
        let descriptor = device.create_context_descriptor(&ContextAttributes { flags, version })?;
        let mut context = device.create_context(&descriptor, None)?;

        let native_widget = connection.create_native_widget_from_window_handle(
            window_handle,
            euclid::default::Size2D::new(size.width as i32, size.height as i32),
        )?;
        let surface = device.create_surface(
            &context,
            SurfaceAccess::GPUOnly,
            SurfaceType::Widget { native_widget },
        )?;
        // On failure the surface comes back with the error so it can be
        // destroyed rather than leaked — `surfman`'s own convention, and the
        // shape `servo-paint-api` uses at the same call.
        if let Err((error, mut surface)) = device.bind_surface_to_context(&mut context, surface) {
            let _ = device.destroy_surface(&mut context, &mut surface);
            let _ = device.destroy_context(&mut context);
            return Err(error);
        }
        device.make_context_current(&context)?;

        // SAFETY: `get_proc_address` resolves symbols against the context made
        // current on the line above, and the returned `glow::Context` is used
        // only while that context is current — `make_current` runs before every
        // frame below. This is the same call, with the same invariant, that
        // `servo-paint-api` makes to build the context the shell's chrome draws
        // through.
        let gl = unsafe {
            glow::Context::from_loader_function(|symbol| device.get_proc_address(&context, symbol))
        };

        Ok(Self {
            gl: Arc::new(gl),
            device: RefCell::new(device),
            context: RefCell::new(context),
        })
    }

    /// Make this the current GL context for the thread. Ignored on failure for
    /// the reason the shell ignores it: a context that will not become current
    /// makes the next frame a no-op, which is a blank window, not a crash.
    fn make_current(&self) {
        let _ = self.device.borrow().make_context_current(&self.context.borrow());
    }

    /// Point GL at the surface's framebuffer before drawing into it.
    ///
    /// Without this the first frame lands on framebuffer 0 — which on a bound
    /// `surfman` widget surface is not the surface — and the window stays
    /// blank while everything reports success.
    fn prepare_for_rendering(&self) {
        let device = self.device.borrow();
        let context = self.context.borrow();
        let framebuffer = device
            .context_surface_info(&context)
            .unwrap_or(None)
            .and_then(|info| info.framebuffer_object);
        // SAFETY: `glow`'s whole surface is `unsafe`; the context is current
        // (every caller calls `make_current` first) and the framebuffer name
        // came from the device that owns it.
        unsafe { self.gl.bind_framebuffer(glow::FRAMEBUFFER, framebuffer) };
    }

    /// Resize the bound surface. A zero dimension — a minimised window on some
    /// platforms — is skipped rather than passed down, because `surfman`
    /// refuses it and the refusal is not interesting.
    fn resize(&self, size: winit::dpi::PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        let size = euclid::default::Size2D::new(size.width as i32, size.height as i32);
        let device = self.device.borrow();
        if let Err(error) = device.resize_bound_surface(&mut self.context.borrow_mut(), size) {
            log::warn!("could not resize the window surface: {error:?}");
        }
    }

    /// Swap the drawn frame to the window.
    fn present(&self) {
        let device = self.device.borrow();
        if let Err(error) = device.present_bound_surface(&mut self.context.borrow_mut()) {
            log::warn!("could not present a frame: {error:?}");
        }
    }
}

impl Drop for WindowSurface {
    fn drop(&mut self) {
        let device = self.device.borrow();
        let _ = device.destroy_context(&mut self.context.borrow_mut());
    }
}

/// The two states a winit application is in, copied from the shell's `App`:
/// everything a window is needed for has to wait for `resumed`, and on some
/// platforms that is the only moment a window may be created at all.
enum App {
    /// Before the first `resumed`. Holds what the running state will need —
    /// including the connection state and the tab list, because the connection
    /// thread starts before the loop runs and its first reports can land before
    /// there is a window to draw them in. Dropping those would leave a client
    /// that connected during startup showing "not started" forever.
    Initial {
        server: String,
        endpoint: String,
        proxy: winit::event_loop::EventLoopProxy<ClientEvent>,
        state: net::ConnectionState,
        tabs: Vec<TabInfo>,
    },
    /// After it. Owns the window, its surface and its interface.
    ///
    /// Boxed because [`Running`] holds `EguiGlow`, which is several kilobytes,
    /// and an enum sized to its largest variant would make every move of this
    /// value a multi-kilobyte copy. The shell's own `App::Running` sidesteps
    /// this by holding an `Rc<Shared>`; this crate has no such handle, so the
    /// indirection is spelled out.
    Running(Box<Running>),
}

/// Everything the client owns once it has a window.
///
/// The shell keeps its `Gui` in a `thread_local!` because it cannot live on the
/// `Rc<Shared>` every callback holds. **That is not copied here**, deliberately:
/// this binary has no `Rc<Shared>` and no engine callbacks, so the interface can
/// simply be a field, and copying the workaround without the problem would be
/// cargo-culting a borrow hazard's cure into a crate that has no borrow hazard.
///
/// Field order is drop order, and it is chosen: the interface releases its GL
/// resources first, then the surface destroys its context, and the window —
/// whose native handle both of those were built against — goes last.
struct Running {
    egui: EguiGlow,
    /// The interface. It lives here as a plain field, not in a
    /// `thread_local!` — see the type's doc above.
    chrome: Chrome,
    surface: WindowSurface,
    window: Window,
    /// The server this client was pointed at, as the human typed it.
    server: String,
    /// The endpoint derived from it once, at startup, and kept so a reconnect
    /// opens the same socket rather than re-deriving and possibly differing.
    endpoint: String,
    /// A proxy for the connection thread a reconnect spawns.
    proxy: winit::event_loop::EventLoopProxy<ClientEvent>,
    /// Where the connection is. The only thing the interface knows about the
    /// network.
    state: net::ConnectionState,
    /// The server's agent tabs, held **in the order the server sent them**.
    tabs: Vec<TabInfo>,
    /// Set by a redraw request and cleared once the frame is drawn, so one
    /// window event produces at most one frame.
    redraw: Cell<bool>,
}

impl Running {
    /// Draw one frame and apply what the human asked for.
    ///
    /// The intents are collected inside the closure and applied **after** it
    /// returns, which is the whole point of the convention: egui holds borrows
    /// for the duration of the frame, and the state a control was drawn from is
    /// one frame old by the time the click is read.
    fn draw(&mut self) {
        self.surface.make_current();
        // The fields below are captured individually rather than through
        // `self`, which is what lets the interface be borrowed mutably while
        // `self.egui` is: edition 2021 closures capture disjoint fields, so
        // nothing here needs the move-out-and-back the server's chrome does for
        // its own view state.
        let Running { egui, chrome, window, server, state, tabs, .. } = self;
        let credential_file = net::credential_file();
        let mut actions = Vec::new();
        egui.run(window, |ui| {
            actions = chrome.update(ui, &View {
                server,
                state,
                tabs,
                credential_file: credential_file.as_deref(),
            });
        });
        self.surface.prepare_for_rendering();
        self.egui.paint(&self.window);
        self.surface.present();
        self.report_geometry();
        self.apply(actions);
    }

    /// Apply the frame's intents, now that the interface's borrows have dropped.
    fn apply(&mut self, actions: Vec<UiAction>) {
        for action in actions {
            match action {
                UiAction::Reconnect => {
                    self.state = net::ConnectionState::NotStarted;
                    self.tabs.clear();
                    net::spawn(self.endpoint.clone(), report_to(self.proxy.clone()));
                    self.window.request_redraw();
                },
            }
        }
    }

    /// Under the test hook, put this frame's control geometry on standard
    /// output as one line.
    ///
    /// **This is also the client's readiness signal**, and it has to be its own
    /// rather than the shell's: `harness.start_shell` waits for the control
    /// socket to appear, and a client owns no socket to wait for. A line the
    /// client itself writes after a frame has actually been drawn is a stronger
    /// signal anyway — the process being alive says nothing about whether GL
    /// came up.
    ///
    /// Gated on the same variable and for the same reason as the server's
    /// geometry hook: a production frame builds nothing and writes nothing.
    fn report_geometry(&self) {
        let rects = self.chrome.chrome_rects();
        if rects.is_empty() {
            return;
        }
        if let Ok(line) = serde_json::to_string(rects) {
            println!("{GEOMETRY_PREFIX}{line}");
            // Flushed, because standard output is block-buffered when it is a
            // pipe — which is exactly what the harness gives it, so an unflushed
            // readiness line would arrive only when the client exited.
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.surface.make_current();
        self.egui.destroy();
    }
}

impl ApplicationHandler<ClientEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let App::Initial { server, endpoint, proxy, state, tabs } = self else {
            return;
        };
        let server = std::mem::take(server);
        let endpoint = std::mem::take(endpoint);
        let proxy = proxy.clone();
        let state = state.clone();
        let tabs = std::mem::take(tabs);
        let display_handle = event_loop.display_handle().expect("display handle");
        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("Talaria — remote view")
                    .with_inner_size(winit::dpi::LogicalSize::new(
                        INITIAL_SIZE.0,
                        INITIAL_SIZE.1,
                    )),
            )
            .expect("create window");
        let window_handle = window.window_handle().expect("window handle");
        let surface = WindowSurface::new(display_handle, window_handle, window.inner_size())
            .expect("create window surface");
        surface.make_current();

        let egui = EguiGlow::new(event_loop, surface.gl.clone(), None, None, false);
        let mut fonts = egui::FontDefinitions::default();
        // The same icon font the shell registers, registered the same way, so
        // the two front ends do not diverge visually.
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        egui.egui_ctx.set_fonts(fonts);

        *self = App::Running(Box::new(Running {
            egui,
            chrome: Chrome::new(),
            surface,
            window,
            server,
            endpoint,
            proxy,
            state,
            tabs,
            redraw: Cell::new(true),
        }));
    }

    /// Drain whatever arrived off-loop, then decide how long to sleep.
    ///
    /// The shell's `new_events` drains its pending queues here and then sets its
    /// wait; this is the same discipline with nothing yet to drain. `05-09`'s
    /// frame queue and `05-10`'s cadence deadline are what fill it in.
    fn new_events(&mut self, event_loop: &ActiveEventLoop, _cause: winit::event::StartCause) {
        event_loop.set_control_flow(ControlFlow::Wait);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: ClientEvent) {
        let ClientEvent::Net(update) = event;
        // Handled in both arms on purpose: an update that arrives before the
        // first `resumed` is stashed rather than dropped. See
        // [`App::Initial`].
        let (state, tabs) = match self {
            App::Initial { state, tabs, .. } => (state, tabs),
            App::Running(running) => (&mut running.state, &mut running.tabs),
        };
        match update {
            net::Update::State(next) => *state = next,
            // Assigned wholesale, never merged and never sorted: the snapshot
            // the server sent **is** the list, in the server's order.
            net::Update::Tabs(next) => *tabs = next,
        }
        if let App::Running(running) = self {
            running.window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let App::Running(running) = self else {
            return;
        };
        match &event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            },
            WindowEvent::Resized(size) => {
                running.surface.resize(*size);
                running.redraw.set(true);
            },
            WindowEvent::RedrawRequested => running.redraw.set(true),
            _ => {},
        }
        let response = running.egui.on_window_event(&running.window, &event);
        if response.repaint && !matches!(event, WindowEvent::RedrawRequested) {
            running.window.request_redraw();
        }
        if running.redraw.replace(false) {
            running.draw();
        }
    }
}

/// The one bridge from the connection thread to the loop.
///
/// A named function rather than a closure at each call site, so the spawn in
/// `main` and the spawn a reconnect performs cannot come to differ.
fn report_to(
    proxy: winit::event_loop::EventLoopProxy<ClientEvent>,
) -> impl Fn(net::Update) + Send + 'static {
    move |update| {
        // Ignored explicitly: a send that fails means the loop is already
        // gone, and there is nothing for a thread on its way out to do about
        // that.
        let _ = proxy.send_event(ClientEvent::Net(update));
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // First, and load-bearing — but for a different reason than the identical
    // line in `crates/talaria-shell/src/main.rs`. There it is installed for the
    // engine's own network stack; here there is no engine, and this is for
    // *this binary's own* transport security: the `wss://` connection in
    // `net.rs` verifies the server's certificate through `rustls`, and `rustls`
    // refuses to run without a process-wide crypto provider.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("install crypto provider");

    // The client installs its own logger, where the shell installs none.
    //
    // The shell's comment says it defers to `servo.setup_logging()`, which
    // installs the global logger and panics if one already exists. That reason
    // does not transfer: there is no engine here to defer to, so without this
    // line every `log::` call in this binary would go nowhere and a connection
    // that failed for a nameable reason would leave no trace outside the
    // window. `RUST_LOG` selects the level, exactly as it does for the shell.
    //
    // `try_init` rather than `init`: a logger already installed is not a reason
    // to refuse to start a browser client.
    let _ = env_logger::try_init();

    let mut arguments = std::env::args().skip(1);
    let (Some(server), None) = (arguments.next(), arguments.next()) else {
        // Direct printing, in the entry point, before the loop — the one place
        // the shell reserves it for.
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    // Parsed here only to refuse an unusable argument early and readably; the
    // endpoint the client actually connects to is derived from this same string
    // in `net.rs`, from this one source, so the two cannot disagree.
    if url::Url::parse(&server).is_err() {
        eprintln!("talaria-client: {server:?} is not a URL.\n\n{USAGE}");
        std::process::exit(2);
    }

    // Derived from the one address the human gave, so the address on screen and
    // the socket that opens cannot disagree. Refused here, before a window
    // exists, because a base URL this client will not use is a mistake to
    // correct at the shell rather than a state to render.
    let Some(endpoint) = net::view_endpoint(&server) else {
        eprintln!(
            "talaria-client: {server:?} is not a server address this client will connect to.\n\n\
             Use an https:// address, or an http:// address on this machine's own loopback.\n\
             An http:// address anywhere else would put the credential on the wire in the clear,\n\
             so it is refused rather than downgraded.\n\n{USAGE}"
        );
        std::process::exit(2);
    };

    let event_loop = EventLoop::<ClientEvent>::with_user_event().build()?;
    // Spawned with a cloned proxy **before** the loop runs, so the connection
    // is already in flight by the time the window draws its first frame.
    let proxy = event_loop.create_proxy();
    net::spawn(endpoint.clone(), report_to(proxy.clone()));
    let mut app = App::Initial {
        server,
        endpoint,
        proxy,
        state: net::ConnectionState::NotStarted,
        tabs: Vec::new(),
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

# Phase 5: Distributed Mode (v2) — Pattern Map

**Mapped:** 2026-08-21
**Scope:** DIST-01 + DIST-02 only (D-05-04). Nothing here maps `manifest.rs` or reconnect/resync — those are Phase 5.1.
**Files analyzed:** 12 new/modified
**Analogs found:** 11 / 12 (one partial, one with no analog)

---

## File Classification

| New/Modified File | Role | Data Flow | Closest Analog | Match Quality |
|-------------------|------|-----------|----------------|---------------|
| `Cargo.toml` (workspace) — `axum` w/ `ws`, `talaria-client` member | config | — | itself, `Cargo.toml:35-92` | exact |
| `crates/talaria-client/Cargo.toml` | config | — | `crates/talaria-shell/Cargo.toml:1-45` | exact |
| `crates/talaria-client/src/main.rs` | entry point | event-driven | `crates/talaria-shell/src/main.rs`, `app.rs:906-1010` | role-match (no Servo) |
| `crates/talaria-client/src/present.rs` | component (GL/egui) | streaming | `crates/talaria-shell/src/gui.rs:516-557, 2504-2527` | partial |
| `crates/talaria-client/src/net.rs` | service | streaming / pub-sub | `crates/talaria-mcp/src/socket.rs`, `control.rs:143-200` | role-match |
| `crates/talaria-client/src/input.rs` | utility | event-driven | `crates/talaria-shell/src/app.rs:1316-1394` | role-match |
| `crates/talaria-client/src/rate.rs` | service (controller) | transform | — | **no analog** |
| `crates/talaria-protocol/src/wire.rs` | model (wire types) | request-response + streaming | `crates/talaria-protocol/src/lib.rs:73-238` | exact |
| `crates/talaria-protocol/src/local.rs` (move, `cfg(unix)`) | utility | file-I/O | `crates/talaria-protocol/src/lib.rs:20-71` (verbatim move) | exact |
| `crates/talaria-shell/src/http.rs` — `/view` route | route + middleware | streaming | `http.rs:1044-1170` (`build_router`), `:1256+` (`note_streaming_connection`) | exact |
| `crates/talaria-shell/src/view.rs` (frame pump, tile diff, encoder thread) | service | streaming | `app.rs:399-421` (`capture_now`), `:2494-2519` (`encode_screenshot`), `control.rs:85-95` (`spawn`) | role-match |
| `crates/talaria-shell/src/remote_input.rs` | middleware (validation) | event-driven | `app.rs:1316-1394`, `keyutils.rs:11-62`, `settings.rs:110-116` | role-match |
| `crates/talaria-shell/src/keyutils.rs` — `keyboard_event_from_wire` | utility | transform | `keyutils.rs:11-62` (same file, sibling fn) | exact |
| `crates/talaria-shell/src/app.rs` — `forward_*` generalised, new `AppEvent` variants | controller | event-driven | `app.rs:49-110` (`AppEvent`), `:1316-1394` | exact |
| `crates/talaria-shell/src/oauth.rs` + `http.rs` — advertised URL | config/security | request-response | `oauth.rs:143-155, 1160-1163`; `http.rs:1103` | exact |
| `tests/e2e/remote_view_test.py` | test | request-response | `tests/e2e/revocation_test.py:165-326` | exact |
| `tests/e2e/remote_latency_test.py` | test | streaming | `tests/e2e/revocation_test.py:271-320` (`_pump`) | role-match |
| `tests/e2e/harness.py` — `CLIENT_BINARY`, ws helper | test utility | — | `harness.py:40-63, 146-158` | exact |
| `tests/e2e/run_all.py` — register suites | test config | — | `run_all.py:73-77` | exact |

---

## Pattern Assignments

### `crates/talaria-client/` — the new binary crate (entry point, event-driven)

**Analog:** `crates/talaria-shell` — but only three of its four startup concerns.

**Workspace membership** (`Cargo.toml:1-7`) — add the member; the list is unsorted-but-grouped, append:

```toml
[workspace]
resolver = "2"
members = [
    "crates/talaria-shell",
    "crates/talaria-protocol",
    "crates/talaria-mcp",
]
```

**Crate manifest** — copy the shape of `crates/talaria-shell/Cargo.toml:1-11`, including the explicit `[[bin]]` (the shell's binary is `talaria`, not `talaria-shell`; the client's should likewise be named deliberately):

```toml
[package]
name = "talaria-shell"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Talaria — a lightweight, lightning-fast web browser for humans and agents"

[[bin]]
name = "talaria"
path = "src/main.rs"
```

Every dependency must be `{ workspace = true }` (`crates/talaria-shell/Cargo.toml:12-44`) — no version strings in a member manifest. The one exception in the tree is `log = "0.4"` at `Cargo.toml:45`, which is a wart, not a pattern; declare `log` in `[workspace.dependencies]` if the client needs it.

**Startup order** — `crates/talaria-shell/src/main.rs:27-66`:

```rust
fn main() -> Result<(), Box<dyn Error>> {
    // No env_logger here: servo's `setup_logging()` installs the global
    // logger (and panics if one is already set); it honors RUST_LOG and
    // covers our own log:: macros too.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("install crypto provider");
    ...
    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    control::spawn(event_loop.create_proxy());
    let mut app = App::new(&event_loop, url);
    event_loop.run_app(&mut app)?;
    Ok(())
}
```

**What the client copies:** `Result<(), Box<dyn Error>>` on `main`; `EventLoop::<ClientEvent>::with_user_event()`; spawn the network thread with the proxy *before* `run_app`; `expect()` only for startup invariants.

**What the client copies with a changed reason:** the `rustls` provider install stays first and stays load-bearing — but for the *client's own TLS to `wss://`*, not for Servo. The comment must be rewritten; copying the "servo's `setup_logging()`" comment into a crate with no Servo would be a lie. The client has no `setup_logging()` to defer to, so it needs its own logger init (or none).

**What the client must NOT copy:**
- `control::forward_to_running_instance` and the whole single-instance dance (`main.rs:51-60`) — that exists because one process owns the Unix socket *and the Servo profile dir*. A client owns neither. Two clients on one machine is a legitimate state.
- `servo::Opts { config_dir }` + `ServoBuilder` + `servo.setup_logging()` (`app.rs:927-943`) — the client links no Servo.
- `WindowRenderingContext::new(...)` from `servo` (`app.rs:911-919`) — this is Servo's GL context type. The client needs its own `glutin`/`glow` surface, or `egui_glow`'s own winit integration. **This is the single genuine gap in the analog and should be treated as a small spike inside the client plan**, not assumed to fall out of `Gui::new`.
- The default-URL/search-engine resolution (`main.rs:35-45`) — the client's argument is a server URL, not a page.

**`ApplicationHandler` shape** — `app.rs:906-1010`. Copy the two-state enum (`App::Initial { .. } / App::Running(state)`) and the `fn resumed` window creation:

```rust
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
```

and the drain-then-set-wait discipline at `app.rs:999-1007`:

```rust
    fn new_events(&mut self, event_loop: &ActiveEventLoop, _cause: winit::event::StartCause) {
        if let Some(state) = self.state() {
            state.process_pending_captures();
            state.process_pending_evals();
            state.process_pending_history_writes();
            set_wait(event_loop, state);
        }
    }
```

The client's equivalent drains decoded frames and applies them to a texture. The `thread_local! { static GUI: RefCell<Option<Gui>> }` idiom (`app.rs:902-904`) exists because `Gui` cannot live on the `Rc<Shared>`; the client should only copy it if it hits the same borrow problem — do not copy it reflexively.

**egui-into-GL setup** — `gui.rs:516-530`:

```rust
impl Gui {
    pub fn new(
        event_loop: &ActiveEventLoop,
        rendering_context: Rc<WindowRenderingContext>,
    ) -> Self {
        let _ = rendering_context.make_current();
        let context = EguiGlow::new(
            event_loop,
            rendering_context.glow_gl_api(),
            None,
            None,
            false,
        );
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        context.egui_ctx.set_fonts(fonts);
```

`rendering_context.glow_gl_api()` is the Servo-supplied `Arc<glow::Context>`; the client substitutes its own. Everything after that line transfers verbatim, including the phosphor font registration — the client's chrome should use the same icon font so the two front ends do not diverge visually.

**Presenting the frame** — the shell's blit, `gui.rs:2504-2527`, is the shape the client's texture present replaces:

```rust
            if let Some((webview, tab_context)) = displayed {
                let width = (available.width() * scale).round().max(1.0) as u32;
                let height = (available.height() * scale).round().max(1.0) as u32;
                let current = webview.size();
                if (current.width as u32, current.height as u32) != (width, height) {
                    webview.resize(PhysicalSize::new(width, height));
                }
                webview.paint();

                if let Some(render_to_parent) = tab_context.render_to_parent_callback() {
                    ctx.layer_painter(LayerId::background()).add(PaintCallback {
                        rect: available,
                        callback: Arc::new(CallbackFn::new(move |info, painter| {
                            let clip = info.viewport_in_pixels();
                            let rect = euclid::Rect::new(
                                euclid::Point2D::new(clip.left_px, clip.from_bottom_px),
                                euclid::Size2D::new(clip.width_px, clip.height_px),
                            );
                            render_to_parent(painter.gl(), rect);
                        })),
                    });
                }
            }
```

The `PaintCallback` into `LayerId::background()` is exactly the right seam for the client: the client's callback uploads decoded tiles into a `glow` texture and draws it, instead of calling Servo's `render_to_parent`. Note the resize-on-mismatch above it — the client's equivalent is *telling the server* the new viewport size, which forces a keyframe.

**`UiAction` discipline** — `gui.rs:709` (`pub fn update(&mut self, shared: &Rc<Shared>) -> Vec<UiAction>`), applied at `app.rs:1455`. The client's chrome must return intents, not mutate. This is a CLAUDE.md-level convention, not a preference.

---

### `crates/talaria-shell/src/http.rs` — the `/view` WebSocket route (route, streaming)

**Analog:** itself. Every hook the route needs already exists.

**Where the route mounts** (`http.rs:1146-1166`) — `mcp_routes(...)` builds the router, then two `.layer(...)` calls wrap it. A `/view` route is `.route("/view", get(view_upgrade))` merged into the `mcp_routes` result **before** those two layers, so it inherits both:

```rust
    let tracking = StreamTracking { connections, state: Arc::clone(&state) };
    let router = mcp_routes(state, &mount, http_handler)
        .layer(rust_mcp_axum::axum::middleware::from_fn_with_state(
            tracking,
            note_streaming_connection,
        ))
        // **Outermost, so it runs first and covers every route this process
        // serves** — including the authorization-server routes, which the SDK
        // dispatches on `compose(&[], ..)` and which therefore carry none of
        // the chain above.
        .layer(rust_mcp_axum::axum::middleware::from_fn_with_state(
            Arc::<str>::from(bound),
            refuse_page_originated,
        ));
    (router, directory)
```

**The layer the route inherits** (`http.rs:1198-1218`) — this is the CSWSH defence and the reason a browser viewer is structurally impossible:

```rust
async fn refuse_page_originated(
    rust_mcp_axum::axum::extract::State(bound): rust_mcp_axum::axum::extract::State<Arc<str>>,
    request: rust_mcp_axum::axum::extract::Request,
    next: rust_mcp_axum::axum::middleware::Next,
) -> rust_mcp_axum::axum::response::Response {
    if !is_public_discovery(request.uri().path()) {
        let headers = request.headers();
        let addressed_here = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|host| host.eq_ignore_ascii_case(&bound));
        if headers.contains_key(header::ORIGIN) || !addressed_here {
            log::debug!(
                "remote access refused a page-originated or misaddressed request to {}",
                request.uri().path()
            );
            return page_originated_refusal();
        }
    }
    next.run(request).await
}
```

**`/view` must NOT be added to the allowlist.** `PUBLIC_DISCOVERY_PATHS` (`http.rs:243`, checked by `is_public_discovery` at `:252-254`) is a 2-element array of metadata paths — a fixed-size array, so adding an entry is a deliberate, visible edit. Leave it at 2. `http.rs:1682` (`discovery_is_public_and_every_other_route_is_origin_and_host_checked`) is the existing test that pins this; extend it to name `/view`.

**Authentication** — the route must go through the same `AuthMiddleware`. The chain is built at `http.rs:1101-1106`:

```rust
    let middlewares: Vec<Arc<dyn Middleware>> = vec![
        Arc::new(RefuseOriginHeader),
        Arc::new(DnsRebindProtector::new(Some(vec![bound.to_owned()]), None)),
        Arc::new(AuthMiddleware::new(auth.clone())),
        Arc::new(NoteVerifiedIdentity),
    ];
    let http_handler = McpHttpHandler::new(Some(auth), middlewares, None);
```

**The trap the planner must carry:** that `Vec` is handed to `McpHttpHandler`, which the SDK composes only for *its own* transport handlers. An axum route merged into the router does **not** pass through it — that is the whole reason `refuse_page_originated` was lifted out to an axum layer in the first place (see the comment at `http.rs:1156-1160`). So `/view` needs its bearer check written as an axum extractor/layer that calls the same `Arc<dyn AuthProvider>` (`TalariaAuth`), not as an assumption that the SDK chain covers it. Audience is compared byte-for-byte against `oauth::canonical_resource(bound)` (`oauth.rs:143-145`), so a viewer token minted for a different resource is refused by the existing code — do not add a second comparison.

**Revocation must close the WebSocket.** `note_streaming_connection` (`http.rs:1256+`) is the machinery, and it currently gates on two exact paths:

```rust
    let get = request.method() == Method::GET;
    let streaming = get && request.uri().path() == MCP_PATH;
    let sse_handshake = get && request.uri().path() == SSE_PATH;
```

The connection descriptor comes from axum's `ConnectInfo`, never the client's word (`http.rs:1280-1283`), and is torn down by `Connections::disconnect` (`http.rs:586`) / `StreamRegistry::terminate_client` (`http.rs:724`). A `/view` upgrade is a third shape and must be registered the same way, or a revoked viewer keeps receiving frames while the Access panel shows the row gone — the exact CR-02 failure documented at `http.rs:1265-1272`.

---

### `crates/talaria-shell/src/view.rs` — frame pump + tile diff + encoder thread (service, streaming)

**Analog for the capture:** `app.rs:399-421`.

```rust
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
```

**Copy:** the `paint()` → `size2d().to_i32()` → `Box2D::from_origin_and_size` → `read_to_image(rect)` sequence, and the `Option` return meaning "framebuffer read failed".
**Do not copy:** `hide_after: true` per tick (the lease holds `show()` for the session), and the `encode_screenshot` call at the end — the pump hands the `RgbaImage` to a channel instead.

**Analog for the encoder, and the thing to sit beside without disturbing:** `app.rs:2494-2519`.

```rust
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
```

**This function stays byte-identical.** It is `Compression::Default` by omission — measured at 21 ms page / 175 ms photo (D-05-05) — and that is correct for a one-shot lossless agent screenshot. The new encoder in `view.rs` copies its *structure* (builder, `set_color`, `set_depth`, `write_header`, `write_image_data`, `let Ok(...) else` on each fallible step) and diverges on exactly three lines: add `encoder.set_compression(png::Compression::Fast)`, add `encoder.set_filter(png::FilterType::Sub)`, and drop the base64 entirely — the frame channel is binary.

The one existing call site is `capture_now` (`app.rs:419`). The `Command::Screenshot` dispatch that reaches it is `app.rs:2189-2229`, including the `pending_captures` push with the 1.5 s deadline:

```rust
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
```

**The pump must not use this path** (RESEARCH § Anti-patterns): a static page never produces `notify_new_frame_ready`, so at 33 Hz this is 33 dead 1.5 s deadlines a second. Paint unconditionally on the tick and let the tile diff decide.

**Analog for the encoder thread:** `control.rs:85-95` is the minimal spawn; `http.rs:864-893` is the richer one with degrade-on-failure.

```rust
pub fn spawn(proxy: EventLoopProxy<AppEvent>) {
    std::thread::Builder::new()
        .name("talaria-control".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("control socket runtime");
            runtime.block_on(serve(proxy));
        })
        .expect("spawn control thread");
}
```

vs. `http.rs:870-893`, which is the one to copy because the frame pump must degrade rather than abort:

```rust
    let thread = std::thread::Builder::new()
        .name("talaria-http".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(error) => {
                    // Degrade, never abort: a browser that cannot start a
                    // listener is still a browser.
                    log::error!("remote access runtime could not be built: {error}");
                    let _ = proxy.send_event(AppEvent::RemoteListenerFailed {
                        addr: format!("{BIND_HOST}:{port}"),
                        error: error.to_string(),
                    });
                    return;
                },
            };
            runtime.block_on(serve(proxy, agents, port, shutdown_rx, installed));
        });
    if let Err(error) = thread {
        log::error!("remote access thread could not be spawned: {error}");
    }
```

Note the named thread (`talaria-http`, `talaria-control` → `talaria-frames`), the `match`-on-build with an `AppEvent` report, and `if let Err` on the spawn itself rather than `expect`.

**Pending-queue idiom** — `app.rs:423-451` is the model for anything the pump defers to the loop:

```rust
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
```

The scoped `borrow_mut` that is dropped before the work runs is the load-bearing part — `Shared`'s `RefCell`s are borrowed from Servo callbacks too. Callbacks themselves use `try_borrow_mut` and skip (`app.rs:2816-2827`):

```rust
        match self.pending_captures.try_borrow_mut() {
            ...
            Err(_) => log::error!("capture-ready mark lost: pending_captures busy"),
```

**Tick scheduling** — `next_capture_deadline` (`app.rs:678-680`) feeds `set_wait` at `app.rs:1217`. The pump's cadence must join that computation, not install a second `ControlFlow` source.

**Cross-thread route** — `Shared::event_proxy`, documented at `app.rs:126-143`:

```rust
    /// `Shared` is `Rc`-based and therefore not `Send`,
    /// so a spawned thread cannot touch any of it; cloning this proxy *before*
    /// the spawn and sending an [`AppEvent`] back is the only route, and
    /// `user_event` is the only place that route lands.
    pub event_proxy: EventLoopProxy<AppEvent>,
```

and the clone-before-spawn at `app.rs:1408-1412`:

```rust
    // Cloned out here, before the spawn: `state` is an `Rc<Shared>` and `Rc`
    // is not `Send`, so the closure inside `http::spawn` can never borrow it.
    // The proxy is the whole of what crosses the thread boundary.
    let proxy = state.event_proxy.clone();
```

New `AppEvent` variants follow `app.rs:49-110` — struct variants with named fields and a `///` doc comment that says what the field means and what it must never be used for. `RemoteListenerBound { addr }`'s comment is the template:

```rust
    /// The remote MCP listener bound, raised from the `talaria-http` thread.
    ///
    /// `addr` is the address that was **actually bound**, read back off the
    /// socket — never the configured one.
    RemoteListenerBound { addr: String },
```

---

### `crates/talaria-shell/src/remote_input.rs` + `app.rs` `forward_*` (middleware, event-driven)

**Analog:** `app.rs:1316-1394`. All three functions, verbatim, because all three change identically.

```rust
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
        euclid::Size2D::new(size.width, size.height),
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
```

```rust
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
        euclid::Size2D::new(size.width, size.height),
    );
    if !viewport.contains(point) {
        return;
    }

    let mouse_button = match button {
        winit::event::MouseButton::Left => ServoMouseButton::Left,
        ...
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
```

```rust
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
```

**Three concrete facts a remote input frame must land on the same place as a local one:**

1. **`state.tabs.borrow().displayed()` is line 1 of all three.** The generalisation is to take `&WebView` + a viewport point; the local caller resolves `displayed()` and subtracts the toolbar first. `notify_input_event` is a `WebView` method, so this is a signature change, not a redesign.
2. **`point.y -= state.toolbar_height_device()`** (`app.rs:1321`) appears in `forward_mouse_move` only — and `forward_mouse_button`/`forward_wheel` inherit it via `state.webview_point.get()`, which `forward_mouse_move` set. So the offset reaches all three through one cached `Cell`. `toolbar_height_device` is `app.rs:863-865`:
   ```rust
       fn toolbar_height_device(&self) -> f32 {
           self.toolbar_height.get() * self.hidpi_scale()
       }
   ```
   A remote client draws no server toolbar. Applying this to remote coordinates is a silent ~40 px offset on every remote click. It also means **remote input must not write `state.webview_point`** — that `Cell` is the local cursor and sharing it would cross-contaminate the two input sources.
3. **`viewport.contains(point)`** is the clamp that already exists and is exactly the check untrusted wire coordinates need — run it against the *target tab's* `webview.size()`, never the displayed tab's.

**The owner check goes at the tab lookup, not after it.** `tabs.rs:35-37` and `tabs.rs:292-294`:

```rust
    pub fn is_agent(&self) -> bool {
        matches!(self, TabOwner::Agent { .. })
    }
```
```rust
    pub fn agent_tabs(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().filter(|t| t.owner.is_agent())
    }
```

D-05-02 wants a lookup that returns `None` for a Me tab — structural, not checked. The `Option<&T>` lookup convention is already `tabs.rs:223`:

```rust
    pub fn get(&self, id: u64) -> Option<&Tab> {
```

so `fn agent_tab(&self, id: u64) -> Option<&Tab>` beside it is idiomatic and is the whole of the filter. `me_tabs`/`agent_tabs` (`tabs.rs:288-294`) is the precedent for a filtered accessor pair.

**Chrome unreachable:** the remote path must reach `webview.notify_input_event` and nothing else — not `handle_browser_shortcut` (`app.rs:1246`), not `Gui`, not `apply_ui_actions` (`app.rs:1455`). The local `window_event` arms at `app.rs:1145-1177` route to the chrome *first*; the remote path must have no equivalent branch. Structurally: put the remote entry in `remote_input.rs`, and let it call only the generalised `forward_*`.

**Keyboard** — `keyutils.rs:11-62` is the whole file and the whole table:

```rust
pub fn keyboard_event_from_winit(event: &KeyEvent) -> Option<KeyboardEvent> {
    let state = match event.state {
        ElementState::Pressed => KeyState::Down,
        ElementState::Released => KeyState::Up,
    };

    let key = match &event.logical_key {
        WinitKey::Character(text) => Key::Character(text.to_string()),
        WinitKey::Named(named) => Key::Named(match named {
            WinitNamedKey::Enter => NamedKey::Enter,
            ...
            WinitNamedKey::Space => return Some(KeyboardEvent::from_state_and_key(
                state,
                Key::Character(" ".to_owned()),
            )),
            _ => return None,
        }),
        _ => return None,
    };

    Some(KeyboardEvent::from_state_and_key(state, key))
}
```

`keyboard_event_from_wire(state, key: Option<&str>, named: Option<&str>) -> Option<KeyboardEvent>` sits beside it in the same file, sharing the `NamedKey` arms (extract the `WinitNamedKey → NamedKey` match into a `&str → NamedKey` table both call, or the two will drift). `-> Option<_>` with `return None` for anything unmapped is the existing failure mode and the right one for the wire: unknown key name is a refusal, not a default. Do **not** put `winit::KeyEvent` on the wire — not all its fields are constructible.

**Hand-mapped parsing, no serde derive in the shell** — `settings.rs:110-116`:

```rust
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            name: value.get("name")?.as_str()?.to_owned(),
            url_template: value.get("url_template")?.as_str()?.to_owned(),
        })
    }
```

`to_json` sits at `settings.rs:95` immediately above it. Keep the pair adjacent so a field added to one is visibly missing from the other. `?` on every field is what makes a malformed message a refusal rather than a defaulted struct — the validation table row "malformed wire input is refused, not defaulted" is asserting exactly this shape.

---

### `crates/talaria-protocol/src/wire.rs` and `local.rs` (model)

**Analog:** the same crate, `lib.rs`. `talaria-protocol` **does** derive (unlike the shell) — `crates/talaria-protocol/Cargo.toml:8-10`:

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

**Envelope tagging convention** — copy exactly (`lib.rs:73-90`, `:143-166`):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    ...
        #[serde(flatten)]
```
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
```
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
```

Internally tagged by a domain-specific key (`type` / `command` / `outcome` / `event`), `rename_all = "snake_case"`, `#[serde(flatten)]` on the nested envelope, `#[serde(default)]` on optional fields (`lib.rs:200, 217, 219`). A `ViewMessage` / `InputMessage` follows the same shape with its own tag key.

**Reusable on the wire unchanged:** `Command` (`lib.rs:88`), `Outcome` (`:162`), `ResultPayload` (`:169`, `#[serde(untagged)]`), `TabInfo` (`:188`), `Event` (`:225`).

**Must move behind `cfg(unix)` / into `local`:** `socket_dir` (`lib.rs:26`), `socket_path` (`:36`), `ensure_socket_dir` (`:46`), `current_uid` (`:63`) and the raw `extern "C" getuid` shim (`:69`). Four of ~11 public items. Their callers are `control.rs:8-12` (the import list) and the e2e harness's socket path helper.

**Must change:** the module header at `lib.rs:1-14` claims the crate is the distributed wire:

```rust
//! Wire types for Talaria's local control socket.
//!
//! The shell listens on a Unix domain socket (see [`socket_path`]). Clients
//! (the `talaria-mcp` stdio proxy today, the distributed-mode client later)
//! speak newline-delimited JSON: one [`ClientMessage`] per line in, one
//! [`ServerMessage`] per line out.
```

Rewrite to "shared vocabulary"; and update the matching claim in `PROJECT.md` and `control.rs:1-4`:

```rust
//! Local control socket: the in-process precursor to the distributed
//! protocol.
```

**Do not extend `Command` with input variants.** `Command` is the agent tool vocabulary dispatched by `talaria_mcp::dispatch`; a remote-input variant there would make remote clicks reachable from the MCP tool surface. `ChromeRect` (`lib.rs:135`) is the in-crate precedent for refusing exactly that.

**Tests** — `lib.rs:239-310` already has `#[cfg(test)] mod tests` with three round-trip tests. The new `wire` module needs the same, at the density the validation map asks for ("every channel tag, every input kind, every frame-header field").

---

### `oauth.rs` + `http.rs` — the advertised URL vs the bound address (config/security)

**Analog:** the current derivation, which is what D-05-03 changes.

`http.rs:114`:
```rust
const BIND_HOST: &str = "127.0.0.1";
```
(and `settings.rs:LOOPBACK_BIND`, the second copy, deliberately — "`http.rs` owns what is *bound*, and this owns what a file on disk is *allowed to ask for*")

`oauth.rs:143-155`:
```rust
pub fn canonical_resource(bound: &str) -> String {
    format!("http://{bound}/mcp")
}
...
pub fn metadata_url(bound: &str) -> String {
    format!("http://{bound}{OAUTH_PROTECTED_RESOURCE_BASE}")
}
```

`oauth.rs:1160-1163`:
```rust
            resource: canonical_resource(bound),
            metadata_url: metadata_url(bound),
            bound: bound.to_owned(),
            issuer: format!("http://{bound}"),
```

`http.rs:1103`:
```rust
        Arc::new(DnsRebindProtector::new(Some(vec![bound.to_owned()]), None)),
```

**All four derive from one `bound: &str`, which comes from `listener.local_addr()` (`http.rs:930-940`) — the socket itself, never the configured value.** That single-source discipline is the pattern to preserve: introduce *one* advertised-identity value threaded the same way, not four independent `format!`s. `http.rs:1198` (`refuse_page_originated`) compares `Host` against the same string; that comparison must learn the advertised host or every Serve-proxied request is refused. `oauth.rs:1163`'s `http://` becomes `https://` on the advertised path.

`settings.rs:LOOPBACK_BIND`'s doc comment already names this phase:

> Phase 5 is where a non-loopback bind gets considered, and it must bring TLS with it

D-05-03 says the bind does **not** widen — so that comment needs correcting too, not just honouring.

---

### `tests/e2e/remote_view_test.py` and `remote_latency_test.py` (test)

**Analog:** `tests/e2e/revocation_test.py` — the newest, richest, and structurally closest suite.

**The raw-socket stream reader** (`revocation_test.py:164-196`) — the class docstring is the pattern as much as the code:

```python
class Stream:
    """A standalone event stream, on a raw socket — ``GET /mcp`` or ``GET /sse``.
    ...
    Read with ``select`` on a bare socket rather than through
    ``urllib``/``makefile``: a buffered reader that has timed out once refuses
    every later read, and this class has to be able to say both "nothing yet,
    still open" and "ended" about the same connection.
    ...
    **The framing matters, and getting it wrong makes this test lie.**
    """
```

**Hand-written request line + headers** (`revocation_test.py:197-213`) — this is exactly how a WebSocket upgrade must be sent, since the suites are stdlib-only:

```python
    def __init__(self, host, port, token, session_id=None, path="/mcp"):
        self.socket = socket.create_connection((host, port), timeout=10)
        self.path = path
        lines = [
            f"GET {path} HTTP/1.1",
            f"Host: {host}:{port}",
            "Accept: text/event-stream",
            f"Authorization: Bearer {token}",
        ]
        ...
        lines.append("Connection: keep-alive")
        self.socket.sendall(("\r\n".join(lines) + "\r\n\r\n").encode())
        self.socket.settimeout(None)
```

For `/view` this becomes `Upgrade: websocket` / `Connection: Upgrade` / `Sec-WebSocket-Key` / `Sec-WebSocket-Version: 13`, and `_decode` swaps chunked-transfer framing for RFC 6455 frame framing (a ~30-line stdlib decoder). **Everything else transfers.** Note that the DIST-01 assertions "a WebSocket carrying an `Origin` header is refused" and "wrong `Host` is refused" are one extra line in this `lines` list each — the hand-written request is what makes those assertions cheap.

**The three-state read primitive** (`revocation_test.py:271-320`) — copy all four methods:

```python
    def _pump(self, timeout):
        """One read. False on timeout, False and ``ended`` set on an ending."""
        if timeout <= 0:
            return False
        if not select.select([self.socket], [], [], timeout)[0]:
            return False
        try:
            chunk = self.socket.recv(65536)
        except (ConnectionResetError, OSError):
            self.ended = True
            return False
        if not chunk:
            self.ended = True
            return False
        self.buf += chunk
        self._decode()
        return True
```

```python
    def delivering(self, timeout):
        """True once any byte of stream *body* has arrived within ``timeout``.
```
```python
    def still_open(self, settle=1.5):
        """False once the read side has seen the stream end."""
```
```python
    def closed_within(self, timeout):
        """True once the read side sees the stream end within ``timeout``.

        **The assertion this suite exists for.** Asserting instead that a fresh
        request now fails would pass whether or not the stream ever closed.
        """
```

`closed_within` is verbatim what the revocation extension needs for the viewer WebSocket. `_pump`'s per-read timeout is also the primitive `remote_latency_test.py` builds cadence measurement on — timestamp each successful `_pump` that yields a frame, and the passive-vs-takeover transition is a diff of arrival intervals.

**Assert through the control socket, not the path under test** (`revocation_test.py:328-334`):

```python
def rpc(command, client="revocation-e2e", **params):
    """One control-socket request on its own connection.

    Every assertion about what the *browser* did goes over this socket rather
```

This is the validation map's "asserted on the tab's URL over the **control socket**, so the assertion does not depend on the frame path under test." Copy `rpc` wholesale.

**Harness helpers to reuse, not reinvent:**

- `harness.py:40-63` — `REPO`, `DISPLAY`, `TARGET`, `BINARY`, `SOCK`. `BINARY = os.path.join(TARGET, "release", "talaria")` gains a sibling `CLIENT_BINARY` constant.
- `harness.py:146-157` — `start_shell(url, log, rust_log, wait, **env_extra)`; readiness is "the control socket file exists", then a settle. A `start_client` follows the same shape with its own readiness signal (do not copy the `os.path.exists(SOCK)` check — a client owns no socket).
- `harness.py:159-174` — `free_port()`, with its documented accepted race.
- `harness.py:191-230` — `write_agents(talaria_dir, client_id, client_name, token, audience=..., ...)`; seeds a real authorization record before launch. Its docstring explains why this is not a bypass. `audience` must be `http://127.0.0.1:PORT/mcp` byte-for-byte per `oauth::canonical_resource` — **and D-05-03 changes what that string is**, so this helper is a place the advertised-URL change will surface.
- `harness.py:250-289` — `chrome_rects(client=...)`, returning `(rects, scale)`, requires `TALARIA_TEST_HOOKS=1`. This is how "remote input cannot reach the chrome" is asserted: get the `toolbar.credentials` rect, aim remote input at it, assert no panel opened.
- `harness.py:324-348` — `click_rect(name, wid, env, ...)`, the real-pointer local click via `xdotool`. Read it for the logical→screen conversion (`origin + (x + width/2) * scale`); the remote path's coordinates are page-relative and skip both the window origin and the toolbar, which is the contrast worth encoding in a test.

**Registration** (`run_all.py:73-77`) — both suites start their own shell, so they go in the second block:

```python
for name in ("keyboard_nav_test", "takeover_test", "download_bounds_test", "vault_test",
             "vault_ui_test", "history_test", "bookmarks_test", "downloads_list_test",
             "panel_click_test", "http_transport_test", "oauth_flow_test",
             "revocation_test", "vault_nobus_test"):
    run(name, [])
```

The module docstring at `run_all.py:2-19` lists every suite by name and calls out the slow ones — update it, it is not decoration.

**Python style for these suites:** deliberately terse locals (`s`, `f`, `m`, `rid`) — `harness.py:270-282` — module constants SCREAMING_CASE, private helpers `_leading_underscore` (`harness.py:89, 102`), a `"""docstring"""` at the top of every module stating what the suite pins down and its preconditions. This is the **opposite** of the Rust style in the same repo; do not carry Rust's spell-it-out rule into Python here.

---

## Shared Patterns

### Error handling — values, not panics
**Source:** `app.rs:419-421`, `control.rs:161-176`, `http.rs:876-885`
**Apply to:** `view.rs`, `remote_input.rs`, the `/view` route, `talaria-client`

```rust
        match image {
            Some(image) => encode_screenshot(image),
            None => Outcome::Error { message: "framebuffer read failed".into() },
        }
```

No `anyhow`, no `thiserror`, no `unwrap()` in shell paths. `expect()` only for genuine startup invariants (`main.rs:33`, `app.rs:915`). Fallible steps use `let Ok(x) = ... else { return ...error }` (`app.rs:2503`). Degrade rather than abort (`http.rs:880`: "a browser that cannot start a listener is still a browser").

### Refusals say nothing
**Source:** `http.rs:1225-1236`, `control.rs:170-173`
**Apply to:** every `/view` and remote-input rejection

```rust
fn page_originated_refusal() -> rust_mcp_axum::axum::response::Response {
    use rust_mcp_axum::axum::response::IntoResponse as _;
    (
        StatusCode::FORBIDDEN,
        "requests carrying an Origin header, or addressed to a host this server is not \
         bound to, are refused: this endpoint is for MCP clients, not for pages",
    )
        .into_response()
}
```

One expression for all refusals — "two refusals that differ are two bits an attacker did not have." The control socket's version writes nothing back at all (`control.rs:170-172`): "the peer gets a closed connection, not a hint about what the check was." A refused `/view` upgrade should not distinguish "Me tab" from "no such tab".

### Logging levels
**Source:** `control.rs:91, 129, 163`; `http.rs:880, 1207`
**Apply to:** all new modules

`log::error!` for an unrecoverable subsystem failure (bind failed, thread not spawned); `log::warn!` for a degraded fallback the user should know about (a refused connection); `log::info!` for lifecycle (`"remote access listening at http://{bound}/mcp"`, `http.rs:958`); `log::debug!` for routine churn (a refused request path, `http.rs:1207`). Inline format captures (`{error}`, `{bound}`) — never positional. **Never interpolate a raw token or an `Authorization` value** (`oauth.rs:157-161`).

### Ignored results are explicit
**Source:** `app.rs:882`, `control.rs:88`, `http.rs:959`
`let _ = proxy.send_event(...)`, `let _ = std::fs::remove_file(&path)`. Never a bare expression statement.

### Module headers and design-decision citation
**Source:** `control.rs:1-4`, `tabs.rs:1-3`, `lib.rs:1-14`
Every Rust file opens with `//!` stating its role in one or two sentences. `///` on public items and on non-obvious struct fields. Design decisions cite the decision by name inline — `http.rs` cites `T-2`, `D-04-04`, `CR-02`, `04-02-SPIKE.md`. Phase 5's new files should cite `D-05-01` … `D-05-06` the same way.

### Function-shape conventions
**Source:** `tabs.rs:223, 231, 288-294`; `gui.rs:709`
Lookups return `Option<&T>`; the `_mut` suffix pairs with the shared-borrow variant (`get`/`get_mut`, `displayed`/`displayed_mut`); no-op-capable mutations return `bool` (`TabManager::close`); iteration returns `impl Iterator<Item = &Tab>`, never an allocated `Vec`; the GUI returns `Vec<UiAction>` and mutates nothing.

### Dependency landing
**Source:** `Cargo.toml:15-33` (the lockfile warning block), `:68-88` (the two feature-provenance comments)
**Apply to:** the `axum` + `ws` landing

Declare the version once in `[workspace.dependencies]`, `{ workspace = true }` in the member. **Never `cargo add`, never `cargo update`.** Edit the manifest, run `cargo metadata`, inspect the lock diff, restore on rejection. The precedent for documenting *why* a feature edge exists is `Cargo.toml:68-78` (the `sse` feature paragraph) and `:82-88` (the `rust-mcp-axum` legitimacy note) — a new dependency line that carries a surprising transitive should get the same treatment. Assert `primeorder 0.14.0-rc.14` survives.

---

## No Analog Found

| File | Role | Data Flow | Reason |
|------|------|-----------|--------|
| `crates/talaria-client/src/rate.rs` | service (controller) | transform | Nothing in this codebase does EWMA, hysteresis, or a degrade ladder. The nearest thing in spirit is the timeout *ordering* discipline (socket 30 s > `promise_wait` timeout−2 s > `LOAD_WAIT` 20 s > background capture 1.5 s, `app.rs:2222`) — a fixed ladder whose rungs are named in one place and must keep their order. Copy that *property*: name every rung in one place, and make the ordering a unit-tested invariant. The algorithm itself comes from RESEARCH § "What the product should do when it cannot meet the target". |

**Partial-analog warning:** `crates/talaria-client/src/present.rs` has an analog for the egui side (`gui.rs:516-530`) and for the paint-callback seam (`gui.rs:2513-2525`), but **not** for creating a GL context without Servo. `WindowRenderingContext` and `glow_gl_api()` are Servo types. This is the client's one unmapped construction step and should be a named, bounded task rather than an assumed one.

---

## Metadata

**Analog search scope:** `crates/talaria-shell/src/`, `crates/talaria-protocol/src/`, `crates/talaria-mcp/src/`, `tests/e2e/`, workspace and member `Cargo.toml`
**Files scanned:** 20 Rust modules, 26 Python suites, 4 manifests
**Pattern extraction date:** 2026-08-21

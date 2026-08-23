/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 *
 * Portions derived from Servo's servoshell and from this repository's own
 * `app.rs`/`gui.rs` (servo v0.4.0); this file keeps the MPL 2.0 header per
 * its file-level copyleft. */

//! Throwaway spike for plan 05-02 — deleted in Task 2 of the same plan.
//!
//! Measures what `OffscreenRenderingContext::read_to_image` costs when it is
//! driven the way the frame pump will drive it — a sustained fixed-interval
//! loop of `paint()` + full-surface readback on a `show()`n webview, with the
//! window's own egui chrome still rendering — rather than the way
//! `Command::Screenshot` drives it today. Then re-runs `05-RESEARCH.md`'s
//! codec benchmark against the buffers this loop actually produced, instead
//! of against synthetic frames.
//!
//! Only the findings survive, in `.planning/phases/05-distributed-mode/05-02-SPIKE.md`.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use egui::{LayerId, PaintCallback};
use egui_glow::CallbackFn;
use egui_glow::winit::EguiGlow;
use euclid::Scale;
use servo::{
    InputEvent, OffscreenRenderingContext, RenderingContext, ServoBuilder, WebView,
    WebViewBuilder, WheelDelta, WheelEvent, WheelMode, WindowRenderingContext,
};
use url::Url;
use webrender_api::units::DevicePixel;
use winit::application::ApplicationHandler;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;

/// The geometry `tests/e2e/harness.py` creates its Xvfb screen at, so the
/// measurement runs at the size the suites actually use.
const VIEWPORT: winit::dpi::PhysicalSize<u32> = winit::dpi::PhysicalSize::new(1280, 800);

/// Tile edge for the dirty-tile scan `05-RESEARCH.md` benchmarked.
const TILE: u32 = 64;

#[derive(Clone)]
struct Waker(EventLoopProxy<()>);

impl embedder_traits::EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn embedder_traits::EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        let _ = self.0.send_event(());
    }
}

struct Options {
    url: Url,
    label: String,
    ticks: usize,
    cadence: Duration,
    scroll: bool,
}

fn options() -> Options {
    let mut args = std::env::args().skip(1);
    let mut url = None;
    let mut label = String::from("unlabelled");
    let mut ticks = 300usize;
    let mut cadence_ms = 30u64;
    let mut scroll = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--label" => label = args.next().unwrap_or_default(),
            "--ticks" => ticks = args.next().and_then(|v| v.parse().ok()).unwrap_or(300),
            "--cadence-ms" => cadence_ms = args.next().and_then(|v| v.parse().ok()).unwrap_or(30),
            "--scroll" => scroll = true,
            other => url = Url::parse(other).ok(),
        }
    }
    Options {
        url: url.unwrap_or_else(|| {
            Url::parse("https://servo.org").expect("literal url parses")
        }),
        label,
        ticks,
        cadence: Duration::from_millis(cadence_ms),
        scroll,
    }
}

/// mean / median / p95 / max over a sample, in milliseconds. A mean alone
/// hides the p95 that a user actually feels.
struct Distribution {
    count: usize,
    mean: f64,
    median: f64,
    p95: f64,
    max: f64,
    min: f64,
}

fn distribution(samples: &[f64]) -> Distribution {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).expect("no NaN in a duration"));
    let count = sorted.len();
    let index = |fraction: f64| -> f64 {
        if count == 0 {
            return 0.0;
        }
        let position = ((count as f64 - 1.0) * fraction).round() as usize;
        sorted[position]
    };
    Distribution {
        count,
        mean: if count == 0 { 0.0 } else { sorted.iter().sum::<f64>() / count as f64 },
        median: index(0.5),
        p95: index(0.95),
        max: if count == 0 { 0.0 } else { sorted[count - 1] },
        min: if count == 0 { 0.0 } else { sorted[0] },
    }
}

fn report(name: &str, samples: &[f64]) {
    let d = distribution(samples);
    println!(
        "{name:<26} n={:<5} mean={:>8.3} median={:>8.3} p95={:>8.3} max={:>9.3} min={:>7.3}  (ms)",
        d.count, d.mean, d.median, d.p95, d.max, d.min
    );
}

fn report_counts(name: &str, samples: &[usize]) {
    let as_float: Vec<f64> = samples.iter().map(|value| *value as f64).collect();
    let d = distribution(&as_float);
    println!(
        "{name:<26} n={:<5} mean={:>8.1} median={:>8.0} p95={:>8.0} max={:>9.0} min={:>7.0}  (tiles)",
        d.count, d.mean, d.median, d.p95, d.max, d.min
    );
}

/// `encode_screenshot`'s configuration, copied verbatim from
/// `crates/talaria-shell/src/app.rs` rather than called, because the spike may
/// not modify or share that function. Colour and depth are set and nothing
/// else, so this runs at `png::Compression::Default` and png 0.17's default
/// filter by omission.
fn encode_existing_screenshot_path(image: &servo::RgbaImage) -> (f64, usize, f64, usize) {
    let (width, height) = (image.width(), image.height());
    let raw = image.as_raw();
    let started = Instant::now();
    let mut png_data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_data, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(raw).expect("png encode");
    }
    let png_elapsed = started.elapsed().as_secs_f64() * 1000.0;
    let base64_started = Instant::now();
    let encoded = base64::engine::general_purpose::STANDARD.encode(&png_data);
    let base64_elapsed = base64_started.elapsed().as_secs_f64() * 1000.0;
    (png_elapsed, png_data.len(), base64_elapsed, encoded.len())
}

/// One PNG encode at an explicit compression/filter pair, so the frame path's
/// three-line divergence from the screenshot path is measured rather than
/// asserted.
fn encode_png(
    pixels: &[u8],
    width: u32,
    height: u32,
    compression: png::Compression,
    filter: png::FilterType,
) -> (f64, usize) {
    let started = Instant::now();
    let mut png_data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_data, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(compression);
        encoder.set_filter(filter);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(pixels).expect("png encode");
    }
    (started.elapsed().as_secs_f64() * 1000.0, png_data.len())
}

/// The frame path's sibling encoder, diverging on exactly the three lines
/// D-05-05 names: `Compression::Fast`, `FilterType::Sub`, no base64.
fn encode_frame_path(pixels: &[u8], width: u32, height: u32) -> (f64, usize) {
    encode_png(pixels, width, height, png::Compression::Fast, png::FilterType::Sub)
}

/// Copy a tile rectangle out of a full RGBA frame, tightly packed.
fn crop(image: &servo::RgbaImage, x: u32, y: u32, width: u32, height: u32) -> Vec<u8> {
    let stride = image.width() as usize * 4;
    let raw = image.as_raw();
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for row in 0..height {
        let start = (y + row) as usize * stride + x as usize * 4;
        out.extend_from_slice(&raw[start..start + width as usize * 4]);
    }
    out
}

/// The whole-frame dirty-tile scan `05-RESEARCH.md` benchmarked at 0.14 ms:
/// 64×64 tiles, row-slice comparison, short-circuiting on the first differing
/// row. Returns the elapsed milliseconds and the dirty-tile count.
fn dirty_tiles(previous: &servo::RgbaImage, current: &servo::RgbaImage) -> (f64, usize, usize) {
    let width = current.width();
    let height = current.height();
    let stride = width as usize * 4;
    let old = previous.as_raw();
    let new = current.as_raw();
    let started = Instant::now();
    let mut dirty = 0usize;
    let mut total = 0usize;
    let mut y = 0;
    while y < height {
        let tile_height = TILE.min(height - y);
        let mut x = 0;
        while x < width {
            let tile_width = TILE.min(width - x);
            total += 1;
            let mut changed = false;
            for row in 0..tile_height {
                let start = (y + row) as usize * stride + x as usize * 4;
                let end = start + tile_width as usize * 4;
                if old[start..end] != new[start..end] {
                    changed = true;
                    break;
                }
            }
            if changed {
                dirty += 1;
            }
            x += TILE;
        }
        y += TILE;
    }
    (started.elapsed().as_secs_f64() * 1000.0, dirty, total)
}

struct Spike {
    options: Options,
    waker: Waker,
    started: bool,
}

impl ApplicationHandler<()> for Spike {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.started {
            return;
        }
        self.started = true;
        self.run(event_loop);
        std::process::exit(0);
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        _event: winit::event::WindowEvent,
    ) {
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {}
}

impl Spike {
    #[allow(clippy::too_many_lines)]
    fn run(&mut self, event_loop: &ActiveEventLoop) {
        let display_handle = event_loop.display_handle().expect("display handle");
        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("readback spike")
                    .with_inner_size(VIEWPORT),
            )
            .expect("create window");
        let window_handle = window.window_handle().expect("window handle");
        let window_rendering_context = Rc::new(
            WindowRenderingContext::new(display_handle, window_handle, window.inner_size())
                .expect("create window rendering context"),
        );
        let _ = window_rendering_context.make_current();

        // Which GL implementation actually served this run. Under the
        // harness's Xvfb this is expected to say llvmpipe; the whole reason
        // two figures are asked for is that a software rasteriser and a real
        // GPU do not distribute cost the same way between paint and readback.
        {
            use glow::HasContext as _;
            let gl = window_rendering_context.glow_gl_api();
            unsafe {
                println!("GL_VENDOR   {}", gl.get_parameter_string(glow::VENDOR));
                println!("GL_RENDERER {}", gl.get_parameter_string(glow::RENDERER));
                println!("GL_VERSION  {}", gl.get_parameter_string(glow::VERSION));
            }
        }
        println!(
            "DISPLAY     {}",
            std::env::var("DISPLAY").unwrap_or_else(|_| "(unset)".into())
        );

        // A profile of its own, so a spike run never touches the profile a
        // real Talaria instance owns.
        let config_dir =
            std::env::temp_dir().join(format!("talaria-readback-spike-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&config_dir);
        let opts = servo::Opts {
            config_dir: Some(config_dir.clone()),
            ..Default::default()
        };
        let servo = ServoBuilder::default()
            .opts(opts)
            .event_loop_waker(Box::new(self.waker.clone()))
            .build();
        servo.setup_logging();

        let mut chrome = EguiGlow::new(
            event_loop,
            window_rendering_context.glow_gl_api(),
            None,
            None,
            false,
        );

        // Two webviews, exactly as the product will have them: one the local
        // human is looking at (blitted into the chrome every frame) and one a
        // remote viewer has leased (pumped, never displayed locally).
        let local_context = Rc::new(window_rendering_context.offscreen_context(VIEWPORT));
        let local: WebView = WebViewBuilder::new(&servo, local_context.clone())
            .url(Url::parse("https://example.com").expect("literal url parses"))
            .hidpi_scale_factor(Scale::new(1.0))
            .build();
        local.show();
        local.focus();

        let pump_context = Rc::new(window_rendering_context.offscreen_context(VIEWPORT));
        let pump: WebView = WebViewBuilder::new(&servo, pump_context.clone())
            .url(self.options.url.clone())
            .hidpi_scale_factor(Scale::new(1.0))
            .build();

        println!("label       {}", self.options.label);
        println!("url         {}", self.options.url);
        println!("scroll      {}", self.options.scroll);
        println!("cadence     {} ms", self.options.cadence.as_millis());
        println!("ticks       {}", self.options.ticks);

        // Wait for both loads, then let the page settle. A load that never
        // completes still proceeds — an unfinished page is a page shape too,
        // and the run says so rather than hanging.
        let load_deadline = Instant::now() + Duration::from_secs(45);
        loop {
            servo.spin_event_loop();
            let done = pump.load_status() == servo::LoadStatus::Complete
                && local.load_status() == servo::LoadStatus::Complete;
            if done || Instant::now() >= load_deadline {
                println!(
                    "load        pump={:?} local={:?}",
                    pump.load_status(),
                    local.load_status()
                );
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let settle = Instant::now() + Duration::from_secs(3);
        while Instant::now() < settle {
            servo.spin_event_loop();
            self.chrome_frame(&mut chrome, &window, &window_rendering_context, &local, &local_context);
            std::thread::sleep(Duration::from_millis(10));
        }

        // ── Phase A ── the local chrome's own frame, with no readback loop
        // running. This is T-05-12's denominator: "the frame pump costs N% of
        // the loop" is only meaningful against it.
        let mut chrome_only = Vec::new();
        for _ in 0..180 {
            servo.spin_event_loop();
            let started = Instant::now();
            self.chrome_frame(&mut chrome, &window, &window_rendering_context, &local, &local_context);
            chrome_only.push(started.elapsed().as_secs_f64() * 1000.0);
            std::thread::sleep(Duration::from_millis(5));
        }

        // The locally displayed tab's pixels before the pump takes its lease.
        let _ = window_rendering_context.make_current();
        local.paint();
        let before = read_full(&local_context).map(|image| checksum(&image));

        // ── Phase B ── the pump's real shape: hold the leased tab shown for
        // the life of the session and paint unconditionally on the tick.
        pump.show();
        let mut paint_ms = Vec::new();
        let mut read_ms = Vec::new();
        let mut tick_ms = Vec::new();
        let mut chrome_ms = Vec::new();
        let mut overruns = 0usize;
        let mut read_failures = 0usize;
        let mut captured: Vec<servo::RgbaImage> = Vec::new();
        let mut previous: Option<servo::RgbaImage> = None;
        let mut scan_ms = Vec::new();
        let mut dirty_counts: Vec<usize> = Vec::new();
        let mut tile_total = 0usize;
        let rect = euclid::Box2D::from_origin_and_size(
            euclid::Point2D::origin(),
            euclid::Size2D::new(VIEWPORT.width as i32, VIEWPORT.height as i32),
        );

        for tick in 0..self.options.ticks {
            let tick_start = Instant::now();
            servo.spin_event_loop();

            if self.options.scroll {
                // A small scripted scroll between ticks, so the readback is
                // not the only thing changing.
                let point = euclid::Point2D::<f32, DevicePixel>::new(640.0, 400.0);
                pump.notify_input_event(InputEvent::Wheel(WheelEvent::new(
                    WheelDelta { x: 0.0, y: -40.0, z: 0.0, mode: WheelMode::DeltaPixel },
                    point.into(),
                )));
            }

            let _ = window_rendering_context.make_current();
            let paint_started = Instant::now();
            pump.paint();
            paint_ms.push(paint_started.elapsed().as_secs_f64() * 1000.0);

            let read_started = Instant::now();
            let image = pump_context.read_to_image(rect);
            read_ms.push(read_started.elapsed().as_secs_f64() * 1000.0);
            match image {
                Some(image) => {
                    // The dirty-tile scan against the frame before it, every
                    // tick, against real Servo output rather than synthetic
                    // frames. Timed separately because in the shipped design
                    // this work is on the encoder thread, not on the loop —
                    // it is reported beside the readback, never inside it.
                    if let Some(previous) = previous.as_ref() {
                        let (elapsed, dirty, total) = dirty_tiles(previous, &image);
                        scan_ms.push(elapsed);
                        dirty_counts.push(dirty);
                        tile_total = total;
                    }
                    if tick % 75 == 3 && captured.len() < 4 {
                        captured.push(image.clone());
                    }
                    previous = Some(image);
                },
                None => read_failures += 1,
            }

            // The chrome keeps rendering while the pump runs — the number
            // that matters is what the readback costs *alongside* the local
            // render, not in isolation.
            let chrome_started = Instant::now();
            self.chrome_frame(&mut chrome, &window, &window_rendering_context, &local, &local_context);
            chrome_ms.push(chrome_started.elapsed().as_secs_f64() * 1000.0);

            let elapsed = tick_start.elapsed();
            tick_ms.push(elapsed.as_secs_f64() * 1000.0);
            match self.options.cadence.checked_sub(elapsed) {
                Some(remaining) => std::thread::sleep(remaining),
                None => overruns += 1,
            }
        }
        pump.hide();

        // Did holding a second tab shown for the whole loop disturb the tab
        // the local human is looking at? The per-tab-framebuffer invariant
        // says it must not; this is where that is proven rather than cited.
        let _ = window_rendering_context.make_current();
        local.paint();
        let after = read_full(&local_context).map(|image| checksum(&image));

        println!();
        println!("── readback at cadence ─────────────────────────────────────────────");
        report("paint()", &paint_ms);
        // The first handful of ticks carry whatever the page was still doing
        // when the lease was taken; both figures are reported so neither the
        // warm-up spike nor the steady state can hide the other.
        let warm = paint_ms.len().min(10);
        report("paint() after tick 10", &paint_ms[warm..]);
        report("read_to_image()", &read_ms);
        report("paint+read", &paint_ms
            .iter()
            .zip(read_ms.iter())
            .map(|(p, r)| p + r)
            .collect::<Vec<_>>());
        report("chrome frame (during)", &chrome_ms);
        report("chrome frame (alone)", &chrome_only);
        report("dirty-tile scan", &scan_ms);
        report("whole tick", &tick_ms);
        report_counts("dirty tiles per tick", &dirty_counts);
        let clean = dirty_counts.iter().filter(|count| **count == 0).count();
        let over_third = dirty_counts.iter().filter(|count| **count * 3 > tile_total).count();
        println!(
            "ticks with nothing to send: {clean}/{} ; ticks over a third of the {tile_total} tiles: {over_third}/{}",
            dirty_counts.len(),
            dirty_counts.len()
        );
        println!(
            "overruns of the {} ms interval: {overruns}/{}",
            self.options.cadence.as_millis(),
            self.options.ticks
        );
        println!("read_to_image returned None: {read_failures}/{}", self.options.ticks);
        println!(
            "displayed-tab pixels before/after the lease: {}",
            match (before, after) {
                (Some(before), Some(after)) if before == after => "identical".to_string(),
                (Some(before), Some(after)) => format!("DIFFER ({before:#x} -> {after:#x})"),
                _ => "unreadable".to_string(),
            }
        );

        // The off-thread encoder design rests on the readback buffer being
        // `Send`; one line proves it in this tree rather than assuming it.
        fn assert_send<T: Send>() {}
        assert_send::<servo::RgbaImage>();
        println!("servo::RgbaImage: Send — statically asserted in this tree");

        // ── Phase C ── the codec, against buffers this loop actually produced.
        println!();
        println!("── codec against real Servo readback ───────────────────────────────");
        for (index, image) in captured.iter().enumerate() {
            let width = image.width();
            let height = image.height();

            let tile = crop(image, 320, 240, TILE, TILE);
            let (tile_ms, tile_bytes) = encode_frame_path(&tile, TILE, TILE);

            let cluster_width = TILE * 2;
            let cluster_height = TILE * 2;
            let cluster = crop(image, 320, 240, cluster_width, cluster_height);
            let (cluster_ms, cluster_bytes) =
                encode_frame_path(&cluster, cluster_width, cluster_height);

            let (key_ms, key_bytes) = encode_frame_path(image.as_raw(), width, height);
            let (nofilter_ms, nofilter_bytes) = encode_png(
                image.as_raw(),
                width,
                height,
                png::Compression::Fast,
                png::FilterType::NoFilter,
            );
            let (best_ms, best_bytes) = encode_png(
                image.as_raw(),
                width,
                height,
                png::Compression::Best,
                png::FilterType::Sub,
            );
            // What `05-RESEARCH.md` believed the screenshot path was doing:
            // `Compression::Default` set explicitly. png 0.17's `Info` default
            // is `Fast`, so the shipped encoder never reaches this line.
            let (default_ms, default_bytes) = encode_png(
                image.as_raw(),
                width,
                height,
                png::Compression::Default,
                png::FilterType::Sub,
            );
            let (existing_ms, existing_bytes, base64_ms, existing_base64) =
                encode_existing_screenshot_path(image);

            println!("frame {index} ({width}x{height}):");
            println!("  one 64x64 tile        {tile_ms:>8.3} ms  {tile_bytes:>9} B   Fast+Sub");
            println!("  4-tile cluster        {cluster_ms:>8.3} ms  {cluster_bytes:>9} B   Fast+Sub");
            println!("  keyframe              {key_ms:>8.3} ms  {key_bytes:>9} B   Fast+Sub");
            println!("  keyframe              {nofilter_ms:>8.3} ms  {nofilter_bytes:>9} B   Fast+NoFilter");
            println!("  keyframe              {best_ms:>8.3} ms  {best_bytes:>9} B   Best+Sub");
            println!("  keyframe              {default_ms:>8.3} ms  {default_bytes:>9} B   Default+Sub (explicit)");
            println!(
                "  existing encoder      {existing_ms:>8.3} ms  {existing_bytes:>9} B   as shipped (+{base64_ms:.3} ms base64 -> {existing_base64} B)"
            );
        }

        // The engine profile is left on disk deliberately: removing it here
        // races Servo's still-running ResourceManager thread. The run's caller
        // reaps `{temp}/talaria-readback-spike-*`.
        let _ = &config_dir;
        println!();
        println!("done: {}", self.options.label);
    }

    /// One chrome frame with the locally displayed tab blitted into it — the
    /// same shape `Gui::update`/`Gui::paint` perform, reduced to what the
    /// measurement needs.
    #[allow(deprecated)]
    fn chrome_frame(
        &self,
        chrome: &mut EguiGlow,
        window: &Window,
        window_rendering_context: &Rc<WindowRenderingContext>,
        local: &WebView,
        local_context: &Rc<OffscreenRenderingContext>,
    ) {
        let _ = window_rendering_context.make_current();
        chrome.run(window, |ctx| {
            egui::Panel::top("toolbar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let _ = ui.selectable_label(true, "Me");
                    let _ = ui.selectable_label(false, "Agents");
                    let _ = ui.button("<");
                    let _ = ui.button(">");
                    let _ = ui.button("reload");
                    ui.label("readback spike");
                });
            });
            let available = ctx.available_rect();
            let scale = ctx.pixels_per_point();
            let width = (available.width() * scale).round().max(1.0) as u32;
            let height = (available.height() * scale).round().max(1.0) as u32;
            let current = local.size();
            if (current.width as u32, current.height as u32) != (width, height) {
                local.resize(winit::dpi::PhysicalSize::new(width, height));
            }
            local.paint();
            if let Some(render_to_parent) = local_context.render_to_parent_callback() {
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
        });
        let _ = window_rendering_context.make_current();
        window_rendering_context.prepare_for_rendering();
        chrome.paint(window);
        window_rendering_context.present();
    }
}

fn read_full(context: &Rc<OffscreenRenderingContext>) -> Option<servo::RgbaImage> {
    let size = context.size2d().to_i32();
    let rect = euclid::Box2D::from_origin_and_size(
        euclid::Point2D::origin(),
        euclid::Size2D::new(size.width, size.height),
    );
    context.read_to_image(rect)
}

/// FNV-1a over the raw pixels — enough to answer "did these bytes change".
fn checksum(image: &servo::RgbaImage) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in image.as_raw() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

fn main() {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("install crypto provider");
    let event_loop = EventLoop::<()>::with_user_event().build().expect("event loop");
    let waker = Waker(event_loop.create_proxy());
    let mut spike = Spike { options: options(), waker, started: false };
    event_loop.run_app(&mut spike).expect("run app");
}

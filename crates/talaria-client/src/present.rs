//! What arrives on the frame channel, and where it is drawn.
//!
//! **The seam.** The server's own chrome draws its engine's surface by handing
//! the interface a texture and asking it to paint that texture into a
//! rectangle (`crates/talaria-shell/src/gui.rs`, the paint callback into the
//! background layer). This module does the same thing with different contents:
//! the pixels come off the wire as encoded bytes rather than out of a
//! compositor, they are uploaded into a texture the interface owns, and the
//! rectangle they are painted into is decided by [`fit`] below. Everything
//! about the arrangement transfers; only where the pixels came from differs.
//!
//! Note what sits immediately above that code in the server's chrome: a
//! **resize of the webview** when the available area no longer matches. **This
//! module has no equivalent, deliberately.** Its response to the client's own
//! window changing is to recompute [`fit`], and nothing on the wire asks the
//! server to make the page a different size.
//!
//! That is worth stating rather than leaving as an absence, because the
//! alternative is tempting. The client could tell the server its window size
//! and have the tab resized to match, which would make coordinates one-to-one
//! and delete the transform entirely. It would also **reflow an agent's page
//! because a human started watching**, reflow it again the moment the local
//! human displayed that tab, and make an agent's layout a function of who is
//! looking at it. The viewer adapts to the page; the page does not adapt to the
//! viewer. The transform below is the cheaper of the two costs.
//!
//! ## The three rules this module holds
//!
//! 1. **One transform.** [`fit`] is the only place the layout of the picture is
//!    decided, and [`crate::input`] *inverts* the value it returns rather than
//!    computing a second one. See [`fit`]'s own comment for the failure that
//!    avoids.
//! 2. **Ordering.** A frame is applied only when its sequence exceeds the last
//!    one applied for that attachment, and a frame arriving before any keyframe
//!    is discarded. [`decide`] is the whole of it and it is one comparison.
//! 3. **A bad frame costs one frame.** A payload that does not decode is
//!    discarded with a log line; the picture already held is untouched and the
//!    connection is undisturbed.

use talaria_protocol::wire::{FrameHeader, FrameKind};

/// The size of the picture the server is sending, and the divisor it painted
/// at.
///
/// Two sizes live in here and telling them apart is the whole reason this is a
/// type rather than a pair of `u32`s: [`SurfaceSize::width`] is the size of the
/// **texture**, in the pixels a frame actually carries, and
/// [`SurfaceSize::page_width`] is the size of the **page**, in the device
/// pixels a pointer coordinate is expressed in. They differ exactly when the
/// server is painting at reduced resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceSize {
    /// The texture's own width — [`FrameHeader::frame_width`].
    pub width: u32,
    /// The texture's own height — [`FrameHeader::frame_height`].
    pub height: u32,
    /// The divisor the server applied to the tab's real size before painting:
    /// 1 for full resolution, 2 for half.
    ///
    /// **Honoured now, although nothing sets it above one yet.** 05-10's rate
    /// ladder is what will, and a client that ignored this field would show a
    /// quarter-sized page the first time the ladder stepped down — a bug that
    /// reads as a rendering fault rather than as a protocol one, which is the
    /// worst kind to go looking for.
    pub denominator: u8,
}

impl SurfaceSize {
    /// The size this frame declares, taken off a header.
    pub fn of(header: &FrameHeader) -> Self {
        Self {
            width: header.frame_width,
            height: header.frame_height,
            denominator: header.scale_denominator,
        }
    }

    /// The page's own width, in the device pixels an input coordinate uses.
    ///
    /// The texture's width times the scale denominator: a half-resolution
    /// frame covers the whole page at half the pixel dimensions, so the page is
    /// twice as wide as the picture of it.
    pub fn page_width(self) -> u32 {
        self.width.saturating_mul(u32::from(self.denominator))
    }

    /// The page's own height, in the device pixels an input coordinate uses.
    pub fn page_height(self) -> u32 {
        self.height.saturating_mul(u32::from(self.denominator))
    }
}

/// Where the server's surface is drawn inside the client's own window.
///
/// A scale and an origin, and that pair is the entire transform between a
/// human's pointer and a page's coordinate space.
///
/// **This value is computed in exactly one place — [`fit`] — and inverted in
/// exactly one other — [`crate::input::page_position`].** Neither end computes
/// its own. The failure that avoids is specific and was seen on the server side
/// of this same phase: two transforms written separately disagree the first
/// time either changes, and the disagreement there was a forty-pixel offset on
/// every click that nothing errored on. A click that lands forty pixels from
/// where a human read is not a rendering bug; in a page with a destructive
/// control near a benign one it is a privilege escalation wearing a rendering
/// bug's clothes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// The client's page area, in its own window's logical points. The client's
    /// own interface is everything outside this rectangle.
    pub content: egui::Rect,
    /// The letterbox margin: how far inside [`Fit::content`] the picture starts.
    /// Non-negative in both axes, and zero in whichever axis the picture fills.
    pub origin: egui::Vec2,
    /// How many logical points one **page** device pixel occupies on screen.
    pub scale: f32,
    /// The page's size in its own device pixels, carried so the inverse can
    /// refuse a coordinate outside it without recomputing anything.
    pub page: (u32, u32),
}

impl Fit {
    /// The rectangle the picture occupies, in the client's window coordinates.
    ///
    /// **The one place the client's own interface offset enters the mapping.**
    /// [`crate::present`] draws into this rectangle and [`crate::input`]
    /// inverts against it, so applying the offset twice or forgetting it is not
    /// expressible at either end — there is one expression of it and both ends
    /// call it.
    pub fn surface_rect(self) -> egui::Rect {
        let (page_width, page_height) = self.page;
        egui::Rect::from_min_size(
            self.content.min + self.origin,
            egui::vec2(page_width as f32 * self.scale, page_height as f32 * self.scale),
        )
    }
}

/// Lay `surface` out inside `content`, preserving aspect ratio and centring.
///
/// The only place the layout of the picture is decided. One scale factor — the
/// smaller of the two axes' ratios, so the picture fits in both — and an origin
/// that centres what is left, leaving margins on the two shorter sides.
///
/// The scale denominator composes here rather than at the upload: a
/// half-resolution frame is a *smaller picture of the same page*, so it is the
/// page's size that is fitted and the texture is stretched across the result.
/// Fitting the texture's own size instead would make a page shrink on screen
/// the moment the link degraded, which is the visible half of the bug this
/// field exists to prevent.
///
/// `None` for a surface or a content area with no area, which are both real
/// states — a minimised window, and an attachment with no frame yet.
pub fn fit(surface: SurfaceSize, content: egui::Rect) -> Option<Fit> {
    let page_width = surface.page_width();
    let page_height = surface.page_height();
    if page_width == 0 || page_height == 0 {
        return None;
    }
    if content.width() <= 0.0 || content.height() <= 0.0 || !content.width().is_finite()
        || !content.height().is_finite()
    {
        return None;
    }
    let scale =
        (content.width() / page_width as f32).min(content.height() / page_height as f32);
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let drawn = egui::vec2(page_width as f32 * scale, page_height as f32 * scale);
    Some(Fit {
        content,
        origin: ((content.size() - drawn) / 2.0).max(egui::Vec2::ZERO),
        scale,
        page: (page_width, page_height),
    })
}

/// What a frame should do to the surface the client already holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Application {
    /// Replace the whole surface, at this size.
    Replace(SurfaceSize),
    /// Upload into exactly this rectangle of the surface already held, leaving
    /// every other pixel alone.
    Patch { x: u32, y: u32, width: u32, height: u32 },
    /// Change nothing at all.
    Discard,
}

/// The ordering rule, and the whole of it.
///
/// `held` is the size of the surface the client already has for this attachment
/// and the last frame sequence it applied, or `None` before any keyframe has
/// arrived.
///
/// Four refusals, none of which substitutes a plausible default:
///
/// - A frame before any keyframe. A delta has nothing to composite over, and
///   compositing it onto a blank surface would show one changed region floating
///   in nothing.
/// - A sequence that does not **exceed** the last applied. This is what stops
///   a late arrival painting stale pixels over fresh ones, and it covers the
///   awkward case by construction: a delta whose sequence is greater than some
///   earlier frame's, arriving after a keyframe with a higher sequence, is
///   discarded by the same one comparison.
/// - A delta whose declared frame size is not the size of the surface held. Its
///   tile coordinates were computed against a different picture, so they name a
///   region of this one that does not correspond to anything.
/// - Nothing else. A keyframe of a differing size is not a refusal — it is the
///   server telling the client the page changed size, and it replaces.
pub fn decide(held: Option<(SurfaceSize, u64)>, header: &FrameHeader) -> Application {
    let declared = SurfaceSize::of(header);
    let Some((size, last_applied)) = held else {
        return match header.kind {
            FrameKind::Keyframe => Application::Replace(declared),
            FrameKind::Tile => Application::Discard,
        };
    };
    if header.frame_seq <= last_applied {
        return Application::Discard;
    }
    match header.kind {
        FrameKind::Keyframe => Application::Replace(declared),
        FrameKind::Tile if declared == size => Application::Patch {
            x: header.tile_x,
            y: header.tile_y,
            width: header.tile_width,
            height: header.tile_height,
        },
        FrameKind::Tile => Application::Discard,
    }
}

/// One frame payload's pixels, as the interface's own image type.
///
/// `None` for bytes that are not an image of exactly `width` by `height` in
/// eight-bit RGBA. Every one of those is a discard rather than a conversion:
/// the only producer on this channel is the server's own encoder, so a payload
/// that is anything else is a payload from something that is not it, and
/// guessing at what it meant is how a hostile peer gets a decoder it did not
/// ask for.
pub fn decode(payload: &[u8], width: u32, height: u32) -> Option<egui::ColorImage> {
    let decoder = png::Decoder::new(payload);
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    if info.width != width || info.height != height {
        return None;
    }
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let mut pixels = vec![0u8; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut pixels).ok()?;
    let pixels = pixels.get(..frame.buffer_size())?;
    // The one shape `ColorImage` will accept, checked rather than assumed: its
    // constructor asserts on a mismatch, and an assertion inside a decoder fed
    // by a network peer is a panic a peer can reach.
    let expected = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
    if pixels.len() != expected {
        return None;
    }
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        pixels,
    ))
}

/// How a frame texture is sampled when the fit is not one-to-one.
///
/// Linear, because the fit almost never is one-to-one: a client window and a
/// server tab are different sizes, and a page scaled down with nearest-neighbour
/// sampling drops every other row of text.
const SAMPLING: egui::TextureOptions = egui::TextureOptions::LINEAR;

/// The surface currently held for an attachment.
struct Held {
    texture: egui::TextureHandle,
    size: SurfaceSize,
    last_seq: u64,
}

/// The client's picture of one attached tab.
///
/// Holds at most one surface, because a client is attached to at most one tab
/// at a time. The surface is a texture the interface owns and this type uploads
/// into — **per frame rather than rebuilt**, which is the entire reason the
/// tile protocol exists: a page that changed one caret costs a small
/// sub-region upload rather than a full texture replacement.
#[derive(Default)]
pub struct Presenter {
    /// The tab being watched, if any.
    attached: Option<u64>,
    /// The picture of it, if a keyframe has arrived.
    held: Option<Held>,
    /// The last input sequence the server said it had applied when it painted.
    /// Read by the interface's readings, and by 05-10's controller after it.
    last_applied_input: u64,
}

impl Presenter {
    /// Begin watching `tab`.
    ///
    /// Clears whatever was held, so a new attachment cannot briefly show the
    /// previous tab's pixels — see [`Presenter::detach`] for why that is a
    /// privacy property and not only a correctness one.
    pub fn attach(&mut self, tab: u64) {
        self.attached = Some(tab);
        self.held = None;
        self.last_applied_input = 0;
    }

    /// Stop watching, and **clear the surface**.
    ///
    /// The clear is deliberate. Two agent tabs may belong to two different
    /// agents, so a surface left behind would show one agent's page to a viewer
    /// who has just attached to the other, for the whole gap between attaching
    /// and the first keyframe arriving.
    pub fn detach(&mut self) {
        self.attached = None;
        self.held = None;
        self.last_applied_input = 0;
    }

    /// The tab being watched, if any.
    pub fn attached(&self) -> Option<u64> {
        self.attached
    }

    /// The size of the surface held, if any.
    pub fn size(&self) -> Option<SurfaceSize> {
        self.held.as_ref().map(|held| held.size)
    }

    /// The last frame sequence applied, or zero before any frame was.
    pub fn last_frame_seq(&self) -> u64 {
        self.held.as_ref().map_or(0, |held| held.last_seq)
    }

    /// The last input sequence the server had applied when it painted the most
    /// recent frame this client accepted.
    ///
    /// The client's half of the input-to-photon estimate: it knows when it sent
    /// that sequence and when this frame arrived, and both readings come off
    /// **its own** clock, so nothing has to be synchronised across the two
    /// machines.
    pub fn last_applied_input(&self) -> u64 {
        self.last_applied_input
    }

    /// Apply one frame, or discard it.
    ///
    /// Returns whether the picture changed, which is what the caller turns into
    /// a redraw request.
    pub fn apply(
        &mut self,
        ctx: &egui::Context,
        header: &FrameHeader,
        payload: &[u8],
    ) -> bool {
        // A frame for a tab this client is not watching. Not an error and not
        // fatal: an attach and a detach cross on the wire, and the frames
        // already in flight for the old tab arrive after the new attachment.
        if self.attached != Some(header.tab_id) {
            return false;
        }
        let held = self.held.as_ref().map(|held| (held.size, held.last_seq));
        match decide(held, header) {
            // A frame that changes nothing: it arrived after one that
            // superseded it, or before this attachment had any surface to
            // composite onto. Debug rather than warn — on a lossy link this is
            // the ordering rule working, not a fault.
            Application::Discard => {
                log::debug!(
                    "discarding frame {} for tab {}, which does not advance what is held",
                    header.frame_seq,
                    header.tab_id,
                );
                false
            },
            Application::Replace(size) => {
                let Some(image) = decode(payload, size.width, size.height) else {
                    // One frame, not the view: the picture already held is left
                    // exactly as it was and the connection is undisturbed.
                    log::warn!(
                        "discarding a keyframe for tab {} that did not decode",
                        header.tab_id,
                    );
                    return false;
                };
                match self.held.as_mut() {
                    // Same texture dimensions: assign into the texture that
                    // already exists rather than allocating a second one.
                    Some(existing)
                        if (existing.size.width, existing.size.height)
                            == (size.width, size.height) =>
                    {
                        existing.texture.set(image, SAMPLING);
                        existing.size = size;
                        existing.last_seq = header.frame_seq;
                    },
                    // A differing size is the server saying the page changed
                    // size. The surface is **resized** — a new texture of the
                    // new dimensions — and the fit recomputed from it. Nothing
                    // asks the server for a size; this is the client adapting.
                    _ => {
                        self.held = Some(Held {
                            texture: ctx.load_texture("talaria-view-surface", image, SAMPLING),
                            size,
                            last_seq: header.frame_seq,
                        });
                    },
                }
                self.last_applied_input = header.last_applied_input;
                true
            },
            Application::Patch { x, y, width, height } => {
                let Some(image) = decode(payload, width, height) else {
                    log::warn!(
                        "discarding a delta for tab {} that did not decode",
                        header.tab_id,
                    );
                    return false;
                };
                let Some(held) = self.held.as_mut() else { return false };
                held.texture.set_partial([x as usize, y as usize], image, SAMPLING);
                held.last_seq = header.frame_seq;
                self.last_applied_input = header.last_applied_input;
                true
            },
        }
    }

    /// Draw the picture into `content`, and answer with the transform used.
    ///
    /// `None` when there is nothing to show, in which case a **defined
    /// placeholder** is drawn instead of whatever was last in that part of the
    /// window — the same clear-on-detach reasoning, applied to the pixels the
    /// interface itself would otherwise leave lying around.
    pub fn draw(&self, ui: &egui::Ui, content: egui::Rect) -> Option<Fit> {
        let painter = ui.painter_at(content);
        let Some(held) = self.held.as_ref() else {
            painter.rect_filled(content, 0.0, ui.visuals().extreme_bg_color);
            painter.text(
                content.center(),
                egui::Align2::CENTER_CENTER,
                match self.attached {
                    Some(tab) => format!("Waiting for the first frame of tab {tab}."),
                    None => "No tab is being watched.".to_owned(),
                },
                egui::TextStyle::Body.resolve(ui.style()),
                ui.visuals().weak_text_color(),
            );
            return None;
        };
        let fit = fit(held.size, content)?;
        painter.rect_filled(content, 0.0, ui.visuals().extreme_bg_color);
        painter.image(
            held.texture.id(),
            fit.surface_rect(),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        Some(fit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(kind: FrameKind, seq: u64, frame: (u32, u32), tile: (u32, u32, u32, u32)) -> FrameHeader {
        FrameHeader {
            kind,
            scale_denominator: 1,
            tab_id: 7,
            frame_seq: seq,
            last_applied_input: 0,
            tile_x: tile.0,
            tile_y: tile.1,
            tile_width: tile.2,
            tile_height: tile.3,
            frame_width: frame.0,
            frame_height: frame.1,
        }
    }

    fn keyframe(seq: u64, frame: (u32, u32)) -> FrameHeader {
        header(FrameKind::Keyframe, seq, frame, (0, 0, frame.0, frame.1))
    }

    fn size(width: u32, height: u32) -> SurfaceSize {
        SurfaceSize { width, height, denominator: 1 }
    }

    fn area(width: f32, height: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(360.0, 0.0), egui::vec2(width, height))
    }

    /// A real PNG of the given size, encoded exactly as the server encodes one.
    fn png_of(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let Ok(mut writer) = encoder.write_header() else {
                panic!("a header for a positive size must write");
            };
            let pixels = vec![0x40u8; (width as usize) * (height as usize) * 4];
            let Ok(()) = writer.write_image_data(&pixels) else {
                panic!("a buffer of exactly the declared size must write");
            };
        }
        bytes
    }

    #[test]
    fn a_wider_content_area_leaves_the_margin_on_the_left_and_right() {
        let Some(fitted) = fit(size(100, 100), area(400.0, 200.0)) else {
            panic!("a positive surface in a positive area must fit");
        };
        assert_eq!(fitted.scale, 2.0, "the shorter axis is what the scale follows");
        assert_eq!(fitted.origin, egui::vec2(100.0, 0.0));
        let rect = fitted.surface_rect();
        assert_eq!(rect.min, egui::pos2(460.0, 0.0));
        assert_eq!(rect.size(), egui::vec2(200.0, 200.0));
    }

    #[test]
    fn a_taller_content_area_leaves_the_margin_above_and_below() {
        let Some(fitted) = fit(size(100, 100), area(200.0, 400.0)) else {
            panic!("a positive surface in a positive area must fit");
        };
        assert_eq!(fitted.scale, 2.0);
        assert_eq!(fitted.origin, egui::vec2(0.0, 100.0));
        assert_eq!(fitted.surface_rect().size(), egui::vec2(200.0, 200.0));
    }

    #[test]
    fn an_exactly_matching_content_area_has_no_margin_and_unit_scale() {
        let Some(fitted) = fit(size(200, 400), area(200.0, 400.0)) else {
            panic!("a positive surface in a positive area must fit");
        };
        assert_eq!(fitted.scale, 1.0);
        assert_eq!(fitted.origin, egui::Vec2::ZERO);
        assert_eq!(fitted.surface_rect(), area(200.0, 400.0));
    }

    #[test]
    fn a_half_resolution_frame_occupies_the_same_area_as_a_full_one() {
        // The same page, painted at half resolution: half the texture in each
        // axis, and the *same* rectangle on screen. A client that fitted the
        // texture's own size would show this page at a quarter of the area.
        let full = fit(size(1200, 800), area(600.0, 500.0));
        let half = fit(
            SurfaceSize { width: 600, height: 400, denominator: 2 },
            area(600.0, 500.0),
        );
        let (Some(full), Some(half)) = (full, half) else {
            panic!("both surfaces are positive and must fit");
        };
        assert_eq!(full.surface_rect(), half.surface_rect(),
                   "the denominator did not compose with the fit");
        assert_eq!(half.page, (1200, 800), "the page size ignored the denominator");
    }

    #[test]
    fn a_surface_or_an_area_with_no_area_does_not_fit() {
        assert!(fit(size(0, 100), area(100.0, 100.0)).is_none());
        assert!(fit(size(100, 0), area(100.0, 100.0)).is_none());
        assert!(fit(size(100, 100), area(0.0, 100.0)).is_none());
        assert!(fit(size(100, 100), area(100.0, 0.0)).is_none());
    }

    #[test]
    fn the_first_frame_of_an_attachment_must_be_a_keyframe() {
        assert_eq!(
            decide(None, &keyframe(1, (200, 100))),
            Application::Replace(size(200, 100)),
        );
        assert_eq!(
            decide(None, &header(FrameKind::Tile, 1, (200, 100), (0, 0, 64, 64))),
            Application::Discard,
            "a delta before any keyframe has nothing to composite over",
        );
    }

    #[test]
    fn a_sequence_that_does_not_exceed_the_last_applied_is_discarded() {
        let held = Some((size(200, 100), 9));
        assert_eq!(
            decide(held, &header(FrameKind::Tile, 9, (200, 100), (0, 0, 64, 64))),
            Application::Discard,
            "the rule is 'exceeds', so an equal sequence must not apply",
        );
        assert_eq!(
            decide(held, &header(FrameKind::Tile, 8, (200, 100), (0, 0, 64, 64))),
            Application::Discard,
        );
        assert!(matches!(
            decide(held, &header(FrameKind::Tile, 10, (200, 100), (0, 0, 64, 64))),
            Application::Patch { .. },
        ));
    }

    #[test]
    fn a_delta_arriving_after_a_higher_numbered_keyframe_is_discarded() {
        // The awkward ordering case, and it needs no rule of its own: the
        // keyframe raised the high-water mark past this delta's sequence, so
        // the one comparison already refuses it.
        let after_keyframe = Some((size(200, 100), 40));
        assert_eq!(
            decide(after_keyframe, &header(FrameKind::Tile, 39, (200, 100), (0, 0, 64, 64))),
            Application::Discard,
        );
    }

    #[test]
    fn a_delta_names_exactly_its_own_region() {
        let held = Some((size(200, 100), 1));
        assert_eq!(
            decide(held, &header(FrameKind::Tile, 2, (200, 100), (128, 64, 64, 32))),
            Application::Patch { x: 128, y: 64, width: 64, height: 32 },
        );
    }

    #[test]
    fn a_keyframe_of_a_different_size_replaces_and_resizes() {
        let held = Some((size(200, 100), 5));
        assert_eq!(
            decide(held, &keyframe(6, (320, 240))),
            Application::Replace(size(320, 240)),
            "a page that changed size is a resize of the client's surface",
        );
    }

    #[test]
    fn a_delta_against_a_surface_of_another_size_is_discarded() {
        let held = Some((size(200, 100), 5));
        assert_eq!(
            decide(held, &header(FrameKind::Tile, 6, (320, 240), (0, 0, 64, 64))),
            Application::Discard,
            "the tile coordinates were computed against a different picture",
        );
    }

    #[test]
    fn a_keyframe_whose_denominator_changed_is_a_replacement_at_the_new_scale() {
        let held = Some((size(600, 400), 5));
        let mut halved = keyframe(6, (600, 400));
        halved.scale_denominator = 2;
        assert_eq!(
            decide(held, &halved),
            Application::Replace(SurfaceSize { width: 600, height: 400, denominator: 2 }),
        );
    }

    #[test]
    fn a_payload_that_is_not_an_image_decodes_to_nothing() {
        assert!(decode(b"", 8, 8).is_none());
        assert!(decode(b"this is not a png", 8, 8).is_none());
        assert!(decode(&png_of(8, 8)[..12], 8, 8).is_none());
    }

    #[test]
    fn a_payload_of_the_wrong_size_decodes_to_nothing() {
        let bytes = png_of(16, 8);
        assert!(decode(&bytes, 8, 8).is_none(), "a wider image than declared");
        assert!(decode(&bytes, 16, 16).is_none(), "a shorter image than declared");
    }

    #[test]
    fn a_payload_of_the_declared_size_decodes_to_exactly_that_many_pixels() {
        let Some(image) = decode(&png_of(12, 5), 12, 5) else {
            panic!("an eight-bit RGBA image of the declared size must decode");
        };
        assert_eq!(image.size, [12, 5]);
        assert_eq!(image.pixels.len(), 60);
    }

    #[test]
    fn the_upload_size_of_a_half_resolution_keyframe_is_the_texture_size() {
        // The denominator composes with the *fit*, not with the upload: the
        // bytes on the wire are the texture's own size and are decoded at it.
        let mut halved = keyframe(1, (300, 200));
        halved.scale_denominator = 2;
        let Application::Replace(size) = decide(None, &halved) else {
            panic!("a first keyframe must replace");
        };
        assert_eq!((size.width, size.height), (300, 200));
        assert_eq!((size.page_width(), size.page_height()), (600, 400));
        assert!(decode(&png_of(300, 200), size.width, size.height).is_some());
    }
}

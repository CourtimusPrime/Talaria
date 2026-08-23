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

use std::time::{Duration, Instant};

use servo::{OffscreenRenderingContext, RenderingContext, WebView};
use tokio::sync::mpsc::UnboundedSender;

use talaria_protocol::wire::{
    Channel, ClientView, FrameHeader, FrameKind, InputMessage, Rung, ServerView, TabList,
    FRAME_HEADER_LEN,
};
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
/// generator.
///
/// **It is a per-connection number and it is not the one that bounds the
/// cost**, which is the correction CR-02 made. This doc used to close with
/// "the number of connections is bounded by the token holder, and by a
/// revoke", and that clause was the whole hole: in this threat the token
/// holder *is* the attacker, so a per-connection ceiling bounds a number the
/// attacker chooses. The two caps that actually bound the readback cost are
/// [`DEFAULT_MAX_VIEW_CONNECTIONS`] and [`DEFAULT_MAX_TOTAL_ATTACH`]; this one
/// stays because refusing before anything is leased is still the right place
/// to say no to one connection asking for too much.
const DEFAULT_MAX_ATTACH: usize = 8;

/// The default ceiling on how many `/view` sockets this process serves at
/// once, whoever owns them.
///
/// **A total, because the per-connection cap bounds nothing an attacker has to
/// respect (CR-02).** One accepted token, K WebSocket upgrades, eight
/// attachments each: `take_due` walks every session and every attachment, so
/// the pump performs `K × 8` paints and framebuffer readbacks per interval on
/// the winit main thread. The browser stops answering the human, and the
/// consent panel that would revoke the token is drawn by the loop that is now
/// saturated — the operator's recovery path is the thing that stops
/// responding.
///
/// Four is chosen from the shape this phase exists for rather than from a
/// benchmark: one human at one other machine, watching. That is one live
/// socket. Four leaves room for a second window, for a reconnect racing a
/// socket the far end has already abandoned but this end has not yet noticed,
/// and for one spare — while staying an order of magnitude below any number at
/// which the socket bookkeeping itself matters. The expensive resource is
/// attachments rather than connections, and that is capped in its own right by
/// [`DEFAULT_MAX_TOTAL_ATTACH`].
pub const DEFAULT_MAX_VIEW_CONNECTIONS: usize = 4;

/// The default ceiling on live attachments across **every** connection.
///
/// The number that bounds what the winit loop pays: one attachment is one
/// `paint()` and one framebuffer readback per tick, and 05-02 measured that
/// pair at about a fifth of a 30 ms tick. Eight of them is already more than
/// one tick can hold at the fastest rung, which is what the rung ladder exists
/// to degrade — but past this point degrading is not enough and the honest
/// answer is to refuse the lease.
///
/// Deliberately the same number as [`DEFAULT_MAX_ATTACH`], so a single viewer
/// can still reach the full complement that cap always advertised, and so a
/// second viewer cannot multiply it.
const DEFAULT_MAX_TOTAL_ATTACH: usize = 8;

/// How many frames may be waiting for one viewer's socket before the pump
/// stops producing for it.
///
/// **The bound on the outbound queue, and it is applied by declining to
/// produce rather than by discarding what was produced** (CR-03). The channel
/// itself has to stay unbounded, because a bounded *send* would mean the winit
/// event loop waiting on a socket — the one thing it must never do. What was
/// missing was the other half: a policy for a reader that falls behind. A
/// viewer that stops reading (hostile, or ordinary on the 13 Mbit/s relayed
/// path 05-RESEARCH measured, where one 522 KB keyframe takes ~300 ms) closes
/// its receive window, `socket.send` stops returning, and the queue grew
/// without bound in the process holding the encrypted credential vault.
///
/// **Why not drop the oldest and keep the newest**, which is the usual answer
/// for a frame stream and is the wrong one here: this stream is *deltas*. A
/// client composites each tile frame onto the surface the previous one left,
/// so discarding an intermediate frame desynchronises the tile state and the
/// only repair is a whole keyframe — half a megabyte, sent to the one viewer
/// that has just proved it cannot drain half a megabyte. Declining to produce
/// leaves the queue holding a short run of small, in-order deltas that are all
/// still valid, so the viewer catches up to live by reading them rather than
/// by being resynchronised. Nothing is discarded, so nothing can desynchronise
/// and no keyframe is forced.
///
/// The cost is staleness, bounded by this number times the rung's interval —
/// at four frames and the fastest rung, about an eighth of a second, and the
/// client's own latency estimate will have walked the ladder down long before
/// it matters. The tick that is skipped also skips its `paint()` and its
/// framebuffer readback, so a stalled viewer stops costing the winit loop
/// anything rather than costing it the same and throwing the result away.
const MAX_QUEUED_FRAMES: usize = 4;

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

/// The connection cap's environment override,
/// `TALARIA_VIEW_MAX_CONNECTIONS`.
///
/// The same shape [`max_attachments`] uses, and for the same reason: an unset
/// variable, a value that is not a number and a value of zero all fall back to
/// the default — **never to zero and never to unbounded**.
pub fn max_view_connections() -> usize {
    parse_max_view_connections(std::env::var("TALARIA_VIEW_MAX_CONNECTIONS").ok().as_deref())
}

/// The connection cap the override spells, or the default. Split out with the
/// raw value as a parameter so a bad value is assertable without one test's
/// environment becoming another's answer.
fn parse_max_view_connections(raw: Option<&str>) -> usize {
    raw.and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_VIEW_CONNECTIONS)
}

/// The total-attachment cap's environment override,
/// `TALARIA_VIEW_MAX_TOTAL_ATTACH`. Same shape, same refusals.
pub fn max_total_attachments() -> usize {
    parse_max_total_attachments(std::env::var("TALARIA_VIEW_MAX_TOTAL_ATTACH").ok().as_deref())
}

/// The total cap the override spells, or the default.
fn parse_max_total_attachments(raw: Option<&str>) -> usize {
    raw.and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_TOTAL_ATTACH)
}

/// The idle threshold, overridable through
/// [`talaria_protocol::wire::VIEW_IDLE_ENV`].
///
/// The same shape [`max_attachments`] and `TALARIA_COMMAND_TIMEOUT_SECS` use:
/// an unset variable, a value that is not a number and a value of zero all
/// fall back to the default — **never to zero and never to unbounded**. The
/// parsing itself lives in the shared vocabulary, because the client's own
/// rate controller reads the same variable and the two must be one rule.
pub fn view_idle() -> Duration {
    parse_view_idle(std::env::var(talaria_protocol::wire::VIEW_IDLE_ENV).ok().as_deref())
}

/// The threshold the override spells, or the default. Split out with the raw
/// value as a parameter so a bad value is assertable without one test's
/// environment becoming another's answer.
fn parse_view_idle(raw: Option<&str>) -> Duration {
    Duration::from_millis(talaria_protocol::wire::parse_view_idle_ms(raw))
}

/// The interval one attachment should tick at, given the rung its viewer asked
/// for and how long ago that viewer's last input was accepted.
///
/// **The transition is one comparison, and it is not this function's.** It is
/// [`talaria_protocol::wire::is_driving`], in the shared vocabulary, because
/// the client's own rate controller makes the identical decision about the
/// identical quantity — the two ends are running **one rule** rather than two
/// that happen to agree today. Written out once there: an attachment enters the
/// driven cadence on the first accepted input for that tab on that connection
/// and returns to the passive cadence once the elapsed time since the last
/// accepted input *reaches* the threshold, with a new input always re-entering.
///
/// What is decided here is only which interval that answer selects: the rung's
/// own while driving, and the ladder's slowest rung's while not. The slowest
/// rung is the ladder's floor, so this is never faster than the rung asked for.
fn tick_interval(rung: Rung, since_last_input: Option<Duration>, idle: Duration) -> Duration {
    match talaria_protocol::wire::is_driving(since_last_input, idle) {
        true => rung.interval(),
        false => Rung::SLOWEST.interval(),
    }
}

/// One tab's pixels, off the engine and safe to move to another thread.
///
/// A plain buffer rather than the engine's own image type, and that is the
/// point: it is what makes the tile comparison and the encode a pure function
/// of two byte slices, testable without a live engine and — because it is
/// `Send` — movable off the winit loop, which is the whole of the design 05-02
/// confirmed.
///
/// `pixels` is RGBA8, row-major, exactly `width * height * 4` bytes.
pub struct Surface {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Surface {
    /// The engine's readback, taken apart into a plain buffer.
    fn from_image(image: servo::RgbaImage) -> Self {
        let (width, height) = (image.width(), image.height());
        Self { width, height, pixels: image.into_raw() }
    }

    /// Whether two surfaces describe the same geometry. A surface that changed
    /// size cannot be compared tile by tile with its predecessor, which is why
    /// this is one of the keyframe triggers rather than a case the comparison
    /// tries to handle.
    fn same_size_as(&self, other: &Surface) -> bool {
        self.width == other.width && self.height == other.height
    }

    /// One row-slice, `width` pixels wide starting at `(x, y)`. `None` when
    /// the request runs off the row or off the surface, which is a bug rather
    /// than a routine case and is therefore refused rather than silently
    /// clamped — a clamp here would produce a frame that decodes and is wrong.
    ///
    /// **The `x` bound is checked against the row's own width and not only
    /// against the buffer's length**, because a horizontal overrun on any row
    /// but the last one lands inside the buffer: it would read the beginning
    /// of the *next* row, which is a silently wrapped scanline rather than an
    /// out-of-range read anything would catch.
    fn row(&self, x: u32, y: u32, width: u32) -> Option<&[u8]> {
        if x.checked_add(width)? > self.width || y >= self.height {
            return None;
        }
        let start = ((y as usize) * (self.width as usize) + x as usize) * 4;
        let end = start + (width as usize) * 4;
        self.pixels.get(start..end)
    }

    /// How many tiles the grid over this surface has, partial edge tiles
    /// included. The denominator the keyframe threshold is a fraction of.
    fn tile_count(&self) -> usize {
        let across = self.width.div_ceil(TILE_SIZE) as usize;
        let down = self.height.div_ceil(TILE_SIZE) as usize;
        across * down
    }

    /// The whole surface as one region.
    fn whole(&self) -> Region {
        Region { x: 0, y: 0, width: self.width, height: self.height }
    }
}

/// The side of one comparison tile, in pixels.
///
/// 64 is what 05-02 measured against real engine output: a whole-frame scan of
/// the resulting 260-tile grid at 1280×800 costs 0.33 ms on a static page —
/// the scan's *worst* case, because nothing short-circuits when nothing
/// changed — and 0.11 ms while scrolling, when almost every tile differs on
/// its first compared row.
const TILE_SIZE: u32 = 64;

/// The keyframe threshold, as a fraction of the tile grid: send a whole
/// keyframe once more than nine twenty-sixths of the tiles have changed.
///
/// **Why a threshold exists at all, which is not obvious from the number.** It
/// is a bandwidth argument before it is a CPU one. Once most of the frame is
/// dirty the per-tile payloads add up to about what a whole keyframe costs on
/// the wire — 05-02 measured 260 scroll tiles at ≈517 KB against a 548 KB
/// keyframe — while the tile path additionally pays one envelope and one
/// header per tile. Time is what separates them: a 64×64 tile encode costs
/// ~0.02 ms including its fixed overhead and a whole page-like keyframe costs
/// 1.5–1.9 ms, and they meet at about 90 of 260 tiles.
///
/// **A fraction and not the literal 90**, so a viewport resize does not
/// silently re-tune the threshold: nine twenty-sixths is exactly 90/260 at
/// 1280×800 and stays a third of the grid at any other size.
const KEYFRAME_DIRTY_NUMERATOR: usize = 9;
/// The denominator of [`KEYFRAME_DIRTY_NUMERATOR`].
const KEYFRAME_DIRTY_DENOMINATOR: usize = 26;

/// One surface reduced by `denominator`, by nearest-neighbour sampling.
///
/// **The destination dimensions round up, and that is the whole of the
/// arithmetic worth commenting.** A source width that is not divisible by the
/// denominator has a partial column left over at its right-hand edge; a floor
/// would drop that strip, and the bottom strip with it, leaving a page whose
/// last few pixels are simply never sent. On screen that reads as a *rendering*
/// fault — a sliver of the page missing — rather than as an arithmetic choice
/// somebody made, which is exactly the kind of bug that gets looked for in the
/// wrong module. Rounding up costs at most one duplicated column and one
/// duplicated row, both of which are pixels the source actually has.
///
/// Nearest neighbour rather than an average: this runs on the encoder thread
/// once per tick per attachment, the destination is then compared tile by tile
/// against its predecessor, and an averaging filter would make a one-pixel
/// change dirty its neighbours' tiles as well. 05-02 measured the reduction at
/// a fraction of a millisecond against a 30 ms budget.
fn reduce(surface: &Surface, denominator: u8) -> Surface {
    let divisor = u32::from(denominator.max(1));
    if divisor == 1 || surface.width == 0 || surface.height == 0 {
        return Surface {
            width: surface.width,
            height: surface.height,
            pixels: surface.pixels.clone(),
        };
    }
    let width = surface.width.div_ceil(divisor);
    let height = surface.height.div_ceil(divisor);
    let mut pixels = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for y in 0..height {
        // The round-up means the last row and column may name a source pixel
        // one past the edge; they take the edge pixel rather than nothing.
        let source_y = (y * divisor).min(surface.height - 1);
        for x in 0..width {
            let source_x = (x * divisor).min(surface.width - 1);
            match surface.row(source_x, source_y, 1) {
                Some(pixel) => pixels.extend_from_slice(pixel),
                // Unreachable: both coordinates were bounded above. A frame
                // with a hole in it is worse than a frame that is one pixel
                // repeated, so the degrade is opaque black rather than a panic.
                None => pixels.extend_from_slice(&[0, 0, 0, 255]),
            }
        }
    }
    Surface { width, height, pixels }
}

/// A rectangle of a surface, in that surface's own pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    /// Whether this region is the whole of `surface`.
    fn covers(&self, surface: &Surface) -> bool {
        self.x == 0 && self.y == 0 && self.width == surface.width && self.height == surface.height
    }
}

/// Every tile of `current` that differs from `previous`, by tile origin.
///
/// Row slices with a short-circuit on the first differing row, which is why a
/// mostly-static frame is cheap and why a scrolling one is *cheaper still* —
/// almost every tile differs on its first row.
///
/// **Partial tiles at the right and bottom edges are the part worth getting
/// right.** The grid does not divide a viewport evenly in general, so the last
/// column and the last row are narrower or shorter than the rest; comparing
/// them at the full tile size would run off the buffer, and rounding them away
/// would leave a strip of the frame never compared and therefore never sent —
/// a stripe of stale pixels that reads as a rendering bug rather than as a
/// protocol error.
///
/// Both surfaces must be the same size; the caller decides that, because a
/// size change is a keyframe rather than a comparison.
fn dirty_tiles(previous: &Surface, current: &Surface) -> Vec<(u32, u32)> {
    let mut dirty = Vec::new();
    let mut tile_y = 0;
    while tile_y < current.height {
        let tile_height = TILE_SIZE.min(current.height - tile_y);
        let mut tile_x = 0;
        while tile_x < current.width {
            let tile_width = TILE_SIZE.min(current.width - tile_x);
            let changed = (tile_y..tile_y + tile_height).any(|y| {
                current.row(tile_x, y, tile_width) != previous.row(tile_x, y, tile_width)
            });
            if changed {
                dirty.push((tile_x, tile_y));
            }
            tile_x += TILE_SIZE;
        }
        tile_y += TILE_SIZE;
    }
    dirty
}

/// The smallest rectangle covering every dirty tile.
///
/// One message covering the bounding region rather than one message per tile:
/// a caret and a word of typing are a handful of adjacent tiles, and paying
/// one header and one envelope for the group beats paying several for pixels
/// that were going to travel together anyway.
fn bounding_region(tiles: &[(u32, u32)], surface: &Surface) -> Option<Region> {
    let (first_x, first_y) = *tiles.first()?;
    let (mut min_x, mut min_y) = (first_x, first_y);
    let (mut max_x, mut max_y) = (0, 0);
    for (x, y) in tiles {
        min_x = min_x.min(*x);
        min_y = min_y.min(*y);
        max_x = max_x.max(x + TILE_SIZE.min(surface.width.saturating_sub(*x)));
        max_y = max_y.max(y + TILE_SIZE.min(surface.height.saturating_sub(*y)));
    }
    Some(Region {
        x: min_x,
        y: min_y,
        width: max_x.saturating_sub(min_x),
        height: max_y.saturating_sub(min_y),
    })
}

/// What to send for one tick, or `None` for a tick with nothing to say.
///
/// The four keyframe triggers, in the order they are cheapest to decide: the
/// caller asked for one (an attach, a re-attach, or a viewport change), there
/// is no previous frame to diff against, the surface changed size, or more of
/// the grid changed than [`KEYFRAME_DIRTY_NUMERATOR`] allows. `None` is the
/// common case and the delta model's whole payoff: a page nobody is touching
/// costs one readback and one comparison per tick and sends nothing at all.
pub fn select_frame(
    previous: Option<&Surface>,
    current: &Surface,
    keyframe: bool,
) -> Option<(FrameKind, Region)> {
    // A surface with no area cannot be described by a header the wire will
    // accept — `FrameHeader::from_bytes` refuses a zero-sized tile — so it is
    // refused here rather than composed and rejected at the far end.
    if current.width == 0 || current.height == 0 {
        return None;
    }
    let whole = (FrameKind::Keyframe, current.whole());
    if keyframe {
        return Some(whole);
    }
    let Some(previous) = previous else { return Some(whole) };
    if !current.same_size_as(previous) {
        return Some(whole);
    }
    let dirty = dirty_tiles(previous, current);
    if dirty.is_empty() {
        return None;
    }
    if dirty.len() * KEYFRAME_DIRTY_DENOMINATOR
        > current.tile_count() * KEYFRAME_DIRTY_NUMERATOR
    {
        return Some(whole);
    }
    bounding_region(&dirty, current).map(|region| (FrameKind::Tile, region))
}

/// One region of a surface as a contiguous RGBA buffer.
fn crop(surface: &Surface, region: Region) -> Option<Vec<u8>> {
    let mut out =
        Vec::with_capacity((region.width as usize) * (region.height as usize) * 4);
    for y in region.y..region.y.checked_add(region.height)? {
        out.extend_from_slice(surface.row(region.x, y, region.width)?);
    }
    Some(out)
}

/// The frame path's PNG encoder.
///
/// **A sibling of the shell's screenshot encoder, not a modification of it and
/// not a call into it** — `D-05-05`. The two do different jobs and each is
/// right for its own: a one-shot agent screenshot is a whole surface delivered
/// once inside a JSON reply, and a frame is a rectangle delivered thirty times
/// a second down a binary channel. Sharing one function would make one of them
/// wrong, so the structure is copied and the divergence is deliberate.
///
/// The divergence is **one line**, and that is worth stating because the plan
/// this landed under was written expecting three. 05-02 read `png 0.17.16`'s
/// own `Info::default()` and found that the compression level and the filter
/// set explicitly below are already what an unconfigured encoder uses — it
/// proved it by encoding one real frame both ways and getting byte-identical
/// output. They are set here anyway, because a default that happens to agree
/// today is not the same thing as a choice, and this path's choice is a
/// measured one. What actually differs is the last line: raw bytes, because
/// this channel is binary, where the screenshot path's JSON reply forces a
/// text encoding that costs a third again in size and a measurable slice of
/// the budget.
fn encode_frame(pixels: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let mut png_data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_data, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder.set_filter(png::FilterType::Sub);
        let Ok(mut writer) = encoder.write_header() else {
            return None;
        };
        if writer.write_image_data(pixels).is_err() {
            return None;
        }
    }
    Some(png_data)
}

/// One frame-channel message: the tag byte, the fixed header, then the encoded
/// pixels.
///
/// Sized from [`FRAME_HEADER_LEN`] rather than from a second literal, so the
/// writer here and the reader on the far side take the number from one place.
fn frame_message(header: &FrameHeader, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    body.extend_from_slice(&header.to_bytes());
    body.extend_from_slice(payload);
    encode(Channel::Frame, &body)
}

/// One viewer's outbound wire, and how much is waiting on it.
///
/// **The channel is unbounded and the *production* is bounded** — see
/// [`MAX_QUEUED_FRAMES`] for why round that way. This type exists because
/// `tokio`'s `UnboundedSender` cannot be asked its own depth (only the
/// receiver can, and the receiver lives on the listener thread), so the count
/// is kept alongside it: raised by every write and lowered when the socket
/// task takes the item off. It is therefore "queued, and not yet in the socket
/// task's hand" — an undercount of one against "not yet on the wire", which is
/// immaterial at a ceiling of four and is the direction that errs toward
/// producing rather than stalling.
///
/// Cloned onto the encoder thread with the tick, so the thread that writes the
/// bytes is the thread that raises the count.
///
/// `Debug` because [`crate::app::AppEvent`] derives it and winit's user event
/// requires it; it prints the depth and nothing about what is on the wire.
#[derive(Clone)]
pub struct ViewChannel {
    frames: UnboundedSender<Vec<u8>>,
    depth: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl std::fmt::Debug for ViewChannel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ViewChannel").field("depth", &self.depth()).finish()
    }
}

/// The read end of a [`ViewChannel`], held by the socket task.
///
/// Taking an item is the *only* way to get one, and taking is what lowers the
/// depth — so the count cannot drift by a caller forgetting to report a write.
pub struct ViewReader {
    frames: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    depth: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

/// One viewer's outbound wire, both ends.
pub fn view_channel() -> (ViewChannel, ViewReader) {
    let (frames, inbox) = tokio::sync::mpsc::unbounded_channel();
    let depth = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    (
        ViewChannel { frames, depth: std::sync::Arc::clone(&depth) },
        ViewReader { frames: inbox, depth },
    )
}

impl ViewChannel {
    /// Write one message toward the viewer.
    ///
    /// A closed channel is not an error: the socket task has ended and the
    /// session is about to be removed. The count is put back in that case, so
    /// a dead connection does not leave a permanently raised depth behind on
    /// the clones the encoder thread still holds.
    fn send(&self, message: Vec<u8>) {
        self.depth.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.frames.send(message).is_err() {
            self.depth.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// How many messages are queued and not yet taken by the socket task.
    fn depth(&self) -> usize {
        self.depth.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl ViewReader {
    /// The next message for this viewer, or `None` once the main thread has
    /// dropped the connection.
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        let message = self.frames.recv().await;
        if message.is_some() {
            self.depth.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
        message
    }

    /// The next message if one is already queued, without waiting.
    ///
    /// Test-only: the socket task awaits, and a poll that gave up would be a
    /// busy loop on the one path that must not have one. It exists so a suite
    /// can model a viewer that reads, and a viewer that does not.
    #[cfg(test)]
    pub(crate) fn try_recv(&mut self) -> Option<Vec<u8>> {
        let message = self.frames.try_recv().ok();
        if message.is_some() {
            self.depth.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
        message
    }
}

/// One tick's pixels and everything the encoder needs to describe them.
struct Frame {
    connection: u64,
    tab: u64,
    frame_seq: u64,
    last_delivered_input: u64,
    keyframe: bool,
    /// The divisor the rung this attachment is on implies. The reduction runs
    /// **here**, on the encoder thread, and never on the winit loop.
    scale_denominator: u8,
    surface: Surface,
    out: ViewChannel,
}

/// What the main thread asks of the encoder thread.
enum Job {
    /// Compare, encode and send one frame.
    Frame(Frame),
    /// An attachment ended: drop the previous-frame buffer it owned. **This is
    /// the memory bound**, and it is a message rather than a timeout because
    /// nothing else knows when a lease ends.
    Release { connection: u64, tab: u64 },
}

/// The frame encoder thread's handle.
///
/// **Degrades rather than aborting**, following the listener's spawn shape: a
/// checked spawn, a recorded reason, and a browser that keeps working. A
/// browser whose frame encoder could not start is still a browser — the local
/// human loses nothing, and a viewer is refused with the one refusal rather
/// than left attached to a stream that will never produce a frame.
#[derive(Default)]
pub struct FrameEncoder {
    /// The way onto the thread, absent when the thread could not be started.
    jobs: Option<std::sync::mpsc::Sender<Job>>,
    /// Why it could not be started, taken once by the loop so the failure is
    /// reported and not repeated.
    failure: Option<String>,
}

impl FrameEncoder {
    /// Start the thread, or record why it could not start.
    fn start() -> Self {
        let (jobs, inbox) = std::sync::mpsc::channel();
        match std::thread::Builder::new()
            .name("talaria-frames".into())
            .spawn(move || encode_frames(inbox))
        {
            Ok(_) => Self { jobs: Some(jobs), failure: None },
            Err(error) => {
                log::error!("the frame encoder thread could not be spawned: {error}");
                Self { jobs: None, failure: Some(error.to_string()) }
            },
        }
    }

    /// Whether frames can be encoded at all.
    fn running(&self) -> bool {
        self.jobs.is_some()
    }

    fn send(&self, job: Job) {
        let Some(jobs) = self.jobs.as_ref() else { return };
        // A closed channel means the thread has ended, which the log line
        // above already reported if it was ever going to be reported. Frames
        // stop; the browser does not.
        let _ = jobs.send(job);
    }
}

/// The encoder thread: compare, encode, and write onto each connection's own
/// outbound channel.
///
/// **What it owns, and the bound on it.** One previous-frame buffer per live
/// *attachment* — not per tab, because two viewers on one tab have their own
/// sequences and may be at different cadences, so one shared buffer would give
/// whichever of them ticked second a delta against the other's frame. Each
/// buffer is one full surface, and the total is bounded by the concurrent
/// attachment cap 05-05 set times the number of connections, released on
/// detach, on disconnect and on the tab closing.
///
/// Ends when the last sender is dropped, which is when the browser is exiting.
fn encode_frames(inbox: std::sync::mpsc::Receiver<Job>) {
    let mut previous: std::collections::BTreeMap<(u64, u64), Surface> =
        std::collections::BTreeMap::new();
    while let Ok(job) = inbox.recv() {
        match job {
            Job::Release { connection, tab } => {
                previous.remove(&(connection, tab));
            },
            Job::Frame(mut frame) => {
                let key = (frame.connection, frame.tab);
                // **Reduced before anything else looks at it**, so the tile
                // comparison, the keyframe decision, the crop and the header
                // all speak one coordinate space — the one the client's texture
                // is actually in. The previous-frame buffer therefore holds the
                // reduced surface too, and a rung change, which changes the
                // denominator, changes the buffer's size and is independently a
                // keyframe trigger on top of the one the rung change forces.
                if frame.scale_denominator > 1 {
                    frame.surface = reduce(&frame.surface, frame.scale_denominator);
                }
                let selected =
                    select_frame(previous.get(&key), &frame.surface, frame.keyframe);
                let delivered = match selected {
                    // Nothing changed. The buffer already held is still an
                    // accurate picture of what the client has.
                    None => true,
                    Some((kind, region)) => match compose(&frame, kind, region) {
                        Some(message) => {
                            frame.out.send(message);
                            true
                        },
                        None => {
                            log::warn!(
                                "frame {} for tab {} on view connection {} could not be \
                                 encoded; the viewer keeps the frame it has",
                                frame.frame_seq,
                                frame.tab,
                                frame.connection
                            );
                            false
                        },
                    },
                };
                // Only what the client actually holds becomes the reference.
                // Recording a frame that was never delivered would make every
                // later delta a diff against pixels nobody has.
                if delivered {
                    previous.insert(key, frame.surface);
                }
            },
        }
    }
}

/// One frame, header and payload, ready for the wire.
fn compose(frame: &Frame, kind: FrameKind, region: Region) -> Option<Vec<u8>> {
    let surface = &frame.surface;
    let payload = match region.covers(surface) {
        true => encode_frame(&surface.pixels, region.width, region.height)?,
        false => encode_frame(&crop(surface, region)?, region.width, region.height)?,
    };
    let header = FrameHeader {
        kind,
        scale_denominator: frame.scale_denominator,
        tab_id: frame.tab,
        frame_seq: frame.frame_seq,
        // Stamped from the value the input path recorded on this connection,
        // read at the moment the tick painted. It is the whole of the latency
        // instrumentation: the client knows when it sent that sequence and
        // when this frame arrived, both off its own clock, so no clock has to
        // be shared between the two machines.
        last_delivered_input: frame.last_delivered_input,
        tile_x: region.x,
        tile_y: region.y,
        tile_width: region.width,
        tile_height: region.height,
        // The **reduced** dimensions, which is what the header means by "the
        // whole frame's width at the declared scale". The client multiplies
        // them by the denominator to recover the page's own size, so it
        // reconstructs the placement from two declared facts rather than
        // inferring a scale from the ratio of two sizes — which is what it
        // would have to do if this carried the source dimensions instead, and
        // what would guess wrong on a source size the denominator does not
        // divide.
        frame_width: surface.width,
        frame_height: surface.height,
    };
    Some(frame_message(&header, &payload))
}

/// What one tick of the pump got from the engine.
///
/// Two answers rather than an `Option`, so a skipped tick cannot be read as an
/// empty frame at the call site: a failed framebuffer read leaves the lease
/// perfectly good and the next tick tries again. 05-02 saw this outcome zero
/// times in nine hundred sustained ticks, which is why it is worth a line in
/// the log and nothing more drastic.
pub enum CaptureOutcome {
    /// The pixels this tick painted.
    Painted(Surface),
    /// The framebuffer read did not answer.
    ReadFailed,
}

/// Paint a tab's webview and read its framebuffer back.
///
/// **Unconditionally, without waiting for a frame-ready notification, and
/// without the screenshot path's queue.** The engine's own documentation says
/// an embedder may paint and present without one and that Servo repaints even
/// when it is not first told to (`servo-0.4.0/webview.rs:77`), which is what
/// licenses this. Waiting would be wrong twice over: a static page never
/// produces such a notification at all, and the screenshot queue answers the
/// wait with a one-and-a-half-second deadline, which is a screenshot-shaped
/// answer to a frame-shaped question — at thirty-three ticks a second it would
/// be thirty-three dead deadlines and a static page delivering nothing until
/// each one expired. Painting unconditionally and letting the tile comparison
/// decide whether anything is worth sending costs one readback and one
/// comparison on a page nobody is touching, and sends nothing.
///
/// Takes the two handles rather than the tab table, so the caller can clone
/// them out under a scoped borrow and release it **before** the slowest step
/// in the tick. The engine's cells are borrowed from its own callbacks too,
/// and a borrow held across a readback is a borrow held across the slowest
/// thing in the loop.
pub fn capture(webview: &WebView, context: &OffscreenRenderingContext) -> CaptureOutcome {
    webview.paint();
    let size = context.size2d().to_i32();
    let rect = euclid::Box2D::from_origin_and_size(
        euclid::Point2D::origin(),
        euclid::Size2D::new(size.width, size.height),
    );
    match context.read_to_image(rect) {
        Some(image) => CaptureOutcome::Painted(Surface::from_image(image)),
        None => CaptureOutcome::ReadFailed,
    }
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
    /// Hold an agent-owned tab shown for a viewer, answering whether the hold
    /// was taken. A tab the human owns takes none.
    fn hold_for_view(&mut self, tab: u64) -> bool;
    /// Release one hold. Releasing one that was never taken, or one on a tab
    /// that has since closed, is not an error.
    fn release_view_hold(&mut self, tab: u64);
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

    /// Straight through to the tab table's own hold, which is agent-only for
    /// the same reason [`TabManager::agent_tab`] is: the filter is a lookup
    /// rather than a check, so a tab the human owns cannot be held at all.
    fn hold_for_view(&mut self, tab: u64) -> bool {
        TabManager::hold_for_view(self, tab)
    }

    fn release_view_hold(&mut self, tab: u64) {
        TabManager::release_view_hold(self, tab)
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

/// One tab a connection holds a view lease on, and everything the pump needs
/// to decide when to paint it next.
///
/// Per attachment and never per tab: two viewers on one tab have their own
/// sequence spaces, their own keyframes and their own cadences, so a record
/// shared between them would let either one's scroll reset the other's frame
/// numbering.
struct Attachment {
    /// The agent-owned tab this lease is on.
    tab: u64,
    /// The sequence the next frame for this attachment will carry. Starts at
    /// one and only ever increases; a tick whose readback failed still
    /// consumes its number, because the contract is *strictly increasing* and
    /// not *contiguous*, and a client that re-used a number could not tell a
    /// superseded frame from a fresh one.
    next_frame_seq: u64,
    /// When this attachment is next due to be painted.
    due: Instant,
    /// When this attachment last had a tick taken, if ever. `None` before the
    /// first one.
    ///
    /// **The floor the ladder is enforced against**, and the reason it is
    /// recorded rather than inferred from `due`: `due` is a deadline that
    /// anything may pull forward, so it cannot also be the record of when the
    /// last tick happened. See [`Attachment::pull_forward`].
    last_tick: Option<Instant>,
    /// When this connection's last input aimed at this tab was accepted, if
    /// any. `None` is a viewer that has only ever watched — see
    /// [`tick_interval`] for the transition this drives.
    last_input: Option<Instant>,
    /// Whether the next frame must be a whole keyframe regardless of what
    /// changed: set on attach, on a re-attach that lost its acknowledgement,
    /// and on a viewport change.
    keyframe: bool,
}

impl Attachment {
    fn new(tab: u64, now: Instant) -> Self {
        Self {
            tab,
            next_frame_seq: 1,
            // Due immediately: the first frame is the one the viewer is
            // waiting on before it can draw anything at all.
            due: now,
            last_tick: None,
            last_input: None,
            keyframe: true,
        }
    }

    /// The interval this attachment is currently ticking at, on `rung`.
    fn interval(&self, rung: Rung, now: Instant, idle: Duration) -> Duration {
        tick_interval(rung, self.last_input.map(|at| now.saturating_duration_since(at)), idle)
    }

    /// Bring the next tick as far forward as the ladder allows, and no
    /// further.
    ///
    /// **The bound on the whole frame pump, and it lives here because the
    /// pull-forward does.** [`ClientView::Cadence`]'s own doc records that a
    /// client-invented interval was *removed* rather than bounded, because it
    /// was "a way for one viewer to pin the engine's loop at whatever rate it
    /// liked" (T-05-12-D). It was not removed: it moved to the paths that pull
    /// this deadline forward — an accepted input, and a re-attach — and until
    /// this floor existed neither had any bound at all. A viewer writing
    /// in-bounds `mouse_move` messages in a tight loop made the attachment due
    /// on every one of them, and each due tick is a `paint()` plus a
    /// framebuffer readback **on the winit main thread**. At the attachment
    /// cap that is a browser the local human cannot get a click into —
    /// including the click that would revoke the token, because the Access
    /// panel is drawn by the loop being pinned.
    ///
    /// So the earliest a tick may be taken is one rung interval after the last
    /// one. That keeps the responsiveness the pull-forward exists for — an
    /// honest click does not wait out a passive interval, because after an idle
    /// stretch the floor is already in the past — while making the rung's
    /// interval the *rate* ceiling it was always documented to be. The floor is
    /// the rung's own rather than the passive one on purpose: a viewer is
    /// entitled to the cadence it asked for, and nothing faster.
    fn pull_forward(&mut self, rung: Rung, now: Instant) {
        let floor = self.last_tick.map_or(now, |last| (last + rung.interval()).max(now));
        self.due = self.due.min(floor);
    }
}

/// One attachment's tick, taken out of the session table so the work can be
/// done with the borrow released.
pub struct DueTick {
    pub connection: u64,
    pub tab: u64,
    /// The sequence this frame will carry, already consumed.
    pub frame_seq: u64,
    /// The highest input sequence that had actually reached a page on this
    /// connection when the tick was taken — stamped into the header the
    /// encoder assembles, and the whole of the latency instrumentation.
    ///
    /// **Deliberately not [`ViewSession::last_applied_input`]**, which
    /// advances on refusals too; see
    /// [`ViewSession::last_delivered_input`] for why echoing that one
    /// reported a latency for a round trip that never happened (WR-10).
    pub last_delivered_input: u64,
    /// Whether this frame must be a keyframe whatever the comparison says.
    pub keyframe: bool,
    /// The divisor this connection's rung implies, carried with the tick so the
    /// encoder thread does not have to reach back into the session table.
    pub scale_denominator: u8,
    /// This connection's outbound frames, cloned so the encoder thread can
    /// write onto it without reaching back into the main thread's tables.
    pub out: ViewChannel,
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
    /// This connection's outbound frames. The channel is unbounded because a
    /// bounded *send* would mean the event loop waiting on a socket, which is
    /// the one thing it must never do; what bounds it instead is
    /// [`MAX_QUEUED_FRAMES`], consulted in [`ViewSessions::take_due`] before
    /// anything is produced.
    out: ViewChannel,
    /// The tabs this connection holds a view lease on, in the order it
    /// attached to them. Bounded by [`max_attachments`].
    attached: Vec<Attachment>,
    /// Whether this connection has been sent a snapshot yet. A connection that
    /// has not must get one even when the snapshot has not changed, or a
    /// viewer joining a quiet browser would wait for a tab to open before
    /// learning there are none.
    snapshotted: bool,
    /// The highest input sequence number this connection has had accepted.
    ///
    /// **One field, two jobs** — it used to be three, and the third is now
    /// [`ViewSession::last_delivered_input`]. It is the replay resistance
    /// inside a connection (a number cannot be used twice), and it is the
    /// ordering rule under coalescing (a late message that lost a race is
    /// dropped rather than applied out of order).
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
    /// The highest input sequence that actually **reached a page** on this
    /// connection.
    ///
    /// A second field rather than a second use of the first, which is the
    /// correction WR-10 made. `last_applied_input` advances on refusals, on
    /// purpose and correctly — but it was also what
    /// [`talaria_protocol::wire::FrameHeader::last_delivered_input`] echoed,
    /// and the client turns that echo into `reading.input_to_photon_ms`, which
    /// `scripts/two-machine-check.sh` names as the phase's headline evidence.
    /// So a viewer clicking in the letterboxed margin, or on a crashed tab, or
    /// on a tab it had never attached to, produced a latency figure for a
    /// round trip that never included a hit test or a repaint — biased *low*,
    /// in the direction that makes the number look better.
    ///
    /// Written only by [`ViewSessions::delivered_input`], from
    /// [`crate::remote_input::apply`]'s `true` return, which is the one place
    /// that knows a page was reached.
    pub last_delivered_input: u64,
    /// The rung of [`talaria_protocol::wire::RUNG_LADDER`] this connection last
    /// asked for.
    ///
    /// **Per connection and not per attachment, because the request is.**
    /// [`ClientView::Cadence`] names no tab: a viewer shows one picture at a
    /// time and what it has learned about the link is a fact about the link,
    /// not about a tab. A second attachment on the same connection inherits it
    /// rather than rediscovering it.
    ///
    /// Starts at the fastest rung, so a viewer that never sends a cadence
    /// request at all — every hand-composed client in the end-to-end suite —
    /// gets exactly the behaviour the frame pump shipped with: full
    /// resolution, the fast interval while driving, the slowest rung's interval
    /// while idle.
    rung: Rung,
}

impl ViewSession {
    /// Push one frame toward this viewer. A closed channel is not an error:
    /// the socket task has ended and the session is about to be removed.
    fn send(&self, frame: Option<Vec<u8>>) {
        if let Some(frame) = frame {
            self.out.send(frame);
        }
    }

    /// How many tabs this connection is attached to. Test-only: nothing on a
    /// shipped path needs the number.
    #[cfg(test)]
    fn attachment_count(&self) -> usize {
        self.attached.len()
    }

    /// Whether this connection holds a lease on `tab`.
    #[cfg(test)]
    fn holds(&self, tab: u64) -> bool {
        self.attached.iter().any(|attachment| attachment.tab == tab)
    }

    fn attachment_mut(&mut self, tab: u64) -> Option<&mut Attachment> {
        self.attached.iter_mut().find(|attachment| attachment.tab == tab)
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
    /// The encoder thread, started on the first attach and never before it: a
    /// browser nobody is watching spawns no thread and holds no buffers.
    encoder: Option<FrameEncoder>,
}

impl ViewSessions {
    /// Record a newly accepted connection.
    pub fn opened(&mut self, connection: u64, client_id: String, out: ViewChannel) {
        self.sessions.push(ViewSession {
            connection,
            client_id,
            out,
            attached: Vec::new(),
            snapshotted: false,
            last_applied_input: 0,
            last_delivered_input: 0,
            rung: Rung::FASTEST,
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
    /// held by a connection that is gone is a webview this browser keeps shown
    /// and repaints for nobody.
    pub fn closed(&mut self, connection: u64, tabs: &mut dyn ViewTabs) {
        let Some(index) = self.sessions.iter().position(|s| s.connection == connection) else {
            return;
        };
        let session = self.sessions.remove(index);
        let released: Vec<u64> = session.attached.iter().map(|a| a.tab).collect();
        for tab in released {
            tabs.release_view_hold(tab);
            self.release_frames(connection, tab);
        }
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
    pub fn message(&mut self, connection: u64, frame: &[u8], tabs: &mut dyn ViewTabs) -> Handled {
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
        let rung = session.rung;
        let Some(attachment) = session.attachment_mut(tab) else { return false };
        // The driven cadence starts here, and it starts on *this* tick rather
        // than at the end of the passive interval already in flight: the input
        // has just landed and the next photon is the one being measured. See
        // [`tick_interval`] for the rule that takes it back down again.
        //
        // **Through the floor, never straight to `now`.** Without it the rung
        // ladder bounded only a silent viewer and a driving one set the loop's
        // rate — see [`Attachment::pull_forward`], which is the whole of that
        // argument.
        let now = Instant::now();
        attachment.last_input = Some(now);
        attachment.pull_forward(rung, now);
        true
    }

    /// Record that input sequence `seq` on `connection` actually reached a
    /// page.
    ///
    /// **The other half of [`ViewSessions::admit_input`], and separate from it
    /// on purpose** (WR-10). Admission answers "may this be delivered", and it
    /// consumes the sequence number whether or not the delivery then succeeds,
    /// because a number that could be reused is a message that could be
    /// replayed (T-05-15). This answers "did it arrive", which is a different
    /// fact and the only one a latency figure may be computed from — see
    /// [`ViewSession::last_delivered_input`].
    ///
    /// Called by [`crate::remote_input::apply`] and by nothing else, because
    /// that is the only code that has a webview's answer. `max` rather than
    /// assignment so the mark cannot go backwards, which matters not for the
    /// ordinary path — the sequence rule already made `seq` the highest
    /// accepted — but so that a later caller cannot lower it by accident.
    pub fn delivered_input(&mut self, connection: u64, seq: u64) {
        if let Some(session) = self.session_mut(connection) {
            session.last_delivered_input = session.last_delivered_input.max(seq);
        }
    }

    /// Answer one control-channel message.
    fn control(&mut self, connection: u64, message: ClientView, tabs: &mut dyn ViewTabs) {
        match message {
            ClientView::Attach { tab } => self.attach(connection, tab, tabs),
            ClientView::Detach { tab } => self.detach(connection, tab, tabs),
            // A resize is a hint about how the viewer wants `tab` painted.
            // What it costs the server is one keyframe: the client is about to
            // reallocate its surface, so every tile it holds is about to be
            // meaningless and a delta composited onto a resized texture would
            // be a stripe of stale pixels. It is still answered on the one
            // question this module can decide first: whether this connection
            // holds a lease on the tab it named. A viewer that could resize a
            // tab it never attached to would be reaching a tab it was refused.
            ClientView::Viewport { tab, .. } => match self.require_keyframe(connection, tab) {
                true => {},
                false => self.refuse(connection),
            },
            // A named rung, looked up in the one table both ends read. A rung
            // this build knows takes effect from the next tick and is not
            // acknowledged — the server delivers at whatever rate the link can
            // carry, and a reply here would be a commitment this side cannot
            // keep. A name this build does not know gets the one reasonless
            // refusal, exactly as every other refusal on this channel does.
            ClientView::Cadence { rung } => match rung {
                Some(rung) => self.set_rung(connection, rung),
                None => self.refuse(connection),
            },
        }
    }

    /// Begin a view lease on `tab` for `connection`.
    ///
    /// **Every refusal below is the same refusal**, with no field to differ in
    /// ([`ServerView::Refused`]). A tab the human owns, a tab that was never
    /// created, and a tab beyond this connection's cap all produce identical
    /// bytes — because a viewer that could tell them apart could enumerate the
    /// human's own browsing without ever reaching a page of it (T-05-05-A).
    fn attach(&mut self, connection: u64, tab: u64, tabs: &mut dyn ViewTabs) {
        // Through the agent-only lookup, so a human-owned tab yields nothing
        // rather than yielding a tab that is then rejected (`D-05-02`).
        let Some((width, height)) = tabs.agent_viewport(tab) else {
            self.refuse(connection);
            return;
        };
        // **An attachment that cannot be encoded is refused rather than
        // accepted and left silent.** A lease whose frames will never be
        // produced holds a webview shown for nobody and gives the viewer a
        // stream that never starts — the failure it would learn about only by
        // waiting. It is the one refusal, as every refusal here is.
        if !self.encoder().running() {
            self.refuse(connection);
            return;
        }
        let cap = max_attachments();
        // **The total, counted before the per-connection one is consulted.**
        // The per-connection cap bounds a number the attacker chooses, because
        // nothing stops that attacker opening more connections; this is the
        // one that bounds what the winit loop pays per tick (CR-02). It is the
        // same refusal, as every refusal here is.
        let total_cap = max_total_attachments();
        let total: usize = self.sessions.iter().map(|session| session.attached.len()).sum();
        let now = Instant::now();
        let mut take_hold = false;
        {
            let Some(session) = self.session_mut(connection) else { return };
            let rung = session.rung;
            match session.attachment_mut(tab) {
                // A client that lost its own acknowledgement and asked again
                // is starting from nothing, so it gets a fresh keyframe as
                // well as the same answer — a delta against a surface it no
                // longer holds would composite onto an empty texture.
                //
                // Through the same floor the input channel goes through: a
                // re-attach is the *second* way to pull this deadline forward,
                // and a viewer writing `Attach` in a loop would otherwise pin
                // the loop exactly as an input flood would.
                Some(attachment) => {
                    attachment.keyframe = true;
                    attachment.pull_forward(rung, now);
                },
                None => {
                    if session.attached.len() >= cap || total >= total_cap {
                        session.send(refused_frame());
                        return;
                    }
                    session.attached.push(Attachment::new(tab, now));
                    take_hold = true;
                },
            }
        }
        // **The hold, and the reason an attachment is worth anything at all.**
        // Servo answers a hit test only for a shown webview, so before this
        // line an attachment neither showed a webview nor ticked a pump and a
        // remote click on a tab the local human was not looking at reached
        // nothing. Taken once for the life of the lease rather than per frame:
        // a show-and-hide cycle thirty-three times a second is a different
        // cost profile from showing once, and it is one the local loop pays.
        //
        // It changes nothing on this window. The tab is shown and **not**
        // focused, nothing here touches which tab is displayed or which view
        // the human is in, and every tab renders into its own framebuffer — so
        // the tab the human is looking at keeps its own pixels.
        if take_hold && !tabs.hold_for_view(tab) {
            // Unreachable in one turn — the viewport lookup above just
            // resolved this tab through the same agent-only filter — but a
            // lease on a tab that took no hold would be a pump painting
            // something nobody is showing, so it is refused rather than
            // recorded.
            self.drop_attachment(connection, tab);
            self.refuse(connection);
            return;
        }
        // Acknowledged either way: a client that lost its own answer and asked
        // again gets the same one rather than a refusal it cannot act on.
        if let Some(session) = self.session_mut(connection) {
            session.send(control_frame(&ServerView::Attached { tab, width, height }));
        }
    }

    /// Adopt `rung` for this connection, from the next tick.
    ///
    /// **A rung whose denominator differs forces a keyframe on every
    /// attachment the connection holds**, because the client's surface geometry
    /// has just changed: it is about to reallocate a texture of a different
    /// size, and a delta whose tile coordinates were computed against the old
    /// one names a region of the new one that corresponds to nothing. A rung
    /// that changes only the interval needs no keyframe — the picture is the
    /// same picture, arriving less often.
    fn set_rung(&mut self, connection: u64, rung: Rung) {
        let Some(session) = self.session_mut(connection) else { return };
        let resized = session.rung.scale_denominator() != rung.scale_denominator();
        session.rung = rung;
        if resized {
            for attachment in &mut session.attached {
                attachment.keyframe = true;
            }
        }
    }

    /// Forget one attachment without answering the viewer. The bookkeeping
    /// half of every release path, so the three of them cannot drift.
    fn drop_attachment(&mut self, connection: u64, tab: u64) -> bool {
        let Some(session) = self.session_mut(connection) else { return false };
        let before = session.attached.len();
        session.attached.retain(|attachment| attachment.tab != tab);
        before != session.attached.len()
    }

    /// Require the next frame for `connection`'s lease on `tab` to be a
    /// keyframe. Answers whether the lease exists, which is the same question
    /// the resize refusal asks.
    fn require_keyframe(&mut self, connection: u64, tab: u64) -> bool {
        let Some(session) = self.session_mut(connection) else { return false };
        let Some(attachment) = session.attachment_mut(tab) else { return false };
        attachment.keyframe = true;
        true
    }

    /// Release `connection`'s lease on `tab`.
    ///
    /// Releasing something that was never held is not an error, and the answer
    /// is the same either way: "you are not attached to this tab" is the state
    /// afterwards in both cases, and two different answers would tell a caller
    /// which tabs it had — cheap to keep uniform, so kept uniform.
    fn detach(&mut self, connection: u64, tab: u64, tabs: &mut dyn ViewTabs) {
        if self.drop_attachment(connection, tab) {
            // Only when a lease was actually held: a release for a hold that
            // was never taken would decrement a count two other viewers are
            // relying on.
            tabs.release_view_hold(tab);
            self.release_frames(connection, tab);
        }
        if let Some(session) = self.session_mut(connection) {
            session.send(control_frame(&ServerView::Detached { tab }));
        }
    }

    /// Whether `connection` currently holds a lease on `tab`.
    #[cfg(test)]
    fn holds(&mut self, connection: u64, tab: u64) -> bool {
        self.session_mut(connection).is_some_and(|session| session.holds(tab))
    }

    /// When the pump is next due to paint anything, or `None` when no
    /// attachment exists at all.
    ///
    /// **Joined into the loop's existing wait computation and never installed
    /// as a second control-flow source** — two things deciding when the loop
    /// wakes is how a loop stops waking. `None` is the ordinary case and it
    /// costs the browser nothing: with no viewer attached there is no tick, no
    /// readback and no tab held shown.
    pub fn next_tick(&self) -> Option<Instant> {
        self.sessions
            .iter()
            .flat_map(|session| session.attached.iter().map(|attachment| attachment.due))
            .min()
    }

    /// Every attachment due to be painted at `now`, with its frame sequence
    /// consumed and its cadence advanced.
    ///
    /// Taken out of the table so the caller can drop the borrow **before** the
    /// paint and the readback, which is the deferred-drain idiom this file's
    /// neighbours use and which matters more here than anywhere else: the
    /// engine borrows these same cells from its own callbacks, and a borrow
    /// held across a readback is a borrow held across the slowest thing in the
    /// loop.
    pub fn take_due(&mut self, now: Instant, idle: Duration) -> Vec<DueTick> {
        let mut due = Vec::new();
        for session in &mut self.sessions {
            let last_delivered_input = session.last_delivered_input;
            let rung = session.rung;
            // **The one question asked before any pixels are paid for**: is
            // this viewer reading what it already has? See
            // [`MAX_QUEUED_FRAMES`]. A backed-up viewer is one whose socket is
            // not draining, so producing for it grows a queue in the process
            // that holds the vault and costs the winit loop a paint plus a
            // readback for a frame nobody will see any sooner.
            let backed_up = session.out.depth() >= MAX_QUEUED_FRAMES;
            for attachment in &mut session.attached {
                if attachment.due > now {
                    continue;
                }
                let interval = attachment.interval(rung, now, idle);
                if backed_up {
                    // The deadline still advances, so the loop does not spin
                    // on an instant already past — and the sequence number is
                    // **not** consumed and the forced keyframe is **not**
                    // cleared, because nothing was produced and therefore
                    // nothing was lost. The frames already queued are valid,
                    // in order, and still the ones this client needs; it
                    // catches up by reading them rather than by being
                    // resynchronised.
                    attachment.due = now + interval;
                    attachment.last_tick = Some(now);
                    continue;
                }
                // From `now` rather than from the old deadline: a loop that
                // ran late must not then try to catch up by ticking twice in
                // a row, which is how a slow machine turns a missed frame
                // into a burst.
                attachment.due = now + interval;
                // Recorded alongside, because this is the fact
                // [`Attachment::pull_forward`] measures its floor from and
                // `due` cannot be it: `due` is what the floor exists to bound.
                attachment.last_tick = Some(now);
                let frame_seq = attachment.next_frame_seq;
                attachment.next_frame_seq += 1;
                due.push(DueTick {
                    connection: session.connection,
                    tab: attachment.tab,
                    frame_seq,
                    last_delivered_input,
                    keyframe: std::mem::take(&mut attachment.keyframe),
                    scale_denominator: rung.scale_denominator(),
                    out: session.out.clone(),
                });
            }
        }
        due
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
        let mut gone_frames: Vec<(u64, u64)> = Vec::new();
        for session in &mut self.sessions {
            let gone: Vec<u64> = session
                .attached
                .iter()
                .map(|attachment| attachment.tab)
                .filter(|tab| !live.contains(tab))
                .collect();
            for tab in gone {
                // Nothing is released back to the *tab table* here, and that
                // is not an omission: a tab absent from the snapshot has been
                // removed from the table entirely, so its hold count went with
                // it and there is no visibility left to restore. The encoder's
                // buffer is a different matter — it lives on another thread
                // and outlives the tab unless it is told, which is what the
                // release below is.
                session.attached.retain(|attachment| attachment.tab != tab);
                session.send(control_frame(&ServerView::Detached { tab }));
                gone_frames.push((session.connection, tab));
            }
            if changed || !session.snapshotted {
                session.send(Some(frame.clone()));
                session.snapshotted = true;
            }
        }
        self.published = Some(frame);
        for (connection, tab) in gone_frames {
            self.release_frames(connection, tab);
        }
    }

    /// The encoder thread, started on first use.
    ///
    /// Lazy rather than eager because the cost of a viewer should be paid by
    /// a viewer: a browser nobody has attached to spawns no thread. A spawn
    /// that failed is remembered as a failure and not retried — this is an
    /// operating system refusing a thread, not a transient.
    fn encoder(&mut self) -> &FrameEncoder {
        self.encoder.get_or_insert_with(FrameEncoder::start)
    }

    /// Why the encoder thread could not start, if it could not, taken once.
    ///
    /// Taken rather than read so the loop reports it exactly once. The viewer
    /// already has its answer — the attach was refused — and this is the other
    /// half: the fact reaching the main thread as an event rather than only a
    /// log line.
    pub fn take_encoder_failure(&mut self) -> Option<String> {
        self.encoder.as_mut().and_then(|encoder| encoder.failure.take())
    }

    /// One tick's pixels, handed off the loop.
    ///
    /// **Everything after this line happens on the encoder thread**: the tile
    /// comparison, the keyframe decision, the encode and the write. The loop
    /// has already spent its readback and is done.
    pub fn frame_captured(&mut self, tick: &DueTick, surface: Surface) {
        self.encoder().send(Job::Frame(Frame {
            connection: tick.connection,
            tab: tick.tab,
            frame_seq: tick.frame_seq,
            last_delivered_input: tick.last_delivered_input,
            keyframe: tick.keyframe,
            scale_denominator: tick.scale_denominator,
            surface,
            out: tick.out.clone(),
        }));
    }

    /// Tell the encoder an attachment has ended, so it drops the full-surface
    /// buffer that attachment owned.
    fn release_frames(&mut self, connection: u64, tab: u64) {
        if let Some(encoder) = self.encoder.as_ref() {
            encoder.send(Job::Release { connection, tab });
        }
    }

    /// Put back a keyframe a tick consumed but could not deliver.
    ///
    /// A forced keyframe is the only part of a tick that must survive a failed
    /// readback: the sequence number is spent either way (strictly increasing,
    /// not contiguous), and a delta is correct against an unchanged previous
    /// frame — but a client that was promised a whole surface and got nothing
    /// would composite the next delta onto a texture it has not been given.
    pub fn require_keyframe_again(&mut self, tick: &DueTick) {
        self.require_keyframe(tick.connection, tick.tab);
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
        /// How many viewers hold each tab shown, keyed by tab id. The same
        /// count [`crate::tabs::Tab::held_for_view`] keeps, so the arithmetic
        /// the visibility synchronisation consults is assertable without an
        /// engine.
        pub(crate) holds: std::collections::BTreeMap<u64, usize>,
    }

    impl FakeTabs {
        pub(crate) fn with(tabs: &[(u64, bool)]) -> Self {
            Self { tabs: tabs.to_vec(), holds: std::collections::BTreeMap::new() }
        }

        pub(crate) fn none() -> Self {
            Self { tabs: Vec::new(), holds: std::collections::BTreeMap::new() }
        }

        /// How many viewers hold `tab` shown.
        pub(crate) fn held(&self, tab: u64) -> usize {
            self.holds.get(&tab).copied().unwrap_or(0)
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

        /// Agent-only, exactly as the real table's is — a fake that held the
        /// human's tabs too could not fail the filter test.
        fn hold_for_view(&mut self, tab: u64) -> bool {
            if !self.tabs.iter().any(|(id, agent)| *id == tab && *agent) {
                return false;
            }
            *self.holds.entry(tab).or_insert(0) += 1;
            true
        }

        fn release_view_hold(&mut self, tab: u64) {
            if let Some(count) = self.holds.get_mut(&tab) {
                *count = count.saturating_sub(1);
            }
        }
    }

    /// One connected viewer, plus the read end of its outbound channel.
    pub(crate) struct Viewer {
        pub(crate) connection: u64,
        frames: ViewReader,
    }

    impl Viewer {
        /// Every frame written to this viewer since the last drain.
        pub(crate) fn drain(&mut self) -> Vec<Vec<u8>> {
            let mut frames = Vec::new();
            while let Some(frame) = self.frames.try_recv() {
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
        let (out, frames) = view_channel();
        sessions.opened(connection, client_id.to_owned(), out);
        Viewer { connection, frames }
    }

    pub(crate) fn attach(
        sessions: &mut ViewSessions,
        viewer: &Viewer,
        tab: u64,
        tabs: &mut dyn ViewTabs,
    ) {
        let frame = control_request(&ClientView::Attach { tab });
        assert!(
            matches!(sessions.message(viewer.connection, &frame, tabs), Handled::Done),
            "the connection was closed",
        );
    }

    pub(crate) fn detach(
        sessions: &mut ViewSessions,
        viewer: &Viewer,
        tab: u64,
        tabs: &mut dyn ViewTabs,
    ) {
        let frame = control_request(&ClientView::Detach { tab });
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

    /// The interval an attachment that never named a rung ticks at while its
    /// viewer is driving. Read out of the ladder rather than restated here,
    /// because the whole point of the table is that the number has one home.
    fn driven_tick_ms() -> u64 {
        u64::from(Rung::FASTEST.interval_ms())
    }

    /// And the interval it ticks at while nobody is — the ladder's slowest
    /// rung, whichever rung the connection is on.
    fn passive_tick_ms() -> u64 {
        u64::from(Rung::SLOWEST.interval_ms())
    }

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
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        let _ = viewer.drain();
        attach(&mut sessions, &viewer, 1, &mut tabs);
        assert_eq!(
            viewer.control(),
            vec![ServerView::Attached { tab: 1, width: 1280, height: 736 }]
        );
    }

    /// T-05-05: a tab the human owns is refused, and it is refused by the
    /// lookup yielding nothing rather than by a check after the fact.
    #[test]
    fn attaching_to_a_tab_the_human_owns_is_refused() {
        let mut tabs = FakeTabs::with(&[(1, false), (2, true)]);
        assert!(tabs.agent_viewport(1).is_none(), "a human-owned tab resolved on the view path");
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        let _ = viewer.drain();
        attach(&mut sessions, &viewer, 1, &mut tabs);
        assert_eq!(viewer.control(), vec![ServerView::Refused]);
    }

    /// T-05-05-A: the two refusals are **byte-identical**, so a viewer cannot
    /// enumerate the human's tabs by watching which ids refuse differently.
    #[test]
    fn a_human_owned_tab_and_a_tab_that_does_not_exist_refuse_identically() {
        let mut tabs = FakeTabs::with(&[(1, false)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        let _ = viewer.drain();

        attach(&mut sessions, &viewer, 1, &mut tabs);
        let owned = viewer.drain();
        attach(&mut sessions, &viewer, 999, &mut tabs);
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
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        attach(&mut sessions, &viewer, 1, &mut tabs);
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
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut first = connect(&mut sessions, 1, "client-a");
        let mut second = connect(&mut sessions, 2, "client-b");
        let _ = first.drain();
        let _ = second.drain();

        attach(&mut sessions, &first, 1, &mut tabs);
        attach(&mut sessions, &second, 1, &mut tabs);
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
        let mut tabs = FakeTabs::with(&all);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        let _ = viewer.drain();

        for id in 1..=DEFAULT_MAX_ATTACH as u64 {
            attach(&mut sessions, &viewer, id, &mut tabs);
        }
        assert_eq!(
            viewer.control().len(),
            DEFAULT_MAX_ATTACH,
            "an attach inside the cap was refused"
        );
        attach(&mut sessions, &viewer, DEFAULT_MAX_ATTACH as u64 + 1, &mut tabs);
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

    /// CR-02: the attachment cap is per connection, so the cost the browser
    /// pays is bounded only by a **total** the attacker cannot multiply by
    /// opening more sockets.
    #[test]
    fn attaching_beyond_the_total_cap_is_refused_across_connections() {
        let all: Vec<(u64, bool)> = (1..=(DEFAULT_MAX_TOTAL_ATTACH as u64 + 2))
            .map(|id| (id, true))
            .collect();
        let mut tabs = FakeTabs::with(&all);
        let mut sessions = ViewSessions::default();
        // Spread over two connections, each staying inside its own per-
        // connection cap, so the only thing that can refuse is the total.
        let mut first = connect(&mut sessions, 1, "client-a");
        let mut second = connect(&mut sessions, 2, "client-a");
        let _ = first.drain();
        let _ = second.drain();

        let half = DEFAULT_MAX_TOTAL_ATTACH as u64 / 2;
        assert!(
            (half as usize) < DEFAULT_MAX_ATTACH,
            "each connection must stay inside its own cap or this asserts the wrong one",
        );
        for id in 1..=half {
            attach(&mut sessions, &first, id, &mut tabs);
        }
        for id in (half + 1)..=(half * 2) {
            attach(&mut sessions, &second, id, &mut tabs);
        }
        assert_eq!(first.control().len(), half as usize, "an attach inside the total was refused");
        assert_eq!(second.control().len(), half as usize);

        // The next one, on either connection, is over the total.
        attach(&mut sessions, &second, DEFAULT_MAX_TOTAL_ATTACH as u64 + 1, &mut tabs);
        assert_eq!(second.control(), vec![ServerView::Refused]);
        assert_eq!(
            sessions.session_mut(2).expect("the session").attachment_count(),
            half as usize,
            "a refused attach was recorded anyway",
        );
        assert_eq!(tabs.held(DEFAULT_MAX_TOTAL_ATTACH as u64 + 1), 0, "a refused attach took a hold");
    }

    /// Both new caps' overrides land on their defaults for anything they
    /// cannot read, and never on zero and never on unbounded.
    #[test]
    fn the_connection_and_total_caps_fall_back_to_their_defaults_and_never_to_zero() {
        assert_eq!(parse_max_view_connections(Some("2")), 2);
        assert_eq!(parse_max_total_attachments(Some("  16  ")), 16);
        for bad in [None, Some(""), Some("0"), Some("-1"), Some("lots"), Some("2.5")] {
            assert_eq!(
                parse_max_view_connections(bad),
                DEFAULT_MAX_VIEW_CONNECTIONS,
                "{bad:?} did not fall back to the connection default"
            );
            assert_eq!(
                parse_max_total_attachments(bad),
                DEFAULT_MAX_TOTAL_ATTACH,
                "{bad:?} did not fall back to the total default"
            );
        }
    }

    /// Detaching releases the lease; detaching one that was never held is not
    /// an error and is answered the same way.
    #[test]
    fn detaching_releases_a_lease_and_detaching_nothing_is_not_an_error() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let _ = viewer.drain();

        let frame = control_request(&ClientView::Detach { tab: 1 });
        assert!(matches!(sessions.message(1, &frame, &mut tabs), Handled::Done));
        assert_eq!(viewer.control(), vec![ServerView::Detached { tab: 1 }]);
        assert_eq!(sessions.session_mut(1).expect("the session").attachment_count(), 0);

        // And again, holding nothing.
        assert!(
            matches!(sessions.message(1, &frame, &mut tabs), Handled::Done),
            "a redundant detach closed the connection",
        );
        assert_eq!(viewer.control(), vec![ServerView::Detached { tab: 1 }]);
    }

    /// A tab that closes underneath a viewer produces a detach notice and is
    /// absent from the next snapshot — for **every** viewer holding it.
    #[test]
    fn a_tab_closing_detaches_every_viewer_holding_it() {
        let mut open = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let mut first = connect(&mut sessions, 1, "client-a");
        let mut second = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &first, 1, &mut open);
        attach(&mut sessions, &second, 1, &mut open);
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
        let mut tabs = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        let survivor = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        attach(&mut sessions, &viewer, 2, &mut tabs);
        attach(&mut sessions, &survivor, 1, &mut tabs);

        sessions.closed(1, &mut tabs);
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
        let mut tabs = FakeTabs::with(&[(1, true)]);
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
                matches!(sessions.message(viewer.connection, &frame, &mut tabs), Handled::Close),
                "a frame this server cannot answer left the connection open: {frame:?}"
            );
        }
    }

    /// A well-formed input message is handed **back** to the caller rather
    /// than acted on here — this module owns no path to a webview, and that
    /// absence is the reason `remote_input` can be the only one.
    #[test]
    fn a_well_formed_input_message_is_handed_back_and_delivered_nowhere() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let _ = viewer.drain();

        let frame = encode(
            Channel::Input,
            InputMessage::MouseMove { tab: 1, seq: 7, x: 4.0, y: 5.0 }
                .to_json()
                .expect("well formed")
                .as_bytes(),
        );
        let handled = sessions.message(viewer.connection, &frame, &mut tabs);
        assert!(
            matches!(handled, Handled::Input(InputMessage::MouseMove { seq: 7, .. })),
            "a decoded input message was not handed back",
        );
        // Nothing was sent and nothing was recorded: admitting the message is
        // `remote_input`'s call, made through `admit_input`.
        assert!(viewer.drain().is_empty(), "decoding an input message answered the viewer");
        assert_eq!(sessions.session_mut(1).expect("the session").last_applied_input, 0);
    }

    /// WR-10: the ordering mark and the latency echo are two different facts,
    /// and the header carries the second.
    ///
    /// The bug this pins: one field did both jobs, so the phase's headline
    /// evidence — `reading.input_to_photon_ms`, which
    /// `scripts/two-machine-check.sh` calls "THE EVIDENCE for Success
    /// Criterion 2" — was computed from a counter that advances on *refused*
    /// inputs. A click in the letterboxed margin, on a crashed tab, or on a
    /// tab the viewer never attached to produced a latency for a round trip
    /// that never included a hit test, biased low.
    #[test]
    fn a_refused_input_advances_the_ordering_mark_and_not_the_latency_echo() {
        let mut tabs = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);

        // Accepted and delivered: both marks move, and the tick echoes it.
        assert!(sessions.admit_input(viewer.connection, 1, 4));
        sessions.delivered_input(viewer.connection, 4);

        // Accepted by the sequence rule and refused downstream — tab 2 is
        // agent-owned and unattached, so `remote_input::apply` never reaches
        // its delivery call and never records anything.
        assert!(!sessions.admit_input(viewer.connection, 2, 5));

        let session = sessions.session_mut(1).expect("the session");
        assert_eq!(
            session.last_applied_input, 5,
            "a refused input did not consume its number, so it could be replayed (T-05-15)",
        );
        assert_eq!(
            session.last_delivered_input, 4,
            "a refused input advanced the number the frame header echoes, so the latency \
             figure counts a round trip that never included a hit test",
        );

        // And the tick carries the delivered one.
        let tick = sessions.take_due(Instant::now(), view_idle()).remove(0);
        assert_eq!(tick.last_delivered_input, 4);
    }

    /// The input channel's sequence high-water mark is per connection, and it
    /// only ever advances.
    #[test]
    fn the_input_sequence_mark_is_per_connection_and_only_advances() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let first = connect(&mut sessions, 1, "client-a");
        let second = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &first, 1, &mut tabs);
        attach(&mut sessions, &second, 1, &mut tabs);

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
        let mut tabs = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let _ = viewer.drain();

        let held = control_request(&ClientView::Viewport { tab: 1, width: 800, height: 600 });
        assert!(matches!(sessions.message(1, &held, &mut tabs), Handled::Done));
        assert!(viewer.control().is_empty(), "a resize of an attached tab was answered");

        let other = control_request(&ClientView::Viewport { tab: 2, width: 800, height: 600 });
        assert!(matches!(sessions.message(1, &other, &mut tabs), Handled::Done));
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

    // ---- the visibility hold -------------------------------------------

    /// An attachment holds its tab shown. Without this a remote click reaches
    /// nothing, because Servo answers a hit test only for a shown webview.
    #[test]
    fn attaching_holds_the_tab_shown() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        assert_eq!(tabs.held(1), 0, "a tab was held before anyone attached");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        assert_eq!(tabs.held(1), 1);
    }

    /// The two-viewer arithmetic, which is the whole reason the marker is a
    /// count and not a flag: the first detach must not release the tab out
    /// from under the second viewer.
    #[test]
    fn two_viewers_hold_one_tab_and_the_first_detach_does_not_release_it() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let first = connect(&mut sessions, 1, "client-a");
        let second = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &first, 1, &mut tabs);
        attach(&mut sessions, &second, 1, &mut tabs);
        assert_eq!(tabs.held(1), 2);

        detach(&mut sessions, &first, 1, &mut tabs);
        assert_eq!(tabs.held(1), 1, "the first detach released a tab the second was watching");
        detach(&mut sessions, &second, 1, &mut tabs);
        assert_eq!(tabs.held(1), 0, "the last detach did not release the tab");
    }

    /// Attaching twice from **one** connection holds once. The count follows
    /// leases, not messages, or a client that resent an attach would pin a tab
    /// shown forever.
    #[test]
    fn attaching_twice_from_one_connection_holds_the_tab_once() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        attach(&mut sessions, &viewer, 1, &mut tabs);
        assert_eq!(tabs.held(1), 1);
        detach(&mut sessions, &viewer, 1, &mut tabs);
        assert_eq!(tabs.held(1), 0);
    }

    /// Detaching a tab that was never attached releases nothing — a release
    /// for a hold that was never taken would decrement a count another viewer
    /// is relying on.
    #[test]
    fn detaching_a_tab_that_was_never_held_releases_nothing() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let watcher = connect(&mut sessions, 1, "client-a");
        let meddler = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &watcher, 1, &mut tabs);
        assert_eq!(tabs.held(1), 1);

        detach(&mut sessions, &meddler, 1, &mut tabs);
        detach(&mut sessions, &meddler, 1, &mut tabs);
        assert_eq!(tabs.held(1), 1, "a viewer released a hold it never took");
    }

    /// `D-05-02`: a tab the human owns takes no hold, because the hold goes
    /// through the same agent-only lookup the viewport does.
    #[test]
    fn a_tab_the_human_owns_is_never_held() {
        let mut tabs = FakeTabs::with(&[(1, false)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        assert_eq!(tabs.held(1), 0);
        assert!(!sessions.holds(1, 1), "a refused attach recorded a lease anyway");
    }

    /// A connection ending releases every hold it had, and only its own.
    #[test]
    fn a_connection_ending_releases_every_hold_it_had() {
        let mut tabs = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let leaving = connect(&mut sessions, 1, "client-a");
        let staying = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &leaving, 1, &mut tabs);
        attach(&mut sessions, &leaving, 2, &mut tabs);
        attach(&mut sessions, &staying, 1, &mut tabs);
        assert_eq!((tabs.held(1), tabs.held(2)), (2, 1));

        sessions.closed(1, &mut tabs);
        assert_eq!(
            (tabs.held(1), tabs.held(2)),
            (1, 0),
            "a connection ending released the wrong holds"
        );
        assert!(sessions.holds(2, 1), "one connection ending took another's lease");
    }

    /// A tab closing removes the attachment. Nothing is released back to the
    /// table, because a tab absent from the snapshot has been removed from it
    /// entirely and took its count with it.
    #[test]
    fn a_tab_closing_removes_the_attachment_that_held_it() {
        let mut open = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut open);
        attach(&mut sessions, &viewer, 2, &mut open);
        sessions.publish(&open);

        sessions.publish(&FakeTabs::with(&[(2, true)]));
        assert!(!sessions.holds(1, 1), "the lease on a closed tab survived it");
        assert!(sessions.holds(1, 2), "closing one tab dropped the lease on another");
        assert!(sessions.next_tick().is_some(), "the surviving lease stopped ticking");
    }

    // ---- the tick ------------------------------------------------------

    /// A browser with no viewer has no tick at all, so it never wakes the loop
    /// and never reads a framebuffer back.
    #[test]
    fn with_no_attachment_there_is_no_tick_and_nothing_is_due() {
        let mut sessions = ViewSessions::default();
        assert_eq!(sessions.next_tick(), None);
        assert!(sessions.take_due(Instant::now(), view_idle()).is_empty());

        // And a connection with no attachment is still no tick: it is the
        // lease that costs something, not the socket.
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let _viewer = connect(&mut sessions, 1, "client-a");
        assert_eq!(sessions.next_tick(), None);
        assert!(sessions.take_due(Instant::now(), view_idle()).is_empty());
        let _ = &mut tabs;
    }

    /// An attachment is due immediately — the first frame is the one the
    /// viewer cannot draw anything without — and not due again until its
    /// interval has passed.
    #[test]
    fn an_attachment_is_due_at_once_and_then_not_until_its_interval_passes() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);

        let now = Instant::now();
        assert!(sessions.next_tick().is_some_and(|due| due <= now));
        assert_eq!(sessions.take_due(now, view_idle()).len(), 1);
        assert!(sessions.take_due(now, view_idle()).is_empty(), "one tick painted twice");

        // The passive interval, since nothing has been driven.
        let passive = Duration::from_millis(passive_tick_ms());
        assert!(sessions.take_due(now + passive - Duration::from_millis(1), view_idle()).is_empty());
        assert_eq!(sessions.take_due(now + passive, view_idle()).len(), 1);
    }

    /// The first frame of an attachment is a keyframe and the next is not: a
    /// client holds nothing to composite a delta onto until it has one.
    #[test]
    fn the_first_frame_of_an_attachment_is_a_keyframe_and_the_second_is_not() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);

        let now = Instant::now();
        let first = sessions.take_due(now, view_idle());
        assert!(first[0].keyframe, "the first frame after an attach was not a keyframe");
        let later = now + Duration::from_millis(passive_tick_ms());
        let second = sessions.take_due(later, view_idle());
        assert!(!second[0].keyframe, "every frame is a keyframe, so nothing is a delta");
    }

    /// A viewport change and a re-attach both require the next frame to be a
    /// whole keyframe — the client's surface is about to be, or has already
    /// been, thrown away.
    #[test]
    fn a_viewport_change_and_a_reattach_each_require_a_keyframe() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let now = Instant::now();
        assert!(sessions.take_due(now, view_idle())[0].keyframe);

        let resize = control_request(&ClientView::Viewport { tab: 1, width: 640, height: 480 });
        assert!(matches!(sessions.message(1, &resize, &mut tabs), Handled::Done));
        let after_resize = now + Duration::from_millis(passive_tick_ms());
        assert!(
            sessions.take_due(after_resize, view_idle())[0].keyframe,
            "a viewport change did not produce a keyframe"
        );

        attach(&mut sessions, &viewer, 1, &mut tabs);
        let after_reattach = after_resize + Duration::from_millis(passive_tick_ms());
        assert!(
            sessions.take_due(after_reattach, view_idle())[0].keyframe,
            "a re-attach did not produce a keyframe"
        );
    }

    /// A tick whose readback failed puts its forced keyframe back. The
    /// sequence number is spent either way; the promise of a whole surface is
    /// not.
    #[test]
    fn a_keyframe_a_failed_tick_consumed_is_put_back() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);

        let now = Instant::now();
        let lost = sessions.take_due(now, view_idle()).remove(0);
        assert!(lost.keyframe);
        sessions.require_keyframe_again(&lost);
        let next = sessions.take_due(now + Duration::from_millis(passive_tick_ms()), view_idle());
        assert!(next[0].keyframe, "a keyframe lost to a failed readback was never re-sent");
        assert!(
            next[0].frame_seq > lost.frame_seq,
            "a failed tick's sequence number was re-used"
        );
    }

    /// Frame sequences are strictly increasing per attachment, and the two
    /// attachments on one tab have their own spaces.
    #[test]
    fn frame_sequences_increase_per_attachment_and_never_across_them() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let first = connect(&mut sessions, 1, "client-a");
        let second = connect(&mut sessions, 2, "client-b");
        attach(&mut sessions, &first, 1, &mut tabs);
        attach(&mut sessions, &second, 1, &mut tabs);

        let mut now = Instant::now();
        let mut seen: Vec<(u64, u64)> = Vec::new();
        for _ in 0..3 {
            for tick in sessions.take_due(now, view_idle()) {
                seen.push((tick.connection, tick.frame_seq));
            }
            now += Duration::from_millis(passive_tick_ms());
        }
        let sequences = |connection: u64| -> Vec<u64> {
            seen.iter().filter(|(c, _)| *c == connection).map(|(_, s)| *s).collect()
        };
        assert_eq!(sequences(1), vec![1, 2, 3]);
        assert_eq!(sequences(2), vec![1, 2, 3], "two viewers shared one sequence space");
    }

    /// A tick that ran late does not then tick twice to catch up: the next
    /// deadline is measured from when the frame was actually taken.
    #[test]
    fn a_late_tick_does_not_burst_to_catch_up() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);

        let very_late = Instant::now() + Duration::from_millis(passive_tick_ms() * 10);
        assert_eq!(sessions.take_due(very_late, view_idle()).len(), 1);
        assert!(
            sessions.take_due(very_late, view_idle()).is_empty(),
            "a loop that ran late produced a burst of frames instead of one"
        );
    }

    // ---- the cadence ---------------------------------------------------

    /// The transition, at the threshold and one millisecond either side of it.
    /// The boundary is the part somebody will test, so it is pinned here.
    #[test]
    fn the_cadence_falls_back_to_passive_exactly_at_the_idle_threshold() {
        let idle = Duration::from_millis(1000);
        let driven = Duration::from_millis(driven_tick_ms());
        let passive = Duration::from_millis(passive_tick_ms());

        let rung = Rung::FASTEST;
        assert_eq!(
            tick_interval(rung, None, idle),
            passive,
            "a viewer that never drove was driven"
        );
        assert_eq!(tick_interval(rung, Some(Duration::ZERO), idle), driven);
        assert_eq!(tick_interval(rung, Some(idle - Duration::from_millis(1)), idle), driven);
        assert_eq!(
            tick_interval(rung, Some(idle), idle),
            passive,
            "the threshold itself was still driven, so 'reaches' meant 'exceeds'"
        );
        assert_eq!(tick_interval(rung, Some(idle + Duration::from_millis(1)), idle), passive);
    }

    /// The requested driven cadence is 30 ms and the passive one is inside the
    /// 200–500 ms band the requirement names — asserted rather than left to a
    /// comment, because these are the two numbers a later edit would move.
    ///
    /// Both are now read out of the ladder, so this is also the assertion that
    /// the pump's default behaviour did not move when the rungs arrived.
    #[test]
    fn the_two_cadences_are_the_measured_ones() {
        let passive = passive_tick_ms();
        assert_eq!(driven_tick_ms(), 30);
        assert!(
            (200..=500).contains(&passive),
            "the passive cadence left the band DIST-02 names: {passive}"
        );
    }

    // ---- the rung ---------------------------------------------------------

    /// A named rung is adopted and takes effect on the next tick, both in the
    /// interval it implies and in the denominator it implies.
    #[test]
    fn a_cadence_request_naming_a_known_rung_takes_effect_on_the_next_tick() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let now = Instant::now();
        let first = sessions.take_due(now, view_idle()).remove(0);
        assert_eq!(
            first.scale_denominator,
            Rung::FASTEST.scale_denominator(),
            "a connection that named no rung did not start at the fastest one",
        );
        let _ = viewer.control();

        let slow = Rung::HalfResolutionSlow;
        let request = control_request(&ClientView::Cadence { rung: Some(slow) });
        assert!(matches!(sessions.message(1, &request, &mut tabs), Handled::Done));
        assert!(
            viewer.control().is_empty(),
            "a rung this build knows was answered; it is a request, not a negotiation",
        );

        // Driving, so the rung's own interval is the one in force — and the
        // input brings the next tick forward to that interval's floor rather
        // than to the present moment, so the tick is taken *there* (CR-01).
        assert!(sessions.admit_input(1, 1, 1));
        let at = sessions.next_tick().expect("an attached connection is due");
        assert_eq!(
            at,
            now + slow.interval(),
            "an input did not bring the next tick forward to exactly one rung interval",
        );
        let driven = sessions.take_due(at, view_idle()).remove(0);
        assert_eq!(driven.scale_denominator, slow.scale_denominator());
        let too_soon = at + slow.interval() - Duration::from_millis(1);
        assert!(
            sessions.take_due(too_soon, view_idle()).is_empty(),
            "the tick came earlier than the rung asked for",
        );
        assert_eq!(sessions.take_due(at + slow.interval(), view_idle()).len(), 1);
    }

    /// A rung this build does not know gets the one reasonless refusal rather
    /// than ending the connection, so a rung added later is not a breaking
    /// change.
    #[test]
    fn a_cadence_request_naming_an_unrecognised_rung_is_refused_with_no_reason() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let _ = viewer.control();

        let unknown = encode(
            Channel::Control,
            b"{\"view\":\"cadence\",\"rung\":\"quarter_resolution_someday\"}",
        );
        assert!(
            matches!(sessions.message(1, &unknown, &mut tabs), Handled::Done),
            "an unknown rung ended the connection instead of being answered",
        );
        assert_eq!(viewer.control(), vec![ServerView::Refused]);

        // And the connection is still on the rung it was on.
        let tick = sessions.take_due(Instant::now(), view_idle()).remove(0);
        assert_eq!(tick.scale_denominator, Rung::FASTEST.scale_denominator());
    }

    /// A rung whose denominator differs forces a whole keyframe, because the
    /// client is about to reallocate a texture of a different size.
    #[test]
    fn a_rung_change_that_resizes_the_surface_forces_a_keyframe() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let now = Instant::now();
        assert!(sessions.take_due(now, view_idle())[0].keyframe);
        let mut at = now + Duration::from_millis(passive_tick_ms());
        assert!(!sessions.take_due(at, view_idle())[0].keyframe);

        // Same denominator, slower interval: the picture is the same picture.
        let steady = control_request(&ClientView::Cadence {
            rung: Some(Rung::FullResolutionSteady),
        });
        assert!(matches!(sessions.message(1, &steady, &mut tabs), Handled::Done));
        at += Duration::from_millis(passive_tick_ms());
        assert!(
            !sessions.take_due(at, view_idle())[0].keyframe,
            "a rung change that did not resize anything still cost a whole keyframe",
        );

        // A different denominator: the surface geometry changed.
        let half =
            control_request(&ClientView::Cadence { rung: Some(Rung::HalfResolutionSteady) });
        assert!(matches!(sessions.message(1, &half, &mut tabs), Handled::Done));
        at += Duration::from_millis(passive_tick_ms());
        let tick = sessions.take_due(at, view_idle()).remove(0);
        assert!(tick.keyframe, "a rung change that halved the surface sent a delta");
        assert_eq!(tick.scale_denominator, 2);
    }

    // ---- the reduction ----------------------------------------------------

    /// A denominator of one is the identity, which is what makes the shipped
    /// full-resolution path free of the reduction entirely.
    #[test]
    fn a_denominator_of_one_produces_the_source_dimensions_unchanged() {
        let surface = flat(200, 136, 0x30);
        let reduced = reduce(&surface, 1);
        assert_eq!((reduced.width, reduced.height), (200, 136));
        assert_eq!(reduced.pixels, surface.pixels);
    }

    /// The divisible case: half the width, half the height, and the sampled
    /// pixels are the source's own.
    #[test]
    fn a_denominator_above_one_halves_a_divisible_surface_exactly() {
        let mut surface = flat(8, 4, 0x00);
        // A recognisable pixel at (2, 2), which is (1, 1) after halving.
        let at = ((2 * 8) + 2) * 4;
        surface.pixels[at] = 0x77;
        let reduced = reduce(&surface, 2);
        assert_eq!((reduced.width, reduced.height), (4, 2));
        assert_eq!(reduced.pixels.len(), 4 * 2 * 4);
        // Row 1, column 1 of a four-wide destination.
        let sampled = (4 + 1) * 4;
        assert_eq!(reduced.pixels[sampled], 0x77, "the sampled pixel is not the source's");
    }

    /// **The rounding, at a deliberately awkward width.** 201 and 137 are both
    /// odd, so a floor would drop the rightmost column and the bottom row —
    /// a strip of the page never sent, which reads as a rendering fault.
    #[test]
    fn a_source_dimension_the_denominator_does_not_divide_loses_no_column_or_row() {
        let mut surface = flat(201, 137, 0x00);
        // The last pixel of the last row: the one a floor would throw away.
        let corner = ((136 * 201) + 200) * 4;
        surface.pixels[corner] = 0x99;
        let reduced = reduce(&surface, 2);
        assert_eq!(
            (reduced.width, reduced.height),
            (101, 69),
            "the reduction rounded down and dropped the edge strips",
        );
        // Every source pixel is covered: the destination times the denominator
        // reaches at least the source's own size.
        assert!(reduced.width * 2 >= 201 && reduced.height * 2 >= 137);
        let last = ((68 * 101) + 100) * 4;
        assert_eq!(
            reduced.pixels[last], 0x99,
            "the source's last pixel did not survive the reduction",
        );
    }

    /// The header a reduced frame carries: the **reduced** dimensions plus the
    /// denominator, which is the pair a client multiplies back to the page's
    /// own size rather than inferring a scale from a ratio.
    #[test]
    fn a_reduced_frame_declares_its_denominator_and_its_reduced_dimensions() {
        let (out, _frames) = view_channel();
        let surface = reduce(&flat(201, 137, 0x30), 2);
        let whole = surface.whole();
        let frame = Frame {
            connection: 1,
            tab: 5,
            frame_seq: 2,
            last_delivered_input: 0,
            keyframe: true,
            scale_denominator: 2,
            surface,
            out,
        };
        let message = compose(&frame, FrameKind::Keyframe, whole).expect("a frame message");
        let (_, body) = split_channel(&message).expect("a framed message");
        let header = FrameHeader::from_bytes(body).expect("a header the wire accepts");
        assert_eq!(header.scale_denominator, 2);
        assert_eq!((header.frame_width, header.frame_height), (101, 69));
        assert_eq!((header.tile_width, header.tile_height), (101, 69));
    }

    /// An accepted input takes the attachment off the passive interval already
    /// in flight and puts it on the driven one — and puts it there, rather
    /// than straight onto the present moment, which is the bound the ladder
    /// would otherwise not have (CR-01).
    #[test]
    fn an_accepted_input_moves_the_attachment_to_the_driven_cadence() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let driven = Duration::from_millis(driven_tick_ms());
        let passive = Duration::from_millis(passive_tick_ms());
        assert!(driven < passive, "the two cadences are the same, so this asserts nothing");

        let now = Instant::now();
        assert_eq!(sessions.take_due(now, view_idle()).len(), 1, "the first frame was not due");
        assert_eq!(
            sessions.next_tick(),
            Some(now + passive),
            "nothing has driven yet, so the deadline in flight is the passive one",
        );

        assert!(sessions.admit_input(viewer.connection, 1, 1));
        let due = sessions.next_tick().expect("an attachment is attached and therefore due");
        assert!(
            due < now + passive,
            "an input did not bring the next frame forward off the passive interval",
        );
        assert!(
            due >= now + driven,
            "an input pulled the deadline inside the rung's own interval, so the ladder \
             bounds only a silent viewer (CR-01)",
        );

        // And the interval that follows a painted tick is the driven one, well
        // inside the passive interval that was in flight a moment ago.
        let painted = Instant::now().max(due);
        assert_eq!(sessions.take_due(painted, view_idle()).len(), 1);
        assert_eq!(sessions.next_tick(), Some(painted + driven));
    }

    /// CR-03: a viewer that is not reading its socket stops the pump producing
    /// for it, rather than growing a queue in the process that holds the
    /// vault.
    ///
    /// The bug this pins: `out` was an unbounded channel with no depth check
    /// anywhere and a cadence driven entirely by the server's own clock, so a
    /// peer that closed its receive window — hostile, or ordinary on a stalled
    /// relayed path — grew it without bound while the pump kept producing.
    #[test]
    fn a_viewer_that_is_not_reading_stops_the_pump_producing_for_it() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let _ = viewer.drain();

        let now = Instant::now();
        assert_eq!(sessions.take_due(now, view_idle()).len(), 1, "the first frame was not due");
        let _ = viewer.drain();

        // Fill the queue and read none of it. A refusal is a message on the
        // same wire the pictures ride, which is what makes it usable here —
        // the pixels themselves need a live engine and these do not.
        for _ in 0..MAX_QUEUED_FRAMES {
            attach(&mut sessions, &viewer, 4242, &mut tabs);
        }

        let passive = Duration::from_millis(passive_tick_ms());
        let mut at = now + passive;
        assert!(
            sessions.take_due(at, view_idle()).is_empty(),
            "the pump produced a frame for a viewer that is not reading the last four",
        );
        assert!(
            sessions.next_tick().is_some_and(|due| due > at),
            "a skipped tick left the deadline in the past, so the loop spins",
        );

        // And it resumes on its own the moment the viewer reads — with the
        // frame sequence the skipped ticks did **not** consume, so the client's
        // stream has no hole in it and needs no keyframe to recover.
        let _ = viewer.drain();
        at += passive;
        let resumed = sessions.take_due(at, view_idle());
        assert_eq!(resumed.len(), 1, "the pump did not resume once the viewer read");
        assert_eq!(
            resumed[0].frame_seq, 2,
            "a skipped tick consumed a frame sequence number, so a frame was dropped rather \
             than never produced",
        );
    }

    /// CR-01: a viewer sending input as fast as it can does not get to set the
    /// pump's rate.
    ///
    /// The bug this pins: `admit_input` pulled the deadline straight to the
    /// present moment, so the rung's interval bounded the pump only while the
    /// viewer was *silent*. Each due tick is a `paint()` plus a framebuffer
    /// readback on the winit main thread, so a party holding nothing but a
    /// token could pin the local human's browser — including the Access
    /// panel's Revoke control, which is drawn by the loop being pinned.
    #[test]
    fn a_flood_of_accepted_input_cannot_tick_the_pump_faster_than_its_rung() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let driven = Duration::from_millis(driven_tick_ms());

        let start = Instant::now();
        assert_eq!(sessions.take_due(start, view_idle()).len(), 1, "the first frame was not due");

        // The attacker's loop: a thousand in-bounds messages with the sequence
        // climbing, every one of them accepted, all inside one rung interval.
        let mut ticks = 0;
        for seq in 1..=1000u64 {
            assert!(sessions.admit_input(viewer.connection, 1, seq), "input {seq} was refused");
            ticks += sessions
                .take_due(start + driven - Duration::from_millis(1), view_idle())
                .len();
        }
        assert_eq!(
            ticks, 0,
            "a thousand accepted inputs inside one rung interval ticked the pump {ticks} times",
        );

        // And the pump is bounded rather than stopped: the interval passes and
        // the tick the viewer is entitled to arrives.
        assert_eq!(
            sessions.take_due(Instant::now() + driven, view_idle()).len(),
            1,
            "the floor stopped the pump instead of bounding it",
        );
    }

    /// The same bound, on the *other* path that pulls the deadline forward.
    ///
    /// A re-attach is answered with a fresh keyframe and brings the next tick
    /// forward, so a viewer writing `Attach` in a loop is the identical attack
    /// wearing a control message's shape.
    #[test]
    fn a_flood_of_reattaches_cannot_tick_the_pump_faster_than_its_rung() {
        let mut tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        attach(&mut sessions, &viewer, 1, &mut tabs);
        let driven = Duration::from_millis(driven_tick_ms());

        let start = Instant::now();
        assert_eq!(sessions.take_due(start, view_idle()).len(), 1, "the first frame was not due");

        let mut ticks = 0;
        for _ in 0..1000 {
            attach(&mut sessions, &viewer, 1, &mut tabs);
            ticks += sessions
                .take_due(start + driven - Duration::from_millis(1), view_idle())
                .len();
        }
        assert_eq!(ticks, 0, "a thousand re-attaches inside one rung interval ticked the pump");
    }

    // ---- the tile comparison -------------------------------------------

    /// A surface of a solid colour, so a changed pixel is unambiguous.
    fn flat(width: u32, height: u32, value: u8) -> Surface {
        Surface {
            width,
            height,
            pixels: vec![value; (width as usize) * (height as usize) * 4],
        }
    }

    /// The same surface with one pixel made different.
    fn with_pixel(base: &Surface, x: u32, y: u32) -> Surface {
        let mut pixels = base.pixels.clone();
        let at = ((y as usize) * (base.width as usize) + x as usize) * 4;
        pixels[at] = pixels[at].wrapping_add(0x40);
        Surface { width: base.width, height: base.height, pixels }
    }

    /// A single-pixel change in the interior is found, and found in exactly
    /// one tile.
    #[test]
    fn a_single_pixel_change_in_the_interior_dirties_exactly_its_tile() {
        let before = flat(256, 192, 0x20);
        let after = with_pixel(&before, 70, 70);
        assert_eq!(dirty_tiles(&before, &after), vec![(64, 64)]);
    }

    /// The right edge, where the tiles are **partial**: 200 is not a multiple
    /// of 64, so the last column is 8 pixels wide and an off-by-one here would
    /// either run off the buffer or leave a strip never compared.
    #[test]
    fn a_change_at_the_right_edge_is_found_in_the_partial_tile() {
        let before = flat(200, 192, 0x20);
        let after = with_pixel(&before, 199, 10);
        assert_eq!(dirty_tiles(&before, &after), vec![(192, 0)]);
    }

    /// The bottom edge, where the last row of tiles is partial for the same
    /// reason.
    #[test]
    fn a_change_at_the_bottom_edge_is_found_in_the_partial_tile() {
        let before = flat(192, 200, 0x20);
        let after = with_pixel(&before, 10, 199);
        assert_eq!(dirty_tiles(&before, &after), vec![(0, 192)]);
    }

    /// The bottom-right corner, where the tile is partial in **both**
    /// directions — the one pixel that two separate off-by-ones both reach.
    #[test]
    fn a_change_at_the_bottom_right_corner_is_found() {
        let before = flat(200, 200, 0x20);
        let after = with_pixel(&before, 199, 199);
        assert_eq!(dirty_tiles(&before, &after), vec![(192, 192)]);
        // And the region for it stops at the surface rather than at the tile
        // grid: a region running past the frame is refused on decode.
        let region = bounding_region(&dirty_tiles(&before, &after), &after).expect("a region");
        assert_eq!(region, Region { x: 192, y: 192, width: 8, height: 8 });
    }

    /// Every pixel of a partial edge tile is compared. A comparison that
    /// walked only whole tiles would miss all of these.
    #[test]
    fn every_pixel_of_a_partial_edge_tile_is_compared() {
        let before = flat(72, 72, 0x20);
        for (x, y) in [(64, 0), (71, 0), (0, 64), (0, 71), (64, 64), (71, 71)] {
            let after = with_pixel(&before, x, y);
            assert!(
                !dirty_tiles(&before, &after).is_empty(),
                "a change at ({x}, {y}) was never compared"
            );
        }
    }

    /// An unchanged frame is silent — the delta model's whole payoff, and the
    /// assertion a pump built on the screenshot path would fail.
    #[test]
    fn an_unchanged_frame_yields_no_dirty_tiles_and_no_message() {
        let before = flat(256, 192, 0x20);
        let after = flat(256, 192, 0x20);
        assert!(dirty_tiles(&before, &after).is_empty());
        assert!(
            select_frame(Some(&before), &after, false).is_none(),
            "a page nobody touched produced a message"
        );
    }

    /// Several adjacent tiles become **one** delta over their bounding region,
    /// not one message per tile.
    #[test]
    fn adjacent_changed_tiles_become_one_bounding_region() {
        let before = flat(256, 192, 0x20);
        let mut after = with_pixel(&before, 70, 70);
        for (x, y) in [(130, 70), (70, 130), (130, 130)] {
            after = with_pixel(&after, x, y);
        }
        assert_eq!(dirty_tiles(&before, &after).len(), 4);
        let (kind, region) = select_frame(Some(&before), &after, false).expect("a delta");
        assert_eq!(kind, FrameKind::Tile);
        assert_eq!(region, Region { x: 64, y: 64, width: 128, height: 128 });
    }

    // ---- keyframe selection --------------------------------------------

    /// No previous frame at all is a keyframe: a client holds nothing to
    /// composite a delta onto.
    #[test]
    fn the_first_frame_of_an_attachment_selects_a_keyframe() {
        let current = flat(256, 192, 0x20);
        let (kind, region) = select_frame(None, &current, false).expect("a keyframe");
        assert_eq!(kind, FrameKind::Keyframe);
        assert_eq!(region, current.whole());
    }

    /// A surface that changed size is a keyframe rather than a comparison —
    /// the client's texture is a different shape and every tile it holds is
    /// meaningless.
    #[test]
    fn a_surface_that_changed_size_selects_a_keyframe() {
        let before = flat(256, 192, 0x20);
        let after = flat(320, 240, 0x20);
        let (kind, region) = select_frame(Some(&before), &after, false).expect("a keyframe");
        assert_eq!(kind, FrameKind::Keyframe);
        assert_eq!(region, after.whole(), "the keyframe did not declare the new size");
        assert_eq!((region.width, region.height), (320, 240));
    }

    /// Below the threshold a delta; above it a keyframe. Asserted at the
    /// boundary rather than at a comfortable distance from it.
    #[test]
    fn the_dirty_tile_threshold_selects_a_keyframe_above_a_third_of_the_grid() {
        // A 16x16 grid of tiles: 1024x1024, 256 tiles, threshold 9/26 of that.
        let before = flat(1024, 1024, 0x20);
        let total = before.tile_count();
        assert_eq!(total, 256);
        let limit = total * KEYFRAME_DIRTY_NUMERATOR / KEYFRAME_DIRTY_DENOMINATOR;
        assert_eq!(limit, 88, "the measured threshold moved without the comment moving");

        let dirty_up_to = |count: usize| {
            let mut after = before.pixels.clone();
            for tile in 0..count {
                let x = (tile % 16) * 64;
                let y = (tile / 16) * 64;
                let at = (y * 1024 + x) * 4;
                after[at] = after[at].wrapping_add(0x40);
            }
            Surface { width: 1024, height: 1024, pixels: after }
        };

        let under = dirty_up_to(limit);
        assert_eq!(dirty_tiles(&before, &under).len(), limit);
        assert_eq!(
            select_frame(Some(&before), &under, false).map(|(kind, _)| kind),
            Some(FrameKind::Tile),
            "the threshold itself was already a keyframe, so 'exceeds' meant 'reaches'"
        );

        let over = dirty_up_to(limit + 1);
        assert_eq!(
            select_frame(Some(&before), &over, false).map(|(kind, _)| kind),
            Some(FrameKind::Keyframe),
            "a scroll-sized change was sent as hundreds of tiles"
        );
    }

    /// A forced keyframe wins over everything the comparison would have said,
    /// including "nothing changed".
    #[test]
    fn a_forced_keyframe_overrides_an_unchanged_frame() {
        let before = flat(256, 192, 0x20);
        let after = flat(256, 192, 0x20);
        let (kind, region) = select_frame(Some(&before), &after, true).expect("a keyframe");
        assert_eq!(kind, FrameKind::Keyframe);
        assert_eq!(region, after.whole());
    }

    /// A surface with no area produces nothing at all, because a zero-sized
    /// tile is refused on decode and a message the far end must reject is
    /// worse than a message not sent.
    #[test]
    fn a_surface_with_no_area_produces_no_frame() {
        assert!(select_frame(None, &flat(0, 0, 0), true).is_none());
        assert!(select_frame(None, &flat(64, 0, 0), true).is_none());
        assert!(select_frame(None, &flat(0, 64, 0), true).is_none());
    }

    // ---- the encoder and the header ------------------------------------

    /// The sibling encoder produces a PNG, and one the wire's own decoder can
    /// pair with a header it accepts. This is the round trip that proves the
    /// assembly rather than describing it.
    #[test]
    fn a_composed_frame_carries_a_header_the_wire_accepts() {
        let (out, _frames) = view_channel();
        let surface = flat(200, 200, 0x30);
        let frame = Frame {
            connection: 7,
            tab: 42,
            frame_seq: 9,
            last_delivered_input: 1839,
            keyframe: false,
            scale_denominator: Rung::FASTEST.scale_denominator(),
            surface,
            out,
        };
        let region = Region { x: 192, y: 192, width: 8, height: 8 };
        let message = compose(&frame, FrameKind::Tile, region).expect("a frame message");

        let (channel, body) = split_channel(&message).expect("a framed message");
        assert_eq!(channel, Channel::Frame);
        let header = FrameHeader::from_bytes(body).expect("a header the wire accepts");
        assert_eq!(header.kind, FrameKind::Tile);
        assert_eq!(header.tab_id, 42);
        assert_eq!(header.frame_seq, 9);
        assert_eq!(header.last_delivered_input, 1839, "the latency echo was not stamped");
        assert_eq!((header.tile_x, header.tile_y), (192, 192));
        assert_eq!((header.tile_width, header.tile_height), (8, 8));
        assert_eq!((header.frame_width, header.frame_height), (200, 200));
        assert_eq!(header.scale_denominator, Rung::FASTEST.scale_denominator());
        // And the payload is the PNG, sized from the wire's own constant.
        assert!(body.len() > FRAME_HEADER_LEN, "the message carried no payload");
        assert_eq!(&body[FRAME_HEADER_LEN..FRAME_HEADER_LEN + 8], b"\x89PNG\r\n\x1a\n");
    }

    /// A keyframe's region is the whole surface, which is the case the header
    /// is most easily assembled wrongly for — a tile that runs one pixel past
    /// its own frame is refused on decode.
    #[test]
    fn a_keyframe_covers_its_whole_surface_and_still_decodes() {
        let (out, _frames) = view_channel();
        let surface = flat(200, 136, 0x30);
        let whole = surface.whole();
        let frame = Frame {
            connection: 1,
            tab: 1,
            frame_seq: 1,
            last_delivered_input: 0,
            keyframe: true,
            scale_denominator: Rung::FASTEST.scale_denominator(),
            surface,
            out,
        };
        let message = compose(&frame, FrameKind::Keyframe, whole).expect("a frame message");
        let (_, body) = split_channel(&message).expect("a framed message");
        let header = FrameHeader::from_bytes(body).expect("a header the wire accepts");
        assert_eq!(header.kind, FrameKind::Keyframe);
        assert_eq!((header.tile_width, header.tile_height), (200, 136));
        assert_eq!((header.frame_width, header.frame_height), (200, 136));
    }

    /// The crop is the region's pixels and nothing else, at every edge.
    #[test]
    fn a_cropped_region_is_exactly_its_own_pixels() {
        let surface = flat(200, 200, 0x11);
        let corner = crop(&surface, Region { x: 192, y: 192, width: 8, height: 8 })
            .expect("a crop");
        assert_eq!(corner.len(), 8 * 8 * 4);
        assert!(corner.iter().all(|byte| *byte == 0x11));
        // A region running off the surface is refused rather than clamped: a
        // clamp would produce a frame that decodes and is wrong.
        assert!(crop(&surface, Region { x: 196, y: 0, width: 8, height: 8 }).is_none());
        assert!(crop(&surface, Region { x: 0, y: 196, width: 8, height: 8 }).is_none());
    }

    /// The encoder refuses a buffer that does not match the geometry it was
    /// given, rather than writing a truncated image.
    #[test]
    fn the_frame_encoder_refuses_a_buffer_that_does_not_match_its_geometry() {
        assert!(encode_frame(&vec![0u8; 64 * 64 * 4], 64, 64).is_some());
        assert!(encode_frame(&[0u8; 4], 64, 64).is_none());
        assert!(encode_frame(&[], 0, 0).is_none());
    }

    /// The frame channel is binary: the tag byte, the fixed header, then the
    /// payload, with nothing between them.
    #[test]
    fn a_frame_message_is_the_tag_then_the_header_then_the_payload() {
        let header = FrameHeader {
            kind: FrameKind::Tile,
            scale_denominator: Rung::FASTEST.scale_denominator(),
            tab_id: 3,
            frame_seq: 4,
            last_delivered_input: 5,
            tile_x: 0,
            tile_y: 0,
            tile_width: 64,
            tile_height: 64,
            frame_width: 128,
            frame_height: 128,
        };
        let message = frame_message(&header, &[0xAA, 0xBB, 0xCC]);
        assert_eq!(message.len(), 1 + FRAME_HEADER_LEN + 3);
        assert_eq!(message[0], Channel::Frame.tag());
        assert_eq!(&message[1..1 + FRAME_HEADER_LEN], &header.to_bytes());
        assert_eq!(&message[1 + FRAME_HEADER_LEN..], &[0xAA, 0xBB, 0xCC]);
    }

    /// The idle threshold's override lands on the default for anything it
    /// cannot read, and never on zero and never on unbounded.
    #[test]
    fn the_idle_threshold_falls_back_to_its_default_and_never_to_zero() {
        assert_eq!(parse_view_idle(Some("250")), Duration::from_millis(250));
        assert_eq!(parse_view_idle(Some("  4000 ")), Duration::from_millis(4000));
        let default = Duration::from_millis(talaria_protocol::wire::DEFAULT_VIEW_IDLE_MS);
        for bad in [None, Some(""), Some("0"), Some("-1"), Some("ages"), Some("2.5")] {
            assert_eq!(parse_view_idle(bad), default, "{bad:?} did not fall back to the default");
        }
    }
}

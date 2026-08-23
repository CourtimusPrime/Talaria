//! The multiplexed view wire: the vocabulary a remote viewer speaks.
//!
//! This is **not** the local control socket's line protocol. That one is
//! newline-delimited JSON over a Unix domain socket and lives in the crate
//! root; this one is a length-framed binary WebSocket carrying five
//! channels, one byte of tag apiece, and it exists so a client on another
//! machine can list a server's agent tabs, watch them paint, and drive them.
//!
//! It deliberately reuses the crate root's types rather than restating them:
//! [`crate::TabInfo`] on the tabs channel, [`crate::Event`] on the event
//! channel, [`crate::Outcome`] wherever a reply needs a shape. That
//! vocabulary has held unchanged across four phases, and a second spelling of
//! the same fact is a second thing to keep in step.
//!
//! **Structural refusal lives here; semantic refusal does not.** A tag that
//! names no channel, a header shorter than its own layout, a header whose
//! tile does not fit inside the frame it declares, a missing JSON field, a
//! coordinate that is not a finite number — all of those are refused in this
//! module, because deciding them needs nothing but the bytes. A coordinate
//! outside a *particular tab's* viewport, a sequence that did not increase on
//! *this* connection, a key name that maps to no key, a tab that is not
//! agent-owned — none of those are decidable here, because every one needs
//! state this crate does not have and must not acquire. They belong to the
//! shell's remote-input module.
//!
//! Per `D-05-02`, the view channel carries no agent tool vocabulary at all: a
//! viewer lists, watches and drives, and cannot open, navigate, evaluate,
//! close or download. Per `D-05-05`, the frame channel carries raw PNG bytes
//! behind a fixed binary header rather than a base64 string inside a JSON
//! envelope.

use serde::{Deserialize, Serialize};

use crate::TabInfo;

/// The view wire's own version, announced by [`ServerView::Hello`].
///
/// A client built against a different wire is told so on the control channel
/// rather than left to misparse a frame header into plausible geometry.
pub const PROTOCOL_VERSION: u32 = 1;

/// Channel tag for the JSON view-control channel ([`ClientView`], [`ServerView`]).
pub const CHANNEL_CONTROL: u8 = 0x01;

/// Channel tag for the JSON tabs channel ([`TabList`]).
pub const CHANNEL_TABS: u8 = 0x02;

/// Channel tag for the JSON event channel ([`crate::Event`]).
pub const CHANNEL_EVENT: u8 = 0x03;

/// Channel tag for the JSON input channel ([`InputMessage`]).
pub const CHANNEL_INPUT: u8 = 0x04;

/// Channel tag for the binary frame channel ([`FrameHeader`] then raw PNG bytes).
pub const CHANNEL_FRAME: u8 = 0x10;

/// Which channel a message's leading tag byte selects.
///
/// The tags are split into two blocks on purpose, and the gap between
/// [`CHANNEL_INPUT`] and [`CHANNEL_FRAME`] is the split rather than an
/// accident: the low block is human-readable JSON and the high block is
/// binary, so one byte tells a reader which world it is in before it decodes
/// anything. New JSON channels take the next low tag; new binary channels
/// take the next high one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Attach, detach, viewport and cadence, both directions.
    Control,
    /// The server's filtered list of agent tabs.
    Tabs,
    /// Unsolicited shell events, the same ones the control socket carries.
    Event,
    /// Pointer and keyboard input from the viewer into a page.
    Input,
    /// A frame header followed by the tile's encoded bytes.
    Frame,
}

impl Channel {
    /// The tag byte that selects this channel.
    pub fn tag(self) -> u8 {
        match self {
            Channel::Control => CHANNEL_CONTROL,
            Channel::Tabs => CHANNEL_TABS,
            Channel::Event => CHANNEL_EVENT,
            Channel::Input => CHANNEL_INPUT,
            Channel::Frame => CHANNEL_FRAME,
        }
    }

    /// The channel a tag byte selects, or `None` for a byte that selects
    /// none. A tag this function does not recognise is a refusal, never a
    /// fallthrough to a default channel: dispatching an unknown tag to the
    /// control decoder would hand a hostile peer a decoder it did not ask
    /// for.
    pub fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            CHANNEL_CONTROL => Some(Channel::Control),
            CHANNEL_TABS => Some(Channel::Tabs),
            CHANNEL_EVENT => Some(Channel::Event),
            CHANNEL_INPUT => Some(Channel::Input),
            CHANNEL_FRAME => Some(Channel::Frame),
            _ => None,
        }
    }
}

/// The tabs channel's payload: the server's agent tabs, in the order the
/// server keeps them.
///
/// `tabs` carries no `#[serde(default)]`, and that absence is deliberate. An
/// empty list is a legitimate state — a server with no agent tabs open — and
/// it must stay distinguishable from a message that forgot the field. A
/// default here would collapse the two into one, which is exactly the class
/// of silent substitution this wire refuses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabList {
    /// The agent tabs the viewer may attach to. Filtered server-side per
    /// `D-05-02`; a tab the human owns never appears here and is refused if
    /// asked for by identifier anyway.
    pub tabs: Vec<TabInfo>,
}

/// The frame header's format version. A header carrying any other value is
/// refused by [`FrameHeader::from_bytes`].
pub const FRAME_FORMAT_VERSION: u8 = 1;

/// The exact size of a [`FrameHeader`] on the wire.
///
/// Named once so the shell's writer and the client's reader size their buffers
/// from one place rather than from two literals that can drift apart.
pub const FRAME_HEADER_LEN: usize = 51;

/// Whether a frame message carries a whole frame or one changed tile of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    /// The whole frame at its declared size. Sent on attach, on resync after
    /// a reconnect, on a tab switch, on a viewport resize, and whenever the
    /// changed-tile count makes a whole frame the cheaper message.
    Keyframe,
    /// One changed tile, to be composited at the header's tile origin over
    /// whatever the client already holds.
    Tile,
}

impl FrameKind {
    /// The byte this kind occupies in the header layout.
    pub fn code(self) -> u8 {
        match self {
            FrameKind::Keyframe => 0,
            FrameKind::Tile => 1,
        }
    }

    /// The kind a header byte names, or `None` for a byte naming none.
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(FrameKind::Keyframe),
            1 => Some(FrameKind::Tile),
            _ => None,
        }
    }
}

/// The fixed-layout header in front of every frame message's encoded bytes.
///
/// The layout is explicit and little-endian, and [`FrameHeader::to_bytes`] and
/// [`FrameHeader::from_bytes`] sit adjacent so that a field added to one is
/// visibly missing from the other:
///
/// | Offset | Width | Field |
/// |--------|-------|-------|
/// | 0  | 1 | format version |
/// | 1  | 1 | kind |
/// | 2  | 1 | scale denominator |
/// | 3  | 8 | tab id |
/// | 11 | 8 | frame sequence |
/// | 19 | 8 | last applied input sequence |
/// | 27 | 4 | tile origin x |
/// | 31 | 4 | tile origin y |
/// | 35 | 4 | tile width |
/// | 39 | 4 | tile height |
/// | 43 | 4 | frame width |
/// | 47 | 4 | frame height |
///
/// Fifty-one bytes, [`FRAME_HEADER_LEN`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Whole frame, or one changed tile of it.
    pub kind: FrameKind,
    /// The divisor the server applied to the tab's real size before painting:
    /// 1 for full resolution, 2 for half. Carried rather than inferred,
    /// because a client that divided the tile size by the frame size would
    /// guess wrong on an odd-sized viewport, and the degrade ladder sends
    /// half-resolution frames whenever the link cannot carry full ones.
    pub scale_denominator: u8,
    /// The tab this frame paints. Always an agent-owned tab, per `D-05-02`.
    pub tab_id: u64,
    /// Strictly increasing within one connection and one tab, so a client can
    /// discard a frame it has already superseded. The enforcement is the
    /// server's; the contract is stated here so the two ends cannot disagree
    /// about it.
    pub frame_seq: u64,
    /// The input sequence the server had already applied when it painted this
    /// frame.
    ///
    /// This field is here for two jobs that are not obvious from its name, and
    /// it should not be deleted as redundant with `frame_seq`. First, it makes
    /// input-to-photon latency measurable with no clock synchronised across
    /// the two machines: the client knows when it sent that sequence and when
    /// this frame arrived, and both readings come off its own clock. Second,
    /// it lets a client drop a frame that predates its own most recent input,
    /// rather than briefly painting a stale page over a click it has already
    /// made.
    pub last_applied_input: u64,
    /// Tile origin in frame coordinates, x.
    pub tile_x: u32,
    /// Tile origin in frame coordinates, y.
    pub tile_y: u32,
    /// Tile width. Never zero; a zero-sized tile is refused on decode.
    pub tile_width: u32,
    /// Tile height. Never zero; a zero-sized tile is refused on decode.
    pub tile_height: u32,
    /// The whole frame's width at the declared scale, so the client can size
    /// its texture and notice a viewport change without a separate message.
    pub frame_width: u32,
    /// The whole frame's height at the declared scale.
    pub frame_height: u32,
}

impl FrameHeader {
    /// This header in its fixed little-endian layout.
    pub fn to_bytes(&self) -> [u8; FRAME_HEADER_LEN] {
        let mut bytes = [0u8; FRAME_HEADER_LEN];
        bytes[0] = FRAME_FORMAT_VERSION;
        bytes[1] = self.kind.code();
        bytes[2] = self.scale_denominator;
        bytes[3..11].copy_from_slice(&self.tab_id.to_le_bytes());
        bytes[11..19].copy_from_slice(&self.frame_seq.to_le_bytes());
        bytes[19..27].copy_from_slice(&self.last_applied_input.to_le_bytes());
        bytes[27..31].copy_from_slice(&self.tile_x.to_le_bytes());
        bytes[31..35].copy_from_slice(&self.tile_y.to_le_bytes());
        bytes[35..39].copy_from_slice(&self.tile_width.to_le_bytes());
        bytes[39..43].copy_from_slice(&self.tile_height.to_le_bytes());
        bytes[43..47].copy_from_slice(&self.frame_width.to_le_bytes());
        bytes[47..51].copy_from_slice(&self.frame_height.to_le_bytes());
        bytes
    }

    /// One header back out of its layout, or `None` for bytes that are not
    /// one.
    ///
    /// Six separate refusals, none of which substitutes a plausible default: a
    /// slice shorter than [`FRAME_HEADER_LEN`], a format version this build
    /// does not know, a kind byte naming no kind, a zero scale denominator, a
    /// tile with no area, and a tile that does not fit inside the frame it
    /// declares. The last is the one that matters most at a trust boundary —
    /// a header that claims a tile beyond its own frame is asking a client to
    /// composite outside its texture.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < FRAME_HEADER_LEN {
            return None;
        }
        if bytes[0] != FRAME_FORMAT_VERSION {
            return None;
        }
        let kind = FrameKind::from_code(bytes[1])?;
        let scale_denominator = bytes[2];
        if scale_denominator == 0 {
            return None;
        }
        let header = Self {
            kind,
            scale_denominator,
            tab_id: read_u64(bytes, 3)?,
            frame_seq: read_u64(bytes, 11)?,
            last_applied_input: read_u64(bytes, 19)?,
            tile_x: read_u32(bytes, 27)?,
            tile_y: read_u32(bytes, 31)?,
            tile_width: read_u32(bytes, 35)?,
            tile_height: read_u32(bytes, 39)?,
            frame_width: read_u32(bytes, 43)?,
            frame_height: read_u32(bytes, 47)?,
        };
        if header.tile_width == 0 || header.tile_height == 0 {
            return None;
        }
        if header.tile_x.checked_add(header.tile_width)? > header.frame_width {
            return None;
        }
        if header.tile_y.checked_add(header.tile_height)? > header.frame_height {
            return None;
        }
        Some(header)
    }
}

/// One little-endian `u64` at `offset`, or `None` if the slice ends first.
fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}

/// One little-endian `u32` at `offset`, or `None` if the slice ends first.
fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
}

/// Which pointer button an input message names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

/// Whether a pointer button or a key went down or came up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonAction {
    Down,
    Up,
}

/// Whether a wheel delta counts lines or pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WheelMode {
    Line,
    Pixel,
}

/// One message on the input channel.
///
/// Every variant carries the tab it targets and its own sequence number, and
/// the pointer variants carry their own coordinates rather than leaning on a
/// cached previous position. That is deliberate and is the difference from the
/// shell's *local* pointer path, which does keep one: a cached position is
/// per-window state, and two viewers driving two tabs through one window would
/// share it. The remote path must not.
///
/// Coordinates are in the target tab's own device pixels with the page's
/// top-left as the origin — never the window's. The toolbar offset the local
/// path subtracts is a local-window concern and has no meaning here.
///
/// `seq` is strictly increasing within one connection. A non-increasing value
/// is dropped by the server, which is what gives ordering under coalescing and
/// replay resistance inside a connection; the enforcement needs the
/// connection's own high-water mark and so is the shell's, but the contract is
/// written here once so the two ends cannot disagree about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputMessage {
    /// The pointer moved to `(x, y)`.
    MouseMove { tab: u64, seq: u64, x: f64, y: f64 },
    /// A pointer button went down or came up at `(x, y)`.
    MouseButton {
        tab: u64,
        seq: u64,
        x: f64,
        y: f64,
        button: PointerButton,
        action: ButtonAction,
    },
    /// The wheel turned by `(dx, dy)` in `mode` units, with the pointer at
    /// `(x, y)`.
    Wheel {
        tab: u64,
        seq: u64,
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
        mode: WheelMode,
    },
    /// A key went down or came up. Exactly one of `key` and `named` is
    /// present: `key` for a character the viewer typed, `named` for a key that
    /// produces none — Enter, Backspace, ArrowLeft. Both together, or neither,
    /// is a message that means two things or nothing, and
    /// [`InputMessage::from_json`] refuses it.
    Key {
        tab: u64,
        seq: u64,
        state: ButtonAction,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        named: Option<String>,
    },
}

impl InputMessage {
    /// Whether this message is structurally sound: finite coordinates, and a
    /// key naming exactly one of a character or a named key.
    ///
    /// Structural only. Whether the coordinate lands inside the target tab,
    /// whether the sequence advanced, whether the named key maps to anything,
    /// and whether the tab is agent-owned are all the shell's questions,
    /// because each needs state this crate does not hold.
    pub fn is_well_formed(&self) -> bool {
        match self {
            InputMessage::MouseMove { x, y, .. } => x.is_finite() && y.is_finite(),
            InputMessage::MouseButton { x, y, .. } => x.is_finite() && y.is_finite(),
            InputMessage::Wheel { x, y, dx, dy, .. } => {
                x.is_finite() && y.is_finite() && dx.is_finite() && dy.is_finite()
            },
            InputMessage::Key { key, named, .. } => key.is_some() != named.is_some(),
        }
    }

    /// This message as its channel payload, or `None` if it is not
    /// well-formed. Kept adjacent to [`InputMessage::from_json`] so the two
    /// directions cannot drift.
    pub fn to_json(&self) -> Option<String> {
        if !self.is_well_formed() {
            return None;
        }
        serde_json::to_string(self).ok()
    }

    /// One input-channel payload back into a message, or `None` for a payload
    /// that is not one.
    ///
    /// A missing field, an unknown kind, a coordinate that is not a finite
    /// number, and a key naming both or neither all produce nothing. None of
    /// them produces a message with a substituted zero — a decoder that
    /// defaulted a bad coordinate would be a decoder that let a malformed
    /// message move a real pointer.
    pub fn from_json(payload: &str) -> Option<Self> {
        let message: Self = serde_json::from_str(payload).ok()?;
        message.is_well_formed().then_some(message)
    }
}

/// A message the viewer sends on the control channel.
///
/// No variant carries the crate root's agent tool vocabulary, and that absence
/// is the design rather than an oversight. `D-05-02`, read literally against
/// Success Criterion 1, says a viewer lists, watches and drives: it clicks,
/// scrolls and types into a page, and it does not open tabs, navigate them,
/// evaluate script in them, close them or download through them. Putting that
/// vocabulary on the human input channel would create a second tool surface
/// reachable by a party whose only qualification is holding a token — and
/// [`crate::ChromeRect`]'s own doc comment already made this argument once,
/// for agents. It applies verbatim to the party this phase adds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "view", rename_all = "snake_case")]
pub enum ClientView {
    /// Begin receiving frames for `tab`. Refused unless the tab is
    /// agent-owned.
    Attach { tab: u64 },
    /// Stop receiving frames for `tab`, releasing the server's render lease.
    Detach { tab: u64 },
    /// The viewer's window changed size; paint `tab` at this size from now on.
    Viewport { tab: u64, width: u32, height: u32 },
    /// Ask for frames roughly every `interval_ms`. A request, not a promise:
    /// the server may deliver more slowly when the link cannot carry the rate.
    Cadence { interval_ms: u32 },
}

/// A message the server sends on the control channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "view", rename_all = "snake_case")]
pub enum ServerView {
    /// Sent first on every connection. `protocol` is [`PROTOCOL_VERSION`], so
    /// a client built against a different wire is told rather than left to
    /// misparse the frames that follow.
    Hello { protocol: u32 },
    /// The attach succeeded; frames for `tab` follow.
    Attached { tab: u64 },
    /// The lease on `tab` is released, whether the viewer asked or the server
    /// decided.
    Detached { tab: u64 },
    /// The request is refused, and **carries no reason**.
    ///
    /// This has no field on purpose. Two refusals that differ are two bits an
    /// attacker did not have before, and a refused attach must not let a
    /// caller tell "that tab belongs to the human" from "there is no such
    /// tab". Keeping the two answers identical is what stops this channel
    /// becoming an enumeration oracle for the human's own browsing.
    Refused,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tab(tab_id: u64) -> TabInfo {
        TabInfo {
            tab_id,
            url: "https://example.com/".into(),
            title: "Example".into(),
            owner: "agent-one".into(),
            focused: false,
            crashed: false,
            loading: false,
        }
    }

    fn sample_header() -> FrameHeader {
        FrameHeader {
            kind: FrameKind::Tile,
            scale_denominator: 2,
            tab_id: 7,
            frame_seq: 1841,
            last_applied_input: 1839,
            tile_x: 128,
            tile_y: 64,
            tile_width: 64,
            tile_height: 64,
            frame_width: 640,
            frame_height: 400,
        }
    }

    #[test]
    fn every_channel_tag_round_trips_through_its_meaning() {
        for channel in [
            Channel::Control,
            Channel::Tabs,
            Channel::Event,
            Channel::Input,
            Channel::Frame,
        ] {
            assert_eq!(Channel::from_tag(channel.tag()), Some(channel));
        }
        assert_eq!(Channel::Control.tag(), 0x01);
        assert_eq!(Channel::Tabs.tag(), 0x02);
        assert_eq!(Channel::Event.tag(), 0x03);
        assert_eq!(Channel::Input.tag(), 0x04);
        assert_eq!(Channel::Frame.tag(), 0x10);
    }

    #[test]
    fn a_byte_that_names_no_channel_is_refused() {
        for tag in [0x00u8, 0x05, 0x0f, 0x11, 0x7f, 0xff] {
            assert_eq!(Channel::from_tag(tag), None, "tag {tag:#04x} should name no channel");
        }
    }

    #[test]
    fn client_view_messages_round_trip() {
        let messages = [
            ClientView::Attach { tab: 7 },
            ClientView::Detach { tab: 7 },
            ClientView::Viewport { tab: 7, width: 1280, height: 800 },
            ClientView::Cadence { interval_ms: 30 },
        ];
        for message in messages {
            let json = serde_json::to_string(&message).expect("serializable");
            let back: ClientView = serde_json::from_str(&json).expect("deserializable");
            assert_eq!(back, message, "{json}");
        }
        let json = serde_json::to_string(&ClientView::Attach { tab: 7 }).expect("serializable");
        assert!(json.contains("\"view\":\"attach\""), "{json}");
    }

    #[test]
    fn server_view_messages_round_trip() {
        let messages = [
            ServerView::Hello { protocol: PROTOCOL_VERSION },
            ServerView::Attached { tab: 7 },
            ServerView::Detached { tab: 7 },
            ServerView::Refused,
        ];
        for message in messages {
            let json = serde_json::to_string(&message).expect("serializable");
            let back: ServerView = serde_json::from_str(&json).expect("deserializable");
            assert_eq!(back, message, "{json}");
        }
    }

    /// The refusal has no field to be informative with, which is the property
    /// rather than the implementation: its whole serialised form is its tag.
    #[test]
    fn a_refusal_carries_no_discriminating_reason() {
        let json = serde_json::to_string(&ServerView::Refused).expect("serializable");
        assert_eq!(json, "{\"view\":\"refused\"}");
    }

    #[test]
    fn every_input_kind_round_trips() {
        let messages = [
            InputMessage::MouseMove { tab: 7, seq: 1841, x: 412.0, y: 260.0 },
            InputMessage::MouseButton {
                tab: 7,
                seq: 1842,
                x: 412.0,
                y: 260.0,
                button: PointerButton::Left,
                action: ButtonAction::Down,
            },
            InputMessage::Wheel {
                tab: 7,
                seq: 1843,
                x: 412.0,
                y: 260.0,
                dx: 0.0,
                dy: -76.0,
                mode: WheelMode::Line,
            },
            InputMessage::Key {
                tab: 7,
                seq: 1844,
                state: ButtonAction::Down,
                key: Some("a".into()),
                named: None,
            },
            InputMessage::Key {
                tab: 7,
                seq: 1845,
                state: ButtonAction::Down,
                key: None,
                named: Some("Enter".into()),
            },
        ];
        for message in messages {
            let json = message.to_json().expect("well formed");
            let back = InputMessage::from_json(&json).expect("decodable");
            assert_eq!(back, message, "{json}");
        }
    }

    #[test]
    fn a_key_naming_both_a_character_and_a_named_key_is_refused() {
        let both = InputMessage::Key {
            tab: 7,
            seq: 1,
            state: ButtonAction::Down,
            key: Some("a".into()),
            named: Some("Enter".into()),
        };
        assert!(!both.is_well_formed());
        assert!(both.to_json().is_none());
        let payload = "{\"kind\":\"key\",\"tab\":7,\"seq\":1,\"state\":\"down\",\
                       \"key\":\"a\",\"named\":\"Enter\"}";
        assert_eq!(InputMessage::from_json(payload), None);
    }

    #[test]
    fn a_key_naming_neither_a_character_nor_a_named_key_is_refused() {
        let neither = InputMessage::Key {
            tab: 7,
            seq: 1,
            state: ButtonAction::Down,
            key: None,
            named: None,
        };
        assert!(!neither.is_well_formed());
        assert!(neither.to_json().is_none());
        let payload = "{\"kind\":\"key\",\"tab\":7,\"seq\":1,\"state\":\"down\"}";
        assert_eq!(InputMessage::from_json(payload), None);
    }

    #[test]
    fn a_coordinate_that_is_not_a_finite_number_is_refused_rather_than_clamped() {
        let not_a_number =
            InputMessage::MouseMove { tab: 7, seq: 1, x: f64::NAN, y: 260.0 };
        assert!(!not_a_number.is_well_formed());
        assert!(not_a_number.to_json().is_none());

        let infinite =
            InputMessage::MouseButton {
                tab: 7,
                seq: 2,
                x: f64::INFINITY,
                y: 260.0,
                button: PointerButton::Left,
                action: ButtonAction::Down,
            };
        assert!(!infinite.is_well_formed());
        assert!(infinite.to_json().is_none());

        let infinite_delta = InputMessage::Wheel {
            tab: 7,
            seq: 3,
            x: 1.0,
            y: 1.0,
            dx: 0.0,
            dy: f64::NEG_INFINITY,
            mode: WheelMode::Pixel,
        };
        assert!(!infinite_delta.is_well_formed());

        // And over the wire: a literal too large for `f64` decodes to an
        // infinity, which must be refused rather than accepted as a very
        // energetic click.
        let payload = "{\"kind\":\"mouse_move\",\"tab\":7,\"seq\":1,\"x\":1e400,\"y\":0.0}";
        assert_eq!(InputMessage::from_json(payload), None, "{payload}");
    }

    #[test]
    fn an_input_message_missing_a_field_is_refused() {
        let payload = "{\"kind\":\"mouse_move\",\"tab\":7,\"seq\":1,\"x\":412.0}";
        assert_eq!(InputMessage::from_json(payload), None);
        let unknown_kind = "{\"kind\":\"paste\",\"tab\":7,\"seq\":1}";
        assert_eq!(InputMessage::from_json(unknown_kind), None);
    }

    #[test]
    fn a_frame_header_round_trips_every_field() {
        let header = sample_header();
        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), FRAME_HEADER_LEN);
        let back = FrameHeader::from_bytes(&bytes).expect("decodable");
        assert_eq!(back, header);
        assert_eq!(back.kind, FrameKind::Tile);
        assert_eq!(back.scale_denominator, 2);
        assert_eq!(back.tab_id, 7);
        assert_eq!(back.frame_seq, 1841);
        assert_eq!(back.last_applied_input, 1839);
        assert_eq!(back.tile_x, 128);
        assert_eq!(back.tile_y, 64);
        assert_eq!(back.tile_width, 64);
        assert_eq!(back.tile_height, 64);
        assert_eq!(back.frame_width, 640);
        assert_eq!(back.frame_height, 400);
    }

    #[test]
    fn a_frame_header_round_trips_the_largest_value_each_field_can_hold() {
        let header = FrameHeader {
            kind: FrameKind::Keyframe,
            scale_denominator: u8::MAX,
            tab_id: u64::MAX,
            frame_seq: u64::MAX,
            last_applied_input: u64::MAX,
            tile_x: 0,
            tile_y: 0,
            tile_width: u32::MAX,
            tile_height: u32::MAX,
            frame_width: u32::MAX,
            frame_height: u32::MAX,
        };
        let back = FrameHeader::from_bytes(&header.to_bytes()).expect("decodable");
        assert_eq!(back, header);
    }

    #[test]
    fn a_slice_shorter_than_the_layout_yields_no_frame_header() {
        let bytes = sample_header().to_bytes();
        for length in 0..FRAME_HEADER_LEN {
            assert_eq!(
                FrameHeader::from_bytes(&bytes[..length]),
                None,
                "{length} bytes should decode to nothing"
            );
        }
        assert!(FrameHeader::from_bytes(&bytes).is_some());
    }

    #[test]
    fn an_unrecognised_format_version_yields_no_frame_header() {
        let mut bytes = sample_header().to_bytes();
        bytes[0] = FRAME_FORMAT_VERSION.wrapping_add(1);
        assert_eq!(FrameHeader::from_bytes(&bytes), None);
        bytes[0] = 0;
        assert_eq!(FrameHeader::from_bytes(&bytes), None);
    }

    #[test]
    fn an_unrecognised_frame_kind_yields_no_frame_header() {
        let mut bytes = sample_header().to_bytes();
        bytes[1] = 9;
        assert_eq!(FrameHeader::from_bytes(&bytes), None);
    }

    #[test]
    fn a_zero_scale_denominator_yields_no_frame_header() {
        let mut bytes = sample_header().to_bytes();
        bytes[2] = 0;
        assert_eq!(FrameHeader::from_bytes(&bytes), None);
    }

    #[test]
    fn a_zero_sized_tile_yields_no_frame_header() {
        let mut header = sample_header();
        header.tile_width = 0;
        assert_eq!(FrameHeader::from_bytes(&header.to_bytes()), None);

        let mut header = sample_header();
        header.tile_height = 0;
        assert_eq!(FrameHeader::from_bytes(&header.to_bytes()), None);
    }

    #[test]
    fn a_tile_that_exceeds_the_frame_it_declares_yields_no_frame_header() {
        let mut header = sample_header();
        header.tile_x = header.frame_width - header.tile_width + 1;
        assert_eq!(FrameHeader::from_bytes(&header.to_bytes()), None);

        let mut header = sample_header();
        header.tile_y = header.frame_height - header.tile_height + 1;
        assert_eq!(FrameHeader::from_bytes(&header.to_bytes()), None);

        // The overflow case: an origin near the top of the range plus a size
        // that would wrap must refuse rather than wrap into a fitting value.
        let mut header = sample_header();
        header.tile_x = u32::MAX;
        header.tile_width = 2;
        assert_eq!(FrameHeader::from_bytes(&header.to_bytes()), None);
    }

    #[test]
    fn an_empty_tab_list_round_trips_as_an_empty_list() {
        let json = serde_json::to_string(&TabList { tabs: vec![] }).expect("serializable");
        assert_eq!(json, "{\"tabs\":[]}");
        let back: TabList = serde_json::from_str(&json).expect("deserializable");
        assert!(back.tabs.is_empty());
    }

    /// An empty list is a state; an absent field is a malformed message. The
    /// two must not collapse into one, which is what a `default` here would do.
    #[test]
    fn an_absent_tab_list_field_is_refused_rather_than_defaulted() {
        assert!(serde_json::from_str::<TabList>("{}").is_err());
    }

    #[test]
    fn a_tab_list_round_trips_preserving_order() {
        let list = TabList { tabs: vec![sample_tab(9), sample_tab(3), sample_tab(4)] };
        let json = serde_json::to_string(&list).expect("serializable");
        let back: TabList = serde_json::from_str(&json).expect("deserializable");
        let identifiers: Vec<u64> = back.tabs.iter().map(|tab| tab.tab_id).collect();
        assert_eq!(identifiers, vec![9, 3, 4]);
    }
}

//! Window-system input into wire input, and the only place that happens.
//!
//! This module is the inverse of [`crate::present`]'s fit transform, and it is
//! **the inverse of that value** rather than a second computation of it.
//! No second transform is defined here — a source count of the transform's own
//! name over this file is zero on purpose: two transforms written
//! separately disagree the first time either changes, and the disagreement is
//! silent — a click that lands somewhere the human did not aim at, in a browser
//! they are using to clear a login wall.
//!
//! ## What this module deliberately does not do
//!
//! **It does not pull an outside position to the nearest edge.** A pointer in
//! the letterboxed margin, or over the client's own controls, produces *no
//! message at all*. Rounding such a position onto the edge of the page would
//! turn "the human aimed at the margin" into "the human clicked the edge of the
//! page" — a click they did not make, at the edge of the page, which is where a
//! confirm button lives. The server refuses an out-of-viewport coordinate too;
//! the two refusals are complementary rather than redundant, because the server
//! refuses what it must not trust and this end refuses what it knows was never
//! aimed at a page.
//!
//! **It does not synthesise.** Every pointer message carries the position the
//! pointer is at now. The server's remote path deliberately keeps no cached
//! position — a cached one is per-window state, and two viewers driving two tabs
//! through one window would share it — and this is the client half of that
//! arrangement. The window system reports a position only when the pointer
//! moves, so the last reported one is held here and stamped onto every message;
//! what matters is that the *wire* carries it and the far end caches nothing.
//!
//! **It does not fall back.** A key this build cannot name on the wire is not
//! sent. Sending it as some substitute character would type something the human
//! did not type, which is the keyboard's version of the click that landed
//! somewhere else.
//!
//! ## The rounding rule, which is a real question rather than a detail
//!
//! A window position becomes a page position by subtracting the fitted
//! surface's origin — which carries the client's own interface offset, applied
//! at exactly one place, [`crate::present::Fit::surface_rect`] — and dividing by
//! the scale. That yields a fraction, and the **floor** is taken: a pointer
//! anywhere within the screen pixel that shows page pixel *n* means *n*, which
//! is what a human means by "I clicked on that pixel". Rounding to nearest would
//! give the right half of every pixel to its neighbour, which is arbitrary until
//! somebody aims at a one-pixel target and then is not.
//!
//! ## The sequence, and what one field buys
//!
//! Strictly increasing for the life of the process, starting above zero,
//! incremented once per message sent and **never reused after a reconnect**. It
//! is not bookkeeping. The server drops a value that did not increase, which is
//! replay resistance inside a connection and ordering under coalescing; and the
//! frame header echoes the last one the server had applied when it painted,
//! which is what makes input-to-photon latency measurable with no clock
//! synchronised across the two machines.

use talaria_protocol::wire::{
    ButtonAction, InputMessage, PointerButton, WheelMode,
};
use winit::event::{ElementState, MouseButton, MouseScrollDelta};
use winit::keyboard::{Key as WinitKey, NamedKey as WinitNamedKey};

use crate::present::Fit;

/// Every key this client can name on the wire, and the winit key that produces
/// it.
///
/// **The strings are the server's**, spelled exactly as its own shared table
/// spells them, which is the W3C UI Events spelling. A test below walks this
/// list against that table's source and fails if either end grows a key the
/// other does not have — the assertion that makes "they cannot drift" true
/// rather than intended. The client cannot *link* the server's table: the shell
/// is a binary crate with the web engine compiled into it, and depending on it
/// would undo the one property this whole binary exists for.
const NAMED_KEYS: &[(&str, WinitNamedKey)] = &[
    ("Enter", WinitNamedKey::Enter),
    ("Tab", WinitNamedKey::Tab),
    ("Backspace", WinitNamedKey::Backspace),
    ("Delete", WinitNamedKey::Delete),
    ("Escape", WinitNamedKey::Escape),
    ("ArrowUp", WinitNamedKey::ArrowUp),
    ("ArrowDown", WinitNamedKey::ArrowDown),
    ("ArrowLeft", WinitNamedKey::ArrowLeft),
    ("ArrowRight", WinitNamedKey::ArrowRight),
    ("Home", WinitNamedKey::Home),
    ("End", WinitNamedKey::End),
    ("PageUp", WinitNamedKey::PageUp),
    ("PageDown", WinitNamedKey::PageDown),
    ("Shift", WinitNamedKey::Shift),
    ("Control", WinitNamedKey::Control),
    ("Alt", WinitNamedKey::Alt),
    ("Meta", WinitNamedKey::Meta),
    ("CapsLock", WinitNamedKey::CapsLock),
    ("ContextMenu", WinitNamedKey::ContextMenu),
    ("Insert", WinitNamedKey::Insert),
    ("F1", WinitNamedKey::F1),
    ("F2", WinitNamedKey::F2),
    ("F3", WinitNamedKey::F3),
    ("F4", WinitNamedKey::F4),
    ("F5", WinitNamedKey::F5),
    ("F6", WinitNamedKey::F6),
    ("F7", WinitNamedKey::F7),
    ("F8", WinitNamedKey::F8),
    ("F9", WinitNamedKey::F9),
    ("F10", WinitNamedKey::F10),
    ("F11", WinitNamedKey::F11),
    ("F12", WinitNamedKey::F12),
    ("Space", WinitNamedKey::Space),
];

/// The wire name for a winit named key, or `None` for one this build cannot
/// name.
///
/// `None` and never a substitution — see the module header's third refusal.
fn name_of(named: &WinitNamedKey) -> Option<&'static str> {
    // winit tells Super from Meta and the engine does not, so the fold that the
    // server's table performs on its own side is performed here too. It is a
    // fold rather than a second row, because a second row spelled "Meta" would
    // make the *name* lookup ambiguous — and the name lookup is the direction
    // the far end uses.
    if matches!(named, WinitNamedKey::Super) {
        return Some("Meta");
    }
    NAMED_KEYS.iter().find(|entry| entry.1 == *named).map(|entry| entry.0)
}

/// The page position a window position names, or nothing at all.
///
/// **The inverse of [`crate::present::fit`], and the only one.** Subtract the
/// fitted surface's origin — which is where the client's own interface offset
/// lives, applied once — divide by the scale, take the floor.
///
/// The surface's rectangle is half-open: its top-left corner is inside and its
/// bottom-right corner is not, which is what makes the page's last pixel
/// reachable and the pixel past it unreachable without either being a special
/// case.
///
/// `None` for every position outside that rectangle, and for a transform with
/// no scale to divide by. Nothing is moved to the nearest valid position.
pub fn page_position(fit: &Fit, window: egui::Pos2) -> Option<(f64, f64)> {
    if !fit.scale.is_finite() || fit.scale <= 0.0 {
        return None;
    }
    let rect = fit.surface_rect();
    let x = ((window.x - rect.min.x) / fit.scale).floor();
    let y = ((window.y - rect.min.y) / fit.scale).floor();
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let (page_width, page_height) = fit.page;
    if x < 0.0 || y < 0.0 || x >= page_width as f32 || y >= page_height as f32 {
        return None;
    }
    Some((f64::from(x), f64::from(y)))
}

/// What the client is allowed to send right now, and where.
///
/// Built fresh from the loop's own state on every event rather than cached, so
/// there is no stale copy of "which tab am I driving" to go wrong: the two
/// facts that decide it — the connection's state and the attachment — are the
/// loop's, and this is a reading of them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aim {
    /// Whether the connection is established.
    pub connected: bool,
    /// The tab the client is attached to, if any.
    pub tab: Option<u64>,
    /// How the picture is laid out, if one is being shown.
    pub fit: Option<Fit>,
}

/// The first two refusals, and the coordinate, in one place.
///
/// Returning `None` here is the whole of "nothing is sent while not connected"
/// and "nothing is sent for a tab this client is not attached to". They are
/// checked before the coordinate is even computed, because a message the client
/// knows the far end will refuse costs a sequence number and confuses the
/// latency estimate that reads it.
fn aimed(aim: &Aim, window: egui::Pos2) -> Option<(u64, f64, f64)> {
    // Refusal one: not connected.
    if !aim.connected {
        return None;
    }
    // Refusal two: not attached to anything.
    let tab = aim.tab?;
    let fit = aim.fit?;
    let (x, y) = page_position(&fit, window)?;
    Some((tab, x, y))
}

/// The client's input state: the sequence, and where the pointer last was.
#[derive(Debug, Default)]
pub struct Capture {
    /// The last sequence number used. Zero means none has been, so the first
    /// message sent carries one — the wire's "starting above zero".
    ///
    /// **Never reset**, including by a reconnect. A number reused on a second
    /// connection within one process would make the frame header's echo
    /// ambiguous about which of the two inputs it was reporting.
    seq: u64,
    /// Where the pointer was last reported to be, in the client window's own
    /// logical points. `None` before the pointer has been over the window, and
    /// cleared when it leaves — a button press with no known position sends
    /// nothing rather than a guess.
    pointer: Option<egui::Pos2>,
}

impl Capture {
    /// The next sequence number. Strictly increasing, and the only place one is
    /// produced.
    fn next_seq(&mut self) -> u64 {
        self.seq = self.seq.saturating_add(1);
        self.seq
    }

    /// The last sequence number sent, for the interface's readings.
    pub fn last_seq(&self) -> u64 {
        self.seq
    }

    /// The pointer left the window, so there is no current position any more.
    pub fn pointer_left(&mut self) {
        self.pointer = None;
    }

    /// The pointer moved to `window`, in the client window's logical points.
    pub fn pointer_moved(&mut self, aim: &Aim, window: egui::Pos2) -> Option<InputMessage> {
        // Recorded before the refusals, not after: the position is true whether
        // or not a message may be sent, and a press that follows a motion over
        // the margin must know the pointer is in the margin.
        self.pointer = Some(window);
        let (tab, x, y) = aimed(aim, window)?;
        Some(InputMessage::MouseMove { tab, seq: self.next_seq(), x, y })
    }

    /// A pointer button went down or came up.
    ///
    /// Carries the pointer's current position, because the wire's message does
    /// and the far end holds none.
    pub fn pointer_button(
        &mut self,
        aim: &Aim,
        button: MouseButton,
        state: ElementState,
    ) -> Option<InputMessage> {
        let window = self.pointer?;
        let (tab, x, y) = aimed(aim, window)?;
        let button = match button {
            MouseButton::Left => PointerButton::Left,
            MouseButton::Middle => PointerButton::Middle,
            MouseButton::Right => PointerButton::Right,
            // A button this wire cannot name is not sent as one it can. The
            // back and forward buttons are the ones that turn up, and sending
            // either as a left click would navigate a page by pressing whatever
            // is under the pointer.
            _ => return None,
        };
        let action = match state {
            ElementState::Pressed => ButtonAction::Down,
            ElementState::Released => ButtonAction::Up,
        };
        Some(InputMessage::MouseButton { tab, seq: self.next_seq(), x, y, button, action })
    }

    /// The wheel turned.
    ///
    /// The mode is **whichever the window system reported and nothing else**:
    /// line deltas travel as line deltas and pixel deltas as pixel deltas. This
    /// end does not convert between them, because how far a line is is a
    /// question about the page being scrolled and the server is the end that
    /// knows the answer.
    pub fn wheel(&mut self, aim: &Aim, delta: MouseScrollDelta) -> Option<InputMessage> {
        let window = self.pointer?;
        let (tab, x, y) = aimed(aim, window)?;
        let (dx, dy, mode) = match delta {
            MouseScrollDelta::LineDelta(dx, dy) => {
                (f64::from(dx), f64::from(dy), WheelMode::Line)
            },
            MouseScrollDelta::PixelDelta(delta) => (delta.x, delta.y, WheelMode::Pixel),
        };
        Some(InputMessage::Wheel { tab, seq: self.next_seq(), x, y, dx, dy, mode })
    }

    /// A key went down or came up.
    ///
    /// Takes the **logical key and the state**, not the whole
    /// [`winit::event::KeyEvent`], and that is not only for testability: a
    /// window-system key event is deliberately not what travels on this wire —
    /// the server's own key module says so — because most of its fields have no
    /// meaning on the far end and one of them is not constructible outside
    /// winit at all. Two values go out; two values come in here.
    ///
    /// Needs no pointer and no picture — a human can type into a page whose
    /// pointer is elsewhere — but still needs the connection and the
    /// attachment, which are refusals one and two.
    pub fn key(
        &mut self,
        aim: &Aim,
        logical: &WinitKey,
        state: ElementState,
    ) -> Option<InputMessage> {
        if !aim.connected {
            return None;
        }
        let tab = aim.tab?;
        let state = match state {
            ElementState::Pressed => ButtonAction::Down,
            ElementState::Released => ButtonAction::Up,
        };
        // Exactly one of the two, never both and never neither — the shape the
        // wire's own decoder refuses anything else in.
        let (key, named) = match logical {
            WinitKey::Character(text) => (Some(text.to_string()), None),
            // Refusal three: a key this build cannot name is not sent.
            WinitKey::Named(named) => (None, Some(name_of(named)?.to_owned())),
            _ => return None,
        };
        Some(InputMessage::Key { tab, seq: self.next_seq(), state, key, named })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::present::{fit, SurfaceSize};

    /// The server's own key table, read as source.
    ///
    /// Not a dependency: `include_str!` copies bytes at compile time and links
    /// nothing, which is what lets the drift assertion exist at all in a crate
    /// that must not link the engine.
    const SERVER_KEY_TABLE: &str = include_str!("../../talaria-shell/src/keyutils.rs");

    /// A content area with a deliberate offset, standing in for the client's
    /// own controls being to the left of the page.
    fn area() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(340.0, 0.0), egui::vec2(400.0, 400.0))
    }

    /// A surface twice as wide as it is tall, fitted into the square above: the
    /// picture is 400 by 200 at scale 1, letterboxed 100 points top and bottom.
    fn laid_out() -> Fit {
        let Some(fitted) = fit(SurfaceSize { width: 400, height: 200, denominator: 1 }, area())
        else {
            panic!("a positive surface in a positive area must fit");
        };
        assert_eq!(fitted.scale, 1.0);
        assert_eq!(fitted.origin, egui::vec2(0.0, 100.0));
        fitted
    }

    fn aim() -> Aim {
        Aim { connected: true, tab: Some(42), fit: Some(laid_out()) }
    }

    fn character(text: &str) -> WinitKey {
        WinitKey::Character(text.into())
    }

    #[test]
    fn the_top_left_of_the_surface_is_the_pages_own_origin() {
        assert_eq!(page_position(&laid_out(), egui::pos2(340.0, 100.0)), Some((0.0, 0.0)));
    }

    #[test]
    fn one_point_inside_the_far_edges_is_the_pages_last_pixel() {
        assert_eq!(page_position(&laid_out(), egui::pos2(739.0, 299.0)), Some((399.0, 199.0)));
    }

    #[test]
    fn one_point_outside_the_left_edge_yields_nothing() {
        assert_eq!(page_position(&laid_out(), egui::pos2(339.0, 200.0)), None);
    }

    #[test]
    fn one_point_outside_the_right_edge_yields_nothing() {
        // Exactly on the far boundary is already outside — the rectangle is
        // half-open — and one point past it stays outside.
        assert_eq!(page_position(&laid_out(), egui::pos2(740.0, 200.0)), None);
        assert_eq!(page_position(&laid_out(), egui::pos2(741.0, 200.0)), None);
    }

    #[test]
    fn one_point_outside_the_top_edge_yields_nothing() {
        // Inside the *content area* and outside the *picture*: this is the
        // letterboxed margin, and it is the case a rounded-to-the-edge mapping
        // would turn into a click on the first row of the page.
        assert_eq!(page_position(&laid_out(), egui::pos2(500.0, 99.0)), None);
        assert_eq!(page_position(&laid_out(), egui::pos2(500.0, 0.0)), None);
    }

    #[test]
    fn one_point_outside_the_bottom_edge_yields_nothing() {
        assert_eq!(page_position(&laid_out(), egui::pos2(500.0, 300.0)), None);
        assert_eq!(page_position(&laid_out(), egui::pos2(500.0, 399.0)), None);
    }

    #[test]
    fn a_position_over_the_clients_own_interface_yields_nothing() {
        // The controls occupy everything left of the content area. A press on
        // one of them must not reach a page — the whole of T-05-04-G, and it is
        // true here because the offset is inside the transform rather than
        // beside it.
        for x in [0.0, 100.0, 339.0] {
            assert_eq!(page_position(&laid_out(), egui::pos2(x, 200.0)), None, "x = {x}");
        }
    }

    #[test]
    fn the_rounding_rule_is_the_floor_rather_than_the_nearest() {
        // A picture at twice life size: one page pixel is two points wide, so
        // both points showing page pixel 3 must map to 3. Rounding to nearest
        // would give the second of them to pixel 4.
        let Some(doubled) =
            fit(SurfaceSize { width: 100, height: 100, denominator: 1 },
                egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 200.0)))
        else {
            panic!("a positive surface in a positive area must fit");
        };
        assert_eq!(doubled.scale, 2.0);
        assert_eq!(page_position(&doubled, egui::pos2(6.0, 6.0)), Some((3.0, 3.0)));
        assert_eq!(page_position(&doubled, egui::pos2(7.9, 7.9)), Some((3.0, 3.0)));
        assert_eq!(page_position(&doubled, egui::pos2(8.0, 8.0)), Some((4.0, 4.0)));
    }

    #[test]
    fn a_half_resolution_picture_maps_to_the_pages_own_coordinates() {
        // The texture is half the page in each axis. The coordinate that comes
        // out is the *page's*, because that is the space the far end's viewport
        // test and the page's own layout are both in.
        let Some(halved) =
            fit(SurfaceSize { width: 200, height: 100, denominator: 2 }, area())
        else {
            panic!("a positive surface in a positive area must fit");
        };
        assert_eq!(halved.page, (400, 200));
        assert_eq!(page_position(&halved, egui::pos2(340.0, 100.0)), Some((0.0, 0.0)));
        assert_eq!(page_position(&halved, egui::pos2(739.0, 299.0)), Some((399.0, 199.0)));
    }

    #[test]
    fn the_sequence_starts_above_zero_and_strictly_increases() {
        let mut capture = Capture::default();
        assert_eq!(capture.last_seq(), 0, "nothing has been sent yet");
        let mut seen = Vec::new();
        for _ in 0..4 {
            let Some(message) = capture.pointer_moved(&aim(), egui::pos2(400.0, 200.0)) else {
                panic!("a position inside the picture must produce a message");
            };
            let InputMessage::MouseMove { seq, .. } = message else {
                panic!("a pointer motion must be a motion");
            };
            seen.push(seq);
        }
        assert_eq!(seen, vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_sequence_is_not_spent_on_a_message_that_is_not_sent() {
        let mut capture = Capture::default();
        // A motion into the margin: refused, and the counter must not move, or
        // the frame header's echo would report a number nothing ever sent.
        assert!(capture.pointer_moved(&aim(), egui::pos2(500.0, 10.0)).is_none());
        assert_eq!(capture.last_seq(), 0);
        assert!(capture.pointer_moved(&aim(), egui::pos2(500.0, 200.0)).is_some());
        assert_eq!(capture.last_seq(), 1);
    }

    #[test]
    fn nothing_is_sent_while_the_connection_is_not_established() {
        let mut capture = Capture::default();
        let offline = Aim { connected: false, ..aim() };
        assert!(capture.pointer_moved(&offline, egui::pos2(400.0, 200.0)).is_none());
        assert!(capture.pointer_button(&offline, MouseButton::Left, ElementState::Pressed)
            .is_none());
        assert!(capture.wheel(&offline, MouseScrollDelta::LineDelta(0.0, 1.0)).is_none());
        assert!(capture.key(&offline, &character("a"), ElementState::Pressed).is_none());
        assert_eq!(capture.last_seq(), 0);
    }

    #[test]
    fn nothing_is_sent_for_a_tab_this_client_is_not_attached_to() {
        let mut capture = Capture::default();
        let unattached = Aim { tab: None, ..aim() };
        assert!(capture.pointer_moved(&unattached, egui::pos2(400.0, 200.0)).is_none());
        assert!(capture.key(&unattached, &character("a"), ElementState::Pressed).is_none());
        assert_eq!(capture.last_seq(), 0);
    }

    #[test]
    fn a_press_and_a_release_each_carry_the_pointers_current_position() {
        let mut capture = Capture::default();
        assert!(capture.pointer_moved(&aim(), egui::pos2(400.0, 200.0)).is_some());
        let mut positions = Vec::new();
        for state in [ElementState::Pressed, ElementState::Released] {
            let Some(InputMessage::MouseButton { x, y, action, .. }) =
                capture.pointer_button(&aim(), MouseButton::Left, state)
            else {
                panic!("a button over the picture must produce a button message");
            };
            positions.push((x, y, action));
        }
        assert_eq!(
            positions,
            vec![(60.0, 100.0, ButtonAction::Down), (60.0, 100.0, ButtonAction::Up)],
        );
    }

    #[test]
    fn a_button_with_no_known_pointer_position_sends_nothing() {
        let mut capture = Capture::default();
        assert!(capture.pointer_button(&aim(), MouseButton::Left, ElementState::Pressed)
            .is_none());
        assert!(capture.pointer_moved(&aim(), egui::pos2(400.0, 200.0)).is_some());
        assert!(capture.pointer_button(&aim(), MouseButton::Left, ElementState::Pressed)
            .is_some());
        // And the pointer leaving takes the position with it.
        capture.pointer_left();
        assert!(capture.pointer_button(&aim(), MouseButton::Left, ElementState::Released)
            .is_none());
    }

    #[test]
    fn a_wheel_travels_in_whichever_mode_the_window_system_reported() {
        let mut capture = Capture::default();
        assert!(capture.pointer_moved(&aim(), egui::pos2(400.0, 200.0)).is_some());
        let Some(InputMessage::Wheel { dx, dy, mode, .. }) =
            capture.wheel(&aim(), MouseScrollDelta::LineDelta(0.0, -3.0))
        else {
            panic!("a wheel over the picture must produce a wheel message");
        };
        assert_eq!((dx, dy, mode), (0.0, -3.0, WheelMode::Line));

        let Some(InputMessage::Wheel { dx, dy, mode, .. }) = capture.wheel(
            &aim(),
            MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(1.5, -12.0)),
        ) else {
            panic!("a wheel over the picture must produce a wheel message");
        };
        assert_eq!((dx, dy, mode), (1.5, -12.0, WheelMode::Pixel));
    }

    #[test]
    fn a_character_key_travels_as_a_character_and_a_named_key_as_a_name() {
        let mut capture = Capture::default();
        let Some(InputMessage::Key { key, named, state, .. }) =
            capture.key(&aim(), &character("q"), ElementState::Pressed)
        else {
            panic!("a character key must produce a key message");
        };
        assert_eq!((key, named, state), (Some("q".to_owned()), None, ButtonAction::Down));

        let Some(InputMessage::Key { key, named, .. }) = capture.key(
            &aim(),
            &WinitKey::Named(WinitNamedKey::Enter),
            ElementState::Released,
        ) else {
            panic!("a named key in the table must produce a key message");
        };
        assert_eq!((key, named), (None, Some("Enter".to_owned())));
    }

    #[test]
    fn a_key_this_build_cannot_name_types_nothing_rather_than_something_else() {
        let mut capture = Capture::default();
        // A named key that is genuinely absent from the table. Sent as any
        // substitute it would type something the human did not press.
        assert!(capture
            .key(&aim(), &WinitKey::Named(WinitNamedKey::F20), ElementState::Pressed)
            .is_none());
        assert!(capture
            .key(&aim(), &WinitKey::Named(WinitNamedKey::BrightnessUp), ElementState::Pressed)
            .is_none());
        assert_eq!(capture.last_seq(), 0, "a refused key spent a sequence number");
    }

    #[test]
    fn the_super_key_is_folded_onto_the_name_the_server_knows() {
        assert_eq!(name_of(&WinitNamedKey::Super), Some("Meta"));
        assert_eq!(name_of(&WinitNamedKey::Meta), Some("Meta"));
    }

    #[test]
    fn every_name_this_client_can_emit_is_one_the_server_maps() {
        // The list, walked — not a comment asserting it. The server's table is
        // read as source because the client cannot link it, and the parse is
        // asserted non-empty first so a table that moved fails loudly here
        // rather than passing vacuously.
        let server: Vec<&str> = SERVER_KEY_TABLE
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("(\"") && line.contains("WinitNamedKey::"))
            .filter_map(|line| line.split('"').nth(1))
            .collect();
        assert!(
            server.len() > 20,
            "the server's key table was not found where this test reads it; the parse \
             produced {server:?}",
        );
        for (name, _) in NAMED_KEYS {
            assert!(
                server.contains(name),
                "this client can emit the key name {name:?}, which the server's table does \
                 not map — a viewer pressing it would type nothing at all",
            );
        }
        for name in &server {
            assert!(
                NAMED_KEYS.iter().any(|(ours, _)| ours == name),
                "the server maps the key name {name:?} and this client cannot emit it, so a \
                 human at a viewer cannot press it",
            );
        }
    }
}

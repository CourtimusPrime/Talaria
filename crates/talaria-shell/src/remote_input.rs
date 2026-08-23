//! The one path from the view wire to the engine's input, and deliberately a
//! narrow one.
//!
//! A remote viewer sends raw clicks, scrolls and keystrokes into a live,
//! already-logged-in browsing session running in a process that also holds the
//! credential vault. That is a different and in places worse surface than the
//! agent tool channel Phase 4 built: an agent gets an allowlisted vocabulary of
//! nine commands, and a viewer gets coordinates.
//!
//! **What this module reaches:** a [`servo::WebView`]'s input entry point, for
//! one tab, named by the message.
//!
//! **What it reaches by construction, not at all:** the browser chrome, the
//! browser-shortcut handler, the interface-action queue, any panel, the tab
//! table's active-tab state, the human's view mode, and the window's focus.
//! None of those is refused by a check here — there is simply no type in this
//! module that connects to one, which is what makes the guarantee survive a
//! later reader (T-05-04).
//!
//! The reasoning is not new. `talaria_protocol::ChromeRect`'s own doc comment
//! already refused to hand chrome geometry to agents, **because** an actor that
//! can aim synthetic input at the chrome can aim it at the credentials
//! control, the bookmark star, or a downloads row's open control — which hands
//! a file to the operating system's default application. That was written for
//! agents; it applies verbatim to the party this phase adds, and it is sharper
//! here, because a viewer sends real coordinates rather than tool calls.
//!
//! `D-05-02` supplies the other half: Me tabs stay local to the server
//! machine. Resolution therefore goes through the **agent-only** lookup
//! ([`crate::tabs::TabManager::agent_tab`]), so a tab the human owns is
//! unrepresentable on this path rather than refused downstream — and it takes
//! the same exit a tab that never existed takes, so the channel is not an
//! enumeration oracle for the human's own browsing either (T-05-05).
//!
//! Structural refusal already happened in [`talaria_protocol::wire`], which
//! needs only the bytes. Everything here is semantic, and every one of them is
//! a refusal rather than a substituted default.

use servo::{
    InputEvent, MouseButton as ServoMouseButton, MouseButtonAction, WheelDelta,
    WheelMode as ServoWheelMode,
};
use webrender_api::units::DevicePixel;

use talaria_protocol::wire::{ButtonAction, InputMessage, PointerButton, WheelMode};

use crate::app::{self, Shared};
use crate::keyutils;
use crate::view::{ViewSessions, ViewTabs};

/// Apply one decoded input message from view connection `connection`.
///
/// Returns whether anything reached a page. Every `false` is silent to the
/// peer beyond the fact of not happening, and none of them says which rule was
/// broken.
///
/// The order, stopping at the first failure:
///
/// 1. The connection exists.
/// 2. The sequence strictly increased on **this** connection, and the accepted
///    value is recorded there.
/// 3. The connection holds an attachment on the tab the message names.
/// 4. The tab resolves through the **agent-only** lookup.
/// 5. The tab is not crashed.
/// 6. The message converts — an unmapped key name and an unrepresentable wheel
///    mode are refusals.
/// 7. The delivery functions are called. **Nothing else.**
///
/// Steps 1 to 4 are [`admit`], which needs no engine and carries this module's
/// unit tests. Steps 5 to 7 need a live webview and are proven end to end by
/// `tests/e2e/remote_view_test.py`.
pub fn apply(state: &Shared, connection: u64, message: &InputMessage) -> bool {
    // Two borrows in one scope, released before a webview is touched — the
    // deferred-queue discipline the rest of `app.rs` uses, because a servo
    // callback may be holding either of these tables when a socket message
    // arrives.
    let admitted = {
        let tabs = state.tabs.borrow();
        admit(&mut state.views.borrow_mut(), &*tabs, connection, message)
    };
    if !admitted {
        return false;
    }

    let webview = {
        let tabs = state.tabs.borrow();
        // (4) again, for the webview itself: `agent_tab` and nothing else. A
        // `get` followed by an owner test would work today and stop working
        // the first time somebody added a second call site — and the tab this
        // must never resolve is the one holding the human's autofilled
        // credentials (T-05-05).
        //
        // Deliberately **not** the tab table's displayed-tab accessor, which
        // this module names nowhere: a target that followed whatever the local
        // human is looking at would let a local tab switch silently redirect a
        // remote click into a different page — and, the moment the human
        // flipped to their own view, into a page this channel may not reach at
        // all (T-05-04-C).
        let Some(tab) = tabs.agent_tab(target_tab(message)) else { return false };
        // (5) A crashed tab has no document to receive input, and refusing it
        // here matches how script evaluation refuses one: a value, not a panic,
        // and recovered by navigating.
        if tab.crashed {
            return false;
        }
        tab.webview.clone()
    };

    // (6) and (7). The point goes through **untouched**: it is already in the
    // target tab's own device pixels, the containment test against that tab's
    // viewport belongs to the delivery function, and the local window's
    // toolbar height has no meaning for a client that draws no toolbar. A
    // subtraction added here would be a silent forty-pixel error on every
    // remote click that nothing would report (T-05-04-A).
    //
    // The local cursor cache is likewise never written: it is the local
    // pointer's, the wire carries coordinates on every pointer message
    // precisely so this path needs none, and sharing it would contaminate the
    // two input sources in both directions (T-05-04-B).
    match message {
        InputMessage::MouseMove { x, y, .. } => app::deliver_mouse_move(&webview, point(*x, *y)),
        InputMessage::MouseButton { x, y, button, action, .. } => app::deliver_mouse_button(
            &webview,
            point(*x, *y),
            mouse_button(*button),
            button_action(*action),
        ),
        InputMessage::Wheel { x, y, dx, dy, mode, .. } => app::deliver_wheel(
            &webview,
            point(*x, *y),
            WheelDelta { x: *dx, y: *dy, z: 0.0, mode: wheel_mode(*mode) },
        ),
        InputMessage::Key { state, key, named, .. } => {
            let Some(event) = keyutils::keyboard_event_from_wire(
                key_state(*state),
                key.as_deref(),
                named.as_deref(),
            ) else {
                return false;
            };
            webview.notify_input_event(InputEvent::Keyboard(event));
            true
        },
    }
}

/// Steps 1 to 4: the whole of the decision that needs no engine.
///
/// Split out with the two tables as parameters — rather than reading them off
/// the shared state — for the same reason [`crate::view::parse_max_attachments`]
/// takes its raw value as one: the interesting properties here are the
/// refusals, and every table a real `Shared` holds owns a live `WebView`.
fn admit(
    views: &mut ViewSessions,
    tabs: &dyn ViewTabs,
    connection: u64,
    message: &InputMessage,
) -> bool {
    let tab = target_tab(message);
    // (1), (2) and (3), which are the connection's own state and so live with
    // the connection. See [`ViewSessions::admit_input`].
    if !views.admit_input(connection, tab, sequence_of(message)) {
        return false;
    }
    // (4) The agent-only lookup, through the trait whose implementation *is*
    // `TabManager::agent_tab`. A tab the human owns and a tab that was never
    // created both yield nothing and take this same exit, so the two are
    // indistinguishable from the far side.
    tabs.agent_viewport(tab).is_some()
}

/// The tab an input message names.
///
/// A match rather than an accessor on the wire type: every variant spells the
/// field itself and the crate that owns them deliberately exposes no getter,
/// because the semantic questions are the shell's — so reading the fields is
/// the shell's too.
fn target_tab(message: &InputMessage) -> u64 {
    match message {
        InputMessage::MouseMove { tab, .. }
        | InputMessage::MouseButton { tab, .. }
        | InputMessage::Wheel { tab, .. }
        | InputMessage::Key { tab, .. } => *tab,
    }
}

/// The sequence number an input message carries.
fn sequence_of(message: &InputMessage) -> u64 {
    match message {
        InputMessage::MouseMove { seq, .. }
        | InputMessage::MouseButton { seq, .. }
        | InputMessage::Wheel { seq, .. }
        | InputMessage::Key { seq, .. } => *seq,
    }
}

/// A wire coordinate as an engine point in the target tab's device pixels.
///
/// Finiteness was settled by the decoder, which refuses a coordinate that is
/// not a finite number rather than clamping it. Whether the point is *inside*
/// the target is settled by the delivery function, against that tab's own
/// viewport — never against the tab the local human happens to be displaying.
fn point(x: f64, y: f64) -> euclid::Point2D<f32, DevicePixel> {
    euclid::Point2D::new(x as f32, y as f32)
}

/// The engine button a wire button names.
///
/// The wire carries three buttons and no more: back and forward are history
/// navigation, which is the agent tool vocabulary the view channel deliberately
/// does not carry (`D-05-02`).
fn mouse_button(button: PointerButton) -> ServoMouseButton {
    match button {
        PointerButton::Left => ServoMouseButton::Left,
        PointerButton::Middle => ServoMouseButton::Middle,
        PointerButton::Right => ServoMouseButton::Right,
    }
}

/// The engine action a wire button action names.
fn button_action(action: ButtonAction) -> MouseButtonAction {
    match action {
        ButtonAction::Down => MouseButtonAction::Down,
        ButtonAction::Up => MouseButtonAction::Up,
    }
}

/// The engine key state a wire button action names.
fn key_state(action: ButtonAction) -> servo::KeyState {
    match action {
        ButtonAction::Down => servo::KeyState::Down,
        ButtonAction::Up => servo::KeyState::Up,
    }
}

/// The engine wheel mode a wire wheel mode names.
///
/// Two modes on the wire and two in the engine, and anything else never
/// decodes: [`InputMessage::from_json`] refuses a mode string it does not know
/// rather than falling through to a default, so an unrecognised mode is a
/// message that never becomes one.
fn wheel_mode(mode: WheelMode) -> ServoWheelMode {
    match mode {
        WheelMode::Line => ServoWheelMode::DeltaLine,
        WheelMode::Pixel => ServoWheelMode::DeltaPixel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use talaria_protocol::wire::Channel;

    use crate::view::testing::{attach, connect, control_request, FakeTabs, Viewer};
    use crate::view::{encode, Handled};

    fn moved(tab: u64, seq: u64) -> InputMessage {
        InputMessage::MouseMove { tab, seq, x: 40.0, y: 60.0 }
    }

    /// A viewer connected and attached to `tab`, with its sequence space fresh.
    fn attached(
        sessions: &mut ViewSessions,
        connection: u64,
        tab: u64,
        tabs: &dyn ViewTabs,
    ) -> Viewer {
        let viewer = connect(sessions, connection, "client-a");
        attach(sessions, &viewer, tab, tabs);
        viewer
    }

    #[test]
    fn input_on_a_connection_this_server_never_accepted_is_refused() {
        let tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        assert!(!admit(&mut sessions, &tabs, 99, &moved(1, 1)));
    }

    #[test]
    fn a_sequence_that_increased_is_accepted_and_recorded() {
        let tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = attached(&mut sessions, 1, 1, &tabs);
        assert!(admit(&mut sessions, &tabs, viewer.connection, &moved(1, 5)));
        assert!(admit(&mut sessions, &tabs, viewer.connection, &moved(1, 6)));
    }

    #[test]
    fn a_sequence_that_did_not_increase_is_dropped() {
        let tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = attached(&mut sessions, 1, 1, &tabs);
        assert!(admit(&mut sessions, &tabs, viewer.connection, &moved(1, 5)));
        // Equal, and lower. Both are replays within the connection.
        assert!(!admit(&mut sessions, &tabs, viewer.connection, &moved(1, 5)));
        assert!(!admit(&mut sessions, &tabs, viewer.connection, &moved(1, 4)));
        assert!(!admit(&mut sessions, &tabs, viewer.connection, &moved(1, 0)));
    }

    #[test]
    fn the_sequence_space_is_per_connection_and_two_viewers_do_not_interfere() {
        let tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let first = attached(&mut sessions, 1, 1, &tabs);
        let second = attached(&mut sessions, 2, 1, &tabs);

        assert!(admit(&mut sessions, &tabs, first.connection, &moved(1, 900)));
        // The second viewer's low sequence is its own space, not a replay of
        // the first's. A shared mark would let either starve the other.
        assert!(admit(&mut sessions, &tabs, second.connection, &moved(1, 1)));
    }

    #[test]
    fn a_message_refused_downstream_still_consumes_its_sequence_number() {
        // A number that could be reused is a message that could be replayed
        // later, once the state it was refused for has changed (T-05-15).
        let tabs = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = attached(&mut sessions, 1, 1, &tabs);

        // Refused: tab 2 is agent-owned but unattached.
        assert!(!admit(&mut sessions, &tabs, viewer.connection, &moved(2, 40)));
        // The same number is now spent even for the tab that would have worked.
        assert!(!admit(&mut sessions, &tabs, viewer.connection, &moved(1, 40)));
        assert!(admit(&mut sessions, &tabs, viewer.connection, &moved(1, 41)));
    }

    #[test]
    fn a_tab_this_connection_never_attached_to_is_refused() {
        let tabs = FakeTabs::with(&[(1, true), (2, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = attached(&mut sessions, 1, 1, &tabs);
        // Agent-owned, and still refused: attachment is what the concurrent
        // attachment cap is counted against, so input that skipped it would
        // skip the cap.
        assert!(!admit(&mut sessions, &tabs, viewer.connection, &moved(2, 1)));
    }

    #[test]
    fn a_tab_the_human_owns_is_refused() {
        // `D-05-02`. The Me tab is where the human's history rows, bookmarks
        // and autofilled credentials live.
        let tabs = FakeTabs::with(&[(1, true), (2, false)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        // It cannot even be attached to, so the refusal holds twice over: the
        // attach was refused, and the input is refused again here.
        let frame = control_request(&talaria_protocol::wire::ClientView::Attach { tab: 2 });
        assert!(matches!(sessions.message(viewer.connection, &frame, &tabs), Handled::Done));
        assert!(!admit(&mut sessions, &tabs, viewer.connection, &moved(2, 1)));
    }

    #[test]
    fn a_tab_that_does_not_exist_is_refused_the_same_way() {
        let tabs = FakeTabs::with(&[(1, true), (2, false)]);
        let mut sessions = ViewSessions::default();
        let viewer = connect(&mut sessions, 1, "client-a");
        let human = admit(&mut sessions, &tabs, viewer.connection, &moved(2, 1));
        let absent = admit(&mut sessions, &tabs, viewer.connection, &moved(99999, 2));
        assert!(!human);
        assert_eq!(
            human, absent,
            "a tab the human owns is distinguishable from a tab that never existed",
        );
    }

    #[test]
    fn admitting_input_answers_the_viewer_nothing_at_all() {
        // Every refusal is silent beyond the fact of not happening: the viewer
        // is told nothing, and so is told nothing that differs.
        let tabs = FakeTabs::with(&[(1, true), (2, false)]);
        let mut sessions = ViewSessions::default();
        let mut viewer = attached(&mut sessions, 1, 1, &tabs);
        let _ = viewer.drain();

        assert!(admit(&mut sessions, &tabs, viewer.connection, &moved(1, 1)));
        assert!(!admit(&mut sessions, &tabs, viewer.connection, &moved(2, 2)));
        assert!(!admit(&mut sessions, &tabs, viewer.connection, &moved(1, 1)));
        assert!(viewer.drain().is_empty(), "input produced an answer on the wire");
    }

    #[test]
    fn every_variant_targets_the_tab_it_names_and_carries_its_own_sequence() {
        let messages = [
            InputMessage::MouseMove { tab: 12, seq: 3, x: 1.0, y: 2.0 },
            InputMessage::MouseButton {
                tab: 12,
                seq: 3,
                x: 1.0,
                y: 2.0,
                button: PointerButton::Left,
                action: ButtonAction::Down,
            },
            InputMessage::Wheel {
                tab: 12,
                seq: 3,
                x: 1.0,
                y: 2.0,
                dx: 0.0,
                dy: -3.0,
                mode: WheelMode::Line,
            },
            InputMessage::Key {
                tab: 12,
                seq: 3,
                state: ButtonAction::Down,
                key: Some("a".into()),
                named: None,
            },
        ];
        for message in messages {
            assert_eq!(target_tab(&message), 12, "{message:?}");
            assert_eq!(sequence_of(&message), 3, "{message:?}");
        }
    }

    #[test]
    fn a_key_naming_a_name_this_build_does_not_know_is_refused() {
        assert!(keyutils::keyboard_event_from_wire(
            key_state(ButtonAction::Down),
            None,
            Some("Warp"),
        )
        .is_none());
    }

    #[test]
    fn a_key_naming_a_character_reaches_the_engine_as_that_character() {
        let event =
            keyutils::keyboard_event_from_wire(key_state(ButtonAction::Up), Some("q"), None)
                .expect("a character is a key");
        assert_eq!(event.event.key, servo::Key::Character("q".into()));
        assert_eq!(event.event.state, servo::KeyState::Up);
    }

    #[test]
    fn a_wheel_naming_a_delta_mode_this_wire_does_not_define_never_decodes() {
        // The refusal is the decoder's, and it is a refusal rather than a
        // fallthrough — which is why `wheel_mode` is total on two variants
        // instead of carrying a default arm somebody could widen.
        let payload = r#"{"kind":"wheel","tab":1,"seq":1,"x":1.0,"y":2.0,"dx":0.0,"dy":1.0,"mode":"page"}"#;
        assert_eq!(InputMessage::from_json(payload), None);
        assert_eq!(wheel_mode(WheelMode::Line), ServoWheelMode::DeltaLine);
        assert_eq!(wheel_mode(WheelMode::Pixel), ServoWheelMode::DeltaPixel);
    }

    #[test]
    fn every_pointer_button_maps_to_its_engine_button_and_no_other() {
        assert_eq!(mouse_button(PointerButton::Left), ServoMouseButton::Left);
        assert_eq!(mouse_button(PointerButton::Middle), ServoMouseButton::Middle);
        assert_eq!(mouse_button(PointerButton::Right), ServoMouseButton::Right);
    }

    #[test]
    fn a_button_action_maps_to_its_engine_action_on_both_the_pointer_and_the_key() {
        assert_eq!(button_action(ButtonAction::Down), MouseButtonAction::Down);
        assert_eq!(button_action(ButtonAction::Up), MouseButtonAction::Up);
        assert_eq!(key_state(ButtonAction::Down), servo::KeyState::Down);
        assert_eq!(key_state(ButtonAction::Up), servo::KeyState::Up);
    }

    #[test]
    fn a_wire_coordinate_is_carried_through_untouched() {
        // No toolbar offset, no rounding to a grid, no clamp: the point that
        // arrives is the point that is tested against the target's viewport.
        let converted = point(412.0, 260.0);
        assert_eq!(converted.x, 412.0);
        assert_eq!(converted.y, 260.0);
    }

    #[test]
    fn an_input_frame_reaches_this_module_and_is_delivered_nowhere_before_it() {
        // The wiring assertion: `ViewSessions` decodes and hands back, and
        // acting on the message is this module's alone.
        let tabs = FakeTabs::with(&[(1, true)]);
        let mut sessions = ViewSessions::default();
        let viewer = attached(&mut sessions, 1, 1, &tabs);
        let frame = encode(
            Channel::Input,
            moved(1, 4).to_json().expect("well formed").as_bytes(),
        );
        match sessions.message(viewer.connection, &frame, &tabs) {
            Handled::Input(message) => {
                assert!(admit(&mut sessions, &tabs, viewer.connection, &message));
            },
            _ => panic!("an input frame was not handed back"),
        }
    }
}

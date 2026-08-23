//! Keyboard conversion for both of Talaria's input sources.
//!
//! Two mappings live here — one from the winit key event the local human's
//! keyboard produces, one from the wire message a remote viewer sends
//! ([`crate::remote_input`]) — and they read **one** table. Two tables would
//! drift, and the drift would be invisible until a key worked on one path and
//! stopped working on the other.
//!
//! **A window-system key event is deliberately not what travels on the wire.**
//! Not every field of [`winit::event::KeyEvent`] is constructible outside
//! winit, and forging one would be a lie about where the keystroke came from.
//! The wire carries a state plus either a character or a named key, and a name
//! this build does not know is a refusal — the same failure mode the local
//! path already has for a winit key it does not recognise.
//!
//! Covers character input and the named keys that matter for browsing.
//! (servoshell carries a full W3C mapping table; we start with the useful
//! subset and grow it as gaps surface.)

use servo::{Key, KeyState, KeyboardEvent, NamedKey};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key as WinitKey, NamedKey as WinitNamedKey};

/// The engine key one row of [`NAMED_KEYS`] names.
///
/// Two shapes rather than one, because `Space` is a **character** and not a
/// named key: `keyboard_types::NamedKey` says so in its own doc comment ("use
/// `Key::Character(" ")` instead"), and the local path has always honoured it.
/// Keeping the exception inside the table is what makes the space key behave
/// identically on both paths without either mapping special-casing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Engine {
    Named(NamedKey),
    Character(&'static str),
}

impl Engine {
    /// This row's engine key. Allocates, which is why the table holds the
    /// borrowed form and this is a method rather than a constant.
    fn key(self) -> Key {
        match self {
            Engine::Named(named) => Key::Named(named),
            Engine::Character(text) => Key::Character(text.to_owned()),
        }
    }
}

/// Every key that has a name: the wire's spelling of it, the winit key that
/// produces it locally, and the engine key both map to.
///
/// **One table, read twice.** [`name_from_winit`] reads the middle column to
/// find a row and [`key_from_name`] reads the first, so a key added here works
/// on both paths at once and a key missing here works on neither. An earlier
/// shape of this file wrote the winit arm list out by hand; a second hand-written
/// list for the wire would have drifted from it the first time somebody added a
/// key to one and not the other, and nothing would have failed until a viewer
/// pressed it.
///
/// A name is spelled the way the W3C UI Events key values spell it, which is
/// also how `keyboard_types::NamedKey`'s variants are spelled — so a client
/// author reading the specification writes the right string without consulting
/// this table.
const NAMED_KEYS: &[(&str, WinitNamedKey, Engine)] = &[
    ("Enter", WinitNamedKey::Enter, Engine::Named(NamedKey::Enter)),
    ("Tab", WinitNamedKey::Tab, Engine::Named(NamedKey::Tab)),
    ("Backspace", WinitNamedKey::Backspace, Engine::Named(NamedKey::Backspace)),
    ("Delete", WinitNamedKey::Delete, Engine::Named(NamedKey::Delete)),
    ("Escape", WinitNamedKey::Escape, Engine::Named(NamedKey::Escape)),
    ("ArrowUp", WinitNamedKey::ArrowUp, Engine::Named(NamedKey::ArrowUp)),
    ("ArrowDown", WinitNamedKey::ArrowDown, Engine::Named(NamedKey::ArrowDown)),
    ("ArrowLeft", WinitNamedKey::ArrowLeft, Engine::Named(NamedKey::ArrowLeft)),
    ("ArrowRight", WinitNamedKey::ArrowRight, Engine::Named(NamedKey::ArrowRight)),
    ("Home", WinitNamedKey::Home, Engine::Named(NamedKey::Home)),
    ("End", WinitNamedKey::End, Engine::Named(NamedKey::End)),
    ("PageUp", WinitNamedKey::PageUp, Engine::Named(NamedKey::PageUp)),
    ("PageDown", WinitNamedKey::PageDown, Engine::Named(NamedKey::PageDown)),
    ("Shift", WinitNamedKey::Shift, Engine::Named(NamedKey::Shift)),
    ("Control", WinitNamedKey::Control, Engine::Named(NamedKey::Control)),
    ("Alt", WinitNamedKey::Alt, Engine::Named(NamedKey::Alt)),
    ("Meta", WinitNamedKey::Meta, Engine::Named(NamedKey::Meta)),
    ("CapsLock", WinitNamedKey::CapsLock, Engine::Named(NamedKey::CapsLock)),
    ("ContextMenu", WinitNamedKey::ContextMenu, Engine::Named(NamedKey::ContextMenu)),
    ("Insert", WinitNamedKey::Insert, Engine::Named(NamedKey::Insert)),
    ("F1", WinitNamedKey::F1, Engine::Named(NamedKey::F1)),
    ("F2", WinitNamedKey::F2, Engine::Named(NamedKey::F2)),
    ("F3", WinitNamedKey::F3, Engine::Named(NamedKey::F3)),
    ("F4", WinitNamedKey::F4, Engine::Named(NamedKey::F4)),
    ("F5", WinitNamedKey::F5, Engine::Named(NamedKey::F5)),
    ("F6", WinitNamedKey::F6, Engine::Named(NamedKey::F6)),
    ("F7", WinitNamedKey::F7, Engine::Named(NamedKey::F7)),
    ("F8", WinitNamedKey::F8, Engine::Named(NamedKey::F8)),
    ("F9", WinitNamedKey::F9, Engine::Named(NamedKey::F9)),
    ("F10", WinitNamedKey::F10, Engine::Named(NamedKey::F10)),
    ("F11", WinitNamedKey::F11, Engine::Named(NamedKey::F11)),
    ("F12", WinitNamedKey::F12, Engine::Named(NamedKey::F12)),
    // The one row whose engine key is a character rather than a named key.
    ("Space", WinitNamedKey::Space, Engine::Character(" ")),
];

/// The name [`NAMED_KEYS`] gives a winit named key, or `None` for one this
/// build does not carry.
fn name_from_winit(named: &WinitNamedKey) -> Option<&'static str> {
    // winit distinguishes Super from Meta; the engine does not, and the local
    // path has folded them together since this file existed. The fold lives
    // here rather than as a second `("Meta", Super, …)` row, because a second
    // row spelled "Meta" would make the *name* lookup ambiguous — and the name
    // lookup is the direction a remote viewer uses.
    if matches!(named, WinitNamedKey::Super) {
        return Some("Meta");
    }
    NAMED_KEYS.iter().find(|entry| entry.1 == *named).map(|entry| entry.0)
}

/// The engine key a wire name spells, or `None` for a name this build does not
/// know.
///
/// `None` and never a substitution: a viewer naming a key that is not in the
/// table gets nothing at all, because guessing at "the nearest key" is how a
/// typo becomes a keystroke somebody did not send.
fn key_from_name(name: &str) -> Option<Key> {
    NAMED_KEYS.iter().find(|entry| entry.0 == name).map(|entry| entry.2.key())
}

/// One winit key event into an engine keyboard event — the **local** human's
/// keyboard.
pub fn keyboard_event_from_winit(event: &KeyEvent) -> Option<KeyboardEvent> {
    let state = match event.state {
        ElementState::Pressed => KeyState::Down,
        ElementState::Released => KeyState::Up,
    };

    let key = match &event.logical_key {
        WinitKey::Character(text) => Key::Character(text.to_string()),
        WinitKey::Named(named) => key_from_name(name_from_winit(named)?)?,
        _ => return None,
    };

    Some(KeyboardEvent::from_state_and_key(state, key))
}

/// One wire key message into an engine keyboard event — a **remote** viewer's
/// keyboard, arriving through [`crate::remote_input`].
///
/// Exactly one of `character` and `named` may be present. Both together is a
/// message that means two things and neither is a message that means nothing;
/// the wire's own decoder already refuses that pair
/// ([`talaria_protocol::wire::InputMessage::from_json`]), and it is refused
/// again here so this function is total on its own arguments rather than
/// correct only when its caller was.
///
/// An empty character is refused too: it is a keystroke that types nothing,
/// and delivering it would be substituting a value for a message that carried
/// none.
pub fn keyboard_event_from_wire(
    state: KeyState,
    character: Option<&str>,
    named: Option<&str>,
) -> Option<KeyboardEvent> {
    let key = match (character, named) {
        (Some(text), None) if !text.is_empty() => Key::Character(text.to_owned()),
        (None, Some(name)) => key_from_name(name)?,
        _ => return None,
    };

    Some(KeyboardEvent::from_state_and_key(state, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_mappings_agree_on_every_name_in_the_shared_table() {
        // The test that makes "the two mappings cannot drift" true rather than
        // intended. It walks the table rather than naming keys, so a row added
        // later is covered the moment it is added.
        //
        // It asserts at the level of the two lookups rather than by feeding a
        // `winit::event::KeyEvent` through `keyboard_event_from_winit`, because
        // that type is not constructible here — which is the same fact that
        // keeps it off the wire.
        for (name, winit, engine) in NAMED_KEYS {
            assert_eq!(
                name_from_winit(winit),
                Some(*name),
                "the winit key for {name} does not resolve back to that name",
            );
            assert_eq!(
                key_from_name(name),
                Some(engine.key()),
                "the wire name {name} does not map to the engine key its row names",
            );
        }
    }

    #[test]
    fn a_wire_key_naming_a_character_produces_a_character_key() {
        let event = keyboard_event_from_wire(KeyState::Down, Some("a"), None)
            .expect("a character is a key");
        assert_eq!(event.event.key, Key::Character("a".to_owned()));
        assert_eq!(event.event.state, KeyState::Down);
    }

    #[test]
    fn a_wire_key_naming_a_recognised_named_key_produces_that_named_key() {
        let event = keyboard_event_from_wire(KeyState::Down, None, Some("Enter"))
            .expect("Enter is in the table");
        assert_eq!(event.event.key, Key::Named(NamedKey::Enter));
    }

    #[test]
    fn the_space_key_is_a_character_on_both_paths() {
        // keyboard-types' own rule, and the one row of the table whose engine
        // key is not a named key. Both mappings reach it through that row, so
        // neither has to know about the exception.
        let event = keyboard_event_from_wire(KeyState::Down, None, Some("Space"))
            .expect("Space is in the table");
        assert_eq!(event.event.key, Key::Character(" ".to_owned()));
        assert_eq!(name_from_winit(&WinitNamedKey::Space), Some("Space"));
    }

    #[test]
    fn a_wire_key_naming_nothing_recognised_produces_nothing() {
        assert!(keyboard_event_from_wire(KeyState::Down, None, Some("Warp")).is_none());
        assert!(keyboard_event_from_wire(KeyState::Down, None, Some("enter")).is_none());
        assert!(keyboard_event_from_wire(KeyState::Down, None, Some("")).is_none());
    }

    #[test]
    fn a_wire_key_naming_both_a_character_and_a_named_key_produces_nothing() {
        assert!(keyboard_event_from_wire(KeyState::Down, Some("a"), Some("Enter")).is_none());
    }

    #[test]
    fn a_wire_key_naming_neither_produces_nothing() {
        assert!(keyboard_event_from_wire(KeyState::Down, None, None).is_none());
    }

    #[test]
    fn an_empty_character_is_refused_rather_than_delivered_as_a_key_that_types_nothing() {
        assert!(keyboard_event_from_wire(KeyState::Down, Some(""), None).is_none());
    }

    #[test]
    fn a_key_state_travels_through_unchanged() {
        for state in [KeyState::Down, KeyState::Up] {
            let event = keyboard_event_from_wire(state, Some("z"), None).expect("a character");
            assert_eq!(event.event.state, state);
        }
    }

    #[test]
    fn winit_folds_super_onto_meta_the_way_it_always_has() {
        assert_eq!(name_from_winit(&WinitNamedKey::Super), Some("Meta"));
        assert_eq!(name_from_winit(&WinitNamedKey::Meta), Some("Meta"));
        assert_eq!(key_from_name("Meta"), Some(Key::Named(NamedKey::Meta)));
    }

    #[test]
    fn a_winit_named_key_outside_the_table_resolves_to_no_name() {
        assert_eq!(name_from_winit(&WinitNamedKey::BrowserFavorites), None);
    }
}

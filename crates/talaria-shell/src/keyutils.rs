//! Minimal winit → servo keyboard event conversion.
//!
//! Covers character input and the named keys that matter for browsing.
//! (servoshell carries a full W3C mapping table; we start with the useful
//! subset and grow it as gaps surface.)

use servo::{Key, KeyState, KeyboardEvent, NamedKey};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key as WinitKey, NamedKey as WinitNamedKey};

pub fn keyboard_event_from_winit(event: &KeyEvent) -> Option<KeyboardEvent> {
    let state = match event.state {
        ElementState::Pressed => KeyState::Down,
        ElementState::Released => KeyState::Up,
    };

    let key = match &event.logical_key {
        WinitKey::Character(text) => Key::Character(text.to_string()),
        WinitKey::Named(named) => Key::Named(match named {
            WinitNamedKey::Enter => NamedKey::Enter,
            WinitNamedKey::Tab => NamedKey::Tab,
            WinitNamedKey::Backspace => NamedKey::Backspace,
            WinitNamedKey::Delete => NamedKey::Delete,
            WinitNamedKey::Escape => NamedKey::Escape,
            WinitNamedKey::ArrowUp => NamedKey::ArrowUp,
            WinitNamedKey::ArrowDown => NamedKey::ArrowDown,
            WinitNamedKey::ArrowLeft => NamedKey::ArrowLeft,
            WinitNamedKey::ArrowRight => NamedKey::ArrowRight,
            WinitNamedKey::Home => NamedKey::Home,
            WinitNamedKey::End => NamedKey::End,
            WinitNamedKey::PageUp => NamedKey::PageUp,
            WinitNamedKey::PageDown => NamedKey::PageDown,
            WinitNamedKey::Shift => NamedKey::Shift,
            WinitNamedKey::Control => NamedKey::Control,
            WinitNamedKey::Alt => NamedKey::Alt,
            WinitNamedKey::Super | WinitNamedKey::Meta => NamedKey::Meta,
            WinitNamedKey::CapsLock => NamedKey::CapsLock,
            WinitNamedKey::ContextMenu => NamedKey::ContextMenu,
            WinitNamedKey::Insert => NamedKey::Insert,
            WinitNamedKey::F1 => NamedKey::F1,
            WinitNamedKey::F2 => NamedKey::F2,
            WinitNamedKey::F3 => NamedKey::F3,
            WinitNamedKey::F4 => NamedKey::F4,
            WinitNamedKey::F5 => NamedKey::F5,
            WinitNamedKey::F6 => NamedKey::F6,
            WinitNamedKey::F7 => NamedKey::F7,
            WinitNamedKey::F8 => NamedKey::F8,
            WinitNamedKey::F9 => NamedKey::F9,
            WinitNamedKey::F10 => NamedKey::F10,
            WinitNamedKey::F11 => NamedKey::F11,
            WinitNamedKey::F12 => NamedKey::F12,
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

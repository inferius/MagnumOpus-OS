//! Udalosti od inputd -> klavesy terminalu.

use inputproto::code::*;
use inputproto::{Action, Event, Record, mods};
use term::ui::Key;

/// Klavesa z udalosti; `None` = udalost terminal nezajima (uvolneni,
/// pohyb mysi, fokus).
pub fn key(r: &Record) -> Option<Key> {
    match r.event {
        // Text z rozlozeni (vcetne diakritiky); pri Ctrl/Alt se neposila.
        Event::Text(c) => Some(Key::Char(c)),
        Event::Key { code, action: Action::Press | Action::Repeat, ch, .. } => {
            let shift = r.mods & mods::SHIFT != 0;
            if r.mods & mods::CTRL != 0 {
                return ch.filter(|c| c.is_ascii_alphabetic()).map(|c| Key::Ctrl(c.to_ascii_lowercase()));
            }
            Some(match code {
                KEY_ENTER | KEY_KPENTER => Key::Enter,
                KEY_BACKSPACE => Key::Backspace,
                KEY_DELETE => Key::Delete,
                KEY_TAB if shift => Key::BackTab,
                KEY_TAB => Key::Tab,
                KEY_ESC => Key::Esc,
                KEY_LEFT => Key::Left,
                KEY_RIGHT => Key::Right,
                KEY_UP => Key::Up,
                KEY_DOWN => Key::Down,
                KEY_HOME => Key::Home,
                KEY_END => Key::End,
                KEY_PAGEUP if shift => Key::ScrollPageUp,
                KEY_PAGEDOWN if shift => Key::ScrollPageDown,
                KEY_PAGEUP => Key::PageUp,
                KEY_PAGEDOWN => Key::PageDown,
                _ => return None,
            })
        }
        _ => None,
    }
}

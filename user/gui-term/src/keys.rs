//! Vstup okna (kompozitor) -> klavesy terminalu (`term::ui`).
//!
//! Stejne mapovani jako zalozni konzole (`user/console/src/keys.rs`),
//! jen nad zaznamy vstupu kompozitoru.

use guiproto::window::{InputEvent, InputRecord};
use inputproto::code::*;
use inputproto::mods;
use term::ui::Key;

/// Klavesa ze zaznamu; `None` = udalost terminal nezajima.
pub fn key(r: &InputRecord) -> Option<Key> {
    match r.event {
        // Text z rozlozeni (vcetne diakritiky); pri Ctrl se neposila.
        InputEvent::Text(c) if r.mods & mods::CTRL == 0 && !c.is_control() => Some(Key::Char(c)),
        InputEvent::Key { code, action: 1 | 2, ch } => {
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
                // Alt+sipky: zpet / vpred v historii slozek (`cd -` / `cd +`).
                KEY_LEFT if r.mods & mods::ALT != 0 => Key::AltLeft,
                KEY_RIGHT if r.mods & mods::ALT != 0 => Key::AltRight,
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
        // Kolecko: posun vystupu.
        InputEvent::Scroll { dy, .. } if dy != 0 => Some(Key::Wheel(dy)),
        _ => None,
    }
}

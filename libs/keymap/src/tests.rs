//! Testy rozlozeni (na hostiteli: `cargo test -p keymap`).

use inputproto::code::*;
use inputproto::{Action, mods};

use crate::{Keyboard, LayoutId, Symbol, compose, layout};

/// Napise posloupnost klaves (stisk a uvolneni) a vrati vlozeny text.
fn typed(id: LayoutId, keys: &[u16]) -> String {
    let l = layout(id);
    let mut kb = Keyboard::new();
    let mut out = String::new();
    for &k in keys {
        let o = kb.key(l, k, Action::Press);
        out.extend(o.text.iter().flatten());
        kb.key(l, k, Action::Release);
    }
    out
}

/// Stisk s drzenym modifikatorem.
fn with(id: LayoutId, modifier: u16, key: u16) -> (Option<char>, String, u16) {
    let l = layout(id);
    let mut kb = Keyboard::new();
    kb.key(l, modifier, Action::Press);
    let o = kb.key(l, key, Action::Press);
    (o.ch, o.text.iter().flatten().collect(), o.mods)
}

#[test]
fn us_basic() {
    assert_eq!(
        typed(LayoutId::Us, &[KEY_H, KEY_E, KEY_L, KEY_L, KEY_O, KEY_SPACE, KEY_1]),
        "hello 1"
    );
    assert_eq!(with(LayoutId::Us, KEY_LEFTSHIFT, KEY_1).1, "!");
    assert_eq!(with(LayoutId::Us, KEY_RIGHTSHIFT, KEY_A).1, "A");
    assert_eq!(with(LayoutId::Us, KEY_LEFTSHIFT, KEY_SLASH).1, "?");
}

#[test]
fn cz_qwertz_and_number_row() {
    // Y a Z jsou prohozene, horni rada pise diakritiku.
    assert_eq!(typed(LayoutId::Cz, &[KEY_Y, KEY_Z]), "zy");
    assert_eq!(
        typed(
            LayoutId::Cz,
            &[KEY_2, KEY_3, KEY_4, KEY_5, KEY_6, KEY_7, KEY_8, KEY_9, KEY_0]
        ),
        "ěščřžýáíé"
    );
    assert_eq!(typed(LayoutId::Cz, &[KEY_SEMICOLON, KEY_LEFTBRACE]), "ůú");
    assert_eq!(with(LayoutId::Cz, KEY_LEFTSHIFT, KEY_2).1, "2");
    assert_eq!(with(LayoutId::Cz, KEY_LEFTSHIFT, KEY_SLASH).1, "_");
    assert_eq!(typed(LayoutId::Cz, &[KEY_1, KEY_MINUS]), "+=");
}

#[test]
fn cz_altgr() {
    assert_eq!(with(LayoutId::Cz, KEY_RIGHTALT, KEY_V).1, "@");
    assert_eq!(with(LayoutId::Cz, KEY_RIGHTALT, KEY_E).1, "€");
    assert_eq!(with(LayoutId::Cz, KEY_RIGHTALT, KEY_B).1, "{");
    assert_eq!(with(LayoutId::Cz, KEY_RIGHTALT, KEY_Q).1, "\\");
    let (_, _, m) = with(LayoutId::Cz, KEY_RIGHTALT, KEY_V);
    assert_eq!(m & (mods::ALTGR | mods::ALT), mods::ALTGR);
    // V US je pravy Alt obycejny Alt: zkratka, zadny text.
    let (ch, text, m) = with(LayoutId::Us, KEY_RIGHTALT, KEY_V);
    assert_eq!((ch, text.as_str()), (Some('v'), ""));
    assert_eq!(m & (mods::ALTGR | mods::ALT), mods::ALT);
}

#[test]
fn cz_dead_keys() {
    let l = layout(LayoutId::Cz);
    let mut kb = Keyboard::new();
    let mut text = String::new();
    let mut press = |kb: &mut Keyboard, k| {
        let o = kb.key(l, k, Action::Press);
        text.extend(o.text.iter().flatten());
        kb.key(l, k, Action::Release);
        o
    };
    // Carka + e = e s carkou; mrtva klavesa sama nema znak.
    let o = press(&mut kb, KEY_EQUAL);
    assert_eq!(o.ch, None);
    assert_eq!(kb.dead_pending(), Some('´'));
    press(&mut kb, KEY_E);
    // Hacek (Shift) + c.
    kb.key(l, KEY_LEFTSHIFT, Action::Press);
    press(&mut kb, KEY_EQUAL);
    kb.key(l, KEY_LEFTSHIFT, Action::Release);
    press(&mut kb, KEY_C);
    // Carka + mezera = samotna carka.
    press(&mut kb, KEY_EQUAL);
    press(&mut kb, KEY_SPACE);
    // Carka + x (nejde slozit) = obe.
    press(&mut kb, KEY_EQUAL);
    press(&mut kb, KEY_X);
    // Dvakrat carka = carka.
    press(&mut kb, KEY_EQUAL);
    press(&mut kb, KEY_EQUAL);
    // Prehlaska + u, Shift az po mrtve klavese: velke U s prehlaskou.
    press(&mut kb, KEY_BACKSLASH);
    kb.key(l, KEY_LEFTSHIFT, Action::Press);
    press(&mut kb, KEY_U);
    kb.key(l, KEY_LEFTSHIFT, Action::Release);
    assert_eq!(text, "éč´´x´Ü");
}

#[test]
fn dead_key_cancelled_by_shortcut_and_enter() {
    let l = layout(LayoutId::Cz);
    let mut kb = Keyboard::new();
    kb.key(l, KEY_EQUAL, Action::Press);
    kb.key(l, KEY_EQUAL, Action::Release);
    kb.key(l, KEY_ENTER, Action::Press);
    assert_eq!(kb.dead_pending(), None);
    kb.key(l, KEY_EQUAL, Action::Press);
    kb.key(l, KEY_LEFTCTRL, Action::Press);
    let o = kb.key(l, KEY_C, Action::Press);
    assert_eq!(o.text, [None, None]);
    assert_eq!(o.ch, Some('c'));
    assert_eq!(kb.dead_pending(), None);
}

#[test]
fn ctrl_gives_char_but_no_text() {
    // Ctrl+Z v CZ: znak 'z' na klavese KEY_Y (zkratky podle rozlozeni).
    let (ch, text, m) = with(LayoutId::Cz, KEY_LEFTCTRL, KEY_Y);
    assert_eq!((ch, text.as_str()), (Some('z'), ""));
    assert!(m & mods::CTRL != 0);
}

#[test]
fn caps_lock() {
    let l = layout(LayoutId::Cz);
    let mut kb = Keyboard::new();
    kb.key(l, KEY_CAPSLOCK, Action::Press);
    kb.key(l, KEY_CAPSLOCK, Action::Release);
    assert!(kb.mods(l) & mods::CAPS_LOCK != 0);
    assert_eq!(kb.key(l, KEY_A, Action::Press).text[0], Some('A'));
    // Ceska horni rada: velke pismeno, se Shiftem cislice.
    assert_eq!(kb.key(l, KEY_2, Action::Press).text[0], Some('Ě'));
    kb.key(l, KEY_LEFTSHIFT, Action::Press);
    assert_eq!(kb.key(l, KEY_A, Action::Press).text[0], Some('a'));
    assert_eq!(kb.key(l, KEY_2, Action::Press).text[0], Some('2'));
    kb.key(l, KEY_LEFTSHIFT, Action::Release);
    // Znaky, ktere nejsou pismena, Caps Lock nemeni.
    assert_eq!(kb.key(l, KEY_MINUS, Action::Press).text[0], Some('='));
    // Druhe stisknuti vypne.
    kb.key(l, KEY_CAPSLOCK, Action::Press);
    assert_eq!(kb.key(l, KEY_A, Action::Press).text[0], Some('a'));
}

#[test]
fn keypad_and_num_lock() {
    let l = layout(LayoutId::Cz);
    let mut kb = Keyboard::new();
    assert_eq!(kb.key(l, KEY_KP1, Action::Press).text[0], None);
    assert_eq!(kb.key(l, KEY_KPPLUS, Action::Press).text[0], Some('+'));
    kb.key(l, KEY_NUMLOCK, Action::Press);
    assert_eq!(kb.key(l, KEY_KP1, Action::Press).text[0], Some('1'));
    assert_eq!(kb.key(l, KEY_KPDOT, Action::Press).text[0], Some(','));
    let us = layout(LayoutId::Us);
    assert_eq!(kb.key(us, KEY_KPDOT, Action::Press).text[0], Some('.'));
}

#[test]
fn non_text_keys() {
    let l = layout(LayoutId::Us);
    let mut kb = Keyboard::new();
    for k in [KEY_ENTER, KEY_TAB, KEY_BACKSPACE, KEY_ESC, KEY_UP, KEY_F1, KEY_DELETE] {
        let o = kb.key(l, k, Action::Press);
        assert_eq!((o.ch, o.text), (None, [None, None]), "klavesa {k}");
    }
}

#[test]
fn repeat_writes_again_but_not_dead() {
    let l = layout(LayoutId::Cz);
    let mut kb = Keyboard::new();
    kb.key(l, KEY_A, Action::Press);
    assert_eq!(kb.key(l, KEY_A, Action::Repeat).text[0], Some('a'));
    kb.key(l, KEY_EQUAL, Action::Press);
    assert_eq!(kb.key(l, KEY_EQUAL, Action::Repeat).text, [None, None]);
    assert_eq!(kb.dead_pending(), Some('´'));
}

#[test]
fn ctrl_alt_and_reset() {
    let l = layout(LayoutId::Cz);
    let mut kb = Keyboard::new();
    kb.key(l, KEY_LEFTCTRL, Action::Press);
    assert!(!kb.ctrl_alt_held());
    kb.key(l, KEY_RIGHTALT, Action::Press);
    assert!(kb.ctrl_alt_held());
    kb.key(l, KEY_CAPSLOCK, Action::Press);
    kb.reset();
    assert!(!kb.ctrl_alt_held());
    assert_eq!(kb.mods(l), mods::CAPS_LOCK);
}

#[test]
fn layout_names_and_tables() {
    for id in LayoutId::ALL {
        assert_eq!(LayoutId::from_name(id.name()), Some(id));
        assert_eq!(LayoutId::from_u32(id as u32), Some(id));
        assert_eq!(layout(id).id, id);
        // Kazde pismeno anglicke abecedy je v kazdem rozlozeni.
        let mut letters: Vec<char> = (KEY_Q..=KEY_M)
            .flat_map(|k| match layout(id).symbol(k, 0) {
                Symbol::Char(c) if c.is_ascii_lowercase() => Some(c),
                _ => None,
            })
            .collect();
        letters.sort();
        letters.dedup();
        assert_eq!(letters.len(), 26, "rozlozeni {}", id.name());
    }
    assert_eq!(LayoutId::from_name("de"), None);
}

#[test]
fn compose_table() {
    assert_eq!(compose('ˇ', 'r'), Some('ř'));
    assert_eq!(compose('ˇ', 'U'), None);
    assert_eq!(compose('´', 'U'), Some('Ú'));
    assert_eq!(compose('x', 'a'), None);
}

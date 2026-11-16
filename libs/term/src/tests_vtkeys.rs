//! Vstup z terminalu VT100/xterm: sekvence klaves, rozdeleni do paketu,
//! UTF-8 (cestina), samotne Esc az po pauze.

use alloc::vec;

use crate::ui::Key;
use crate::vtkeys::VtKeys;

fn keys(data: &[u8]) -> alloc::vec::Vec<Key> {
    VtKeys::new().feed(data)
}

#[test]
fn znaky_a_ridici_klavesy() {
    assert_eq!(keys(b"ls\r"), vec![Key::Char('l'), Key::Char('s'), Key::Enter]);
    assert_eq!(keys(b"a\r\nb"), vec![Key::Char('a'), Key::Enter, Key::Char('b')]);
    assert_eq!(keys(b"\x7f\x08\t"), vec![Key::Backspace, Key::Backspace, Key::Tab]);
    assert_eq!(keys(b"\x03\x1a\x04"), vec![Key::Ctrl('c'), Key::Ctrl('z'), Key::Ctrl('d')]);
    assert_eq!(keys("čeština ❯".as_bytes()).len(), 9);
    assert_eq!(keys("ř".as_bytes()), vec![Key::Char('ř')]);
}

#[test]
fn sekvence_kurzoru_a_editace() {
    let cases: &[(&[u8], Key)] = &[
        (b"\x1b[A", Key::Up),
        (b"\x1b[B", Key::Down),
        (b"\x1b[C", Key::Right),
        (b"\x1b[D", Key::Left),
        (b"\x1bOA", Key::Up),
        (b"\x1b[H", Key::Home),
        (b"\x1b[F", Key::End),
        (b"\x1b[1~", Key::Home),
        (b"\x1b[4~", Key::End),
        (b"\x1b[3~", Key::Delete),
        (b"\x1b[5~", Key::PageUp),
        (b"\x1b[6~", Key::PageDown),
        (b"\x1b[5;2~", Key::ScrollPageUp),
        (b"\x1b[6;2~", Key::ScrollPageDown),
        (b"\x1b[Z", Key::BackTab),
        (b"\x1b[1;3D", Key::AltLeft),
        (b"\x1b[1;3C", Key::AltRight),
        (b"\x1bb", Key::AltLeft),
    ];
    for (seq, k) in cases {
        assert_eq!(keys(seq), vec![k.clone()], "{seq:?}");
    }
    // Neznama sekvence se zahodi cela, nic z ni neprijde jako text.
    assert_eq!(keys(b"\x1b[200~x"), vec![Key::Char('x')]);
}

#[test]
fn rozdeleni_do_paketu_a_esc() {
    let mut v = VtKeys::new();
    assert!(v.feed(b"\x1b").is_empty());
    assert!(v.has_pending());
    assert!(v.feed(b"[").is_empty());
    assert_eq!(v.feed(b"A"), vec![Key::Up]);
    // Rozdeleny znak UTF-8.
    let r = "ř".as_bytes();
    assert!(v.feed(&r[..1]).is_empty());
    assert_eq!(v.feed(&r[1..]), vec![Key::Char('ř')]);
    // Samotne Esc az po pauze.
    assert!(v.feed(b"\x1b").is_empty());
    assert_eq!(v.flush(), vec![Key::Esc]);
    assert!(!v.has_pending());
    // Esc Esc: prvni hned, druhe po pauze.
    assert_eq!(v.feed(b"\x1b\x1b"), vec![Key::Esc]);
    assert_eq!(v.flush(), vec![Key::Esc]);
}

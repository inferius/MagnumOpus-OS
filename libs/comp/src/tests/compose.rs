//! Rozpracovany znak mrtve klavesy (`inputproto` `Compose`): jen oknu
//! s fokusem jako `ServerMsg::Compose` (podtrzeny segment, kurzor na
//! konci), umely vstup ne; slozeny znak dal jako `Text`.

use super::*;

#[test]
fn compose_goes_only_to_focused_window() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let b = t.client("b");
    t.window(b, Mode::Standard, BLUE);
    t.frame();
    // Fokus ma okno b (nastaveno primo, poradi zobrazeni neresime).
    let gb = t.c.clients[&b].windows[&1];
    t.c.set_focus(Some(gb));
    t.out();
    t.input(Event::Compose(Some('\u{02C7}')), false);
    let out = t.out();
    assert!(client_msgs(&out, a).is_empty(), "okno bez fokusu nic");
    assert_eq!(
        client_msgs(&out, b),
        [ServerMsg::Compose { window: 1, text: "\u{02C7}".into(), cursor_begin: 2, cursor_end: 2, flags: guiproto::window::compose::UNDERLINE }]
    );
    // Konec a slozeny znak.
    t.input(Event::Compose(None), false);
    t.input(Event::Text('č'), false);
    let m = client_msgs(&t.out(), b);
    assert_eq!(m[0], ServerMsg::Compose { window: 1, text: String::new(), cursor_begin: 0, cursor_end: 0, flags: 0 });
    assert!(matches!(&m[1], ServerMsg::Input { records, .. } if records[0].event == InputEvent::Text('č')));
    // Umely vstup rozpracovany znak nedoruci.
    t.input(Event::Compose(Some('\u{00B4}')), true);
    assert!(client_msgs(&t.out(), b).is_empty());
}

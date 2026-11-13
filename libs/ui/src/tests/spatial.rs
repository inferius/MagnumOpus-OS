//! Prostorova navigace fokusu: sipky, skupiny s pameti, vychozi fokus.

use alloc::string::String;
use alloc::vec::Vec;

use guiproto::a11y::state;
use guiproto::window::InputEvent;
use inputproto::code::*;

use crate::*;

fn tv() -> Node<()> {
    let tile = |s: &str| button::<()>(s).width(120).key(s);
    column([
        row([tile("A1"), tile("A2"), tile("A3"), spacer()]).focus_group(Axis::Horizontal).key("rada A"),
        row([tile("B1"), tile("B2"), tile("B3"), spacer()]).focus_group(Axis::Horizontal).key("rada B"),
        button("Nastavení").default_focus().key("nast"),
    ])
}

fn focused(u: &Ui<()>) -> String {
    u.a11y().into_iter().find(|n| n.state & state::FOCUSED != 0).map(|n| n.name).unwrap_or_default()
}

fn arrow(u: &mut Ui<()>, code: u16) -> Option<(u16, Option<char>, u16)> {
    let r = u.event(InputEvent::Key { code, action: 1, ch: None }, 0);
    u.event(InputEvent::Key { code, action: 0, ch: None }, 0);
    r.unhandled_key
}

#[test]
fn spatial_focus_with_groups_and_memory() {
    let mut u: Ui<()> = Ui::new(crate::Look::ui_fonts(), 700, 300);
    u.rebuild("tv".into(), tv());
    assert_eq!(focused(&u), "Nastavení", "vychozi fokus");
    // Nahoru: do rady B (nejblizsi dlazdice nad stredem tlacitka vlevo).
    assert_eq!(arrow(&mut u, KEY_UP), None);
    let b = focused(&u);
    assert!(b.starts_with('B'), "{b}");
    // Vpravo v rade az na konec; dal se fokus nehne a sipka jde aplikaci.
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..4 {
        let un = arrow(&mut u, KEY_RIGHT);
        seen.push(focused(&u));
        if un.is_some() {
            break;
        }
    }
    assert_eq!(seen.last().map(String::as_str), Some("B3"));
    // Nahoru do rady A na A3 (nejblizsi), vlevo na A1.
    arrow(&mut u, KEY_UP);
    assert_eq!(focused(&u), "A3");
    arrow(&mut u, KEY_LEFT);
    arrow(&mut u, KEY_LEFT);
    assert_eq!(focused(&u), "A1");
    // Dolu: navrat do rady B na naposledy vybranou dlazdici (B3), ne na
    // nejblizsi (B1).
    arrow(&mut u, KEY_DOWN);
    assert_eq!(focused(&u), "B3", "pamet skupiny");
    // A nahoru zpet na A1 (pamet rady A).
    arrow(&mut u, KEY_UP);
    assert_eq!(focused(&u), "A1");
    // Tab zustava linearni.
    u.event(InputEvent::Key { code: KEY_TAB, action: 1, ch: None }, 0);
    assert_eq!(focused(&u), "A2");
    // Zmizely prvek s fokusem: vychozi fokus.
    u.rebuild("tv".into(), column([button::<()>("Nastavení").default_focus().key("nast"), button("Jiné")]));
    assert_eq!(focused(&u), "Nastavení");
}

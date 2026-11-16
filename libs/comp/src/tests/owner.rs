//! Verze 7 protokolu okna: vyjednani verze v `HELLO`, vlastnik okna
//! (`WINDOW_OWNER`) a zavreni popoveru stiskem mimo nej
//! (`POPOVER_DISMISS`).

use super::*;

fn second_window(t: &mut T, cl: ClientId, w: u32, h: u32) {
    t.msg(cl, ClientMsg::WindowCreate { window: 2, mode: Mode::Standard, w, h, title: "Dialog".into() });
    t.msg(cl, ClientMsg::LayerCreate { layer: 20, window: 2, parent: 0, kind: LayerKind::Solid });
    t.msg(cl, ClientMsg::LayerSet { layer: 20, props: LayerProps { mask: prop::RECT | prop::COLOR, rect: Rect::new(0, 0, w, h), color: GREEN, ..Default::default() } });
}

fn commit2(t: &mut T, cl: ClientId, txn: u32) {
    t.msg(cl, ClientMsg::Commit { txn, flags: 0, window: 2, serial: 0, token: 0 });
}

#[test]
fn hello_negotiates_version() {
    let mut t = T::new();
    // Starsi podporovana verze: WELCOME s ni, zpravy verze 7 odmitnuty.
    let old = t.c.add_client(info("stary", "stary"), t.now);
    t.msg(old, ClientMsg::Hello { version: guiproto::MIN_VERSION });
    let out = t.out();
    assert!(client_msgs(&out, old).iter().any(|m| matches!(m, ServerMsg::Welcome { version, .. } if *version == guiproto::MIN_VERSION)));
    t.window(old, Mode::Standard, RED);
    t.msg(old, ClientMsg::WindowOwner { window: 1, owner: 0 });
    assert!(client_msgs(&t.out(), old).iter().any(|m| matches!(m, ServerMsg::Error { op: guiproto::window::op::WINDOW_OWNER, code: guiproto::Error::NotSupported })));
    // Novejsi klient dostane nasi verzi.
    let new = t.c.add_client(info("novy", "novy"), t.now);
    t.msg(new, ClientMsg::Hello { version: guiproto::VERSION + 5 });
    assert!(client_msgs(&t.out(), new).iter().any(|m| matches!(m, ServerMsg::Welcome { version, .. } if *version == guiproto::VERSION)));
    // Prilis stara verze: odmitnuta.
    let ancient = t.c.add_client(info("prastary", "prastary"), t.now);
    t.msg(ancient, ClientMsg::Hello { version: guiproto::MIN_VERSION - 1 });
    assert!(client_msgs(&t.out(), ancient).iter().any(|m| matches!(m, ServerMsg::Error { op: guiproto::window::op::HELLO, code: guiproto::Error::NotSupported })));
}

#[test]
fn owned_window_centers_and_stays_above_owner() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    second_window(&mut t, a, 80, 40);
    t.msg(a, ClientMsg::WindowOwner { window: 2, owner: 1 });
    commit2(&mut t, a, 2);
    t.frame();
    let g1 = t.c.clients[&a].windows[&1];
    let g2 = t.c.clients[&a].windows[&2];
    assert_eq!(t.c.focus, Some(g2), "dialog okna s fokusem fokus prevezme");
    let (o, d) = (t.c.windows[&g1].rect, t.c.windows[&g2].rect);
    assert_eq!(d.x + d.w as i32 / 2, o.x + o.w as i32 / 2, "vystredeno vodorovne");
    assert_eq!(d.y + d.h as i32 / 2, o.y + o.h as i32 / 2, "vystredeno svisle");
    // Jine okno navrch, pak zase vlastnik: dialog musi byt nad nim.
    let b = t.client("b");
    t.window(b, Mode::Standard, BLUE);
    t.frame();
    let gb = t.c.clients[&b].windows[&1];
    t.c.set_focus(Some(gb));
    t.c.set_focus(Some(g1));
    let pos = |g: u32| t.c.z.iter().position(|x| *x == g).unwrap();
    assert!(pos(g2) > pos(g1) && pos(g1) > pos(gb), "poradi: b, vlastnik, dialog");
    // Zanik vlastnika vazbu zrusi.
    t.msg(a, ClientMsg::WindowDestroy { window: 1 });
    assert_eq!(t.c.windows[&g2].owner, None);
}

#[test]
fn owner_must_be_window_of_same_connection() {
    let mut t = T::new();
    let a = t.client("wine");
    let b = t.client("wine");
    t.window(a, Mode::Standard, RED);
    t.window(b, Mode::Standard, BLUE);
    t.out();
    // Mistni id 1 spojeni b je jeho vlastni okno; okno spojeni a (stejna
    // identita) se odkazat neda: neexistujici mistni id = NOT_SUPPORTED.
    t.msg(b, ClientMsg::WindowOwner { window: 1, owner: 7 });
    assert!(client_msgs(&t.out(), b).iter().any(|m| matches!(m, ServerMsg::Error { op: guiproto::window::op::WINDOW_OWNER, code: guiproto::Error::NotSupported })));
    // Sam sobe ani cyklus.
    t.msg(a, ClientMsg::WindowOwner { window: 1, owner: 1 });
    assert!(client_msgs(&t.out(), a).iter().any(|m| matches!(m, ServerMsg::Error { op: guiproto::window::op::WINDOW_OWNER, code: guiproto::Error::InvalidArgs })));
    second_window(&mut t, a, 60, 30);
    t.msg(a, ClientMsg::WindowOwner { window: 2, owner: 1 });
    t.msg(a, ClientMsg::WindowOwner { window: 1, owner: 2 });
    assert!(client_msgs(&t.out(), a).iter().any(|m| matches!(m, ServerMsg::Error { op: guiproto::window::op::WINDOW_OWNER, code: guiproto::Error::InvalidArgs })));
}

#[test]
fn press_outside_popover_dismisses_it() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    let r = t.rect_of(a);
    let pop = Rect::new(10, 10, 60, 40);
    t.msg(a, ClientMsg::LayerCreate { layer: 5, window: 1, parent: 0, kind: LayerKind::Solid });
    t.msg(a, ClientMsg::LayerSet { layer: 5, props: LayerProps { mask: prop::RECT | prop::COLOR | prop::POPOVER, rect: pop, color: GREEN, popover: true, ..Default::default() } });
    t.commit(a, 2, 0, 0, 0);
    t.frame();
    t.out();
    let press = |t: &mut T, x: i32, y: i32| {
        t.move_to(x, y);
        t.input(Event::Button { button: Button::Left, action: Action::Press }, false);
        t.input(Event::Button { button: Button::Left, action: Action::Release }, false);
    };
    // Uvnitr popoveru: nic.
    press(&mut t, r.x + 30, r.y + 30);
    assert!(!client_msgs(&t.out(), a).iter().any(|m| matches!(m, ServerMsg::PopoverDismiss { .. })));
    // Mimo popover (i v okne): zavrit.
    press(&mut t, r.x + 150, r.y + 80);
    assert!(client_msgs(&t.out(), a).contains(&ServerMsg::PopoverDismiss { window: 1 }));
}

#[test]
fn popover_clipped_to_work_area() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    let r = t.rect_of(a);
    // Popover az pod spodni okraj obrazovky (pres listu).
    let pop = Rect::new(0, H as i32 - r.y, 100, 400);
    t.msg(a, ClientMsg::LayerCreate { layer: 5, window: 1, parent: 0, kind: LayerKind::Solid });
    t.msg(a, ClientMsg::LayerSet { layer: 5, props: LayerProps { mask: prop::RECT | prop::COLOR | prop::POPOVER, rect: pop, color: GREEN, popover: true, ..Default::default() } });
    t.commit(a, 2, 0, 0, 0);
    t.frame();
    let g = t.c.clients[&a].windows[&1];
    let work = t.c.work_area();
    for pr in t.c.popover_rects(g) {
        assert!(pr.y as i64 + pr.h as i64 <= work.y as i64 + work.h as i64, "popover nevyjede pod pracovni plochu");
    }
}

/// Popover vlastnika zustane videt, kdyz fokus vezme jeho dialog (Wine:
/// bublina nebo nabidka okna pod dialogem): kresli se hned nad
/// vlastnikem, dialog ho prekryje, mimo dialog je videt.
#[test]
fn owner_popover_stays_under_its_dialog() {
    let mut t = T::new();
    let a = t.client("wine");
    t.window(a, Mode::Standard, RED);
    t.frame();
    let r = t.rect_of(a);
    // Popover vlastnika pod oknem (mimo jeho obdelnik) a pres stred okna.
    let below = Rect::new(0, r.h as i32 + 10, 120, 40);
    t.msg(a, ClientMsg::LayerCreate { layer: 5, window: 1, parent: 0, kind: LayerKind::Solid });
    t.msg(a, ClientMsg::LayerSet { layer: 5, props: LayerProps { mask: prop::RECT | prop::COLOR | prop::POPOVER, rect: below, color: BLUE, popover: true, ..Default::default() } });
    let mid = Rect::new(r.w as i32 / 2 - 10, r.h as i32 / 2 - 10, 20, 20);
    t.msg(a, ClientMsg::LayerCreate { layer: 6, window: 1, parent: 0, kind: LayerKind::Solid });
    t.msg(a, ClientMsg::LayerSet { layer: 6, props: LayerProps { mask: prop::RECT | prop::COLOR | prop::POPOVER, rect: mid, color: BLUE, popover: true, ..Default::default() } });
    t.commit(a, 2, 0, 0, 0);
    t.frame();
    let (bx, by) = (r.x + below.x + 60, r.y + below.y + 20);
    let (mx, my) = (r.x + r.w as i32 / 2, r.y + r.h as i32 / 2);
    assert_eq!(t.px(bx, by), BLUE);
    // Dialog nad stredem vlastnika vezme fokus.
    second_window(&mut t, a, 80, 40);
    t.msg(a, ClientMsg::WindowOwner { window: 2, owner: 1 });
    commit2(&mut t, a, 3);
    t.frame();
    let g2 = t.c.clients[&a].windows[&2];
    assert_eq!(t.c.focus, Some(g2));
    assert_eq!(t.px(bx, by), BLUE, "popover vlastnika mimo dialog zustane videt");
    assert_eq!(t.px(mx, my), GREEN, "dialog je nad popoverem vlastnika");
    // Jen poskozeni oblasti popoveru mimo okno: popover se prekresli.
    t.c.damage(Rect::new(bx - 5, by - 5, 10, 10));
    t.frame();
    assert_eq!(t.px(bx, by), BLUE, "prekresleni jen popoveru");
    // Jine okno s fokusem (ne dialog vlastnika): popover zmizi.
    let b = t.client("b");
    t.window(b, Mode::Standard, RED);
    t.frame();
    let gb = t.c.clients[&b].windows[&1];
    t.c.set_focus(Some(gb));
    t.frame();
    if !t.c.windows[&gb].visual().contains(bx, by) {
        assert_ne!(t.px(bx, by), BLUE, "popover cizi aplikace bez fokusu se neukazuje");
    }
}

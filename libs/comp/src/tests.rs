//! Testy jadra kompozitoru (na hostiteli: `cargo test -p comp`).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gfx::Canvas;
use gfx::text::Text;
use guiproto::Rect;
use guiproto::a11y::{Node, Role, actions};
use guiproto::action;
use guiproto::control::*;
use guiproto::window::*;
use inputproto::event::flags;
use inputproto::{Action, Button, Event, Record};

use crate::*;

const W: u32 = 800;
const H: u32 = 600;

struct T {
    c: Comp,
    frame: Vec<u32>,
    now: u64,
}

fn info(id: &str, name: &str) -> ConnectInfo {
    ConnectInfo {
        id: id.into(),
        name: name.into(),
        perms: vec!["síť".into()],
        actions: Vec::new(),
    }
}

impl T {
    fn new() -> Self {
        let mut n = 0u128;
        let c = Comp::new(
            W,
            H,
            Text::system(),
            Box::new(move || {
                n += 0x1234_5678_9abc;
                n
            }),
        );
        Self {
            c,
            frame: vec![0; (W * H) as usize],
            now: 1_000_000_000,
        }
    }

    fn msg(&mut self, cl: ClientId, m: ClientMsg) {
        self.c.client_msg(cl, m, None, self.now);
    }

    fn client(&mut self, id: &str) -> ClientId {
        let cl = self.c.add_client(info(id, id), self.now);
        self.msg(cl, ClientMsg::Hello { version: 1 });
        cl
    }

    /// Klient s oknem 200x100 a jednou vrstvou SOLID barvy `color`.
    fn window(&mut self, cl: ClientId, mode: Mode, color: u32) {
        self.msg(
            cl,
            ClientMsg::WindowCreate {
                window: 1,
                mode,
                w: 200,
                h: 100,
                title: "Okno".into(),
            },
        );
        self.msg(
            cl,
            ClientMsg::LayerCreate {
                layer: 1,
                window: 1,
                parent: 0,
                kind: LayerKind::Solid,
            },
        );
        self.set_color(cl, color);
        self.commit(cl, 1, 0, 0, 0);
    }

    fn set_color(&mut self, cl: ClientId, color: u32) {
        self.msg(
            cl,
            ClientMsg::LayerSet {
                layer: 1,
                props: LayerProps {
                    mask: prop::RECT | prop::COLOR,
                    rect: Rect::new(0, 0, 200, 100),
                    color,
                    ..Default::default()
                },
            },
        );
    }

    fn commit(&mut self, cl: ClientId, txn: u32, flags: u32, serial: u32, token: u128) {
        self.msg(
            cl,
            ClientMsg::Commit {
                txn,
                flags,
                window: 1,
                serial,
                token,
            },
        );
    }

    /// Tik a potvrzeni zobrazeni.
    fn frame(&mut self) -> Option<Vec<Rect>> {
        self.now += PERIOD;
        let mut canvas = Canvas::new(&mut self.frame, W, H, W).unwrap();
        let r = self.c.tick(self.now, &mut canvas);
        if r.is_some() {
            self.c.presented(self.now);
        }
        r
    }

    fn px(&self, x: i32, y: i32) -> u32 {
        self.frame[(y as u32 * W + x as u32) as usize]
    }

    fn rect_of(&self, cl: ClientId) -> Rect {
        let g = self.c.clients[&cl].windows[&1];
        self.c.windows[&g].rect
    }

    fn out(&mut self) -> Vec<Out> {
        self.c.take_out()
    }

    fn input(&mut self, e: Event, synthetic: bool) {
        let r = Record::new(e, 0, if synthetic { flags::SYNTHETIC } else { 0 }, self.now);
        self.c.input(&[r], self.now);
    }

    fn move_to(&mut self, x: i32, y: i32) {
        let (dx, dy) = (x - self.c.input.px, y - self.c.input.py);
        self.input(Event::Motion { dx, dy }, false);
    }

    fn click(&mut self, x: i32, y: i32) {
        self.move_to(x, y);
        self.input(
            Event::Button {
                button: Button::Left,
                action: Action::Press,
            },
            false,
        );
        self.input(
            Event::Button {
                button: Button::Left,
                action: Action::Release,
            },
            false,
        );
    }

    fn key(&mut self, code: u16, synthetic: bool) {
        self.input(
            Event::Key {
                code,
                scancode: code as u32,
                action: Action::Press,
                ch: None,
            },
            synthetic,
        );
    }
}

fn client_msgs(out: &[Out], cl: ClientId) -> Vec<ServerMsg> {
    out.iter()
        .filter_map(|o| match o {
            Out::Client(c, m) if *c == cl => Some(m.clone()),
            _ => None,
        })
        .collect()
}

const RED: u32 = 0xFFFF_0000;
const BLUE: u32 = 0xFF00_00FF;
const GREEN: u32 = 0xFF00_FF00;

#[test]
fn hello_window_commit_compose_present() {
    let mut t = T::new();
    let cl = t.client("org.test.a");
    let out = t.out();
    assert!(matches!(client_msgs(&out, cl)[0], ServerMsg::Welcome { w: W, h: H, .. }));
    t.window(cl, Mode::Standard, RED);
    let out = t.out();
    let msgs = client_msgs(&out, cl);
    assert!(msgs.iter().any(|m| matches!(m, ServerMsg::Configure { w: 200, h: 100, .. })));
    assert!(msgs.iter().any(|m| matches!(m, ServerMsg::Input { records, .. } if records[0].event == InputEvent::Focus(true))));
    assert!(t.frame().is_some());
    let r = t.rect_of(cl);
    assert_eq!(t.px(r.x + 10, r.y + 10), RED);
    // Titulek systemu nad obsahem (barva titulku okna s fokusem).
    assert_eq!(t.px(r.x + 150, r.y - 4), render::color::TITLE_FOCUSED);
    let out = t.out();
    assert!(client_msgs(&out, cl).contains(&ServerMsg::Presented { txn: 1, time: t.now }));
    // Nic se nemeni: zadny dalsi tik.
    assert_eq!(t.c.next_wakeup(t.now).map(|w| w > t.now + PERIOD), Some(true), "jen ping");
}

#[test]
fn commit_is_atomic() {
    let mut t = T::new();
    let cl = t.client("a");
    t.window(cl, Mode::Standard, RED);
    t.frame();
    // Rozpracovana zmena neni videt.
    t.set_color(cl, BLUE);
    t.c.damage_all();
    t.frame();
    let r = t.rect_of(cl);
    assert_eq!(t.px(r.x + 5, r.y + 5), RED);
    t.commit(cl, 2, 0, 0, 0);
    t.frame();
    assert_eq!(t.px(r.x + 5, r.y + 5), BLUE);
}

#[test]
fn sync_group_applies_together_or_on_timeout() {
    let mut t = T::new();
    let a = t.client("a");
    let b = t.client("b");
    t.window(a, Mode::Standard, RED);
    t.window(b, Mode::Standard, RED);
    t.frame();
    t.out();
    t.msg(a, ClientMsg::SyncGroup { count: 2 });
    let token = match client_msgs(&t.out(), a).pop() {
        Some(ServerMsg::SyncToken { token }) => token,
        o => panic!("{o:?}"),
    };
    t.set_color(a, BLUE);
    t.commit(a, 2, commit::GROUP, 0, token);
    t.frame();
    let (ra, rb) = (t.rect_of(a), t.rect_of(b));
    assert_eq!(t.px(ra.x + 5, ra.y + 5), RED, "a ceka na b");
    t.set_color(b, GREEN);
    t.commit(b, 2, commit::GROUP, 0, token);
    t.frame();
    assert_eq!(t.px(ra.x + 5, ra.y + 5), BLUE);
    assert_eq!(t.px(rb.x + 5, rb.y + 5), GREEN);
    // Neznamy token = chyba a commit se uplatni hned.
    t.out();
    t.set_color(a, RED);
    t.commit(a, 3, commit::GROUP, 0, 999);
    assert!(client_msgs(&t.out(), a).iter().any(|m| matches!(m, ServerMsg::Error { .. })));
    t.frame();
    assert_eq!(t.px(ra.x + 5, ra.y + 5), RED);
    // Skupina, kde jeden clen nedorazi: po terminu se uplatni.
    t.msg(a, ClientMsg::SyncGroup { count: 2 });
    let token = match client_msgs(&t.out(), a).pop() {
        Some(ServerMsg::SyncToken { token }) => token,
        o => panic!("{o:?}"),
    };
    t.set_color(a, GREEN);
    t.commit(a, 4, commit::GROUP, 0, token);
    t.frame();
    assert_eq!(t.px(ra.x + 5, ra.y + 5), RED);
    t.now += SYNC_TIMEOUT;
    t.frame();
    assert_eq!(t.px(ra.x + 5, ra.y + 5), GREEN);
}

#[test]
fn configure_waits_for_ack_or_timeout() {
    let mut t = T::new();
    let cl = t.client("a");
    t.window(cl, Mode::Standard, RED);
    t.frame();
    t.out();
    let g = t.c.clients[&cl].windows[&1];
    t.c.configure(g, 300, 150, t.now);
    let serial = match client_msgs(&t.out(), cl).pop() {
        Some(ServerMsg::Configure { w: 300, h: 150, serial, .. }) => serial,
        o => panic!("{o:?}"),
    };
    t.frame();
    assert_eq!(t.rect_of(cl).w, 200, "ram ceka na aplikaci");
    t.commit(cl, 2, commit::ACK_CONFIGURE, serial, 0);
    assert_eq!((t.rect_of(cl).w, t.rect_of(cl).h), (300, 150));
    // Bez potvrzeni: po terminu novy ram.
    t.c.configure(g, 250, 120, t.now);
    t.now += SYNC_TIMEOUT + 1;
    t.frame();
    assert_eq!((t.rect_of(cl).w, t.rect_of(cl).h), (250, 120));
}

#[test]
fn pointer_keyboard_routing_and_focus() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let b = t.client("b");
    t.window(b, Mode::Standard, BLUE);
    t.frame();
    t.out();
    let ra = t.rect_of(a);
    // Okno b je nahore a ma fokus; klik do viditelne casti a ho prevezme.
    t.click(ra.x + 3, ra.y + 3);
    let out = t.out();
    let ma = client_msgs(&out, a);
    assert!(ma.iter().any(|m| matches!(m, ServerMsg::Input { records, .. } if records[0].event == InputEvent::Focus(true))));
    assert!(ma.iter().any(|m| matches!(m, ServerMsg::Input { records, .. }
        if records[0].event == InputEvent::Button { code: inputproto::code::BTN_LEFT, pressed: true, x: 3, y: 3 })));
    assert!(client_msgs(&out, b).iter().any(|m| matches!(m, ServerMsg::Input { records, .. } if records[0].event == InputEvent::Focus(false))));
    assert_eq!(t.c.focus, Some(t.c.clients[&a].windows[&1]));
    // Klavesa jde oknu s fokusem, umely vstup s priznakem.
    t.key(30, true);
    let out = t.out();
    let ma = client_msgs(&out, a);
    assert!(ma.iter().any(|m| matches!(m, ServerMsg::Input { records, .. } if records[0].synthetic && matches!(records[0].event, InputEvent::Key { code: 30, action: 1, .. }))));
    assert!(client_msgs(&out, b).is_empty());
}

#[test]
fn title_drag_moves_window_without_app() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    t.out();
    let r = t.rect_of(a);
    t.move_to(r.x + 85, r.y - 10);
    t.input(
        Event::Button {
            button: Button::Left,
            action: Action::Press,
        },
        false,
    );
    t.move_to(r.x + 135, r.y + 20);
    t.input(
        Event::Button {
            button: Button::Left,
            action: Action::Release,
        },
        false,
    );
    let n = t.rect_of(a);
    assert_eq!((n.x, n.y), (r.x + 50, r.y + 30));
    // Aplikace nedostala zadne tlacitko (jen pohyb ukazatele v obsahu).
    assert!(!client_msgs(&t.out(), a).iter().any(|m| matches!(m, ServerMsg::Input { records, .. } if matches!(records[0].event, InputEvent::Button { .. }))));
}

#[test]
fn custom_title_regions_and_system_buttons() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::CustomTitle, RED);
    t.msg(
        a,
        ClientMsg::Regions {
            window: 1,
            regions: vec![(Rect::new(0, 0, 200, 32), RegionKind::Drag), (Rect::new(0, 0, 60, 32), RegionKind::NoDrag)],
        },
    );
    t.commit(a, 2, 0, 0, 0);
    t.frame();
    t.out();
    let r = t.rect_of(a);
    let g = t.c.clients[&a].windows[&1];
    // Tlacitka kresli system: zavrit vpravo v titulku.
    let close = t.c.windows[&g].buttons()[0];
    assert_eq!(t.px(close.x + 2, close.y + 12), render::color::CLOSE);
    assert_eq!(t.c.hit(close.x + 2, close.y + 2), Some((g, input::Zone::Button(0))));
    // Odznak duvery na pevnem miste vlevo: badge nad obsahem aplikace.
    assert_eq!(t.c.hit(r.x + 10, r.y + 12), Some((g, input::Zone::Badge)));
    // NoDrag uvnitr Drag = obsah (karta aplikace).
    assert!(matches!(t.c.hit(r.x + 50, r.y + 30), Some((_, input::Zone::Content(50, 30)))));
    assert_eq!(t.c.hit(r.x + 100, r.y + 10), Some((g, input::Zone::Title)));
    // Zavrit = zadost aplikaci (rozhodne ona).
    t.click(close.x + 5, close.y + 5);
    assert!(client_msgs(&t.out(), a).contains(&ServerMsg::Close { window: 1 }));
}

#[test]
fn badge_is_drawn_over_app_content() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::CustomTitle, RED);
    t.frame();
    let r = t.rect_of(a);
    // Aplikace kresli cervenou az do titulku, odznak je presto zeleny.
    assert_eq!(t.px(r.x + 2, r.y + 50), RED);
    assert_eq!(t.px(r.x + 8, r.y + 16), render::color::BADGE);
}

#[test]
fn frameless_badge_on_hover_and_minimize_restore() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Frameless, RED);
    t.frame();
    let r = t.rect_of(a);
    assert_eq!(t.px(r.x + 8, r.y + 16), RED);
    t.move_to(r.x + 40, r.y + 2);
    t.frame();
    assert_eq!(t.px(r.x + 8, r.y + 16), render::color::BADGE, "odznak pri najeti k okraji");
    t.now += input::BADGE_SHOW + 1;
    t.frame();
    assert_eq!(t.px(r.x + 8, r.y + 16), RED);
    // Panel: klik na polozku okno minimalizuje (ma fokus) a dalsi obnovi.
    let item = t.c.panel_items()[0].1;
    t.click(item.x + 5, item.y + 5);
    let g = t.c.clients[&a].windows[&1];
    assert!(t.c.windows[&g].minimized);
    t.click(item.x + 5, item.y + 5);
    assert!(!t.c.windows[&g].minimized);
}

#[test]
fn system_dialog_ignores_synthetic_input() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    t.out();
    t.c.dialog_request(
        7,
        DialogRequest::Ask {
            id: 1,
            kind: DialogKind::Permission,
            app: "Ukázka".into(),
            text: "Povolit síť?".into(),
            options: vec!["Povolit".into(), "Zamítnout".into()],
        },
        t.now,
    );
    t.frame();
    // Ztlumeni pres okno.
    let r = t.rect_of(a);
    assert_ne!(t.px(r.x + 5, r.y + 5), RED);
    // Umely Enter ani klik nic neudelaji; klavesy nejdou aplikaci.
    t.key(inputproto::code::KEY_ENTER, true);
    let l = t.c.dialog_layout().unwrap();
    let b = l.buttons[0];
    t.move_to(b.x + 5, b.y + 5);
    t.input(
        Event::Button {
            button: Button::Left,
            action: Action::Press,
        },
        true,
    );
    t.key(30, false);
    let out = t.out();
    assert!(!out.iter().any(|o| matches!(o, Out::Dialog(..))));
    assert!(client_msgs(&out, a).iter().all(|m| !matches!(m, ServerMsg::Input { records, .. } if matches!(records[0].event, InputEvent::Key { .. }))));
    // Skutecny Tab + Enter = druha volba.
    t.key(inputproto::code::KEY_TAB, false);
    t.key(inputproto::code::KEY_ENTER, false);
    assert!(t.out().contains(&Out::Dialog(
        7,
        DialogReply::Answer {
            id: 1,
            choice: 1,
            text: String::new()
        }
    )));
    assert!(t.c.dialogs.active().is_none());
}

#[test]
fn password_dialog_and_click() {
    let mut t = T::new();
    t.c.dialog_request(
        3,
        DialogRequest::Ask {
            id: 9,
            kind: DialogKind::Password,
            app: "Systém".into(),
            text: "Zadejte heslo".into(),
            options: vec!["OK".into(), "Zrušit".into()],
        },
        t.now,
    );
    for ch in "tajné".chars() {
        t.input(Event::Text(ch), false);
    }
    t.input(Event::Text('x'), true);
    t.frame();
    let b = t.c.dialog_layout().unwrap().buttons[0];
    t.click(b.x + 5, b.y + 5);
    assert!(t.out().contains(&Out::Dialog(
        3,
        DialogReply::Answer {
            id: 9,
            choice: 0,
            text: "tajné".into()
        }
    )));
}

#[test]
fn secure_attention_screen() {
    let mut t = T::new();
    t.c.secure_attention(t.now);
    assert!(t.c.status().secure);
    t.key(inputproto::code::KEY_ENTER, true);
    assert!(t.c.status().secure, "umely vstup bezpecnou obrazovku nezavre");
    t.key(inputproto::code::KEY_ENTER, false);
    assert!(t.out().contains(&Out::SecureRelease));
    assert!(!t.c.status().secure);
}

const MANIFEST: &str = r#"
[[action]]
id = "counter.add"
title = "Pricist"

[[action.param]]
name = "by"
type = "int"
required = true

[[action]]
id = "file.delete"
confirm = true
needs = ["fs:write"]
"#;

#[test]
fn actions_validate_confirm_and_route() {
    let mut t = T::new();
    let mut i = info("org.test.app", "Aplikace");
    i.actions = action::parse_manifest(MANIFEST).unwrap();
    let a = t.c.add_client(i, t.now);
    t.msg(a, ClientMsg::Hello { version: 1 });
    t.out();
    t.c.caller_msg(5, "shell", CallerMsg::List);
    assert!(matches!(&t.out()[0], Out::Caller(5, CallerReply::Actions { list }) if list.len() == 2));
    // Spatny typ argumentu: aplikace nic nedostane.
    t.c.caller_msg(
        5,
        "shell",
        CallerMsg::Call {
            call: 1,
            app: "org.test.app".into(),
            action: "counter.add".into(),
            args: vec![("by".into(), action::Value::Text("x".into()))],
            synthetic: false,
        },
    );
    let out = t.out();
    assert!(matches!(&out[0], Out::Caller(5, CallerReply::Result { call: 1, ok: false, .. })));
    assert!(client_msgs(&out, a).is_empty());
    // Spravne volani od shellu jde rovnou.
    t.c.caller_msg(
        5,
        "shell",
        CallerMsg::Call {
            call: 2,
            app: "org.test.app".into(),
            action: "counter.add".into(),
            args: vec![("by".into(), action::Value::Int(3))],
            synthetic: false,
        },
    );
    let id = match client_msgs(&t.out(), a).pop() {
        Some(ServerMsg::ActionInvoke { call, caller, synthetic: false, .. }) if caller == "shell" => call,
        o => panic!("{o:?}"),
    };
    t.msg(
        a,
        ClientMsg::ActionResult {
            call: id,
            ok: true,
            value: Some(action::Value::Int(3)),
            message: String::new(),
        },
    );
    assert!(matches!(&t.out()[0], Out::Caller(5, CallerReply::Result { call: 2, ok: true, value: Some(action::Value::Int(3)), .. })));
    // Agent (umely zdroj): nejdriv dialog, umely Enter ho nepotvrdi.
    t.c.caller_msg(
        6,
        "agent",
        CallerMsg::Call {
            call: 7,
            app: "org.test.app".into(),
            action: "counter.add".into(),
            args: vec![("by".into(), action::Value::Int(1))],
            synthetic: true,
        },
    );
    assert!(t.c.dialogs.active().is_some());
    assert!(client_msgs(&t.out(), a).is_empty());
    // Vychozi volba je "Zamitnout": Enter = zamitnuto.
    t.key(inputproto::code::KEY_ENTER, false);
    assert!(matches!(&t.out()[0], Out::Caller(6, CallerReply::Result { call: 7, ok: false, .. })));
    // Akce s confirm = true: Povolit (Shift+Tab nemame - sipka vlevo).
    t.c.caller_msg(
        5,
        "shell",
        CallerMsg::Call {
            call: 8,
            app: "org.test.app".into(),
            action: "file.delete".into(),
            args: vec![],
            synthetic: false,
        },
    );
    t.key(inputproto::code::KEY_LEFT, false);
    t.key(inputproto::code::KEY_ENTER, false);
    assert!(client_msgs(&t.out(), a).iter().any(|m| matches!(m, ServerMsg::ActionInvoke { action, .. } if action == "file.delete")));
    // Aplikace skonci: rozpracovane volani selze.
    t.c.remove_client(a);
    assert!(matches!(&t.out()[0], Out::Caller(5, CallerReply::Result { call: 8, ok: false, .. })));
}

#[test]
fn inspect_tree_absolute_and_act() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let mut n = Node::new(2, 0, Role::Button, "OK");
    n.layer = 1;
    n.bounds = Rect::new(10, 20, 50, 30);
    n.actions = actions::ACTIVATE;
    t.msg(a, ClientMsg::A11yUpdate { window: 1, nodes: vec![Node::new(1, 0, Role::Window, "Okno"), n] });
    t.commit(a, 2, 0, 0, 0);
    let g = t.c.clients[&a].windows[&1];
    t.c.inspect(1, InspectRequest::Tree { window: g });
    let r = t.rect_of(a);
    let out = t.out();
    let nodes = out
        .iter()
        .find_map(|o| match o {
            Out::Inspect(1, InspectReply::Tree(n)) => Some(n.clone()),
            _ => None,
        })
        .unwrap();
    let ok = nodes.iter().find(|n| n.name == "OK").unwrap();
    assert_eq!(ok.bounds, Rect::new(r.x + 10, r.y + 20, 50, 30));
    t.c.inspect(1, InspectRequest::Act { window: g, node: 2, action: actions::ACTIVATE, value: String::new() });
    let out = t.out();
    assert!(client_msgs(&out, a).contains(&ServerMsg::A11yAction { window: 1, node: 2, action: actions::ACTIVATE, value: String::new() }));
    // Akce, kterou uzel nepodporuje.
    t.c.inspect(1, InspectRequest::Act { window: g, node: 2, action: actions::SET_VALUE, value: "x".into() });
    assert!(matches!(t.out()[0], Out::Inspect(1, InspectReply::Error(_))));
    // Ustaleni: po slozeni.
    t.c.inspect(1, InspectRequest::Settle);
    assert!(t.out().is_empty());
    t.frame();
    assert!(t.out().contains(&Out::Inspect(1, InspectReply::Done)));
    t.c.inspect(1, InspectRequest::Windows);
    assert!(matches!(&t.out()[0], Out::Inspect(1, InspectReply::Windows(w)) if w.len() == 1 && w[0].focused));
}

#[test]
fn bad_client_messages_are_errors() {
    let mut t = T::new();
    let a = t.c.add_client(info("a", "a"), t.now);
    // Pred HELLO nic.
    t.msg(a, ClientMsg::WindowDestroy { window: 1 });
    assert!(matches!(client_msgs(&t.out(), a)[0], ServerMsg::Error { code: guiproto::Error::InvalidArgs, .. }));
    t.msg(a, ClientMsg::Hello { version: 1 });
    t.out();
    t.msg(a, ClientMsg::WindowCreate { window: 1, mode: Mode::Standard, w: 100, h: 100, title: String::new() });
    // Rodic z jineho okna / neexistujici.
    t.msg(a, ClientMsg::LayerCreate { layer: 5, window: 1, parent: 99, kind: LayerKind::Group });
    t.commit(a, 1, 0, 0, 0);
    assert!(client_msgs(&t.out(), a).iter().any(|m| matches!(m, ServerMsg::Error { op: guiproto::window::op::LAYER_CREATE, .. })));
    // Buffer vetsi nez pamet.
    t.c.client_msg(a, ClientMsg::BufferCreate { buffer: 1, w: 100, h: 100, stride: 100 }, Some(Box::new(vec![0u32; 50])), t.now);
    assert!(client_msgs(&t.out(), a).iter().any(|m| matches!(m, ServerMsg::Error { op: guiproto::window::op::BUFFER_CREATE, .. })));
    // Pocet chyb je omezeny.
    for _ in 0..200 {
        t.msg(a, ClientMsg::LayerDestroy { layer: 77 });
        t.commit(a, 1, 0, 0, 0);
    }
    let errs = client_msgs(&t.out(), a).iter().filter(|m| matches!(m, ServerMsg::Error { .. })).count();
    assert!(errs < 100, "{errs}");
}

#[test]
fn buffer_layer_scroll_and_clip() {
    let mut t = T::new();
    let a = t.client("a");
    t.msg(a, ClientMsg::WindowCreate { window: 1, mode: Mode::Frameless, w: 100, h: 100, title: String::new() });
    // Buffer 10 x 40: horni pulka cervena, dolni zelena.
    let mut px = vec![RED; 400];
    for p in &mut px[200..] {
        *p = GREEN;
    }
    t.c.client_msg(a, ClientMsg::BufferCreate { buffer: 1, w: 10, h: 40, stride: 10 }, Some(Box::new(px)), t.now);
    t.msg(a, ClientMsg::LayerCreate { layer: 1, window: 1, parent: 0, kind: LayerKind::Group });
    t.msg(
        a,
        ClientMsg::LayerSet {
            layer: 1,
            props: LayerProps { mask: prop::RECT | prop::CLIP, rect: Rect::new(0, 0, 10, 20), clip: true, ..Default::default() },
        },
    );
    t.msg(a, ClientMsg::LayerCreate { layer: 2, window: 1, parent: 1, kind: LayerKind::Buffer });
    t.msg(
        a,
        ClientMsg::LayerSet {
            layer: 2,
            props: LayerProps { mask: prop::RECT | prop::BUFFER, rect: Rect::new(0, 0, 10, 40), buffer: 1, ..Default::default() },
        },
    );
    t.commit(a, 1, 0, 0, 0);
    t.frame();
    let r = t.rect_of(a);
    assert_eq!(t.px(r.x + 2, r.y + 2), RED);
    assert_eq!(t.px(r.x + 2, r.y + 25), render::color::CONTENT, "orez skupiny");
    // Scroll dela kompozitor: posun obsahu o 20 = zelena.
    t.msg(a, ClientMsg::LayerSet { layer: 1, props: LayerProps { mask: prop::SCROLL, scroll: (0, 20), ..Default::default() } });
    t.commit(a, 2, 0, 0, 0);
    t.frame();
    assert_eq!(t.px(r.x + 2, r.y + 2), GREEN);
    // Animace scrollu zpet na 0.
    t.out();
    t.msg(a, ClientMsg::Animate { layer: 1, prop: AnimProp::ScrollY, target: 0, ms: 100, curve: Curve::Linear });
    t.commit(a, 3, 0, 0, 0);
    let mut done = false;
    for _ in 0..12 {
        t.frame();
        if client_msgs(&t.out(), a).contains(&ServerMsg::AnimationDone { layer: 1 }) {
            done = true;
            break;
        }
    }
    assert!(done);
    assert_eq!(t.px(r.x + 2, r.y + 2), RED);
}

#[test]
fn frame_callback_and_ping() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.commit(a, 2, commit::FRAME, 0, 0);
    t.out();
    t.frame();
    assert!(client_msgs(&t.out(), a).iter().any(|m| matches!(m, ServerMsg::Frame { period: PERIOD, .. })));
    // Ping a nereagujici aplikace.
    t.now += frame::PING_INTERVAL;
    t.c.timers(t.now);
    let serial = match client_msgs(&t.out(), a).pop() {
        Some(ServerMsg::Ping { serial }) => serial,
        o => panic!("{o:?}"),
    };
    t.now += frame::PING_TIMEOUT;
    t.c.timers(t.now);
    assert!(t.c.clients[&a].unresponsive);
    t.msg(a, ClientMsg::Pong { serial });
    assert!(!t.c.clients[&a].unresponsive);
}

/// Ulozi snimek jako PNG (pro kontrolu okem: `target/comp-*.png`).
fn save_png(name: &str, px: &[u32], w: u32, h: u32) {
    let mut raw = Vec::with_capacity(((w * 3 + 1) * h) as usize);
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            let p = px[(y * w + x) as usize];
            raw.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
    }
    // zlib s nekomprimovanymi bloky.
    let mut z = vec![0x78, 0x01];
    for (i, chunk) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        let n = chunk.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(chunk);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&(b << 16 | a).to_be_bytes());
    fn crc(d: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in d {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    let mut png = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10];
    let mut chunk = |ty: &[u8], data: &[u8]| {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = ty.to_vec();
        c.extend_from_slice(data);
        png.extend_from_slice(&c);
        png.extend_from_slice(&crc(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let _ = std::fs::write(dir.join(name), png);
}

#[test]
fn render_scene_snapshot() {
    let mut t = T::new();
    let a = t.client("org.magnumopus.poznamky");
    t.c.clients.get_mut(&a).unwrap().info.name = "Poznámky".into();
    t.window(a, Mode::Standard, 0xFF2A_3340);
    let b = t.client("org.magnumopus.prohlizec");
    t.window(b, Mode::CustomTitle, 0xFF40_3A2A);
    t.c.dialog_request(
        1,
        DialogRequest::Ask {
            id: 1,
            kind: DialogKind::Permission,
            app: "Poznámky (org.magnumopus.poznamky)".into(),
            text: "Aplikace chce přístup k síti. Povolit jen pro tuto relaci?".into(),
            options: vec!["Povolit".into(), "Zamítnout".into()],
        },
        t.now,
    );
    t.c.dialog_request(1, DialogRequest::Phrase { text: "modrý slon".into() }, t.now);
    t.c.damage_all();
    t.frame();
    save_png("comp-scene.png", &t.frame, W, H);
}

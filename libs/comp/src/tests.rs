//! Testy jadra kompozitoru (na hostiteli: `cargo test -p comp`).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gfx::Canvas;
use gfx::text::Family;
use guiproto::Rect;
use guiproto::a11y::{Node, Role, actions};
use guiproto::action;
use guiproto::control::*;
use guiproto::window::*;
use inputproto::event::flags;
use inputproto::{Action, Button, Event, Record};

use crate::*;

mod gpu;

const W: u32 = 800;
const H: u32 = 600;

/// Pismo UI jako v systemu (Inter s vlastnostmi z tokenu).
pub(crate) fn fonts() -> Family {
    let t = theme::Theme::builtin(theme::Mode::Light);
    let feats: Vec<[u8; 4]> = t.font.ui_features.iter().filter(|f| f.1).map(|f| f.0).collect();
    Family::new(font::inter(), &feats)
}

struct T {
    c: Comp,
    frame: Vec<u32>,
    now: u64,
}

fn info(id: &str, name: &str) -> ConnectInfo {
    ConnectInfo {
        system: false,
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
            fonts(),
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

    /// Odznak duvery okna klienta.
    fn badge_rect(&self, cl: ClientId) -> Rect {
        let g = self.c.clients[&cl].windows[&1];
        let w = &self.c.windows[&g];
        w.badge(self.c.badge_width(&self.c.clients[&cl]))
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
        // Dialog prijima vstup az chvili po zobrazeni.
        self.now += dialog::INPUT_DELAY;
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
        self.now += dialog::INPUT_DELAY;
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
    assert_eq!(t.px(r.x + 150, r.y - 4), t.c.style.theme.color.surface_2);
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
            regions: vec![(Rect::new(0, 0, 200, 32), RegionKind::Drag), (Rect::new(85, 0, 12, 32), RegionKind::NoDrag)],
        },
    );
    t.commit(a, 2, 0, 0, 0);
    t.frame();
    t.out();
    let r = t.rect_of(a);
    let g = t.c.clients[&a].windows[&1];
    // Tlacitka kresli system: zavrit vpravo v titulku.
    let close = t.c.windows[&g].buttons()[0];
    assert_ne!(t.px(close.x + 2, close.y + 14), t.c.style.theme.color.danger);
    // Cil kliknuti je vetsi nez viditelne tlacitko.
    assert_eq!(t.c.hit(close.x + 14, close.y - 4), Some((g, input::Zone::Button(0))));
    assert_eq!(t.c.hit(close.x + 2, close.y + 2), Some((g, input::Zone::Button(0))));
    // Najeti: zavrit cervene.
    t.move_to(close.x + 14, close.y + 14);
    t.frame();
    assert_eq!(t.px(close.x + 2, close.y + 14), t.c.style.theme.color.danger);
    // Odznak duvery na pevnem miste vlevo: badge nad obsahem aplikace.
    assert_eq!(t.c.hit(r.x + 10, r.y + 12), Some((g, input::Zone::Badge)));
    // NoDrag uvnitr Drag = obsah (karta aplikace).
    assert!(matches!(t.c.hit(r.x + 90, r.y + 20), Some((_, input::Zone::Content(90, 20)))));
    assert_eq!(t.c.hit(r.x + 79, r.y + 10), Some((g, input::Zone::Title)));
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
    // Aplikace kresli cervenou az do titulku, odznak je presto videt.
    assert_eq!(t.px(r.x + 2, r.y + 50), RED);
    let b = t.badge_rect(a);
    assert_eq!(t.px(b.x + 2, b.y + b.h as i32 / 2), t.c.style.theme.color.trust_bg);
}

#[test]
fn frameless_badge_on_hover_and_minimize_restore() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Frameless, RED);
    t.frame();
    let r = t.rect_of(a);
    let b = t.badge_rect(a);
    let p = (b.x + 2, b.y + b.h as i32 / 2);
    assert_eq!(t.px(p.0, p.1), RED);
    t.move_to(r.x + 40, r.y + 2);
    t.frame();
    assert_eq!(t.px(p.0, p.1), t.c.style.theme.color.trust_bg, "odznak pri najeti k okraji");
    t.now += input::BADGE_SHOW + 1;
    t.frame();
    assert_eq!(t.px(p.0, p.1), RED);
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
    t.c.caller_msg(5, "shell", false, CallerMsg::List, t.now);
    assert!(matches!(&t.out()[0], Out::Caller(5, CallerReply::Actions { list }) if list.len() == 2));
    // Spatny typ argumentu: aplikace nic nedostane.
    t.c.caller_msg(
        5,
        "shell",
        false,
        CallerMsg::Call {
            call: 1,
            app: "org.test.app".into(),
            action: "counter.add".into(),
            args: vec![("by".into(), action::Value::Text("x".into()))],
            synthetic: false,
        },
        t.now,
    );
    let out = t.out();
    assert!(matches!(&out[0], Out::Caller(5, CallerReply::Result { call: 1, ok: false, .. })));
    assert!(client_msgs(&out, a).is_empty());
    // Spravne volani od shellu jde rovnou.
    t.c.caller_msg(
        5,
        "shell",
        false,
        CallerMsg::Call {
            call: 2,
            app: "org.test.app".into(),
            action: "counter.add".into(),
            args: vec![("by".into(), action::Value::Int(3))],
            synthetic: false,
        },
        t.now,
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
        false,
        CallerMsg::Call {
            call: 7,
            app: "org.test.app".into(),
            action: "counter.add".into(),
            args: vec![("by".into(), action::Value::Int(1))],
            synthetic: true,
        },
        t.now,
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
        false,
        CallerMsg::Call {
            call: 8,
            app: "org.test.app".into(),
            action: "file.delete".into(),
            args: vec![],
            synthetic: false,
        },
        t.now,
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
    assert_eq!(t.px(r.x + 2, r.y + 25), t.c.style.theme.color.surface, "orez skupiny");
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

/// Ping se ztratil (plna fronta kanalu): nereagujici aplikace dostane
/// dalsi a odpoved na nej ztlumeni zrusi.
#[test]
fn lost_ping_is_repeated() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.out();
    t.now += frame::PING_INTERVAL;
    t.c.timers(t.now);
    t.out();
    t.now += frame::PING_TIMEOUT;
    t.c.timers(t.now);
    assert!(t.c.clients[&a].unresponsive);
    t.out();
    t.now += frame::PING_INTERVAL;
    t.c.timers(t.now);
    let serial = match client_msgs(&t.out(), a).pop() {
        Some(ServerMsg::Ping { serial }) => serial,
        o => panic!("dalsi ping chybi: {o:?}"),
    };
    t.msg(a, ClientMsg::Pong { serial });
    assert!(!t.c.clients[&a].unresponsive);
    // Po odpovedi zase jen bezny interval.
    assert!(t.c.clients[&a].ping.is_none());
}

/// Ulozi snimek jako PNG (pro kontrolu okem: `target/comp-*.png`).
fn save_png(name: &str, px: &[u32], w: u32, h: u32) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let _ = std::fs::write(dir.join(name), gfx::png::encode(px, w, h, w));
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

/// Okno `w x h` s titulkem a obsahem barvy `color` na pozici (`x`, `y`).
fn look_window(t: &mut T, id: &str, name: &str, title: &str, (x, y, w, h): (i32, i32, u32, u32), color: u32) -> ClientId {
    let cl = t.client(id);
    t.c.clients.get_mut(&cl).unwrap().info.name = name.into();
    t.msg(cl, ClientMsg::WindowCreate { window: 1, mode: Mode::Standard, w, h, title: title.into() });
    t.msg(cl, ClientMsg::LayerCreate { layer: 1, window: 1, parent: 0, kind: LayerKind::Solid });
    t.msg(cl, ClientMsg::LayerSet { layer: 1, props: LayerProps { mask: prop::RECT | prop::COLOR, rect: Rect::new(0, 0, w, h), color, ..Default::default() } });
    t.commit(cl, 1, 0, 0, 0);
    let g = t.c.clients[&cl].windows[&1];
    t.c.windows.get_mut(&g).unwrap().rect = Rect::new(x, y, w, h);
    cl
}

/// Snimky vzhledu podle `design/okna.html` a `design/plocha.html`
/// (`target/look-light.png`, `target/look-dark.png`).
#[test]
fn look_snapshots() {
    for (mode, file) in [(theme::Mode::Light, "look-light.png"), (theme::Mode::Dark, "look-dark.png")] {
        let mut t = T::new();
        t.c.set_mode(mode);
        let surface = t.c.style.theme.color.surface;
        let term = t.c.style.theme.color.term_bg;
        look_window(&mut t, "org.magnumopus.files", "Soubory", "Dokumenty", (60, 90, 360, 220), surface);
        look_window(&mut t, "org.magnumopus.term", "Terminál", "~/Projekty - Terminál", (300, 260, 420, 280), term);
        t.c.set_net(Some(render::NetState::Online));
        t.c.set_volume(Some((60, false)));
        t.c.set_tasks(4, 1);
        t.c.set_clock(Some(("14:05".into(), "so 7. 11.".into())));
        t.c.damage_all();
        let t0 = std::time::Instant::now();
        t.frame();
        let first = t0.elapsed();
        t.c.damage_all();
        let t1 = std::time::Instant::now();
        t.frame();
        std::println!("cela obrazovka {W}x{H}: prvni {first:?} (s plochou), dalsi {:?}", t1.elapsed());
        save_png(file, &t.frame, W, H);
    }
}

#[test]
fn new_device_must_be_confirmed_by_existing_input() {
    let mut t = T::new();
    let r = Record::new(Event::NewDevice { source: 9, state: inputproto::DeviceTrust::Pending }, 0, 0, t.now);
    t.c.input(&[r], t.now);
    assert!(t.c.dialogs.active().is_some());
    // Nove zarizeni samo sebe nepotvrdi (Enter z neovereneho zdroje).
    let mut left = Record::new(Event::Key { code: inputproto::code::KEY_LEFT, scancode: 0, action: Action::Press, ch: None }, 0, 0, t.now);
    left.source = 9;
    let mut enter = Record::new(Event::Key { code: inputproto::code::KEY_ENTER, scancode: 0, action: Action::Press, ch: None }, 0, 0, t.now);
    enter.source = 9;
    t.c.input(&[left, enter], t.now);
    assert!(t.out().is_empty());
    // Stavajici klavesnice: vlevo (Povolit) a Enter.
    left.source = 1;
    enter.source = 1;
    t.now += dialog::INPUT_DELAY;
    t.c.input(&[left, enter], t.now);
    assert!(t.out().contains(&Out::Device { source: 9, trust: true }));
    assert!(t.c.input.untrusted.is_empty());
}

#[test]
fn buffer_released_when_layer_switches() {
    let mut t = T::new();
    let a = t.client("a");
    t.msg(a, ClientMsg::WindowCreate { window: 1, mode: Mode::Standard, w: 10, h: 10, title: String::new() });
    for b in 1..=2 {
        t.c.client_msg(a, ClientMsg::BufferCreate { buffer: b, w: 10, h: 10, stride: 10 }, Some(Box::new(vec![RED; 100])), t.now);
    }
    t.msg(a, ClientMsg::LayerCreate { layer: 1, window: 1, parent: 0, kind: LayerKind::Buffer });
    let set = |b| ClientMsg::LayerSet { layer: 1, props: LayerProps { mask: prop::BUFFER | prop::RECT, rect: Rect::new(0, 0, 10, 10), buffer: b, ..Default::default() } };
    t.msg(a, set(1));
    t.commit(a, 1, 0, 0, 0);
    t.out();
    t.msg(a, set(2));
    t.commit(a, 2, 0, 0, 0);
    assert!(client_msgs(&t.out(), a).contains(&ServerMsg::Released { buffer: 1 }));
    t.msg(a, ClientMsg::LayerDestroy { layer: 1 });
    t.commit(a, 3, 0, 0, 0);
    assert!(client_msgs(&t.out(), a).contains(&ServerMsg::Released { buffer: 2 }));
}

#[test]
fn closed_caller_removes_its_confirmation() {
    let mut t = T::new();
    let mut i = info("org.test.app", "Aplikace");
    i.actions = action::parse_manifest(MANIFEST).unwrap();
    let a = t.c.add_client(i, t.now);
    t.msg(a, ClientMsg::Hello { version: 1 });
    t.c.caller_msg(5, "agent", false, CallerMsg::Call { call: 1, app: "org.test.app".into(), action: "file.delete".into(), args: vec![], synthetic: true }, t.now);
    assert!(t.c.dialogs.active().is_some());
    t.c.caller_gone(5);
    assert!(t.c.dialogs.active().is_none());
}

#[test]
fn dialog_ignores_input_right_after_showing() {
    let mut t = T::new();
    t.c.dialog_request(1, DialogRequest::Ask { id: 1, kind: DialogKind::Confirm, app: String::new(), text: "?".into(), options: vec!["Ano".into(), "Ne".into()] }, t.now);
    // Enter hned po zobrazeni (dvojklik, podstrceny klik) nic.
    let r = Record::new(Event::Key { code: inputproto::code::KEY_ENTER, scancode: 0, action: Action::Press, ch: None }, 0, 0, t.now);
    t.c.input(&[r], t.now + 1000);
    assert!(t.out().is_empty());
    t.key(inputproto::code::KEY_ENTER, false);
    assert!(matches!(t.out()[0], Out::Dialog(1, DialogReply::Answer { choice: 0, .. })));
}

#[test]
fn new_window_does_not_steal_focus_while_typing() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.key(30, false);
    let b = t.client("b");
    t.window(b, Mode::Standard, BLUE);
    assert_eq!(t.c.focus, Some(t.c.clients[&a].windows[&1]), "fokus zustal u psani");
    let _ = b;
}

#[test]
fn group_limit_is_per_client() {
    let mut t = T::new();
    let a = t.client("a");
    let b = t.client("b");
    t.window(b, Mode::Standard, RED);
    t.msg(b, ClientMsg::SyncGroup { count: 2 });
    let tok = match client_msgs(&t.out(), b).pop() { Some(ServerMsg::SyncToken { token }) => token, o => panic!("{o:?}") };
    t.set_color(b, BLUE);
    t.commit(b, 2, commit::GROUP, 0, tok);
    // Cizi klient zalozi hodne skupin: skupina b zustane drzena.
    for _ in 0..100 {
        t.msg(a, ClientMsg::SyncGroup { count: 2 });
    }
    t.frame();
    let r = t.rect_of(b);
    assert_eq!(t.px(r.x + 5, r.y + 5), RED);
}

#[test]
fn app_exit_removes_its_confirmation() {
    let mut t = T::new();
    let mut i = info("org.test.app", "Aplikace");
    i.actions = action::parse_manifest(MANIFEST).unwrap();
    let a = t.c.add_client(i, t.now);
    t.msg(a, ClientMsg::Hello { version: 1 });
    t.c.caller_msg(5, "shell", false, CallerMsg::Call { call: 1, app: "org.test.app".into(), action: "file.delete".into(), args: vec![], synthetic: false }, t.now);
    assert!(t.c.dialogs.active().is_some());
    t.c.remove_client(a);
    assert!(t.c.dialogs.active().is_none());
}

#[test]
fn tasks_icon_shows_count_and_focuses_task_manager() {
    let mut t = T::new();
    assert!(t.c.tasks_icon().is_none(), "nic nebezi = zadna ikona");
    let a = t.client("org.magnumopus.tasks");
    t.window(a, Mode::Standard, RED);
    let b = t.client("b");
    t.window(b, Mode::Standard, BLUE);
    t.c.set_tasks(3, 1);
    t.frame();
    let r = t.c.tasks_icon().unwrap();
    // Uloha agenta: stitek v barve agenta.
    let ai = t.c.style.theme.color.ai_soft;
    assert!((r.x..r.right() as i32).any(|x| t.px(x, r.y + r.h as i32 / 2) == ai), "stitek agenta");
    let ga = t.c.clients[&a].windows[&1];
    t.c.minimize(ga);
    t.click(r.x + 4, r.y + 4);
    assert_eq!(t.c.focus, Some(ga));
    assert!(!t.c.windows[&ga].minimized);
    t.c.set_tasks(0, 0);
    assert!(t.c.tasks_icon().is_none());
}

#[test]
fn input_focus_lost_releases_keys_and_drag() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    let ra = t.rect_of(a);
    // Stisk v okne (bez uvolneni), pak ztrata fokusu vstupu (restart inputd).
    t.move_to(ra.x + 3, ra.y + 3);
    t.input(Event::Button { button: Button::Left, action: Action::Press }, false);
    t.out();
    t.input(Event::Focus(false), false);
    let out = t.out();
    let ma = client_msgs(&out, a);
    let lost = ma.iter().filter(|m| matches!(m, ServerMsg::Input { records, .. } if records[0].event == InputEvent::Focus(false))).count();
    assert_eq!(lost, 1, "okno s fokusem i stiskem dostane jedno Focus(false)");
    // Uvolneni po ztrate fokusu uz oknu nejde (stisk je zapomenuty).
    t.input(Event::Button { button: Button::Left, action: Action::Release }, false);
    assert!(!client_msgs(&t.out(), a).iter().any(|m| matches!(m, ServerMsg::Input { records, .. } if matches!(records[0].event, InputEvent::Button { pressed: false, .. }))));
    t.input(Event::Focus(true), false);
    assert!(client_msgs(&t.out(), a).iter().any(|m| matches!(m, ServerMsg::Input { records, .. } if records[0].event == InputEvent::Focus(true))));
}

const EXPORT: &str = r#"
[[action]]
id = "file.export"
title = "Exportovat"
confirm = true

[[action.param]]
name = "path"
type = "path"
required = true

[[action.param]]
name = "recipient"
type = "text"
required = true
"#;

fn export_call(t: &mut T, path: &str) -> ClientId {
    let mut i = info("org.test.app", "Aplikace");
    i.actions = action::parse_manifest(EXPORT).unwrap();
    let a = t.c.add_client(i, t.now);
    t.msg(a, ClientMsg::Hello { version: 1 });
    t.out();
    t.c.caller_msg(
        5,
        "agent",
        false,
        CallerMsg::Call {
            call: 1,
            app: "org.test.app".into(),
            action: "file.export".into(),
            args: vec![("path".into(), action::Value::Path(path.into())), ("recipient".into(), action::Value::Text("jana@example.com".into()))],
            synthetic: false,
        },
        t.now,
    );
    a
}

#[test]
fn action_confirmation_shows_real_arguments() {
    let mut t = T::new();
    let long = "/home/michal/Dokumenty/projekty/velmi-dlouha-slozka-s-nazvem-ktery-se-nevejde/rozpocet-2027-final.xlsx";
    let a = export_call(&mut t, long);
    let d = t.c.dialogs.active().unwrap();
    assert!(d.text.contains(long), "cela cesta v dialogu");
    assert!(d.text.contains("\"jana@example.com\""), "prijemce v dialogu");
    // Dlouha cesta se zalomi, ale cela zustane videt.
    let l = t.c.dialog_layout().unwrap();
    let joined: String = l.lines.concat();
    assert!(joined.contains("rozpocet-2027-final.xlsx"));
    // Potvrzeni preda presne zobrazene argumenty.
    t.key(inputproto::code::KEY_LEFT, false);
    t.key(inputproto::code::KEY_ENTER, false);
    let out = t.out();
    assert!(client_msgs(&out, a).iter().any(|m| matches!(m, ServerMsg::ActionInvoke { args, .. } if args[0].1 == action::Value::Path(long.into()))));
}

#[test]
fn arguments_changed_after_confirmation_dialog_are_rejected() {
    let mut t = T::new();
    let a = export_call(&mut t, "/home/michal/zprava.txt");
    // Argumenty se zmeni po zobrazeni dialogu (napr. chyba nebo utok).
    t.c.services.tamper(vec![("path".into(), action::Value::Path("/etc/tajne".into())), ("recipient".into(), action::Value::Text("utocnik@example.com".into()))]);
    t.key(inputproto::code::KEY_LEFT, false);
    t.key(inputproto::code::KEY_ENTER, false);
    let out = t.out();
    assert!(client_msgs(&out, a).is_empty(), "aplikace nic nedostala");
    assert!(out.iter().any(|o| matches!(o, Out::Caller(5, CallerReply::Result { ok: false, message, .. }) if message.contains("zmenily"))));
}

#[test]
fn bulk_paths_show_count_and_preview() {
    let line = services::arg_line("files", &action::Value::Paths((1..=20).map(|i| format!("/data/f{i}.txt")).collect()));
    assert!(line.starts_with("files: 20 položek"));
    assert!(line.contains("/data/f1.txt") && line.contains("/data/f8.txt"));
    assert!(!line.contains("/data/f9.txt"));
    assert!(line.contains("... a dalších 12"));
}

#[test]
fn same_length_different_arguments_are_rejected() {
    // Ruzne zakodovani se stejnou delkou: kontrola porovnava cele bajty.
    let a1 = vec![("path".into(), action::Value::Path("/home/a/x".into())), ("recipient".into(), action::Value::Text("jana@example.com".into()))];
    let a2 = vec![("path".into(), action::Value::Path("/home/a/y".into())), ("recipient".into(), action::Value::Text("jana@example.com".into()))];
    assert_eq!(services::args_bytes(&a1).len(), services::args_bytes(&a2).len());
    assert_ne!(services::args_bytes(&a1), services::args_bytes(&a2));
    let mut t = T::new();
    let a = export_call(&mut t, "/home/a/x");
    t.c.services.tamper(a2);
    t.key(inputproto::code::KEY_LEFT, false);
    t.key(inputproto::code::KEY_ENTER, false);
    let out = t.out();
    assert!(client_msgs(&out, a).is_empty());
    assert!(out.iter().any(|o| matches!(o, Out::Caller(5, CallerReply::Result { ok: false, .. }))));
}

#[test]
fn startup_cost() {
    let t0 = std::time::Instant::now();
    let tok = theme::Tokens::builtin();
    let t1 = t0.elapsed();
    let _ = tok.theme(theme::Mode::Light).unwrap();
    let t2 = t0.elapsed();
    let f = fonts();
    let t3 = t0.elapsed();
    let mut c = Comp::new(1280, 800, f, Box::new(|| 1));
    let t4 = t0.elapsed();
    let mut px = vec![0u32; 1280 * 800];
    let mut canvas = Canvas::new(&mut px, 1280, 800, 1280).unwrap();
    c.tick(1, &mut canvas);
    let t5 = t0.elapsed();
    std::println!("parse {t1:?} theme {t2:?} fonts {t3:?} comp {t4:?} first frame {t5:?}");
}

/// Zmena pod listou prekresli celou listu (rozmazane pozadi je jedno).
#[test]
fn bar_blur_redraws_whole_bar() {
    let mut t = T::new();
    t.frame();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    let g = t.c.clients[&a].windows[&1];
    // Okno kousek nad listou: posun dolu poskodi jen jeho oblast.
    let bar = t.c.panel_rect();
    t.c.windows.get_mut(&g).unwrap().rect.y = bar.y - 60;
    t.c.damage_window(g);
    let rects = t.frame().unwrap();
    let covered: u64 = rects.iter().map(|r| r.intersect(&bar).w as u64 * r.intersect(&bar).h as u64).sum();
    assert!(covered >= bar.w as u64 * bar.h as u64, "lista prekreslena cela: {rects:?}");
}

/// Maximalizace a zmena velikosti po `CONFIGURE` poskodi stare misto okna
/// i se stinem (jinak zustanou artefakty).
#[test]
fn maximize_damages_old_place() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    let g = t.c.clients[&a].windows[&1];
    let old = t.c.windows[&g].visual();
    t.c.toggle_maximize(g, t.now);
    let rects = t.frame().unwrap();
    let covers = |r: Rect| rects.iter().any(|d| d.intersect(&r) == r) || {
        let area: u64 = rects.iter().map(|d| { let i = d.intersect(&r); i.w as u64 * i.h as u64 }).sum();
        area >= r.w as u64 * r.h as u64
    };
    assert!(covers(old.intersect(&t.c.screen())), "stare misto: {rects:?} {old:?}");
    // Zpet z maximalizace: po potvrzeni CONFIGURE mensi okno, odkryta
    // plocha se prekresli.
    let big = t.c.windows[&g].visual().intersect(&t.c.screen());
    t.c.toggle_maximize(g, t.now);
    let serial = t.c.windows[&g].configure.unwrap().0;
    t.commit(a, 2, guiproto::window::commit::ACK_CONFIGURE, serial, 0);
    let rects = t.frame().unwrap();
    let area: u64 = rects.iter().map(|d| { let i = d.intersect(&big); i.w as u64 * i.h as u64 }).sum();
    assert!(area >= big.w as u64 * big.h as u64, "plocha po zmenseni: {rects:?}");
}

#[test]
fn snapshot_config_and_shortcut() {
    use crate::diag::{Config, Shortcut};
    use inputproto::code::{KEY_S, KEY_SYSRQ};
    use inputproto::mods::{ALT, CTRL, SHIFT};
    let d = Config::default();
    assert_eq!((d.shortcut, d.auto_hung, d.auto_crash), (Shortcut::Both, false, false), "auto vychozi vypnuto");
    assert!(d.is_shortcut(KEY_SYSRQ, 0) && d.is_shortcut(KEY_S, CTRL | ALT | SHIFT) && !d.is_shortcut(KEY_S, CTRL | ALT));
    let c = Config::parse("zkratka=vypnuto auto_nereaguje=1 interval=5 nevim=x");
    assert!(!c.is_shortcut(KEY_SYSRQ, 0) && c.auto_hung && !c.auto_crash && c.interval_s == 5);
    assert_eq!(Config::parse("zkratka=print").is_shortcut(KEY_S, CTRL | ALT | SHIFT), false);
}

#[test]
fn bar_icons_open_flyouts() {
    let mut t = T::new();
    t.c.status.net = Some(crate::render::NetState::NoDevice);
    t.frame();
    let bar = t.c.bar_layout();
    let (bell, net) = (bar.bell, bar.net.expect("ikona site"));
    t.click(bell.x + 4, bell.y + 4);
    assert_eq!(t.c.flyout, Some(crate::flyout::Flyout::Bell), "klik na zvonek otevre panel");
    let r = t.c.flyout_rect().unwrap();
    assert!(r.bottom() <= t.c.panel_rect().y as i64, "panel nad listou");
    t.click(net.x + 4, net.y + 4);
    assert_eq!(t.c.flyout, Some(crate::flyout::Flyout::Net), "jina ikona prepne panel");
    t.click(net.x + 4, net.y + 4);
    assert_eq!(t.c.flyout, None, "druhy klik zavre");
    t.click(bell.x + 4, bell.y + 4);
    t.key(inputproto::code::KEY_ESC, false);
    assert_eq!(t.c.flyout, None, "Esc zavre");
}

#[test]
fn tasks_icon_tooltip() {
    let mut t = T::new();
    t.c.tasks = (20, 1);
    t.frame();
    let r = t.c.bar_layout().tasks.expect("ikona uloh");
    t.move_to(r.x + 4, r.y + 4);
    assert!(t.c.tasks_tip_visible(), "popisek po najeti");
    assert_eq!(t.c.tasks_tip(), "Úlohy: 20 běží, z toho agent AI 1");
    t.move_to(5, 5);
    assert!(!t.c.tasks_tip_visible());
}

#[test]
fn closing_focused_window_repaints_shadow() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    let g = t.c.clients[&a].windows[&1];
    assert_eq!(t.c.focus, Some(g), "okno ma fokus (vetsi stin)");
    let area = t.c.windows[&g].visual().intersect(&t.c.screen());
    // Plocha bez okna (stejny stav, okno nikdy nebylo).
    let mut r = T::new();
    r.frame();
    t.msg(a, ClientMsg::WindowDestroy { window: 1 });
    t.frame();
    for y in (area.y..area.y + area.h as i32).step_by(3) {
        for x in (area.x..area.x + area.w as i32).step_by(3) {
            assert_eq!(t.px(x, y), r.px(x, y), "duch okna na {x},{y} (oblast {area:?})");
        }
    }
}

/// Stisk / pusteni leveho tlacitka v miste ukazatele.
fn left(t: &mut T, press: bool) {
    let action = if press { Action::Press } else { Action::Release };
    t.input(Event::Button { button: inputproto::Button::Left, action }, false);
}

#[test]
fn close_button_zone_does_not_cover_edge() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    let g = t.c.clients[&a].windows[&1];
    let w = &t.c.windows[&g];
    let (o, close) = (w.outer(), w.buttons()[0]);
    let cy = close.y + close.h as i32 / 2;
    // Primo na tlacitku: zavrit (najeti i klik stejna zona).
    assert_eq!(t.c.hit(close.x + 2, cy), Some((g, crate::input::Zone::Button(0))));
    t.move_to(close.x + 2, cy);
    assert_eq!(t.c.input.hover, Some(crate::render::Hover::Button(g, 0)));
    // Vpravo za ramem (stin, zona okraje): zmena velikosti, ne zavrit.
    let x = o.right() as i32 + 2;
    for y in [cy, cy + 6] {
        match t.c.hit(x, y) {
            Some((_, crate::input::Zone::Edge { right: true, .. })) => {}
            z => panic!("mimo ram na {x},{y}: {z:?}"),
        }
        t.move_to(x, y);
        assert_eq!(t.c.input.hover, None, "zavrit se nezvyrazni mimo okno");
        assert_ne!(t.c.input.cursor, crate::input::Cursor::Arrow, "kurzor zmeny velikosti");
    }
}

#[test]
fn resize_from_left_and_top_keeps_opposite_edge() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    let g = t.c.clients[&a].windows[&1];
    let (o, r0) = (t.c.windows[&g].outer(), t.c.windows[&g].rect);
    // Roh vlevo nahore (za ramem): kurzor sikmy, tah o 20 px doleva a nahoru.
    let (x, y) = (o.x - 2, o.y - 2);
    t.move_to(x, y);
    assert_eq!(t.c.input.cursor, crate::input::Cursor::DiagDown);
    left(&mut t, true);
    t.move_to(x - 20, y - 20);
    left(&mut t, false);
    let (serial, nw, nh, _) = t.c.windows[&g].configure.expect("CONFIGURE");
    assert_eq!((nw, nh), (r0.w + 20, r0.h + 20));
    t.commit(a, 2, guiproto::window::commit::ACK_CONFIGURE, serial, 0);
    let r = t.c.windows[&g].rect;
    assert_eq!((r.right(), r.bottom()), (r0.right(), r0.bottom()), "pravy a dolni okraj na miste");
    assert_eq!((r.x, r.y), (r0.x - 20, r0.y - 20), "poloha posunuta");
    // Minimalni velikost: tah za levy okraj daleko doprava.
    let o = t.c.windows[&g].outer();
    let (x, y) = (o.x - 2, o.y + o.h as i32 / 2);
    t.move_to(x, y);
    assert_eq!(t.c.input.cursor, crate::input::Cursor::Horizontal);
    left(&mut t, true);
    t.move_to(x + 2000, y);
    left(&mut t, false);
    let (_, nw, _, _) = t.c.windows[&g].configure.expect("CONFIGURE");
    assert_eq!(nw, t.c.windows[&g].min.0, "nejmensi sirka");
}

#[test]
fn close_button_zone_is_round_except_maximized_corner() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    let g = t.c.clients[&a].windows[&1];
    let w = &t.c.windows[&g];
    let sq = w.button_hits()[0];
    // Roh ctvercove zony je mimo kruh tlacitka: nezvyrazni se.
    assert_ne!(t.c.hit(sq.right() as i32 - 1, sq.bottom() as i32 - 1), Some((g, crate::input::Zone::Button(0))));
    let b = w.buttons()[0];
    let r = (b.w / 2 + w.geo.btn_slop) as i32 - 1;
    assert_eq!(t.c.hit(b.x + b.w as i32 / 2 + r, b.y + b.h as i32 / 2), Some((g, crate::input::Zone::Button(0))), "kruh + okraj");
    // Maximalizovane okno: roh obrazovky vpravo nahore je zavrit (Fitts).
    t.c.toggle_maximize(g, t.now);
    let serial = t.c.windows[&g].configure.unwrap().0;
    t.commit(a, 2, guiproto::window::commit::ACK_CONFIGURE, serial, 0);
    let o = t.c.windows[&g].outer();
    assert_eq!(t.c.hit(o.right() as i32 - 1, o.y), Some((g, crate::input::Zone::Button(0))), "roh obrazovky {o:?}");
}

#[test]
fn resize_drag_coalesces_configures() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    let g = t.c.clients[&a].windows[&1];
    let (o, r0) = (t.c.windows[&g].outer(), t.c.windows[&g].rect);
    let (x, y) = (o.right() as i32 + 2, o.y + o.h as i32 / 2);
    t.move_to(x, y);
    left(&mut t, true);
    t.c.take_out();
    // Deset pohybu pred potvrzenim: aplikace dostane jen prvni velikost.
    for k in 1..=10 {
        t.move_to(x + 3 * k, y);
    }
    let sizes = |t: &mut T| -> Vec<(u32, u32, u32)> {
        t.c.take_out()
            .into_iter()
            .filter_map(|o| match o {
                Out::Client(c, ServerMsg::Configure { w, h, serial, .. }) if c == a => Some((w, h, serial)),
                _ => None,
            })
            .collect()
    };
    let first = sizes(&mut t);
    assert_eq!(first.len(), 1, "jeden CONFIGURE za nepotvrzenou zadost: {first:?}");
    assert_eq!(first[0].0, r0.w + 3);
    // Po potvrzeni hned posledni poloha (slouceno).
    t.commit(a, 2, guiproto::window::commit::ACK_CONFIGURE, first[0].2, 0);
    let next = sizes(&mut t);
    assert_eq!(next.iter().map(|s| s.0).collect::<Vec<_>>(), vec![r0.w + 30], "posledni velikost");
    left(&mut t, false);
}

#[test]
fn frame_period_follows_refresh_rate() {
    let mut t = T::new();
    assert_eq!(t.c.period(), crate::PERIOD, "vychozi 60 Hz");
    t.c.set_refresh(120_000);
    assert_eq!(t.c.period(), 8_333_333, "120 Hz");
    t.c.set_refresh(240_000);
    assert_eq!(t.c.period(), 4_166_666, "240 Hz");
    t.c.set_refresh(0);
    assert_eq!(t.c.period(), crate::PERIOD, "nezname = vychozi");
}

#[test]
fn bar_launches_closed_apps_and_activates_running() {
    let mut t = T::new();
    t.c.set_launchable(vec![("org.term".into(), "Terminál".into()), ("org.demo".into(), "Ukázka".into())]);
    let a = t.client("org.demo");
    t.window(a, Mode::Standard, RED);
    t.frame();
    let bar = t.c.bar_layout();
    // Ukazka bezi (tlacitko okna), terminal ne (ztlumena ikona).
    assert_eq!(bar.apps.len(), 1);
    assert_eq!(bar.launch.iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![0], "jen nespusteny terminal");
    let r = bar.launch[0].1;
    t.click(r.x + 4, r.y + 4);
    assert_eq!(t.c.take_launch_requests(), vec![alloc::string::String::from("org.term")], "zadost initu");
    // Spoustec: radek Ukazka aktivuje bezici okno (z minimalizace).
    let g = t.c.clients[&a].windows[&1];
    t.c.minimize(g);
    let l = t.c.bar_layout().launcher;
    t.click(l.x + 4, l.y + 4);
    assert_eq!(t.c.flyout, Some(crate::flyout::Flyout::Apps));
    let f = t.c.flyout_rect().unwrap();
    let th = t.c.style.theme.clone();
    let row1 = f.y + (th.space[4] + th.text.body_strong.line + th.space[2] + th.text.body.line + th.space[2] + 2) as i32;
    t.click(f.x + 8, row1);
    assert!(t.c.take_launch_requests().is_empty(), "bezici se nespousti znovu");
    assert!(!t.c.windows[&g].minimized && t.c.focus == Some(g), "aktivovano");
    assert_eq!(t.c.flyout, None);
}

/// Klavesa s modifikatory (fyzicka).
fn key_mods(t: &mut T, code: u16, m: u16) {
    let r = Record::new(Event::Key { code, scancode: code as u32, action: Action::Press, ch: None }, m, 0, t.now);
    t.c.input(&[r], t.now);
}

fn text(t: &mut T, s: &str) {
    for ch in s.chars() {
        let r = Record::new(Event::Text(ch), 0, 0, t.now);
        t.c.input(&[r], t.now);
    }
}

#[test]
fn command_palette_search_run_and_recent() {
    use crate::palette::{Item, SysAction};
    use inputproto::code::{KEY_ENTER, KEY_ESC, KEY_K};
    let mut t = T::new();
    t.c.set_launchable(vec![("org.term".into(), "Terminál".into()), ("org.tasks".into(), "Správce úloh".into())]);
    let a = t.client("org.demo");
    t.window(a, Mode::Standard, RED);
    t.frame();
    t.c.take_out();
    // Ctrl+K otevre; aplikace s fokusem klavesu nedostane.
    key_mods(&mut t, KEY_K, inputproto::mods::CTRL);
    assert!(t.c.palette_open(), "Ctrl+K");
    assert!(client_msgs(&t.c.take_out(), a).iter().all(|m| !matches!(m, ServerMsg::Input { .. })), "klavesa nejde aplikaci");
    assert!(t.c.palette_items().contains(&Item::System(SysAction::Shutdown)), "bez psani sekce System");
    text(&mut t, "o");
    assert!(t.c.palette_items().iter().any(|i| matches!(i, Item::Window(_))), "pri psani i okna");
    t.key(inputproto::code::KEY_BACKSPACE, false);
    // Hledani bez diakritiky: "sprav" najde Spravce uloh.
    text(&mut t, "sprav");
    assert_eq!(t.c.palette_items(), vec![Item::App("org.tasks".into(), "Správce úloh".into())]);
    t.key(KEY_ENTER, false);
    assert!(!t.c.palette_open(), "Enter provede a zavre");
    assert_eq!(t.c.take_launch_requests(), vec![alloc::string::String::from("org.tasks")]);
    // Nedavne nahore; Esc zavre bez akce; Super+K otevre take.
    key_mods(&mut t, KEY_K, inputproto::mods::META);
    assert_eq!(t.c.palette_items()[0], Item::App("org.tasks".into(), "Správce úloh".into()), "nedavne nahore");
    t.key(KEY_ESC, false);
    assert!(!t.c.palette_open());
    assert!(t.c.take_launch_requests().is_empty());
    // Mys: klik mimo paletu ji zavre.
    key_mods(&mut t, KEY_K, inputproto::mods::CTRL);
    t.click(2, 2);
    assert!(!t.c.palette_open(), "klik mimo zavre");
    // Systemova akce: Vypnout jde initu.
    key_mods(&mut t, KEY_K, inputproto::mods::CTRL);
    text(&mut t, "vypnout");
    t.key(KEY_ENTER, false);
    // Nevratne: nejdriv systemove potvrzeni (vychozi volba Zrusit).
    assert!(t.c.take_system_requests().is_empty(), "bez potvrzeni nic");
    assert!(t.c.dialogs.active().is_some(), "dialog potvrzeni");
    t.key(inputproto::code::KEY_TAB, false);
    t.key(KEY_ENTER, false);
    assert_eq!(t.c.take_system_requests(), vec![crate::palette::SystemRequest::Shutdown]);
}

#[test]
fn command_palette_v2_app_menu_actions() {
    use crate::palette::{Item, Row};
    use guiproto::action::{Action, Param, ParamType, Value};
    use guiproto::menu::MenuItem;
    use inputproto::code::{KEY_ENTER, KEY_K, KEY_TAB};
    let mut t = T::new();
    let mut ci = info("org.demo", "Ukázka");
    let act = |id: &str, confirm: bool, by: bool| Action {
        id: id.into(),
        title: id.into(),
        needs: Vec::new(),
        confirm,
        params: if by { vec![Param { name: "by".into(), ty: ParamType::Int, required: true }] } else { Vec::new() },
    };
    ci.actions = vec![act("counter.add", false, true), act("notes.clear", true, false), act("tab.new", false, false)];
    let a = t.c.add_client(ci, t.now);
    t.msg(a, ClientMsg::Hello { version: 1 });
    t.window(a, Mode::Standard, RED);
    t.frame();
    let item = |label: &str, action: &str| MenuItem { label: label.into(), action: action.into(), ..Default::default() };
    let mut add = item("Přičíst 1", "counter.add");
    add.args = vec![("by".into(), Value::Int(1))];
    let mut clear = item("Smazat poznámky", "notes.clear");
    clear.disabled = "žádné poznámky".into();
    let menu = vec![MenuItem {
        label: "Soubor".into(),
        children: vec![add, item("Nová záložka", "tab.new"), clear, item("Vypnout", "tab.new"), item("Cizí", "neni.v.manifestu")],
        ..Default::default()
    }];
    t.msg(a, ClientMsg::MenuUpdate { window: 1, items: menu });
    let g = t.c.clients[&a].windows[&1];
    assert_eq!(t.c.windows[&g].menu.len(), 3, "vyhrazeny nazev a akce mimo manifest zahozeny");
    key_mods(&mut t, KEY_K, inputproto::mods::CTRL);
    // Bez psani: sekce aplikace s jmenem a ikonou, System se stitem.
    let rows = t.c.palette_rows();
    assert!(rows.contains(&Row::Header("Ukázka".into(), "window", false)));
    assert!(rows.iter().any(|r| matches!(r, Row::Header(_, _, true))), "System se stitem");
    // Zkratka zacatku slov: "nz" = Nova zalozka; akce aplikace prvni.
    text(&mut t, "nz");
    assert_eq!(t.c.palette_items()[0], Item::AppAction(g, 1));
    // Predpona `>`: jen akce aplikace.
    t.key(inputproto::code::KEY_BACKSPACE, false);
    t.key(inputproto::code::KEY_BACKSPACE, false);
    text(&mut t, ">pri");
    assert_eq!(t.c.palette_items(), vec![Item::AppAction(g, 0)]);
    t.c.take_out();
    t.key(KEY_ENTER, false);
    let invoked = client_msgs(&t.c.take_out(), a)
        .into_iter()
        .any(|m| matches!(m, ServerMsg::ActionInvoke { action, args, .. } if action == "counter.add" && args == vec![("by".into(), Value::Int(1))]));
    assert!(invoked, "akce pres API akci s pevnymi argumenty");
    // Zakazana polozka se nespusti; Tab prepina rozsah na aplikaci.
    key_mods(&mut t, KEY_K, inputproto::mods::CTRL);
    t.key(KEY_TAB, false);
    assert!(t.c.palette_items().iter().all(|i| matches!(i, Item::AppAction(..))), "rozsah tato aplikace");
    text(&mut t, "smazat");
    t.c.take_out();
    t.key(KEY_ENTER, false);
    assert!(t.c.palette_open(), "zakazana akce paletu nezavre");
    assert!(client_msgs(&t.c.take_out(), a).iter().all(|m| !matches!(m, ServerMsg::ActionInvoke { .. })));
}

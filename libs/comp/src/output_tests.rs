//! Testy zmeny velikosti obrazovky ([`crate::output`]).

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::control::ConnectInfo;
use guiproto::window::{ClientMsg, Mode, ServerMsg};

use crate::{Comp, ClientId, Out};

fn comp(w: u32, h: u32) -> Comp {
    let mut n = 0u128;
    Comp::new(
        w,
        h,
        crate::tests::fonts(),
        Box::new(move || {
            n += 1;
            n
        }),
    )
}

fn window(c: &mut Comp, id: &str, mode: Mode, now: u64) -> ClientId {
    let cl = c.add_client(
        ConnectInfo {
            id: id.into(),
            name: id.into(),
            perms: Vec::new(),
            actions: Vec::new(),
        },
        now,
    );
    c.client_msg(cl, ClientMsg::Hello { version: 1 }, None, now);
    c.client_msg(
        cl,
        ClientMsg::WindowCreate {
            window: 1,
            mode,
            w: 300,
            h: 200,
            title: "Okno".into(),
        },
        None,
        now,
    );
    cl
}

fn configures(c: &mut Comp, cl: ClientId) -> Vec<(u32, u32)> {
    c.take_out()
        .into_iter()
        .filter_map(|o| match o {
            Out::Client(x, ServerMsg::Configure { w, h, .. }) if x == cl => Some((w, h)),
            _ => None,
        })
        .collect()
}

#[test]
fn resize_moves_and_configures_windows() {
    let now = 1_000_000_000;
    let mut c = comp(1920, 1080);
    let std = window(&mut c, "a", Mode::Standard, now);
    let full = window(&mut c, "b", Mode::Fullscreen, now);
    c.take_out();
    // Standardni okno daleko vpravo dole.
    let gid = *c.windows.keys().find(|&&g| c.windows[&g].client == std).unwrap();
    c.windows.get_mut(&gid).unwrap().rect.x = 1700;
    c.windows.get_mut(&gid).unwrap().rect.y = 900;
    c.resize(1024, 768, now);
    assert_eq!((c.w, c.h), (1024, 768));
    let r = c.windows[&gid].rect;
    assert!(r.x + 64 <= 1024 && r.y < 768 - 30, "{r:?}");
    assert!(r.y >= 33);
    // Okno na celou obrazovku dostane novou velikost.
    assert_eq!(configures(&mut c, full), vec![(1024, 768)]);
    assert!(configures(&mut c, std).is_empty());
    // Kurzor zustal na obrazovce, skladani nove velikosti funguje.
    assert!(c.input.px < 1024 && c.input.py < 768);
    let mut px = vec![0u32; 1024 * 768];
    let mut canvas = Canvas::new(&mut px, 1024, 768, 1024).unwrap();
    let rects = c.tick(now + 20_000_000, &mut canvas).unwrap();
    assert!(rects.iter().all(|r| r.x >= 0 && r.right() <= 1024 && r.bottom() <= 768));
    // Stejna velikost nic nedela.
    c.presented(now + 21_000_000);
    c.resize(1024, 768, now);
    assert!(c.take_out().is_empty());
}

#[test]
fn resize_keeps_maximized_maximized() {
    let now = 1_000_000_000;
    let mut c = comp(1280, 800);
    let cl = window(&mut c, "a", Mode::Standard, now);
    let gid = *c.windows.keys().next().unwrap();
    c.toggle_maximize(gid, now);
    c.take_out();
    c.resize(1920, 1080, now);
    let got = configures(&mut c, cl);
    let g = c.style.geo();
    assert_eq!(got, vec![(1920 - 2 * g.border, 1080 - c.bar_reserve() - g.title_h - 2 * g.border)]);
    assert!(c.windows[&gid].restore.is_some());
}

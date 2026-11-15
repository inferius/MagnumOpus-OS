//! Kontext okna a pozadovana velikost (`winsize.rs`).

use alloc::vec::Vec;

use super::*;

fn contexts(out: &[Out], c: ClientId) -> Vec<(u32, Rect, Rect)> {
    client_msgs(out, c)
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::WindowContext { generation, screen, work_area, .. } => Some((generation, screen, work_area)),
            _ => None,
        })
        .collect()
}

fn configures(out: &[Out], c: ClientId) -> Vec<(u32, u32, u32)> {
    client_msgs(out, c)
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::Configure { w, h, serial, .. } => Some((w, h, serial)),
            _ => None,
        })
        .collect()
}

fn create(t: &mut T, c: ClientId) {
    t.msg(c, ClientMsg::WindowCreate { window: 1, mode: Mode::Standard, w: 200, h: 100, title: "Okno".into() });
}

fn size(t: &mut T, c: ClientId, w: u32, h: u32, frac_w: u16) {
    t.msg(c, ClientMsg::WindowSize { window: 1, w, h, min_w: 0, min_h: 0, max_frac_w: frac_w, max_frac_h: 0 });
}

/// Kontext prijde pred prvnim `CONFIGURE`; velikost pozadovana pred
/// prvnim commitem plati hned pri zobrazeni (strop 85 % pracovni plochy).
#[test]
fn context_first_and_initial_size_without_flicker() {
    let mut t = T::new();
    let a = t.client("a");
    t.out();
    create(&mut t, a);
    let out = t.out();
    let msgs = client_msgs(&out, a);
    let ci = msgs.iter().position(|m| matches!(m, ServerMsg::WindowContext { .. })).expect("kontext");
    let fi = msgs.iter().position(|m| matches!(m, ServerMsg::Configure { .. })).unwrap();
    assert!(ci < fi, "kontext pred CONFIGURE");
    let (_, screen, work) = contexts(&out, a)[0];
    assert_eq!(screen, Rect::new(0, 0, W, H));
    assert_eq!(work, t.c.work_area());
    // Terminal: 1040 px, nejvys 85 % sirky pracovni plochy.
    size(&mut t, a, 1040, 300, 850);
    let want = work.w * 850 / 1000;
    let cf = configures(&t.out(), a);
    assert_eq!(cf.len(), 1);
    let (cw, ch, serial) = cf[0];
    assert_eq!((cw, ch), (want, 300));
    // Prvni commit potvrdi novou velikost: okno se ukaze rovnou s ni.
    t.commit(a, 1, guiproto::window::commit::ACK_CONFIGURE, serial, 0);
    assert_eq!((t.rect_of(a).w, t.rect_of(a).h), (want, 300));
    let g = t.c.clients[&a].windows[&1];
    assert!(t.c.windows[&g].mapped);
}

/// Zobrazene okno: zadost o zmenu = `CONFIGURE`, plati po potvrzeni.
/// Maximalizovane okno si zadost zapamatuje na obnoveni.
#[test]
fn request_size_later_and_while_maximized() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.out();
    size(&mut t, a, 300, 150, 0);
    let (cw, ch, serial) = configures(&t.out(), a)[0];
    assert_eq!((cw, ch), (300, 150));
    t.commit(a, 2, guiproto::window::commit::ACK_CONFIGURE, serial, 0);
    assert_eq!((t.rect_of(a).w, t.rect_of(a).h), (300, 150));
    // Maximalizovane: zadna zmena hned, po obnoveni pozadovana velikost.
    let g = t.c.clients[&a].windows[&1];
    t.c.toggle_maximize(g, t.now);
    let s = configures(&t.out(), a).last().unwrap().2;
    t.commit(a, 3, guiproto::window::commit::ACK_CONFIGURE, s, 0);
    t.out();
    size(&mut t, a, 260, 120, 0);
    assert!(configures(&t.out(), a).is_empty(), "maximalizovane okno zustane");
    t.c.toggle_maximize(g, t.now);
    let (cw, ch, _) = *configures(&t.out(), a).last().unwrap();
    assert_eq!((cw, ch), (260, 120));
    // Nejmensi velikost plati i pro zadost (0 = soucasna velikost).
    t.msg(a, ClientMsg::WindowSize { window: 1, w: 20, h: 0, min_w: 100, min_h: 50, max_frac_w: 0, max_frac_h: 0 });
    let (cw, _, _) = *configures(&t.out(), a).last().unwrap();
    assert_eq!(cw, 100);
    // Cizi okno: chyba.
    t.msg(a, ClientMsg::WindowSize { window: 9, w: 10, h: 10, min_w: 0, min_h: 0, max_frac_w: 0, max_frac_h: 0 });
    assert!(client_msgs(&t.out(), a).iter().any(|m| matches!(m, ServerMsg::Error { .. })));
}

/// Zmena rozliseni: nova generace kontextu vsem oknum.
#[test]
fn resize_sends_new_context_generation() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let g0 = t.c.ctx_generation;
    t.out();
    t.c.resize(1024, 700, t.now);
    let cx = contexts(&t.out(), a);
    assert_eq!(cx.len(), 1);
    assert_eq!(cx[0].0, g0 + 1);
    assert_eq!(cx[0].1, Rect::new(0, 0, 1024, 700));
    assert_eq!(cx[0].2, t.c.work_area());
}

/// Rozsireni kontextu: pocitac s mysi a klavesnici, dlazdice podle
/// maximalizace (nova generace), nejmensi cil z tokenu.
#[test]
fn context_ext_tile_follows_maximize() {
    use guiproto::context::{PointerKind, Posture, Tile};
    let mut t = T::new();
    let a = t.client("a");
    t.out();
    t.window(a, Mode::Standard, RED);
    let ext = |out: &[Out]| {
        client_msgs(out, a).into_iter().rev().find_map(|m| match m {
            ServerMsg::WindowContext { ext, generation, .. } => Some((ext, generation)),
            _ => None,
        })
    };
    let (e, g0) = ext(&t.out()).unwrap();
    assert_eq!((e.posture, e.tile, e.keyboard), (Posture::Desktop, Tile::Floating, true));
    assert_eq!(e.pointers[0].kind, PointerKind::Mouse);
    assert_eq!(e.min_hit as u32, t.c.style.theme.size.hit_min_fine);
    let g = t.c.clients[&a].windows[&1];
    t.c.toggle_maximize(g, t.now);
    let (e, g1) = ext(&t.out()).unwrap();
    assert_eq!(e.tile, Tile::Full);
    assert!(g1 > g0);
}

/// Okno s vlastnim titulkem dostane v kontextu plochu odznaku duvery
/// v souradnicich obsahu (tam, kde ho kompozitor kresli); okno se
/// standardnim titulkem prazdnou.
#[test]
fn context_ext_title_badge_for_custom_title() {
    let mut t = T::new();
    let a = t.client("a");
    let b = t.client("b");
    t.out();
    t.window(a, Mode::CustomTitle, RED);
    t.window(b, Mode::Standard, RED);
    let out = t.out();
    let badge = |c| {
        client_msgs(&out, c).into_iter().rev().find_map(|m| match m {
            ServerMsg::WindowContext { ext, .. } => Some(ext.title_badge),
            _ => None,
        })
    };
    let r = t.rect_of(a);
    let drawn = t.badge_rect(a);
    assert_eq!(badge(a), Some(Rect::new(drawn.x - r.x, drawn.y - r.y, drawn.w, drawn.h)));
    assert!(drawn.w > 0);
    assert_eq!(badge(b), Some(Rect::default()));
}

/// Kaskada noveho okna nesmi vysunout pravy okraj mimo malou obrazovku
/// (800 x 600, aarch64 ramfb): i paty siroky okno se cele vejde, at uz
/// velikost prisla pri vytvoreni nebo zpravou `WINDOW_SIZE` pred
/// prvnim zobrazenim.
#[test]
fn cascade_keeps_wide_window_on_screen() {
    let mut t = T::new();
    let border = t.c.style.geo().border as i32;
    for i in 0..5 {
        let c = t.client(&alloc::format!("k{i}"));
        t.msg(c, ClientMsg::WindowCreate { window: 1, mode: Mode::Standard, w: 200, h: 100, title: "Okno".into() });
    }
    let a = t.client("siroke");
    t.msg(a, ClientMsg::WindowCreate { window: 1, mode: Mode::Standard, w: 900, h: 640, title: "Siroke".into() });
    let r = t.rect_of(a);
    assert!(r.x >= border && r.x + r.w as i32 + border <= W as i32, "okno {r:?} na sirku mimo obrazovku {W}");
    let b = t.client("pozdeji");
    t.msg(b, ClientMsg::WindowCreate { window: 1, mode: Mode::Standard, w: 200, h: 100, title: "Okno".into() });
    size(&mut t, b, 900, 300, 0);
    let r = t.rect_of(b);
    assert!(r.x >= border && r.x + r.w as i32 + border <= W as i32, "okno {r:?} po WINDOW_SIZE mimo obrazovku {W}");
}

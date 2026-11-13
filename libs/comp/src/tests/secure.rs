//! Pole s tajemstvim v aplikacich (snimek je zakryje) a ochranna doba
//! systemoveho dialogu.

use alloc::vec;

use gfx::Canvas;

use super::*;

/// Pole s heslem v aplikaci (role `PasswordInput`): diagnosticky snimek
/// ho zakryje stejne jako pole prihlaseni; okraj okna pole orizne,
/// minimalizovane okno se nezakryva.
#[test]
fn app_password_field_is_secure() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let mut n = Node::new(2, 0, Role::PasswordInput, "Heslo");
    n.layer = 1;
    n.bounds = Rect::new(10, 20, 50, 30);
    let mut far = Node::new(3, 0, Role::PasswordInput, "Mimo");
    far.layer = 1;
    far.bounds = Rect::new(-500, -500, 10, 10);
    let other = Node::new(4, 0, Role::TextInput, "Jmeno");
    t.msg(a, ClientMsg::A11yUpdate { window: 1, nodes: vec![Node::new(1, 0, Role::Window, "Okno"), n, far, other] });
    t.commit(a, 2, 0, 0, 0);
    let r = t.rect_of(a);
    assert_eq!(t.c.secure_fields(), vec![Rect::new(r.x + 10, r.y + 20, 50, 30)]);
    let g = t.c.clients[&a].windows[&1];
    t.c.windows.get_mut(&g).unwrap().minimized = true;
    assert!(t.c.secure_fields().is_empty());
}

/// Ochranna doba dialogu bezi od zobrazeni snimku s dialogem, ne od
/// pozadavku: pomale skladani (snimek 1 s) nesmi dovolit odpoved na
/// dialog, ktery uzivatel jeste nevidel (souběh testu QMP v plne sade).
#[test]
fn dialog_accepts_input_only_after_it_was_shown() {
    let mut t = T::new();
    let ask = DialogRequest::Ask { id: 3, kind: DialogKind::Permission, app: "X".into(), text: "?".into(), options: vec!["Povolit".into(), "Zamítnout".into()] };
    t.c.dialog_request(1, ask, t.now);
    let enter = |t: &mut T| {
        let r = Record::new(Event::Key { code: inputproto::code::KEY_ENTER, scancode: 28, action: Action::Press, ch: None }, 0, 0, t.now);
        t.c.input(&[r], t.now);
        t.out().iter().any(|o| matches!(o, Out::Dialog(1, DialogReply::Answer { id: 3, .. })))
    };
    // Dlouho po pozadavku, ale bez slozeneho snimku: nic.
    t.now += 2 * dialog::INPUT_DELAY;
    assert!(!enter(&mut t), "dialog jeste nebyl na displeji");
    // Snimek slozen, displej ho jeste neukazal: nic.
    let mut px = vec![0u32; (W * H) as usize];
    let mut c = Canvas::new(&mut px, W, H, W).unwrap();
    assert!(t.c.tick(t.now, &mut c).is_some());
    t.now += 2 * dialog::INPUT_DELAY;
    assert!(!enter(&mut t), "snimek s dialogem jeste neni zobrazen");
    // Zobrazen: ochranna doba od teto chvile.
    t.c.presented(t.now);
    t.now += dialog::INPUT_DELAY / 2;
    assert!(!enter(&mut t), "ochranna doba po zobrazeni");
    t.now += dialog::INPUT_DELAY;
    assert!(enter(&mut t), "po ochranne dobe dialog vstup prijme");
}

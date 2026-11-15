//! Volba vzhledu a plynuly prechod rezimu (`appearance.rs`, `sun.rs`).

use super::*;
use theme::{Pref, Scheme};

const PRAHA: Option<(f32, f32)> = Some((50.08, 14.43));
/// 2026-06-21 a 2026-12-21, pulnoc UTC.
const LETO: i64 = 1_782_000_000 - 1_782_000_000 % 86_400;
const ZIMA: i64 = LETO + 183 * 86_400;

fn dark() -> Pref {
    Pref { scheme: Scheme::Dark, ..Pref::default() }
}

/// Snimky, dokud prolnuti nedobehne; vraci zpravy klientu za tu dobu.
fn run_transition(t: &mut T) -> Vec<Out> {
    let mut out = Vec::new();
    for _ in 0..400 {
        t.frame();
        out.extend(t.out());
        if !t.c.theme_animating() {
            break;
        }
    }
    out
}

#[test]
fn sun_rise_and_set_in_prague() {
    // Praha 21. 6.: vychod ~02:52 UTC, zapad ~19:15 UTC.
    let (rise, set) = crate::sun::rise_set(50.08, 14.43, LETO / 86_400).unwrap();
    let (r, s) = ((rise - LETO) / 60, (set - LETO) / 60);
    assert!((165..=180).contains(&r), "vychod {r} min");
    assert!((1145..=1165).contains(&s), "zapad {s} min");
    // 21. 12.: vychod ~07:01 UTC, zapad ~15:02 UTC.
    let (rise, set) = crate::sun::rise_set(50.08, 14.43, ZIMA / 86_400).unwrap();
    let (r, s) = ((rise - ZIMA) / 60, (set - ZIMA) / 60);
    assert!((410..=430).contains(&r), "vychod {r} min");
    assert!((890..=910).contains(&s), "zapad {s} min");
    // Den a noc, okamzik zmeny.
    let noon = ZIMA + 12 * 3600;
    assert_eq!(crate::sun::is_day(noon, 3600, PRAHA), (true, set));
    let (day, next) = crate::sun::is_day(ZIMA + 20 * 3600, 3600, PRAHA);
    assert!(!day && next > ZIMA + 86_400);
    // Polarni noc (Svalbard v prosinci) a bez mista: pevne 7-19 h.
    assert_eq!(crate::sun::rise_set(78.2, 15.6, ZIMA / 86_400), None);
    assert!(crate::sun::is_day(ZIMA + 11 * 3600, 3600, Some((78.2, 15.6))).0);
    assert!(!crate::sun::is_day(ZIMA + 19 * 3600, 3600, None).0);
    assert!(crate::sun::is_day(ZIMA + 6 * 3600, 3600, None).0);
}

#[test]
fn user_pref_fades_all_colors_and_apps_get_steps() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.frame();
    t.out();
    let bg0 = t.px(5, 5);
    t.c.session_opened(1, 1000, "alice", t.now);
    t.c.set_appearance_user(1000, dark(), t.now);
    assert_eq!(t.c.mode(), theme::Mode::Dark, "cil hned");
    // Pulka prechodu: plocha mezi svetlou a tmavou.
    t.now += 300_000_000 - PERIOD;
    t.frame();
    let mid = t.px(5, 5);
    assert_ne!(mid, bg0);
    let mut out = t.out();
    out.extend(run_transition(&mut t));
    let dark_bg = t.px(5, 5);
    assert_ne!(mid, dark_bg);
    let l = |c: u32| ((c >> 16 & 0xFF) + (c >> 8 & 0xFF) + (c & 0xFF)) as i32;
    assert!(l(bg0) > l(mid) && l(mid) > l(dark_bg), "{bg0:08x} {mid:08x} {dark_bg:08x}");
    let msgs = client_msgs(&out, a);
    let steps: Vec<u16> = msgs.iter().filter_map(|m| if let ServerMsg::ThemeMix { from: 0, to: 1, t } = m { Some(*t) } else { None }).collect();
    assert!(steps.len() >= 3 && steps.len() <= 5, "{steps:?}");
    assert!(steps.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(msgs.last(), Some(&ServerMsg::Theme { mode: 1 }));
    // Na konci presne tokeny tmaveho rezimu.
    assert_eq!(t.c.style.theme, theme::Theme::builtin(theme::Mode::Dark));
}

#[test]
fn reduce_motion_switches_at_once() {
    let mut t = T::new();
    let a = t.client("a");
    t.frame();
    t.out();
    t.c.session_opened(1, 1000, "alice", t.now);
    t.c.set_appearance_user(1000, Pref { reduce_motion: true, ..dark() }, t.now);
    assert!(!t.c.theme_animating());
    let m = client_msgs(&t.out(), a);
    assert_eq!(m, [ServerMsg::Theme { mode: 1 }]);
    // Kontrast prebiji schema.
    t.c.set_appearance_user(1000, Pref { contrast: true, reduce_motion: true, ..dark() }, t.now);
    assert_eq!(t.c.mode(), theme::Mode::Contrast);
}

#[test]
fn login_screen_uses_system_pref_session_its_own() {
    let mut t = T::new();
    t.frame();
    t.c.set_appearance_system(Pref { reduce_motion: true, ..dark() }, t.now);
    assert_eq!(t.c.mode(), theme::Mode::Dark);
    t.c.set_appearance_user(1000, Pref { scheme: Scheme::Light, reduce_motion: true, ..Pref::default() }, t.now);
    assert_eq!(t.c.mode(), theme::Mode::Dark, "uzivatel neni prihlaseny");
    t.c.session_opened(1, 1000, "alice", t.now);
    assert_eq!(t.c.mode(), theme::Mode::Light);
    t.c.session_closed(1, t.now);
    assert_eq!(t.c.mode(), theme::Mode::Dark);
}

#[test]
fn bar_look_card_chooses_scheme_and_reports_it() {
    let mut t = T::new();
    t.frame();
    t.c.session_opened(1, 1000, "alice", t.now);
    t.c.set_appearance_user(1000, Pref { reduce_motion: true, ..Pref::default() }, t.now);
    t.frame();
    t.out();
    let look = t.c.bar_layout().look;
    t.click(look.x + 4, look.y + 4);
    assert_eq!(t.c.flyout, Some(crate::flyout::Flyout::Look));
    assert!(t.c.look_lines().iter().any(|l| l.starts_with("Teď světlý")));
    // Druhy radek = Tmavy.
    let r = t.c.flyout_rect().unwrap();
    let th = t.c.style.theme.clone();
    let top = r.y + th.space[4] as i32 + (th.text.body_strong.line + th.space[2]) as i32;
    let row = (th.text.body.line + th.space[2]) as i32;
    t.click(r.x + 10, top + row + row / 2);
    assert_eq!(t.c.mode(), theme::Mode::Dark);
    let out = t.out();
    assert!(out.contains(&Out::Appearance { uid: 1000, pref: Pref { reduce_motion: true, ..dark() } }), "{out:?}");
    assert_eq!(t.c.look_icon(), "moon");
    assert_eq!(t.c.look_tip(), "Vzhled: Tmavý");
}

#[test]
fn auto_waits_while_typing() {
    let mut t = T::new();
    t.frame();
    t.c.set_appearance_system(Pref::default(), t.now);
    t.c.set_sun_place(PRAHA);
    t.c.sun_tick(ZIMA + 12 * 3600, 3600, t.now);
    assert_eq!(t.c.mode(), theme::Mode::Light);
    // Pise se: zapad slunce prepnuti odlozi.
    t.key(inputproto::code::KEY_A, false);
    t.c.sun_tick(ZIMA + 20 * 3600, 3600, t.now);
    assert_eq!(t.c.mode(), theme::Mode::Light, "odlozeno");
    assert!(t.c.next_wakeup(t.now).is_some());
    // Po pauze v psani se prepne.
    t.now += crate::appearance::TYPING_PAUSE;
    t.frame();
    assert_eq!(t.c.mode(), theme::Mode::Dark);
    // Automaticky prechod je pomaly (`motion.theme_auto`): po 1 s jeste bezi.
    t.now += 1_000_000_000;
    t.frame();
    assert!(t.c.theme_animating(), "auto prolnuti trva dele");
    run_transition(&mut t);
    assert_eq!(t.c.style.theme.mode, theme::Mode::Dark);
}

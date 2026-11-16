//! Panely listy nad komponentou panel (`barpanel.rs`): panel Zvuk
//! (vystup, vstup, aplikace podle identity, nahravani) posila zadosti
//! sluzbe zvuku (`Out::Audio`).

use super::*;
use crate::render::{AudioApp, AudioDev, AudioInfo};
use guiproto::a11y::{Role, actions, state};

fn sound(t: &mut T) {
    t.c.set_volume(Some((60, false)));
    t.c.set_capturing(alloc::vec![String::from("Hovory")]);
    t.c.audio_ctl = true;
    t.c.set_audio_info(Some(AudioInfo {
        devices: alloc::vec![
            AudioDev { index: 0, name: String::from("HDA vestavěné"), outputs: String::from("line-out"), input: true, active: true },
            AudioDev { index: 1, name: String::from("Sluchátka USB"), outputs: String::from("headphone"), input: false, active: false },
        ],
        apps: alloc::vec![
            AudioApp { id: String::from("org.winehq.wine"), name: String::from("Wine"), volume: 40, playing: true, capturing: false },
            AudioApp { id: String::from("org.magnumopus.calls"), name: String::from("Hovory"), volume: 100, playing: false, capturing: true },
        ],
        input: Some(0),
    }));
}

fn tree(t: &T) -> Vec<guiproto::a11y::Node> {
    t.c.barpanel.as_ref().expect("panel").ui.a11y()
}

#[test]
fn sound_panel_selects_output_input_and_app_volume() {
    let mut t = T::new();
    t.frame();
    sound(&mut t);
    t.frame();
    t.out();
    let vol = t.c.bar_layout().volume.expect("ikona zvuku");
    t.click(vol.x + 4, vol.y + 4);
    assert_eq!(t.c.flyout, Some(crate::flyout::Flyout::Volume));
    let r = t.c.flyout_rect().unwrap();
    let n = tree(&t);
    let find = |role: Role, name: &str| n.iter().find(|x| x.role == role && x.name == name).cloned().unwrap_or_else(|| panic!("{role:?} {name}: {:?}", n.iter().map(|x| &x.name).collect::<Vec<_>>()));
    // Vystup: zvoleny s fajfkou; vstup; nahravani; aplikace podle identity.
    assert_ne!(find(Role::Radio, "HDA vestavěné").state & state::CHECKED, 0);
    assert_eq!(find(Role::Radio, "Sluchátka USB").value, "headphone");
    find(Role::Label, "Nahrává: Hovory");
    find(Role::Label, "Wine");
    find(Role::Label, "VÝSTUP");
    find(Role::Label, "APLIKACE");
    // Klik na sluchatka: zadost o vystup 1.
    let b = find(Role::Radio, "Sluchátka USB").bounds;
    t.click(r.x + b.x + b.w as i32 / 2, r.y + b.y + b.h as i32 / 2);
    assert!(t.out().contains(&Out::Audio(crate::AudioReq::Output(1))));
    // Hlasitost Wine: posuvnik s fokusem, sipka vpravo = +5 %.
    let n = tree(&t);
    let s = n.iter().filter(|x| x.role == Role::Slider).nth(1).cloned().expect("posuvnik aplikace");
    t.c.barpanel.as_mut().unwrap().ui.a11y_action(s.id, actions::FOCUS, "");
    t.key(inputproto::code::KEY_RIGHT, false);
    assert!(t.out().contains(&Out::Audio(crate::AudioReq::App(String::from("org.winehq.wine"), 45))));
    // Esc panel zavre.
    t.key(inputproto::code::KEY_ESC, false);
    assert_eq!(t.c.flyout, None);
    assert!(t.c.barpanel.is_none());
}

#[test]
fn sound_panel_without_service_channel_is_read_only() {
    let mut t = T::new();
    t.frame();
    sound(&mut t);
    t.c.audio_ctl = false;
    t.frame();
    let vol = t.c.bar_layout().volume.unwrap();
    t.click(vol.x + 4, vol.y + 4);
    let n = tree(&t);
    assert!(n.iter().filter(|x| x.role == Role::Slider).all(|x| x.state & state::DISABLED != 0));
}

/// O7: nabidka zastupce aplikace v liste je nabidka knihovny ui (okna
/// s fajfkou aktivniho, Nove okno, vypnute Pripnout s duvodem, Zavrit
/// vsechna): klik na okno ho aktivuje, Zavrit vsechna posle `CLOSE`.
#[test]
fn app_shortcut_menu_is_ui_menu() {
    let mut t = T::new();
    t.c.set_launchable(alloc::vec![(String::from("org.term"), String::from("Terminál"))]);
    t.c.set_multi(alloc::vec![String::from("org.term")]);
    let a = t.client("org.term");
    t.window(a, Mode::Standard, RED);
    let b = t.client("org.term");
    t.window(b, Mode::Standard, BLUE);
    t.frame();
    let (ga, gb) = (t.c.clients[&a].windows[&1], t.c.clients[&b].windows[&1]);
    let bar = t.c.bar_layout();
    let r = bar.apps[0].1;
    t.click(r.x + 4, r.y + 4);
    assert!(matches!(t.c.flyout, Some(crate::flyout::Flyout::AppWindows(_))));
    let m = t.c.bar_panel_rect().expect("nabidka");
    // Obdelnik vcetne okraje stinu; karta je nad listou.
    let card = t.c.barpanel.as_ref().unwrap().ui.popup_rect().unwrap();
    assert!(m.y + card.bottom() as i32 <= t.c.panel_rect().y, "nad listou: {m:?} {card:?}");
    let items: Vec<guiproto::a11y::Node> = tree(&t).into_iter().filter(|n| n.role == Role::MenuItem).collect();
    let names: Vec<&str> = items.iter().map(|n| n.name.as_str()).collect();
    assert!(names.contains(&"Nové okno") && names.contains(&"Zavřít všechna okna"), "{names:?}");
    let pin = items.iter().find(|n| n.name.starts_with("Připnout na lištu")).unwrap_or_else(|| panic!("{names:?}"));
    assert_ne!(pin.state & state::DISABLED, 0);
    // Klik na prvni okno (nabidka je v souradnicich pohledu).
    // Prvni polozka = prvni okno aplikace (poradi `app_windows`).
    let first = t.c.app_windows("org.term")[0];
    assert!(first == ga || first == gb);
    let w0 = items[0].clone();
    t.click(m.x + w0.bounds.x + 8, m.y + w0.bounds.y + w0.bounds.h as i32 / 2);
    assert_eq!(t.c.flyout, None, "vyber nabidku zavre");
    assert_eq!(t.c.focus, Some(first));
    // Znovu: Zavrit vsechna okna.
    t.out();
    t.click(r.x + 4, r.y + 4);
    let m = t.c.bar_panel_rect().unwrap();
    let close = tree(&t).into_iter().find(|n| n.role == Role::MenuItem && n.name == "Zavřít všechna okna").unwrap();
    t.click(m.x + close.bounds.x + 8, m.y + close.bounds.y + close.bounds.h as i32 / 2);
    let out = t.out();
    for c in [a, b] {
        assert!(out.contains(&Out::Client(c, ServerMsg::Close { window: 1 })), "CLOSE pro {c:?}: {out:?}");
    }
}

/// O5: kalendar (dny v mesici, den v tydnu, posun mesicu) a panel hodin
/// s dneskem, listovanim a svetovym casem.
#[test]
fn clock_panel_calendar_and_world_time() {
    use crate::calendar::{CalendarInfo, days_in_month, first_weekday, shift_month};
    // 1. 11. 2026 je nedele, 1. 1. 2024 pondeli; unor 2024 prestupny.
    assert_eq!(first_weekday(2026, 11), 6);
    assert_eq!(first_weekday(2024, 1), 0);
    assert_eq!((days_in_month(2024, 2), days_in_month(2026, 2), days_in_month(2000, 2), days_in_month(1900, 2)), (29, 28, 29, 28));
    assert_eq!(shift_month(2026, 12, 1), (2027, 1));
    assert_eq!(shift_month(2026, 1, -1), (2025, 12));
    let mut t = T::new();
    t.frame();
    t.c.set_clock(Some((String::from("14:05"), String::from("čt 12. 11."))));
    t.c.set_calendar(Some(CalendarInfo { year: 2026, month: 11, day: 12, world: alloc::vec![(String::from("Londýn"), String::from("13:05"), String::from("-1 h"))] }));
    t.frame();
    let clock = t.c.bar_layout().clock.expect("hodiny v liste");
    t.click(clock.x + 4, clock.y + 4);
    assert_eq!(t.c.flyout, Some(crate::flyout::Flyout::Clock));
    let n = tree(&t);
    let has = |n: &[guiproto::a11y::Node], name: &str| n.iter().any(|x| x.name == name);
    assert!(has(&n, "listopad 2026") && has(&n, "Londýn") && has(&n, "13:05") && has(&n, "Po"), "{:?}", n.iter().map(|x| &x.name).collect::<Vec<_>>());
    let next = n.iter().find(|x| x.role == Role::Button && x.name == "Další měsíc").cloned().unwrap();
    let r = t.c.bar_panel_rect().unwrap();
    t.click(r.x + next.bounds.x + next.bounds.w as i32 / 2, r.y + next.bounds.y + next.bounds.h as i32 / 2);
    assert!(has(&tree(&t), "prosinec 2026"), "listovani");
    // Znovu otevreny panel zacina na dnesnim mesici.
    t.c.set_flyout(None);
    t.click(clock.x + 4, clock.y + 4);
    assert!(has(&tree(&t), "listopad 2026"));
}

/// O9: okno otevrene na pozadi hned po otevreni relace (automaticky
/// spustena aplikace) o pozornost nezada; pozdeji ano.
#[test]
fn background_windows_quiet_after_session_start() {
    let mut t = T::new();
    t.c.session_opened(1, 1000, "alice", t.now);
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let b = t.client("b");
    t.window(b, Mode::Standard, BLUE);
    let gb = t.c.clients[&b].windows[&1];
    assert!(!t.c.windows[&gb].attention, "tichy start");
    t.now += crate::launch::STARTUP_QUIET + 1;
    let c = t.client("c");
    t.window(c, Mode::Standard, GREEN);
    let gc = t.c.clients[&c].windows[&1];
    assert!(t.c.windows[&gc].attention, "pozdeji okno na pozadi zada o pozornost");
}

/// Panel Ulohy (O8): radky s prubehem a stitkem agenta, akce podle prav
/// od `taskd` jdou sluzbe (`Out::Task`), "Otevrit Spravce uloh" ho spusti.
#[test]
fn tasks_panel_rows_actions_and_manager() {
    use crate::{TaskAct, TaskRow};
    let mut t = T::new();
    t.c.set_tasks(2, 1);
    let row = |id: u64, name: &str, agent: bool, paused: bool, can: bool| TaskRow {
        id,
        name: String::from(name),
        who: String::from("Soubory"),
        state: String::from(if paused { "pozastaveno" } else { "běží" }),
        agent,
        progress: Some(40),
        paused,
        can_pause: can && !paused,
        can_resume: can && paused,
        can_cancel: can,
    };
    t.c.set_task_rows(Some(alloc::vec![row(7, "Kopírování", false, false, true), row(9, "Úklid", true, true, true), row(3, "Scrub", false, false, false)]));
    t.frame();
    t.out();
    let r = t.c.tasks_icon().unwrap();
    t.click(r.x + 4, r.y + 4);
    assert_eq!(t.c.flyout, Some(crate::flyout::Flyout::Tasks));
    let n = tree(&t);
    let btn = |name: &str| n.iter().find(|x| x.role == Role::Button && x.name == name).cloned().unwrap_or_else(|| panic!("{name}: {:?}", n.iter().map(|x| &x.name).collect::<Vec<_>>()));
    assert!(n.iter().any(|x| x.role == Role::Label && x.name.starts_with("agent")), "stitek agenta");
    assert_ne!(btn("Zrušit: Scrub").state & state::DISABLED, 0, "bez prava neaktivni");
    btn("Pokračovat: Úklid");
    let p = t.c.flyout_rect().unwrap();
    let b = btn("Pozastavit: Kopírování").bounds;
    t.click(p.x + b.x + b.w as i32 / 2, p.y + b.y + b.h as i32 / 2);
    assert!(t.out().contains(&Out::Task(TaskAct::Pause(7))));
    let n = tree(&t);
    let b = n.iter().find(|x| x.role == Role::Button && x.name == "Otevřít Správce úloh").unwrap().bounds;
    t.click(p.x + b.x + b.w as i32 / 2, p.y + b.y + b.h as i32 / 2);
    assert_eq!(t.c.take_launch_requests(), alloc::vec![String::from(crate::render::TASKS_APP)]);
    assert_eq!(t.c.flyout, None);
}

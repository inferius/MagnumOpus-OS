//! Snimkove testy systemovych pohledu nad knihovnou ui (paleta, spoustec)
//! ve stejnem ramci jako komponenty (`libs/ui/src/tests/shots.rs`):
//! reference v `libs/comp/tests/shots/`, tolerance, rozdil do
//! `target/shots/`, prepis jen `cargo xtask test --update-shots`.

extern crate std;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use std::path::PathBuf;

use super::*;

const TOL: u32 = 24;
const MAX_BAD: usize = 2;

fn ch(p: u32, s: u32) -> u32 {
    p >> s & 0xFF
}

/// Vyrez snimku obrazovky.
fn crop(t: &T, r: Rect) -> (Vec<u32>, u32, u32) {
    let r = r.intersect(&Rect::new(0, 0, W, H));
    let mut out = Vec::with_capacity((r.w * r.h) as usize);
    for y in r.y..r.y + r.h as i32 {
        for x in r.x..r.x + r.w as i32 {
            out.push(t.px(x, y) | 0xFF00_0000);
        }
    }
    (out, r.w, r.h)
}

fn compare(name: &str, px: &[u32], w: u32, h: u32) -> Option<String> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("shots");
    let file = dir.join(format!("{name}.png"));
    if std::env::var("UPDATE_SHOTS").is_ok_and(|v| v == "1") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, gfx::png::encode(px, w, h, w)).unwrap();
        return None;
    }
    let Ok(data) = std::fs::read(&file) else { return Some(format!("{name}: chybi reference (cargo xtask test --update-shots)")) };
    let Some((rw, rh, refpx)) = gfx::png::decode(&data) else { return Some(format!("{name}: vadne PNG")) };
    let bad = if (rw, rh) != (w, h) { px.len() } else { px.iter().zip(&refpx).filter(|(a, b)| [16, 8, 0].iter().any(|&s| ch(**a, s).abs_diff(ch(**b, s)) > TOL)).count() };
    if bad * 1000 > px.len() * MAX_BAD {
        let o = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
        let _ = std::fs::create_dir_all(&o);
        let _ = std::fs::write(o.join(format!("{name}-actual.png")), gfx::png::encode(px, w, h, w));
        return Some(format!("{name}: lisi se {bad} z {} pixelu", px.len()));
    }
    None
}

/// Paleta a spoustec ve svetlem, tmavem a vysokem kontrastu.
#[test]
fn shot_palette_and_launcher() {
    let mut errors = Vec::new();
    for (mode, mname) in [(theme::Mode::Light, "svetly"), (theme::Mode::Dark, "tmavy"), (theme::Mode::Contrast, "kontrast")] {
        for launcher in [false, true] {
            let mut t = T::new();
            t.c.set_mode(mode);
            let a = t.client("a");
            t.window(a, Mode::Standard, RED);
            t.frame();
            t.c.set_palette(Some(crate::palette::Palette { launcher, ..Default::default() }));
            t.frame();
            let r = t.c.palette_rect().unwrap();
            let (px, w, h) = crop(&t, r);
            let name = format!("{}-{mname}", if launcher { "spoustec" } else { "paleta" });
            errors.extend(compare(&name, &px, w, h));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// Panely listy nad komponentou panel (Vzhled, Zvuk s vystupem, vstupem,
/// nahravanim a aplikacemi) ve svetlem, tmavem a kontrastnim rezimu.
#[test]
fn shot_bar_panels() {
    use crate::render::{AudioApp, AudioDev, AudioInfo};
    let mut errors = Vec::new();
    for (mode, mname) in [(theme::Mode::Light, "svetly"), (theme::Mode::Dark, "tmavy"), (theme::Mode::Contrast, "kontrast")] {
        for (panel, pname) in [("look", "vzhled"), ("volume", "zvuk")] {
            let mut t = T::new();
            t.c.set_mode(mode);
            t.c.set_volume(Some((60, false)));
            t.c.set_capturing(alloc::vec![String::from("Hovory")]);
            t.c.audio_ctl = true;
            t.c.set_audio_info(Some(AudioInfo {
                devices: alloc::vec![
                    AudioDev { index: 0, name: String::from("Reproduktory"), outputs: String::from("line-out"), input: true, active: true },
                    AudioDev { index: 1, name: String::from("Sluchátka USB"), outputs: String::from("headphone"), input: false, active: false },
                ],
                apps: alloc::vec![
                    AudioApp { id: String::from("org.magnumopus.calls"), name: String::from("Hovory"), volume: 80, playing: true, capturing: true },
                    AudioApp { id: String::from("org.magnumopus.player"), name: String::from("Přehrávač"), volume: 45, playing: true, capturing: false },
                ],
                input: Some(0),
            }));
            let a = t.client("a");
            t.window(a, Mode::Standard, RED);
            t.frame();
            assert!(t.c.open_panel(panel));
            t.frame();
            let Some(r) = t.c.bar_panel_rect() else {
                errors.push(format!("panel {panel} se neotevrel"));
                continue;
            };
            let (px, w, h) = crop(&t, r);
            errors.extend(compare(&format!("panel-{pname}-{mname}"), &px, w, h));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// Nabidka zastupce aplikace v liste (O7) ve svetlem a tmavem rezimu.
#[test]
fn shot_app_shortcut_menu() {
    let mut errors = Vec::new();
    for (mode, mname) in [(theme::Mode::Light, "svetly"), (theme::Mode::Dark, "tmavy")] {
        let mut t = T::new();
        t.c.set_mode(mode);
        t.c.set_launchable(alloc::vec![(String::from("org.term"), String::from("Terminál"))]);
        t.c.set_multi(alloc::vec![String::from("org.term")]);
        for c in [RED, BLUE] {
            let a = t.client("org.term");
            t.window(a, Mode::Standard, c);
        }
        t.frame();
        let r = t.c.bar_layout().apps[0].1;
        t.click(r.x + 4, r.y + 4);
        t.frame();
        let Some(m) = t.c.bar_panel_rect() else {
            errors.push(String::from("nabidka se neotevrela"));
            continue;
        };
        let (px, w, h) = crop(&t, m);
        errors.extend(compare(&format!("nabidka-zastupce-{mname}"), &px, w, h));
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// Spoustec se sekcemi Bezi / Aplikace / Nedavne a patou s uctem (O13,
/// O14, K9) a paleta s prazdnym vysledkem (K8).
#[test]
fn shot_launcher_sections_and_empty() {
    let mut errors = Vec::new();
    for (scheme, mname) in [(theme::Scheme::Light, "svetly"), (theme::Scheme::Dark, "tmavy")] {
        let mut t = T::new();
        // Volba vzhledu systemu i uzivatele bez prechodu (relace ji meni).
        let p = theme::Pref { scheme, contrast: false, reduce_motion: true };
        t.c.set_appearance_system(p, t.now);
        t.c.set_appearance_user(1000, p, t.now);
        t.c.set_launchable(alloc::vec![
            (String::from("org.term"), String::from("Terminál")),
            (String::from("org.files"), String::from("Soubory")),
            (String::from("org.tasks"), String::from("Správce úloh")),
            (String::from("org.demo"), String::from("Ukázka")),
        ]);
        t.c.session_opened(1, 1000, "michal", t.now);
        let a = t.client("org.term");
        t.window(a, Mode::Standard, RED);
        t.frame();
        let g = t.c.clients[&a].windows[&1];
        t.c.palette_recent = alloc::vec![alloc::format!("win:{g}")];
        t.c.set_palette(Some(crate::palette::Palette { launcher: true, ..Default::default() }));
        t.frame();
        let r = t.c.palette_rect().unwrap();
        let (px, w, h) = crop(&t, r);
        errors.extend(compare(&format!("spoustec-sekce-{mname}"), &px, w, h));
        t.c.set_palette(Some(crate::palette::Palette { query: String::from("xyzzy"), ..Default::default() }));
        t.frame();
        let r = t.c.palette_rect().unwrap();
        let (px, w, h) = crop(&t, r);
        errors.extend(compare(&format!("paleta-prazdna-{mname}"), &px, w, h));
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// Panel hodin (O5): kalendar listopadu 2026 s dneskem a svetovy cas.
#[test]
fn shot_clock_panel() {
    let mut errors = Vec::new();
    for (mode, mname) in [(theme::Mode::Light, "svetly"), (theme::Mode::Dark, "tmavy")] {
        let mut t = T::new();
        t.c.set_mode(mode);
        t.c.set_clock(Some((String::from("14:05"), String::from("čt 12. 11."))));
        t.c.set_calendar(Some(crate::calendar::CalendarInfo {
            year: 2026,
            month: 11,
            day: 12,
            world: alloc::vec![(String::from("Londýn"), String::from("13:05"), String::from("-1 h")), (String::from("New York"), String::from("8:05"), String::from("-6 h"))],
        }));
        t.frame();
        assert!(t.c.open_panel("clock"));
        t.frame();
        let r = t.c.bar_panel_rect().unwrap();
        let (px, w, h) = crop(&t, r);
        errors.extend(compare(&format!("panel-hodiny-{mname}"), &px, w, h));
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// Bublina listy (K12): tucny titulek, popis pod nim, u spoustece stitek
/// klavesy Super.
#[test]
fn shot_bar_tooltip() {
    use crate::input::SysKey;
    use crate::tooltip::{DELAY, Tip};
    let mut errors = Vec::new();
    for (mode, mname) in [(theme::Mode::Light, "svetly"), (theme::Mode::Dark, "tmavy")] {
        for (tip, name) in [(Tip::Bar(SysKey::Tasks), "ulohy"), (Tip::Bar(SysKey::Launcher), "spoustec")] {
            let mut t = T::new();
            t.c.set_mode(mode);
            t.c.tasks = (3, 1);
            t.frame();
            let bar = t.c.bar_layout();
            let a = match tip {
                Tip::Bar(SysKey::Tasks) => bar.tasks.unwrap(),
                _ => bar.launcher,
            };
            t.move_to(a.x + 4, a.y + 4);
            t.now += DELAY + 1;
            t.c.timers(t.now);
            t.frame();
            assert_eq!(t.c.tooltip_shown(), Some(tip));
            let r = t.c.tip_rect().unwrap();
            assert!(r.h > t.c.style.theme.text.body_strong.line + 2 * t.c.style.theme.space[2], "dva radky");
            let (px, w, h) = crop(&t, r);
            errors.extend(compare(&format!("bublina-{name}-{mname}"), &px, w, h));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// Bezpecna obrazovka a potvrzeni vypnuti (O26): dlazdice zavaznosti
/// u nadpisu, vychozi (bezpecna) volba jako primarni tlacitko.
#[test]
fn shot_system_dialogs() {
    let mut errors = Vec::new();
    for (mode, mname) in [(theme::Mode::Light, "svetly"), (theme::Mode::Dark, "tmavy")] {
        for (name, power) in [("bezpecna", false), ("vypnout", true)] {
            let mut t = T::new();
            t.c.set_mode(mode);
            if power {
                t.c.confirm_power(false);
            } else {
                t.c.secure_attention(t.now);
            }
            t.frame();
            let d = t.c.dialogs.active().unwrap();
            assert_eq!(d.primary, d.sel, "vychozi volba je primarni");
            let l = t.c.dialog_layout().unwrap();
            let b = l.buttons[d.primary];
            assert_eq!(t.px(b.x + b.w as i32 / 2, b.y + 3), t.c.style.theme.color.accent, "primarni tlacitko accent");
            let (px, w, h) = crop(&t, l.frame);
            errors.extend(compare(&format!("dialog-{name}-{mname}"), &px, w, h));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// Centrum oznameni pod zvonkem: Nerusit, oznameni s akci, Vymazat vse.
#[test]
fn shot_bell_center() {
    use notifyproto::{Action, Notification, Priority, ViewReply};
    let mut errors = Vec::new();
    for (mode, mname) in [(theme::Mode::Light, "svetly"), (theme::Mode::Dark, "tmavy")] {
        let mut t = T::new();
        t.c.set_appearance_system(theme::Pref { reduce_motion: true, ..Default::default() }, t.now);
        t.c.set_mode(mode);
        t.c.session_opened(1, 1000, "jana", t.now);
        let n = |id: u64, title: &str, body: &str, app: &str, acts: &[(&str, &str)]| Notification {
            id,
            app_id: "org.test".into(),
            app_name: app.into(),
            title: title.into(),
            body: body.into(),
            priority: Priority::Low,
            actions: acts.iter().map(|(i, l)| Action { id: (*i).into(), label: (*l).into() }).collect(),
            time: id,
            popup: false,
            replaced: false,
            seen: false,
        };
        let list = alloc::vec![
            n(1, "Stahování dokončeno", "obraz-disku.img je ve složce Stažené.", "Stahování", &[("open", "Otevřít složku")]),
            n(2, "Médium připojeno", "MOTEST (FAT32, 64 MB)", "Systém", &[]),
        ];
        t.c.notify_event(1000, ViewReply::Snapshot { list, last: true }, t.now);
        t.frame();
        assert!(t.c.open_panel("bell"));
        t.frame();
        let r = t.c.bar_panel_rect().unwrap();
        let (px, w, h) = crop(&t, r);
        errors.extend(compare(&format!("panel-oznameni-centrum-{mname}"), &px, w, h));
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

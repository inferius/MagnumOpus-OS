//! Windows vrstva W4 (`docs/cs/windows-vrstva.md`, kapitola W4; sada
//! `wine`, beh s `--wine`, release): schranka z nasi aplikace do Wine,
//! obnova oken Wine po padu kompozitoru. Kompozitor a supervisor testu
//! sdili s W3 (`wine_gui.rs`).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::{Startup, println, time};

use guiproto::control::{INSPECT, WindowInfo};

use super::gui::{pixel, settle, windows};
use super::wine::{TESTS_APP, fs};
use super::wine_gui::{W3, finish, launch, launch_probe, save_candidates, settle_focus, wait_for, wait_window, wine_windows, with};
use super::{TestResult, fail};
use crate::{check, gui};

/// Znacky pro fyzicky vstup z QEMU (musi sedet s `xtask/src/qmp.rs`, `wine`).
const QEMU_OFFER_CLICK: &str = "init: test wine: klik do nabidky textu na ";
const QEMU_PASTE_WINE_CLICK: &str = "init: test wine: klik pro vlozeni ve Wine na ";
const QEMU_SAVE2_KEYS: &str = "init: test wine: uloz vlozeny text";
const QEMU_SAVE2_NAME: &str = "init: test wine: nazev vlozeneho souboru";

const TEXT: &str = "zdravim-z-magnumopus";

/// Schranka nase aplikace -> Wine: `gui-probe kopiruj` nabidne text po
/// fyzickem stisku (klik a klavesa z QEMU), poznamkovy blok ho vlozi
/// (Ctrl+V: zmena schranky = odlozene vlozeni, data az na zadost)
/// a ulozi; obsah souboru se porovna.
pub fn clipboard_to_wine(s: &Startup) -> TestResult {
    with(s, |w| {
        let candidates = save_candidates("u.txt")?;
        let (mut probe, (px, py)) = launch_probe(s, w, &["kopiruj", TEXT])?;
        println!("{QEMU_OFFER_CLICK}{px} {py}");
        wait_for(s, w, &mut probe, "nabidka textu", |_, r| Ok(String::from_utf8_lossy(&r.stdout).contains("nabidnuto").then_some(())))?;
        let mut run = launch(s, w, &["notepad"])?;
        let win = wait_window(s, w, &mut run, "Notepad")?;
        settle_focus(s, w, &mut run, "Notepad")?;
        println!("{QEMU_PASTE_WINE_CLICK}{} {}", win.rect.x + win.rect.w as i32 / 2, win.rect.y + win.rect.h as i32 / 2);
        // Chvili na vlozeni, pak Ctrl+S a nazev souboru v dialogu.
        let end = time::deadline_after(Duration::from_secs(3));
        while time::now() < end {
            run.drain();
            w.sup.step(s, Some(time::deadline_after(Duration::from_millis(50))));
        }
        println!("{QEMU_SAVE2_KEYS}");
        settle_focus(s, w, &mut run, "Save")?;
        time::sleep(Duration::from_millis(500));
        println!("{QEMU_SAVE2_NAME}");
        let fs = fs()?;
        let data = wait_for(s, w, &mut run, "ulozeny vlozeny text", |_, _| Ok(candidates.iter().find_map(|c| fs.read(c, 0, 4096).ok().filter(|d| !d.is_empty()))))?;
        let text = String::from_utf8_lossy(&data);
        check!(text.contains(TEXT), format!("vlozeny text v souboru: {text:?}"));
        finish(s, w, run, "Notepad")?;
        let _ = probe.process.kill();
        Ok(())
    })
}

const RED: u32 = 0xFFFF_0000;
const GREEN: u32 = 0xFF00_FF00;
const BLUE: u32 = 0xFF00_00FF;

/// Obsah vlastnika i dialogu nad nim navzorkovany po 10 px: body s barvou
/// ctverce vlastnika (cervena) a popupu dialogu (zelena; kompozitor kresli
/// popovery jen okna s fokusem). Popup lezi tam, kam ho ovladac Wine dal
/// (posun z programu je jen do hlaseni).
type Samples = Vec<(i32, i32, u32)>;

fn sample_owner(w: &W3, o: guiproto::Rect) -> Result<Samples, String> {
    let mut v = Vec::new();
    for y in (o.y + 5..o.y + o.h as i32).step_by(10) {
        for x in (o.x + 5..o.x + o.w as i32).step_by(10) {
            let p = pixel(&w.inspect, x, y)?;
            if p == RED || p == GREEN {
                v.push((x - o.x, y - o.y, p));
            }
        }
    }
    Ok(v)
}

/// Obdelnik bodu jedne barvy (v obsahu vlastnika), do hlaseni.
fn bbox(v: &Samples, c: u32) -> Option<(i32, i32, i32, i32)> {
    let pts: Vec<_> = v.iter().filter(|p| p.2 == c).collect();
    Some((pts.iter().map(|p| p.0).min()?, pts.iter().map(|p| p.1).min()?, pts.iter().map(|p| p.0).max()?, pts.iter().map(|p| p.1).max()?))
}

/// Obrazovka rezimu `wintest obnova`: modry dialog nad vlastnikem
/// (vystredeny, vlastnictvi = `WINDOW_OWNER`) s fokusem; obsah vlastnika
/// (cerveny ctverec) a zeleny popup dialogu (vrstva POPOVER) navzorkovany.
fn check_scene(w: &W3, popup: (i32, i32), when: &str) -> Result<(WindowInfo, WindowInfo, Samples), String> {
    let owner = wine_windows(w, "obnova-vlastnik")?.into_iter().next().ok_or_else(|| format!("{when}: okno vlastnika chybi"))?;
    let dlg = wine_windows(w, "obnova-dialog")?.into_iter().next().ok_or_else(|| format!("{when}: dialog chybi"))?;
    let (o, d) = (owner.rect, dlg.rect);
    // Seznam oken do hlaseni chyby (popup jako samostatne okno apod.).
    let all = || -> String {
        windows(&w.inspect).unwrap_or_default().iter().map(|x| format!("'{}' {:?}{}", x.title, x.rect, if x.focused { " fokus" } else { "" })).collect::<Vec<_>>().join(", ")
    };
    let (dx, dy) = (d.x + d.w as i32 / 2, d.y + d.h as i32 / 2);
    let end = time::deadline_after(Duration::from_secs(5));
    let mut p = pixel(&w.inspect, dx, dy)?;
    while p != BLUE && time::now() < end {
        time::sleep(Duration::from_millis(100));
        p = pixel(&w.inspect, dx, dy)?;
    }
    check!(p == BLUE, format!("{when}: dialog nad vlastnikem {p:08x} (dialog {d:?}, vlastnik {o:?}; okna: {})", all()));
    // Vystredeny nad vlastnikem (stred dialogu ve stredu vlastnika +/- 2 px).
    let (ox, oy) = (o.x + o.w as i32 / 2, o.y + o.h as i32 / 2);
    check!((dx - ox).abs() <= 2 && (dy - oy).abs() <= 2, format!("{when}: dialog {d:?} neni vystredeny nad vlastnikem {o:?}"));
    check!(dlg.focused && !owner.focused, format!("{when}: fokus ma mit dialog (dialog {}, vlastnik {})", dlg.focused, owner.focused));
    let v = sample_owner(w, o)?;
    println!(
        "init: test wine: {when}: cervena {:?}, zelena {:?} (posun popupu z Wine {popup:?}; okna: {})",
        bbox(&v, RED),
        bbox(&v, GREEN),
        all()
    );
    Ok((owner, dlg, v))
}

/// Posun popupu z vystupu programu (`popup <dx> <dy>`).
fn popup_offset(out: &[u8]) -> Option<(i32, i32)> {
    String::from_utf8_lossy(out).lines().find_map(|l| {
        let mut it = l.strip_prefix("popup ")?.split_whitespace().map(|v| v.parse::<i32>().ok());
        Some((it.next()??, it.next()??))
    })
}

fn paints(out: &[u8], name: &str) -> usize {
    String::from_utf8_lossy(out).lines().filter(|l| *l == format!("paint {name}")).count()
}

/// Obnova oken Wine po padu kompozitoru (`libs/mowin`, `os/restore.rs`):
/// vlastnik, vlastneny dialog a popup; kompozitor zabijeme, supervisor
/// testu ho spusti znovu, `mowin` se pripoji pres listek role Gui a posle
/// stav. Pak: okna zpet se stejnou identitou, pixely, dialog nad
/// vlastnikem a vystredeny (vlastnictvi preneseno), fokus na dialogu,
/// Wine okna prekreslil (`EXPOSE`) a program dal bezi.
pub fn recover_after_compd_crash(s: &Startup) -> TestResult {
    with(s, |w| {
        let exe = format!("{TESTS_APP}/wintest.exe");
        let mut run = launch(s, w, &[&exe, "obnova"])?;
        wait_for(s, w, &mut run, "kresleni oken", |_, r| Ok(String::from_utf8_lossy(&r.stdout).contains("nakresleno").then_some(())))?;
        wait_window(s, w, &mut run, "obnova-dialog")?;
        settle_focus(s, w, &mut run, "obnova-dialog")?;
        let popup = popup_offset(&run.stdout).ok_or_else(|| format!("program nevypsal polohu popupu; {}", run.text()))?;
        let (_, _, before_px) = check_scene(w, popup, "pred padem")?;
        check!(
            before_px.iter().any(|p| p.2 == RED) && before_px.iter().any(|p| p.2 == GREEN),
            format!("pred padem: chybi ctverec vlastnika nebo popup (cervena {:?}, zelena {:?}); {}", bbox(&before_px, RED), bbox(&before_px, GREEN), run.text())
        );
        let before = (paints(&run.stdout, "obnova-vlastnik"), paints(&run.stdout, "obnova-dialog"));
        let killed = w.sup.kill_for_test(gui::NAME).map_err(|e| fail("ukonceni kompozitoru", e))?;
        // Nova instance (supervisor testu), novy kanal inspekce, okna zpet.
        wait_for(s, w, &mut run, "okna po restartu kompozitoru", |w, _| {
            if w.sup.generation(gui::NAME).unwrap_or(0) <= killed {
                return Ok(None);
            }
            let found = match wine_windows(w, "obnova-") {
                Ok(v) => v,
                Err(_) => {
                    if let Ok(ch) = w.gui.channel(INSPECT.as_bytes()) {
                        w.inspect = ch;
                    }
                    return Ok(None);
                }
            };
            Ok((found.len() == 2 && found.iter().any(|x| x.focused)).then_some(()))
        })?;
        // Wine okna prekreslil (EXPOSE po obnove).
        wait_for(s, w, &mut run, "prekresleni po obnove", |_, r| {
            Ok((paints(&r.stdout, "obnova-vlastnik") > before.0 && paints(&r.stdout, "obnova-dialog") > before.1).then_some(()))
        })?;
        settle(&w.inspect)?;
        let (owner, dlg, after_px) = check_scene(w, popup, "po obnove")?;
        // Stejne body obsahu vlastnika i popupu maji po obnove stejnou barvu.
        let lost: Vec<_> = before_px.iter().filter(|p| !after_px.contains(p)).collect();
        check!(
            lost.is_empty(),
            format!("po obnove se lisi {} z {} bodu (cervena {:?} -> {:?}, zelena {:?} -> {:?})", lost.len(), before_px.len(), bbox(&before_px, RED), bbox(&after_px, RED), bbox(&before_px, GREEN), bbox(&after_px, GREEN))
        );
        let all: Vec<_> = wine_windows(w, "obnova-")?;
        check!(all.len() == 2, format!("po obnove prave dve okna: {all:?}"));
        check!(run.process.wait_exit(Some(0)).is_err(), format!("program po padu kompozitoru skoncil; {}", run.text()));
        println!("init: test wine: okna obnovena po padu kompozitoru (vlastnik {:?}, dialog {:?})", owner.rect, dlg.rect);
        finish(s, w, run, "obnova-")
    })
}

/// Ton pres WAV (`wintest ton`): winmm -> mmdevapi -> `winemoaudio.drv`
/// -> `moaudio` -> audiod. Program zapise WAV (1 s, 440 Hz) a prehraje ho
/// synchronne (`PlaySound`); behem prehravani ma audiod bezici stream
/// navic, prehrani trva aspon delku zaznamu a program skonci s 0.
pub fn tone_via_audiod(s: &Startup) -> TestResult {
    with(s, |w| {
        let status = crate::audio::connect().map(audioproto::conn::AudioClient::new).map_err(|e| fail("relace audiod", e))?;
        let base = status.status().map_err(|e| fail("STATUS", e))?.running;
        let exe = format!("{TESTS_APP}/wintest.exe");
        let mut run = launch(s, w, &[&exe, "ton", "ton.wav"])?;
        let mut most = base;
        let end = time::deadline_after(Duration::from_secs(120));
        let code = loop {
            run.drain();
            if let Ok(st) = status.status() {
                most = most.max(st.running);
            }
            if let Ok(c) = run.process.wait_exit(Some(0)) {
                run.drain();
                break c;
            }
            if time::now() > end {
                return Err(format!("ton: program neskoncil; {}", run.text()));
            }
            w.sup.step(s, Some(time::deadline_after(Duration::from_millis(20))));
        };
        let out = String::from_utf8_lossy(&run.stdout).into_owned();
        let ms = out.lines().find_map(|l| l.strip_prefix("ton: ok ")?.trim().parse::<u32>().ok());
        check!(code == 0 && ms.is_some(), format!("ton: kod {code}; {}", run.text()));
        let ms = ms.unwrap_or(0);
        check!(ms >= 800, format!("ton dohral prilis rychle ({ms} ms): nehral pres audiod; {}", run.text()));
        check!(most > base, format!("audiod behem tonu nemel stream navic (bezicich {most}, pred testem {base})"));
        println!("init: test wine: ton pres audiod {ms} ms (streamu {base} -> {most})");
        Ok(())
    })
}

/// PuTTY 0.85 (pinnuty zdroj, prelozeny v `xtask wine`, W4 varianta B):
/// spusteni bez argumentu otevre dialog konfigurace v rozumne velikosti
/// a s nakreslenym obsahem (svetle pozadi dialogu, tmavy text popisku
/// a stromu kategorii). Sit z programu Wine zatim neni (W4b), spojeni se
/// netestuje.
pub fn putty_config(s: &Startup) -> TestResult {
    with(s, |w| {
        let exe = format!("{TESTS_APP}/putty.exe");
        let mut run = launch(s, w, &[&exe])?;
        let win = wait_for(s, w, &mut run, "dialog PuTTY Configuration", |w, _| {
            Ok(wine_windows(w, "PuTTY Configuration")?.into_iter().find(|x| x.rect.w >= 300 && x.rect.h >= 300))
        })
        .map_err(|e| {
            let list: Vec<String> = windows(&w.inspect).unwrap_or_default().iter().map(|x| format!("{} '{}' {:?}", x.app, x.title, x.rect)).collect();
            format!("{e}; okna: {}", list.join(", "))
        })?;
        settle(&w.inspect)?;
        let r = win.rect;
        // Dialog se dokresluje po castech (strom kategorii, ovladaci prvky):
        // vzorkovat znovu, dokud nema text (nejdel 10 s).
        let end = time::deadline_after(Duration::from_secs(10));
        let (mut light, mut dark);
        loop {
            (light, dark) = (0u32, 0u32);
            for y in (r.y + 4..r.y + r.h as i32 - 4).step_by(6) {
                for x in (r.x + 4..r.x + r.w as i32 - 4).step_by(6) {
                    let p = pixel(&w.inspect, x, y)?;
                    let (cr, cg, cb) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
                    if cr >= 0xC0 && cg >= 0xC0 && cb >= 0xC0 {
                        light += 1;
                    } else if cr < 0x60 && cg < 0x60 && cb < 0x60 {
                        dark += 1;
                    }
                }
            }
            if (light > 500 && dark > 30) || time::now() > end {
                break;
            }
            run.drain();
            w.sup.step(s, Some(time::deadline_after(Duration::from_millis(200))));
        }
        check!(light > 500 && dark > 30, format!("obsah dialogu PuTTY: svetlych {light}, tmavych {dark} bodu (okno {r:?})"));
        println!("init: test wine: PuTTY Configuration {r:?} (svetlych {light}, tmavych {dark} bodu)");
        finish(s, w, run, "PuTTY")
    })
}

//! Port SDL 3 (docs/cs/sdl.md): programy SDL v C s oknem kompozitoru pres
//! `libs/mosdl` a ovladac videa `magnumopus`.
//!
//! - `testsprite`: softwarovy renderer; deterministicky snimek
//!   (`--iterations 0` = sprity stoji, seme 0), kontrola pevnych car
//!   a rohu v obraze kompozitoru,
//! - `checkkeys`: umela klavesnice a mys pres inputd -> kompozitor -> SDL;
//!   vypis udalosti programu (klavesa A se scancode 4, text "a", pravy klik),
//!   dvakrat Esc program ukonci.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use guiproto::control::INSPECT;
use inputproto::code::{BTN_RIGHT, KEY_A, KEY_ESC, REL_X, REL_Y};
use inputproto::device::{self, RawEvent};
use libos::abi::params::tag;
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use accountproto::client::{Login, Setup};
use accountproto::{Secret, ids};
use audioproto::session::{Capture, Recipe};
use libos::{Channel, Error, Startup, println, time};

use super::gui::{self as gt, State, pixel, settle, wait_window};
use super::std_app::{Started, start_std_app};
use super::{Test, TestResult, devices, fail};
use crate::gui;
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("sdl: testsprite v okne kompozitoru (softwarovy renderer, pixely)", testsprite),
    ("sdl: checkkeys - klavesnice, text a mys pres kompozitor", checkkeys),
    ("sdl: kurzor, zamek ukazatele, schranka a cela obrazovka (QMP)", pointer_clip_fullscreen),
    ("sdl: mereni cesty videa 1080p (YUV v SDL, kopie do okna)", video_bench),
    ("sdl: zvuk - prehravani, nahravani s povolenim, odvolani (QMP)", audio_permission),
    ("sdl: ukazkove hry snake a woodeneye-008", demo_games),
    // Role Audio (audio_role.rs): stejne jadro zvuku moaudio.
    ("sdl: role Audio - potomek s MO_AUDIO hraje, bez ni ne", super::audio_role::inherit_and_opt_out),
    ("sdl: role Audio - odvolani zvuku zastavi potomka", super::audio_role::revoke_reaches_children),
];

/// Znacky pro fyzicky vstup z QEMU (musi sedet s `xtask/src/qmp.rs`, `sdl`).
const QEMU_CLICK: &str = "init: test sdl: klik mysi na ";
const QEMU_MOVE: &str = "init: test sdl: ceka na posun mysi";
const QEMU_SUPER: &str = "init: test sdl: ceka na Super";
const QEMU_CLICK2: &str = "init: test sdl: druhy klik na ";
const QEMU_PASTE: &str = "init: test sdl: ceka na Ctrl+V";
const QEMU_FULL: &str = "init: test sdl: ceka na F";
const QEMU_ESC: &str = "init: test sdl: ceka na Esc";
const QEMU_PERM: &str = "init: test sdl: dialog povoleni ceka na Enter";

const DIR: &str = "/std-test/sdl";

/// Kompozitor a inspekce: sdilene se sadou `gui` (bezi-li po ni), jinak
/// vlastni start.
fn ensure_gui(startup: &Startup) -> TestResult {
    let mut g = gt::STATE.lock();
    if g.is_some() {
        return Ok(());
    }
    let mut gui = gui::start(startup).map_err(|e| fail("start kompozitoru", e))?;
    let input = devices().and_then(|d| d.input.as_ref()).ok_or("sluzba vstupu nebezi")?;
    gui.take_input(input).map_err(|e| fail("predani vstupu", e))?;
    let inspect = gui.channel(INSPECT.as_bytes()).map_err(|e| fail("inspect", e))?;
    *g = Some(State { gui, inspect, demo: 0 });
    Ok(())
}

/// Ikona testu SDL (z initfs) do FS, aby ji program nacetl pres fsd.
fn icon_to_fs(startup: &Startup) -> Result<String, String> {
    let fs = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    for d in ["/std-test", DIR] {
        match fs.create(d, fsproto::kind::DIR) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(fail(d, e)),
        }
    }
    let initfs = Initfs::open(startup).map_err(|e| fail("initfs", e))?;
    let data = initfs.find("sdl-icon.png").ok_or("sdl-icon.png neni v initfs")?;
    let path = format!("{DIR}/icon.png");
    let _ = fs.remove(&path);
    fs.create(&path, fsproto::kind::FILE).map_err(|e| fail("ikona: vytvoreni", e))?;
    fs.write(&path, 0, data).map_err(|e| fail("ikona: zapis", e))?;
    Ok(path)
}

/// Spusti program SDL s oknem (kanal od kompozitoru, listek), relaci FS
/// a stdout/stderr.
fn launch(startup: &Startup, s: &State, name: &str, args: &[&str]) -> Result<Started, String> {
    launch_audio(startup, s, name, args, None)
}

/// Jako [`launch`], navic relace zvuku s pravy aplikace (`tag::AUDIO`,
/// bez listku).
fn launch_audio(startup: &Startup, s: &State, name: &str, args: &[&str], audio: Option<&Recipe>) -> Result<Started, String> {
    let info = gui::recipe(startup, name).map_err(|e| fail("manifest", e))?;
    let window = s.gui.channel(&info.encode()).map_err(|e| fail("kanal okna", e))?;
    let (lease, _) = Channel::create().map_err(|e| fail("listek", e))?;
    let fs = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    let own = fs.connect().map_err(|e| fail("relace fsd", e))?;
    let extra = vec![
        StartHandle::user(window.into_handle()),
        StartHandle::user(lease.into_handle()),
        StartHandle::tagged(tag::FS, own.into_channel().into_handle()),
    ];
    let mut extra = extra;
    if let Some(r) = audio {
        let ch = crate::audio::app_channel(r).map_err(|e| fail("relace zvuku", e))?;
        extra.push(StartHandle::tagged(tag::AUDIO, ch.into_handle()));
    }
    start_std_app(startup, name, args, &["SDL_VIDEO_DRIVER=magnumopus"], extra)
}

fn testsprite(startup: &Startup) -> TestResult {
    ensure_gui(startup)?;
    let icon = icon_to_fs(startup)?;
    gt::with(|s| {
        let app = launch(startup, s, "testsprite", &["--iterations", "0", "1", &icon])?;
        let r = (|| {
            let w = wait_window(&s.inspect, "org.libsdl.testsprite")?;
            check!(w.rect.w >= 64 && w.rect.h >= 64, "rozmer okna");
            // Prvni snimek: chvili pockat, az program nakresli a kompozitor
            // ukaze (snimky jdou porad, staci kontrolovat opakovane).
            let (x0, y0) = (w.rect.x, w.rect.y);
            let (x1, y1) = (x0 + w.rect.w as i32 - 1, y0 + w.rect.h as i32 - 1);
            let end = time::deadline_after(Duration::from_secs(20));
            let mut last = Vec::new();
            loop {
                settle(&s.inspect)?;
                // Horni pravy roh cerveny bod, horni cara zelena, leva
                // modra, dolni cervena (testsprite.c, "Test points" a "Test
                // lines"). Dolni rohy okna kompozitor zaobluje: tam ne.
                let mid = x0 + w.rect.w as i32 / 2;
                let got = [pixel(&s.inspect, x1, y0)?, pixel(&s.inspect, mid, y0)?, pixel(&s.inspect, x0, y0 + w.rect.h as i32 / 2)?, pixel(&s.inspect, mid, y1)?];
                let want = [0xFFFF_0000, 0xFF00_FF00, 0xFF00_00FF, 0xFFFF_0000];
                if got.iter().zip(want).all(|(g, w)| *g & 0xFF_FFFF == w & 0xFF_FFFF) {
                    break;
                }
                last = got.to_vec();
                if time::now() > end {
                    return Err(format!("obraz testsprite: {last:08x?}, ocekavano {want:08x?}"));
                }
                time::sleep(Duration::from_millis(100));
            }
            let st = s.gui.status().map_err(|e| fail("status", e))?;
            check!(st.presented > 0, "kompozitor ukazal snimky");
            println!("init: test sdl: testsprite {}x{} na {},{}", w.rect.w, w.rect.h, w.rect.x, w.rect.y);
            Ok(())
        })();
        app.kill();
        r
    })
}

fn checkkeys(startup: &Startup) -> TestResult {
    ensure_gui(startup)?;
    let input = devices().and_then(|d| d.input.as_ref()).ok_or("sluzba vstupu nebezi")?;
    gt::with(|s| {
        let app = launch(startup, s, "checkkeys", &[])?;
        let w = match wait_window(&s.inspect, "org.libsdl.checkkeys") {
            Ok(w) => w,
            Err(e) => {
                app.kill();
                return Err(e);
            }
        };
        let inj = input.inject().map_err(|e| fail("inject", e))?;
        let send = |f: Vec<RawEvent>| inj.write(&device::encode_events(&f), Vec::new()).map_err(|e| fail("umely vstup", e));
        inj.write(&device::encode_hello(device::caps::KEYBOARD | device::caps::POINTER, "test sdl"), Vec::new()).map_err(|e| fail("predstaveni", e))?;
        // Okno ma po otevreni fokus; chvili pockat, az SDL zpracuje fokus.
        time::sleep(Duration::from_millis(500));
        let key = |code: u16| -> TestResult {
            send(vec![RawEvent::key(code, true), RawEvent::syn()])?;
            send(vec![RawEvent::key(code, false), RawEvent::syn()])
        };
        key(KEY_A)?;
        // Pravy klik do okna: checkkeys pri stisku prepne textovy vstup.
        let (sw, sh) = s.gui.size;
        let (tx, ty) = (w.rect.x + w.rect.w as i32 / 2, w.rect.y + w.rect.h as i32 / 2);
        send(vec![RawEvent::rel(REL_X, -(sw as i32) * 2), RawEvent::rel(REL_Y, -(sh as i32) * 2), RawEvent::syn()])?;
        send(vec![RawEvent::rel(REL_X, tx), RawEvent::rel(REL_Y, ty), RawEvent::syn()])?;
        send(vec![RawEvent::key(BTN_RIGHT, true), RawEvent::syn()])?;
        send(vec![RawEvent::key(BTN_RIGHT, false), RawEvent::syn()])?;
        time::sleep(Duration::from_millis(300));
        // Dvakrat Esc = konec programu.
        key(KEY_ESC)?;
        key(KEY_ESC)?;
        drop(inj);
        let run = app.finish(Duration::from_secs(20))?;
        let log = format!("{}{}", run.stdout, run.stderr);
        for line in log.lines().filter(|l| l.contains("Key") || l.contains("Text") || l.contains("text input")) {
            println!("init: test sdl: checkkeys: {line}");
        }
        check_eq!(run.code, 0, "checkkeys skoncil po dvou Esc");
        check!(log.contains("scancode 4 = A"), format!("klavesa A (scancode 4) v SDL; vystup: {}", log.trim()));
        check!(log.contains("Text") && log.contains("\"a\""), "text \"a\" v SDL");
        check!(log.contains("text input for window"), "klik mysi v okne SDL");
        Ok(())
    })
}

/// Kurzor aplikace (umely pohyb staci), pak fyzicky vstup z QEMU: klik =
/// zamek ukazatele, posun = relativni pohyb, Super = zruseni zamku, druhy
/// klik = kopie do schranky, Ctrl+V = vlozeni, F = cela obrazovka, Esc =
/// konec. Bez QMP (rucni beh) test skonci na prvni znacce chybou casu.
fn pointer_clip_fullscreen(startup: &Startup) -> TestResult {
    ensure_gui(startup)?;
    let input = devices().and_then(|d| d.input.as_ref()).ok_or("sluzba vstupu nebezi")?;
    gt::with(|s| {
        let mut app = launch(startup, s, "testmagnumopus", &[])?;
        let r = (|| -> TestResult {
            let t = Duration::from_secs(20);
            let ready = app.wait_line("ready", t)?;
            let w = wait_window(&s.inspect, "org.magnumopus.sdltest")?;
            // Skala a rezim displeje podle kompozitoru.
            let (sw, sh) = s.gui.size;
            check!(ready == format!("ready {sw} {sh} 1.00"), format!("displej SDL: {ready}, kompozitor {sw}x{sh}"));
            // Kurzor kriz nad oknem: umely pohyb ukazatele na stred obsahu.
            let (cx, cy) = (w.rect.x + w.rect.w as i32 / 2, w.rect.y + w.rect.h as i32 / 2);
            let inj = input.inject().map_err(|e| fail("inject", e))?;
            inj.write(&device::encode_hello(device::caps::POINTER, "test sdl"), Vec::new()).map_err(|e| fail("predstaveni", e))?;
            for f in [vec![RawEvent::rel(REL_X, -(sw as i32) * 2), RawEvent::rel(REL_Y, -(sh as i32) * 2), RawEvent::syn()], vec![RawEvent::rel(REL_X, cx), RawEvent::rel(REL_Y, cy), RawEvent::syn()]] {
                inj.write(&device::encode_events(&f), Vec::new()).map_err(|e| fail("umely vstup", e))?;
            }
            drop(inj);
            let end = time::deadline_after(t);
            loop {
                settle(&s.inspect)?;
                // Kriz: cerny stred a cerna cara nad nim, bily obrys vedle.
                let (c, up, side) = (pixel(&s.inspect, cx, cy)?, pixel(&s.inspect, cx, cy - 4)?, pixel(&s.inspect, cx + 1, cy - 4)?);
                if c & 0xFF_FFFF == 0 && up & 0xFF_FFFF == 0 && side & 0xFF_FFFF == 0xFF_FFFF {
                    break;
                }
                if time::now() > end {
                    return Err(format!("kurzor kriz: stred {c:08x}, cara {up:08x}, obrys {side:08x}"));
                }
                time::sleep(Duration::from_millis(100));
            }
            // Fyzicky klik do okna: zamek ukazatele.
            println!("{QEMU_CLICK}{} {}", cx, cy);
            check_eq!(app.wait_line("lock", t)?, "lock 1", "zamek ukazatele po fyzickem kliku");
            println!("{QEMU_MOVE}");
            let rel = app.wait_line("rel", t)?;
            check!(rel.starts_with("rel ") && rel != "rel 0 0", format!("relativni pohyb: {rel}"));
            println!("{QEMU_SUPER}");
            check_eq!(app.wait_line("lock", t)?, "lock 0", "Super zrusi zamek ukazatele");
            // Druhy klik: kopie; Ctrl+V: vlozeni pres kompozitor.
            println!("{QEMU_CLICK2}{} {}", cx, cy);
            check_eq!(app.wait_line("copy", t)?, "copy", "kopie do schranky po fyzickem kliku");
            println!("{QEMU_PASTE}");
            check_eq!(app.wait_line("paste", t)?, "paste magnumopus-sdl", "vlozeni ze schranky");
            println!("{QEMU_FULL}");
            check_eq!(app.wait_line("fullscreen", t)?, "fullscreen 1", "cela obrazovka po klavese F");
            let wf = wait_window(&s.inspect, "org.magnumopus.sdltest")?;
            check!(wf.rect.w == sw && wf.rect.h == sh, format!("okno na cele obrazovce: {:?}", wf.rect));
            println!("{QEMU_ESC}");
            Ok(())
        })();
        match r {
            Ok(()) => {
                let run = app.finish(Duration::from_secs(20))?;
                check_eq!(run.code, 0, "testmagnumopus skoncil po Esc");
                Ok(())
            }
            Err(e) => {
                app.kill();
                Err(e)
            }
        }
    })
}

/// Mereni dnesni cesty videa (docs/cs/sdl.md, Vykon): I420 1920x1080 ->
/// prevod v SDL -> okno. Jen vypis do logu (cislo patri do docs s revizi
/// sestaveni a podminkami behu); test hlida, ze mereni dobehne.
fn video_bench(startup: &Startup) -> TestResult {
    ensure_gui(startup)?;
    gt::with(|s| {
        let mut app = launch(startup, s, "testmagnumopus", &["bench"])?;
        let line = match app.wait_line("bench", Duration::from_secs(120)) {
            Ok(l) => l,
            Err(e) => {
                app.kill();
                return Err(e);
            }
        };
        let ms: f64 = line.trim_start_matches("bench ").parse().map_err(|_| format!("vystup mereni: {line}"))?;
        println!("init: test sdl: video 1080p I420 -> okno: {ms:.2} ms na snimek");
        let run = app.finish(Duration::from_secs(20))?;
        check_eq!(run.code, 0, "mereni skoncilo");
        check!(ms > 0.0, "mereni dalo cas");
        Ok(())
    })
}

/// Uzivatel pro rozhodnuti o povoleni: prvni clovek ve vypisu sluzby uctu,
/// jinak zalozi prvni ucet (sada `ucty` bezi pred `sdl` a zalozi vlastni).
fn test_uid() -> Result<u64, String> {
    let login = Login(crate::accounts::channel("login").map_err(|e| fail("kanal login", e))?);
    let list = login.list().map_err(|e| fail("vypis uctu", e))?;
    if let Some(a) = list.iter().find(|a| a.uid >= ids::FIRST_USER) {
        return Ok(a.uid);
    }
    let setup = Setup(crate::accounts::channel("setup").map_err(|e| fail("kanal setup", e))?);
    setup.create_first("sdltest", "SDL test", Secret::from("Zvuk jen s povolenim 42")).map_err(|e| fail("zalozeni uctu", e))
}

const SDL_APP: &str = "org.magnumopus.sdltest";

/// Zvuk pres SDL (ovladac `magnumopus`): ton dohraje; nahravani bez klice
/// v manifestu zamitnuto hned, s nim systemovy dialog (fyzicky Enter z QMP
/// = Povolit), pak ulozene rozhodnuti bez dialogu, vypis a odvolani
/// (prikaz `povoleni`, cesta initu).
fn audio_permission(startup: &Startup) -> TestResult {
    ensure_gui(startup)?;
    let uid = test_uid()?;
    // Duveryhodny dialog kompozitoru pro sluzbu uctu (pri behu plochy ho
    // preda init sam, v testu kompozitor spousti test).
    gt::with(|s| {
        let d = s.gui.channel(guiproto::control::DIALOG.as_bytes()).map_err(|e| fail("kanal dialogu", e))?;
        crate::accounts::give_dialog(d).map_err(|e| fail("dialog pro sluzbu uctu", e))
    })?;
    let _ = crate::perms::for_uid(uid, SDL_APP, "microphone");
    let recipe = |capture| Recipe { app_id: SDL_APP.into(), app_name: "SDL test MagnumOpus".into(), uid, capture };
    let t = Duration::from_secs(20);
    gt::with(|s| {
        // 1) Bez nahravani v manifestu: ton ano, nahravani ne (bez dialogu).
        let mut app = launch_audio(startup, s, "testmagnumopus", &["audio"], Some(&recipe(Capture::No)))?;
        check_eq!(app.wait_line("audio-driver", t)?, "audio-driver magnumopus", "ovladac zvuku SDL");
        check_eq!(app.wait_line("out", t)?, "out ok", "ton dohral pres audiod");
        check_eq!(app.wait_line("capture ", t)?, "capture 0", "nahravani bez manifestu zamitnuto");
        check_eq!(app.finish(t)?.code, 0, "konec programu");
        // 2) S nahravanim: dialog povoleni, fyzicky Enter = Povolit.
        let mut app = launch_audio(startup, s, "testmagnumopus", &["audio"], Some(&recipe(Capture::Ask)))?;
        app.wait_line("capture-ask", t)?;
        // Dialog prijima vstup az po zobrazeni a ochranne dobe.
        time::sleep(Duration::from_millis(500));
        settle(&s.inspect)?;
        time::sleep(Duration::from_millis(400));
        println!("{QEMU_PERM}");
        check_eq!(app.wait_line("capture ", t)?, "capture 1", "nahravani po povoleni");
        let data = app.wait_line("capture-data", t)?;
        check!(data != "capture-data 0", format!("nahrana data: {data}"));
        check_eq!(app.finish(t)?.code, 0, "konec programu");
        // 3) Ulozene rozhodnuti: bez dialogu.
        let mut app = launch_audio(startup, s, "testmagnumopus", &["audio"], Some(&recipe(Capture::Ask)))?;
        check_eq!(app.wait_line("capture ", Duration::from_secs(10))?, "capture 1", "ulozene povoleni bez dialogu");
        check_eq!(app.finish(t)?.code, 0, "konec programu");
        Ok(())
    })?;
    // 4) Vypis a odvolani (jako prikaz `permissions`).
    let list = crate::perms::for_uid(uid, "", "").map_err(|e| fail("vypis povoleni", e))?;
    check!(list.contains(&format!("{SDL_APP}\tmicrophone\tallow")), format!("vypis: {list}"));
    crate::perms::for_uid(uid, SDL_APP, "microphone").map_err(|e| fail("odvolani", e))?;
    let list = crate::perms::for_uid(uid, "", "").map_err(|e| fail("vypis po odvolani", e))?;
    check!(!list.contains(SDL_APP), format!("po odvolani: {list}"));
    Ok(())
}

/// Radek mrizky hry (barvy stredu bunek `cells` x `cell` px na radku `row`).
fn grid_row(s: &State, w: &guiproto::control::WindowInfo, cells: i32, cell: i32, row: i32) -> Result<Vec<u32>, String> {
    (0..cells).map(|i| pixel(&s.inspect, w.rect.x + i * cell + cell / 2, w.rect.y + row * cell + cell / 2)).collect()
}

/// Ukazkove hry SDL (examples/demo, rozhodnuti Q9 A): had se sam pohybuje
/// (radek s hlavou se mezi dvema okamziky zmeni), woodeneye-008 vykresli
/// scenu (obsah okna neni jednobarevny).
fn demo_games(startup: &Startup) -> TestResult {
    ensure_gui(startup)?;
    gt::with(|s| {
        let app = launch(startup, s, "snake", &[])?;
        let r = (|| -> TestResult {
            let w = wait_window(&s.inspect, "org.libsdl.snake")?;
            // 24 x 18 bunek po 24 px, hlava zacina uprostred a jede vpravo.
            check!(w.rect.w == 24 * 24 && w.rect.h == 18 * 24, format!("okno hada {}x{}", w.rect.w, w.rect.h));
            time::sleep(Duration::from_millis(500));
            settle(&s.inspect)?;
            let a = grid_row(s, &w, 24, 24, 9)?;
            time::sleep(Duration::from_millis(1000));
            settle(&s.inspect)?;
            let b = grid_row(s, &w, 24, 24, 9)?;
            check!(a != b, "had se pohnul");
            let colors = b.iter().collect::<alloc::collections::BTreeSet<_>>().len();
            check!(colors >= 2, "na radku je had i pozadi");
            println!("init: test sdl: had {}x{}, radek se meni", w.rect.w, w.rect.h);
            Ok(())
        })();
        app.kill();
        r?;
        let app = launch(startup, s, "woodeneye", &[])?;
        let r = (|| -> TestResult {
            let w = wait_window(&s.inspect, "org.libsdl.woodeneye")?;
            time::sleep(Duration::from_millis(1500));
            settle(&s.inspect)?;
            let mut seen = alloc::collections::BTreeSet::new();
            for i in 1..8 {
                for j in 1..6 {
                    seen.insert(pixel(&s.inspect, w.rect.x + w.rect.w as i32 * i / 8, w.rect.y + w.rect.h as i32 * j / 6)?);
                }
            }
            check!(seen.len() >= 3, format!("scena woodeneye: {} barev", seen.len()));
            println!("init: test sdl: woodeneye {}x{}, {} barev ve vzorku", w.rect.w, w.rect.h, seen.len());
            Ok(())
        })();
        app.kill();
        r
    })
}

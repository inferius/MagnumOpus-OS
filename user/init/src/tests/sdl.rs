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
use libos::{Channel, Error, Startup, println, time};

use super::gui::{self as gt, State, pixel, settle, wait_window};
use super::std_app::{Started, start_std_app};
use super::{Test, TestResult, devices, fail};
use crate::gui;
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("sdl: testsprite v okne kompozitoru (softwarovy renderer, pixely)", testsprite),
    ("sdl: checkkeys - klavesnice, text a mys pres kompozitor", checkkeys),
];

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

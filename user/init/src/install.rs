//! Instalator: start sluzby `installd` na prvni zadost pruvodce (`install`
//! v shellu) a kanaly pro pruvodce; nastaveni systemu z
//! `/system/config/system.toml` pri startu (pasmo, rozlozeni klavesnice).
//!
//! installd dostane raw kanaly vsech disku (zapis do cizich oddilu odmita
//! ovladac, `diskguard::install`). Init je predava jen jemu a jen jednou.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use installplan::config::{self, Settings};
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, println, time};

struct Installd {
    _process: Process,
    control: Channel,
}

static INSTALLD: spin::Mutex<Option<Installd>> = spin::Mutex::new(None);
/// Startovni zprava initu (kvuli initfs).
pub static STARTUP_INITFS: spin::Once<Initfs> = spin::Once::new();
/// VMO initfs (sluzby spoustene za behu, ktere samy spousteji programy
/// z initfs: sshd).
pub static STARTUP_INITFS_VMO: spin::Once<libos::Vmo> = spin::Once::new();

/// Kanal k instalatoru (sluzbu spusti, kdyz jeste nebezi).
pub fn open() -> Result<Channel> {
    let mut g = INSTALLD.lock();
    if g.is_none() {
        *g = Some(start()?);
    }
    let svc = g.as_ref().unwrap();
    let (ours, theirs) = Channel::create()?;
    svc.control.write(b"klient", alloc::vec![theirs.into_handle()])?;
    Ok(ours)
}

fn start() -> Result<Installd> {
    let devices = crate::system::devices().ok_or(Error::NotFound)?;
    let raw = core::mem::take(&mut *devices.raw.lock());
    if raw.is_empty() {
        println!("init: instalator: zadny disk s raw kanalem (NVMe, SATA, USB)");
    }
    let initfs = STARTUP_INITFS.get().ok_or(Error::NotFound)?;
    let image = initfs.find("installd").ok_or(Error::NotFound)?;
    let qemu = initfs.find("accounts-qemu").is_some();
    let mut cfg = String::from(if qemu { "qemu-kdf\n" } else { "hw\n" });
    let mut handles = Vec::new();
    for (ch, desc) in raw {
        cfg.push_str(&desc);
        cfg.push('\n');
        handles.push(StartHandle::user(ch.into_handle()));
    }
    let (process, control) = libos::spawn("installd", image, handles)?;
    control.write(cfg.as_bytes(), Vec::new())?;
    let ok = control.read(Some(time::deadline_after(Duration::from_secs(20))))?;
    if ok.data != b"ok" {
        return Err(Error::IoError);
    }
    println!("init: instalator (installd) bezi");
    // Instalace jako uloha v prehledu uloh (Spravce uloh, terminal).
    if let Err(e) = crate::tasks::give_producer(&control, "installd") {
        println!("init: prehled uloh pro installd: {e}");
    }
    Ok(Installd { _process: process, control })
}

/// Nastaveni z `/system/config/system.toml` (zapsal instalator): pasmo
/// a hodiny pro sluzbu casu, rozlozeni klavesnice pro sluzbu vstupu.
/// Chybi-li soubor (zive USB, starsi instalace), plati vychozi.
pub fn apply_settings(devices: &crate::system::Devices) {
    let Some(fs) = devices.fs.as_ref() else { return };
    let Ok(st) = fs.stat(config::PATH) else { return };
    let Ok(data) = fs.read(config::PATH, 0, st.size.min(16 << 10) as usize) else { return };
    let s = Settings::parse(&String::from_utf8_lossy(&data));
    if let Some(t) = crate::system::TIME.lock().as_ref() {
        let msg = format!("settings {}", s.time_settings());
        if t.channel.write(msg.as_bytes(), Vec::new()).is_ok() {
            let _ = t.channel.read(Some(time::deadline_after(Duration::from_secs(5))));
        }
    }
    if let Some(i) = devices.input.as_ref()
        && let Err(e) = i.set_layouts(&s.keyboards)
    {
        println!("init: rozlozeni klavesnice {:?}: {e}", s.keyboards);
    }
    println!("init: nastaveni systemu: pasmo {}, klavesnice {}", s.zone, s.keyboards.join(", "));
}

/// Soubor odpovedi testu instalace v initfs zive flashky (na obrazu pro
/// uzivatele neni).
fn answers() -> Option<String> {
    STARTUP_INITFS.get().and_then(|f| f.find("install-answers")).map(|a| String::from_utf8_lossy(a).into_owned())
}

/// Hodnota `rozhrani` v souboru odpovedi (`gui`, `gui-vstup`).
fn ui_mode(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let l = l.split('#').next().unwrap_or("").replace(' ', "");
        l.strip_prefix("rozhrani=").map(|v| String::from(v.trim_matches('"')))
    })
}

/// Test GUI instalace: odpovedi s `rozhrani = "gui"` projde graficky
/// instalator (autopilot), ne textova cesta.
fn gui_test(text: &str) -> bool {
    ui_mode(text).as_deref() == Some("gui")
}

/// Test skutecneho vstupu (`rozhrani = "gui-vstup"`): instalator bezi bez
/// autopilota a klika `cargo xtask` pres QMP. Init jen najde inspekci
/// kompozitoru pole casoveho pasma a tlacitko Pokracovat a zaloguje
/// jejich stred na obrazovce, takze klik nezavisi na rozliseni (aarch64
/// ramfb 800 x 600, x86_64 1280 x 800) ani na poloze okna.
fn input_test_targets() {
    let r = libos::thread::spawn("test-vstupu", || {
        if let Err(e) = find_targets() {
            println!("init: test vstupu instalatoru: {e}");
        }
    });
    if let Err(e) = r {
        println!("init: test vstupu instalatoru: vlakno: {e}");
    }
}

fn find_targets() -> core::result::Result<(), String> {
    use guiproto::a11y::{Node, Role};
    use guiproto::control::{INSPECT, InspectReply, InspectRequest};
    let end = time::deadline_after(Duration::from_secs(600));
    let pause = || time::sleep(Duration::from_millis(500));
    // Kompozitor startuje se supervisorem: kanal az bude.
    let ch = loop {
        let c = crate::gui::GUI.lock().as_ref().map(|g| g.channel(INSPECT.as_bytes()));
        match c {
            Some(Ok(ch)) => break ch,
            _ if time::now() > end => return Err(String::from("kompozitor nedal kanal inspect")),
            _ => pause(),
        }
    };
    let ask = |r: InspectRequest| -> core::result::Result<InspectReply, String> {
        while ch.try_read().is_ok() {}
        ch.write(&r.encode(), Vec::new()).map_err(|e| format!("inspekce: {e}"))?;
        let m = ch.read(Some(time::deadline_after(Duration::from_secs(10)))).map_err(|e| format!("inspekce: {e}"))?;
        InspectReply::decode(&m.data).map_err(|e| format!("inspekce: {e}"))
    };
    // Strom z inspekce ma obdelniky uz na obrazovce (vcetne posuvnych
    // vrstev a orezu, `Comp::screen_tree`).
    let center = |n: &Node| (n.bounds.x + n.bounds.w as i32 / 2, n.bounds.y + n.bounds.h as i32 / 2);
    loop {
        if let InspectReply::Windows(ws) = ask(InspectRequest::Windows)?
            && let Some(w) = ws.iter().find(|w| w.app == "org.magnumopus.install")
            && let InspectReply::Tree(t) = ask(InspectRequest::Tree { window: w.id })?
        {
            let zone = t.iter().find(|n| n.role == Role::TextInput && n.name == "Europe/Prague");
            let next = t.iter().find(|n| n.role == Role::Button && n.name == "Pokračovat");
            if let (Some(z), Some(n)) = (zone, next) {
                let (zx, zy) = center(z);
                let (nx, ny) = center(n);
                println!("init: test vstupu instalatoru: pasmo {zx},{zy}; pokracovat {nx},{ny}");
                return Ok(());
            }
        }
        if time::now() > end {
            return Err(String::from("okno instalatoru s polem pasma a tlacitkem Pokracovat se neobjevilo"));
        }
        pause();
    }
}

/// Handly pro graficky instalator: kanal k installd, v testu GUI instalace
/// navic kanal se souborem odpovedi.
pub fn app_handles() -> Result<Vec<StartHandle>> {
    let mut v = alloc::vec![StartHandle::user(open()?.into_handle())];
    if let Some(text) = answers().filter(|t| gui_test(t)) {
        let (ours, theirs) = Channel::create()?;
        ours.write(text.as_bytes(), Vec::new())?;
        v.push(StartHandle::user(theirs.into_handle()));
        println!("init: test GUI instalace: instalator dostal odpovedi");
    }
    Ok(v)
}

/// Test instalace (`cargo xtask test`, soubor `install-answers` v initfs
/// zive flashky): instalace bez dotazu podle odpovedi, stejne jako
/// `install --odpovedi`. S `rozhrani = "gui"` ji dela graficky
/// instalator (`app_handles`). Na obrazu pro uzivatele soubor neni.
pub fn test_hook() {
    use installplan::proto::{Reply, Request};
    let Some(text) = answers() else { return };
    match ui_mode(&text).as_deref() {
        Some("gui") => return,
        Some("gui-vstup") => return input_test_targets(),
        _ => {}
    }
    let run = || -> core::result::Result<String, String> {
        let ans = installplan::answers::parse(&text)?;
        let ch = open().map_err(|e| format!("installd: {e}"))?;
        let call = |req: &Request| -> core::result::Result<Reply, String> {
            ch.write(&req.encode(), Vec::new()).map_err(|e| format!("{e}"))?;
            loop {
                let m = ch.read(Some(time::deadline_after(Duration::from_secs(900)))).map_err(|e| format!("{e}"))?;
                match Reply::decode(&m.data).map_err(|e| format!("{e}"))? {
                    Reply::Progress { step, text } => println!("init: test instalace: krok {}: {text}", step + 1),
                    Reply::Failed(e, t) => return Err(format!("{t} ({e})")),
                    r => return Ok(r),
                }
            }
        };
        let Reply::Disks(list) = call(&Request::Disks)? else { return Err(String::from("disky")) };
        let d = list.iter().find(|d| d.model == ans.disk).ok_or_else(|| format!("disk '{}' nenalezen", ans.disk))?;
        let req = Request::Install { disk: d.index, mode: ans.mode, confirm: ans.confirm, settings: ans.settings, account: ans.account };
        match call(&req)? {
            Reply::Done(s) => Ok(s),
            _ => Err(String::from("neplatna odpoved")),
        }
    };
    match run() {
        Ok(s) => println!("init: test instalace: HOTOVO; {}", s.lines().collect::<Vec<_>>().join(" ")),
        Err(e) => println!("init: test instalace selhala: {e}"),
    }
}

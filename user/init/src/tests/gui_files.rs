//! Spravce souboru v systemu: okno `gui-files` s relaci FS uzivatele
//! a pomocny proces `files-copy`. Prochazeni, kopie pres schranku okna
//! jako uloha (okamzita kopie FS), konflikt "ponechat obe", kos, slozka
//! s 10 000 polozkami (cas otevreni, jen viditelne radky v pristupnosti),
//! sluzba "otevri cestu" (i z terminalu: `start`, hola cesta), panel
//! Vlastnosti s kartou Opravneni (zdedeny grant z fsd `ACL_VIEW`, pridani
//! sdileni a ulozeni pres `SET_ACL`) a snimek obrazovky pro kontrolu
//! vzhledu.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use fsproto::perm::{Acl, EVERYONE, Grant, Profile, grant, rights};
use fsproto::{FsClient, kind};
use guiproto::a11y::{Node, Role, actions};
use guiproto::action::Value;
use guiproto::control::*;
use libos::params::Params;
use libos::spawn::StartHandle;
use libos::{Channel, Startup, println, time};

use super::gui::{ask, has, node, settle, wait_tree, wait_window, with};
use super::{TestResult, devices, fail};
use crate::check;

const HOME: &str = "/home/souboryt";
const UID: u64 = 1005;
/// Jiny uzivatel se zdedenym pravem Zobrazit ve slozce `sdilene`.
const OTHER: u64 = 1006;
const BIG: usize = 10_000;

/// Znacky pro xtask (`xtask/src/qmp.rs`, modul `gui`): fyzicke klavesy
/// (schranka plati jen po fyzickem vstupu) a snimek.
const QEMU_CLICK: &str = "init: test gui: soubory klik mysi na ";
const QEMU_COPY: &str = "init: test gui: soubory ceka na Ctrl+C";
const QEMU_PASTE: &str = "init: test gui: soubory ceka na Ctrl+V";
const QEMU_PASTE_AGAIN: &str = "init: test gui: soubory ceka na dalsi Ctrl+V";
const QEMU_SHOT: &str = "init: test gui: snimek soubory";

/// Obrazek BMP 24 bitu `w` x `h` s prechodem barev.
fn bmp(w: u32, h: u32) -> Vec<u8> {
    let stride = (w * 3).div_ceil(4) * 4;
    let mut b = Vec::new();
    b.extend_from_slice(b"BM");
    for v in [54 + stride * h, 0, 54, 40, w, h] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&[0u8; 24]);
    for y in 0..h {
        for x in 0..stride {
            b.push(if x < w * 3 { ((x * 255 / (w * 3)) ^ (y * 255 / h)) as u8 } else { 0 });
        }
    }
    b
}

fn fs() -> Result<&'static FsClient, String> {
    devices().and_then(|d| d.fs.as_ref()).ok_or_else(|| "sluzba fsd nebezi".into())
}

/// Domov testovaciho uzivatele: dataset s pravy jen pro nej, soubor,
/// dataset Projekty a slozka s `BIG` polozkami.
fn prepare(f: &FsClient) -> TestResult {
    let _ = f.create("/home", kind::DIR);
    let _ = f.delete_dataset(&alloc::format!("{HOME}/Projekty"));
    let _ = f.delete_dataset(HOME);
    let walk = Acl { flags: 0, grants: vec![Grant { who: EVERYONE, rights: rights::TRAVERSE, flags: grant::THIS }] };
    f.set_acl("/", Some(&walk)).map_err(|e| fail("acl /", e))?;
    f.set_acl("/home", Some(&walk)).map_err(|e| fail("acl /home", e))?;
    f.create_dataset(HOME).map_err(|e| fail("domov", e))?;
    f.set_owner(HOME, UID).map_err(|e| fail("vlastnik domova", e))?;
    let own = Acl { flags: 0, grants: vec![Grant { who: UID, rights: rights::ALL, flags: grant::BOTH }] };
    f.set_acl(HOME, Some(&own)).map_err(|e| fail("acl domova", e))?;
    f.create(&alloc::format!("{HOME}/a.txt"), kind::FILE).map_err(|e| fail("a.txt", e))?;
    f.write(&alloc::format!("{HOME}/a.txt"), 0, b"obsah souboru\n").map_err(|e| fail("zapis a.txt", e))?;
    let pic = alloc::format!("{HOME}/obrazek.bmp");
    f.create(&pic, kind::FILE).map_err(|e| fail("obrazek", e))?;
    for (i, part) in bmp(160, 120).chunks(fsproto::MAX_IO).enumerate() {
        f.write(&pic, (i * fsproto::MAX_IO) as u64, part).map_err(|e| fail("zapis obrazku", e))?;
    }
    // Sdilena slozka: grant pro jineho uzivatele se dedi do souboru.
    let shared = alloc::format!("{HOME}/sdilene");
    f.create(&shared, kind::DIR).map_err(|e| fail("sdilene", e))?;
    let share = Acl { flags: 0, grants: vec![Grant { who: OTHER, rights: rights::VIEW, flags: grant::BOTH }] };
    f.set_acl(&shared, Some(&share)).map_err(|e| fail("acl sdilene", e))?;
    f.create(&alloc::format!("{shared}/zprava.txt"), kind::FILE).map_err(|e| fail("sdilene/zprava.txt", e))?;
    f.set_owner(&alloc::format!("{shared}/zprava.txt"), UID).map_err(|e| fail("vlastnik zpravy", e))?;
    f.create_dataset(&alloc::format!("{HOME}/Projekty")).map_err(|e| fail("Projekty", e))?;
    f.create(&alloc::format!("{HOME}/Projekty/plan.md"), kind::FILE).map_err(|e| fail("plan.md", e))?;
    let big = alloc::format!("{HOME}/velka");
    f.create(&big, kind::DIR).map_err(|e| fail("velka", e))?;
    let t = time::now();
    for i in 0..BIG {
        f.create(&alloc::format!("{big}/soubor {i}.txt"), kind::FILE).map_err(|e| fail("soubor velke slozky", e))?;
    }
    println!("init: test soubory: {BIG} souboru zalozeno za {} ms", (time::now() - t) / 1_000_000);
    f.commit().map_err(|e| fail("commit", e))?;
    Ok(())
}

fn params() -> Result<StartHandle, String> {
    Params { cwd: HOME.as_bytes().to_vec(), args: vec![], env: vec![], flags: 0 }.start_handle().map_err(|e| fail("parametry", e))
}

fn user_fs(f: &FsClient) -> Result<Channel, String> {
    Ok(f.connect_as(UID, &[], &Profile::Standard.user_mounts()).map_err(|e| fail("relace uzivatele", e))?.1.into_channel())
}

fn row<'a>(t: &'a [Node], name: &str) -> Option<&'a Node> {
    let pre = alloc::format!("{name}, ");
    t.iter().find(|n| n.role == Role::Row && n.name.starts_with(&pre))
}

fn act(s: &super::gui::State, w: u32, n: &Node) -> TestResult {
    ask(&s.inspect, InspectRequest::Act { window: w, node: n.id, action: actions::ACTIVATE, value: String::new() }).map(|_| ())
}

fn press(s: &super::gui::State, w: u32, name: &str) -> TestResult {
    let t = wait_tree(&s.inspect, w, name, |t| t.iter().any(|n| n.role == Role::Button && n.name == name && n.state & guiproto::a11y::state::DISABLED == 0))?;
    act(s, w, node(&t, Role::Button, name)?)
}

fn select(s: &super::gui::State, w: u32, name: &str) -> TestResult {
    let t = wait_tree(&s.inspect, w, name, |t| row(t, name).is_some())?;
    act(s, w, row(&t, name).unwrap())
}

pub fn files_window(startup: &Startup) -> TestResult {
    let f = fs()?;
    prepare(f)?;
    with(|s| {
        // files-copy jako v relaci (bez prehledu uloh) a okno spravce.
        let (ctl, theirs) = Channel::create().map_err(|e| fail("kanal", e))?;
        let copier = crate::apps::spawn_in(startup, "files-copy", vec![StartHandle::user(user_fs(f)?.into_handle()), StartHandle::user(theirs.into_handle()), params()?], None)
            .map_err(|e| fail("spusteni files-copy", e))?;
        let (mine, client) = Channel::create().map_err(|e| fail("kanal", e))?;
        ctl.write(b"client", vec![mine.into_handle()]).map_err(|e| fail("klient files-copy", e))?;
        let app = s
            .gui
            .launch_extra(startup, "gui-files", vec![StartHandle::user(user_fs(f)?.into_handle()), StartHandle::user(client.into_handle()), params()?])
            .map_err(|e| fail("spusteni gui-files", e))?;
        let r = run(startup, s, f);
        // Uklid: okno a pomocny proces pryc.
        if let Some(a) = s.gui.apps.get(app) {
            let _ = a.process.kill();
            let _ = a.process.wait_exit(Some(time::deadline_after(Duration::from_secs(5))));
        }
        if app + 1 == s.gui.apps.len() {
            drop(s.gui.apps.pop());
        }
        drop(ctl);
        let _ = copier.process.wait_exit(Some(time::deadline_after(Duration::from_secs(5))));
        let _ = copier.process.kill();
        r
    })?;
    f.delete_dataset(&alloc::format!("{HOME}/Projekty")).map_err(|e| fail("uklid Projekty", e))?;
    f.delete_dataset(HOME).map_err(|e| fail("uklid domova", e))?;
    Ok(())
}

fn run(startup: &Startup, s: &mut super::gui::State, f: &FsClient) -> TestResult {
    let w = wait_window(&s.inspect, "org.magnumopus.files")?.id;
    // Domov: polozky, mista a nahled vybraneho souboru.
    let t = wait_tree(&s.inspect, w, "vypis domova", |t| row(t, "a.txt").is_some() && row(t, "Projekty").is_some())?;
    check!(has(&t, Role::ListItem, "Projekty") && has(&t, Role::ListItem, "Koš"), "mista v postrannim panelu");
    check!(t.iter().any(|n| n.role == Role::ColumnHeader && n.name.starts_with("Velikost")), "zahlavi Velikost");
    select(s, w, "a.txt")?;
    wait_tree(&s.inspect, w, "nahled textu", |t| has(t, Role::Label, "obsah souboru"))?;
    // Kopirovat do Projektu klavesami (Ctrl+C / Ctrl+V z QEMU: App::key
    // a systemova schranka se seznamem cest), uloha files-copy.
    // Fyzicky klik na radek: vybere ho a da oknu fokus klavesnice.
    settle(&s.inspect)?;
    let t = super::gui::tree(&s.inspect, w)?;
    let r = row(&t, "a.txt").ok_or("radek a.txt")?;
    let (x, y) = (r.bounds.x + 40, r.bounds.y + r.bounds.h as i32 / 2);
    println!("{QEMU_CLICK}{x} {y}");
    let end = time::deadline_after(Duration::from_secs(20));
    while !super::gui::windows(&s.inspect)?.iter().any(|x| x.id == w && x.focused) {
        check!(time::now() < end, "okno Soubory nema fokus po kliku");
        time::sleep(Duration::from_millis(50));
    }
    settle(&s.inspect)?;
    println!("{QEMU_COPY}");
    wait_tree(&s.inspect, w, "Ctrl+C do schranky", |t| has(t, Role::Label, "Kopírovat: 1 položka ve schránce."))?;
    let t = wait_tree(&s.inspect, w, "misto Projekty", |t| has(t, Role::ListItem, "Projekty"))?;
    act(s, w, node(&t, Role::ListItem, "Projekty")?)?;
    wait_tree(&s.inspect, w, "vypis Projektu", |t| row(t, "plan.md").is_some())?;
    settle(&s.inspect)?;
    println!("{QEMU_PASTE}");
    // Gesto schranky je seriove cislo fyzickeho stisku (ne cas), takze
    // cesta pres schranku se overuje vzdy, i v QEMU bez akcelerace.
    wait_tree(&s.inspect, w, "kopie v Projektech", |t| row(t, "a.txt").is_some())?;
    let copied = f.read(&alloc::format!("{HOME}/Projekty/a.txt"), 0, 64).map_err(|e| fail("cteni kopie", e))?;
    check!(copied == b"obsah souboru\n", "obsah kopie");
    // Znovu: konflikt, ponechat obe.
    settle(&s.inspect)?;
    println!("{QEMU_PASTE_AGAIN}");
    wait_tree(&s.inspect, w, "konflikt", |t| has(t, Role::Label, "\"a.txt\" už v cíli je."))?;
    press(s, w, "Ponechat obě")?;
    wait_tree(&s.inspect, w, "druha kopie", |t| row(t, "a (2).txt").is_some())?;
    // Do kose.
    select(s, w, "a (2).txt")?;
    press(s, w, "Do koše")?;
    wait_tree(&s.inspect, w, "kos", |t| has(t, Role::Label, "1 položka v koši.") && row(t, "a (2).txt").is_none())?;
    check!(f.stat(&alloc::format!("{HOME}/Projekty/a (2).txt")).is_err(), "polozka v kosi");
    // Slozka s 10 000 polozkami pres akci aplikace (jako shell `open`):
    // dvakrat (studena a tepla), merit cas do zobrazeni radku.
    let ch = s.gui.channel(b"actions shell").map_err(|e| fail("kanal akci", e))?;
    let tl = time::now();
    let n = f.list_stat(&alloc::format!("{HOME}/velka")).map_err(|e| fail("list_stat", e))?.len();
    println!("init: test soubory: LIST_STAT_PAGE {n} polozek za {} ms (fsd)", (time::now() - tl) / 1_000_000);
    let show = |call: u32, path: &str| -> TestResult {
        let m = CallerMsg::Call { call, app: "org.magnumopus.files".into(), action: "files.show".into(), args: vec![("path".into(), Value::Path(String::from(path)))], synthetic: false };
        ch.write(&m.encode(), Vec::new()).map_err(|e| fail("akce files.show", e))?;
        let r = ch.read(super::gui::deadline()).map_err(|e| fail("odpoved akce", e))?;
        check!(matches!(CallerReply::decode(&r.data), Ok(CallerReply::Result { ok: true, .. })), "akce files.show");
        Ok(())
    };
    let big = alloc::format!("{HOME}/velka");
    for (k, what) in [(1u32, "poprve"), (3, "podruhe")] {
        let t0 = time::now();
        show(k, &big)?;
        let acted = (time::now() - t0) / 1_000_000;
        let t = wait_tree(&s.inspect, w, "velka slozka", |t| row(t, "soubor 0.txt").is_some())?;
        let ms = (time::now() - t0) / 1_000_000;
        let rows = t.iter().filter(|n| n.role == Role::Row).count();
        println!("init: test soubory: slozka s {BIG} polozkami {what}: akce {acted} ms, radky videt za {ms} ms, radku v pristupnosti {rows}");
        check!(rows < 200, alloc::format!("jen viditelne radky v pristupnosti: {rows}"));
        check!(row(&t, "soubor 1.txt").is_some() && row(&t, "soubor 2.txt").is_some(), "prirozene poradi");
        let t1 = time::now();
        show(k + 1, HOME)?;
        wait_tree(&s.inspect, w, "domov", |t| row(t, "a.txt").is_some())?;
        println!("init: test soubory: zpet do domova (3 polozky) za {} ms", (time::now() - t1) / 1_000_000);
    }
    properties(s, w, f, &ch, &show)?;
    open_service(startup, s, f)?;
    // Snimek vzhledu s nahledem obrazku (xtask porizuje pres QMP).
    select(s, w, "obrazek.bmp")?;
    wait_tree(&s.inspect, w, "nahled obrazku", |t| t.iter().any(|n| n.role == Role::Image && n.name == "Obrázek BMP 160 x 120, 24 bitů"))?;
    settle(&s.inspect)?;
    time::sleep(Duration::from_millis(1500));
    println!("{QEMU_SHOT}");
    time::sleep(Duration::from_millis(1500));
    Ok(())
}

/// Panel Vlastnosti souboru ve sdilene slozce (akce `files.properties`):
/// vlastnik, zdedeny grant s puvodem z fsd, pridani sdileni a ulozeni.
fn properties(s: &super::gui::State, w: u32, f: &FsClient, ch: &Channel, show: &dyn Fn(u32, &str) -> TestResult) -> TestResult {
    let file = alloc::format!("{HOME}/sdilene/zprava.txt");
    show(10, &file)?;
    wait_tree(&s.inspect, w, "soubor ve sdilene slozce", |t| row(t, "zprava.txt").is_some_and(|n| n.state & guiproto::a11y::state::SELECTED != 0))?;
    let m = CallerMsg::Call { call: 11, app: "org.magnumopus.files".into(), action: "files.properties".into(), args: vec![], synthetic: false };
    ch.write(&m.encode(), Vec::new()).map_err(|e| fail("akce files.properties", e))?;
    let r = ch.read(super::gui::deadline()).map_err(|e| fail("odpoved akce", e))?;
    check!(matches!(CallerReply::decode(&r.data), Ok(CallerReply::Result { ok: true, .. })), "akce files.properties");
    let t = wait_tree(&s.inspect, w, "karta Obecne", |t| has(t, Role::Label, "Vlastnosti: zprava.txt") && has(t, Role::Label, "Druh: Soubor"))?;
    act(s, w, node(&t, Role::Tab, "Oprávnění")?)?;
    let from = alloc::format!("Účet {OTHER}: Zobrazit (zděděno ze složky {HOME}/sdilene)");
    let owner = alloc::format!("Vlastník: Účet {UID} (vy)");
    let t = wait_tree(&s.inspect, w, "karta Opravneni", |t| has(t, Role::Label, &from) && has(t, Role::Label, &owner))?;
    check!(t.iter().any(|n| n.role == Role::Label && n.name == from && n.state & guiproto::a11y::state::DISABLED != 0), "zdedeny radek je nemenny");
    // Prvni kandidat bez jmen uctu: Uzivatele pocitace, uroven Zobrazit.
    press(s, w, "Přidat sdílení")?;
    press(s, w, "Uložit oprávnění")?;
    wait_tree(&s.inspect, w, "ulozeno", |t| has(t, Role::Label, "Oprávnění uložena."))?;
    let v = f.acl_view(&file).map_err(|e| fail("ACL_VIEW", e))?;
    let users = Grant { who: 1_000_000, rights: rights::VIEW, flags: grant::THIS };
    check!(v.perms.acl.as_ref().is_some_and(|a| a.grants == [users]), alloc::format!("ulozeny popis: {:?}", v.perms.acl));
    // Od korene: grant vlastnika z domova, pak grant ze sdilene slozky.
    let got: Vec<(u64, u8, &str)> = v.inherited.iter().map(|g| (g.who, g.rights, g.from.as_str())).collect();
    let shared = alloc::format!("{HOME}/sdilene");
    check!(got == [(UID, rights::ALL, HOME), (OTHER, rights::VIEW, shared.as_str())], alloc::format!("zdedene: {got:?}"));
    press(s, w, "Zavřít vlastnosti")?;
    wait_tree(&s.inspect, w, "zpet vypis", |t| row(t, "zprava.txt").is_some())?;
    show(12, HOME)?;
    wait_tree(&s.inspect, w, "domov po vlastnostech", |t| row(t, "a.txt").is_some())?;
    Ok(())
}

/// Obnova uloh `files-copy`: uloha ceka na rozhodnuti konfliktu, proces
/// skonci (jako restart systemu); novy proces stav nacte z domova, klient
/// po `Subscribe` vidi tutez ulohu a po rozhodnuti dobehne.
pub fn copier_resume(startup: &Startup) -> TestResult {
    use fileops::job::{Phase, Resolution};
    use fileops::proto::{ClientMsg, ServerMsg};
    let f = fs()?;
    let home = "/home/souborytr";
    let _ = f.create("/home", kind::DIR);
    let _ = f.delete_dataset(home);
    let walk = Acl { flags: 0, grants: vec![Grant { who: EVERYONE, rights: rights::TRAVERSE, flags: grant::THIS }] };
    f.set_acl("/", Some(&walk)).map_err(|e| fail("acl /", e))?;
    f.set_acl("/home", Some(&walk)).map_err(|e| fail("acl /home", e))?;
    f.create_dataset(home).map_err(|e| fail("domov", e))?;
    f.set_owner(home, UID).map_err(|e| fail("vlastnik", e))?;
    let own = Acl { flags: 0, grants: vec![Grant { who: UID, rights: rights::ALL, flags: grant::BOTH }] };
    f.set_acl(home, Some(&own)).map_err(|e| fail("acl", e))?;
    for p in ["a.txt", "cil/a.txt"] {
        let path = alloc::format!("{home}/{p}");
        if p.contains('/') {
            let _ = f.create(&alloc::format!("{home}/cil"), kind::DIR);
        }
        f.create(&path, kind::FILE).map_err(|e| fail("soubor", e))?;
        f.write(&path, 0, p.as_bytes()).map_err(|e| fail("zapis", e))?;
    }
    let start = |f: &FsClient| -> Result<(crate::apps::App, Channel, Channel), String> {
        let (ctl, theirs) = Channel::create().map_err(|e| fail("kanal", e))?;
        let p = Params { cwd: home.as_bytes().to_vec(), args: vec![], env: vec![], flags: 0 }.start_handle().map_err(|e| fail("parametry", e))?;
        let app = crate::apps::spawn_in(startup, "files-copy", vec![StartHandle::user(user_fs(f)?.into_handle()), StartHandle::user(theirs.into_handle()), p], None).map_err(|e| fail("files-copy", e))?;
        let (mine, client) = Channel::create().map_err(|e| fail("kanal", e))?;
        ctl.write(b"client", vec![mine.into_handle()]).map_err(|e| fail("klient", e))?;
        Ok((app, ctl, client))
    };
    let send = |c: &Channel, m: ClientMsg| c.write(&m.encode(), Vec::new()).map_err(|e| fail("zprava files-copy", e));
    let wait = |c: &Channel, what: &str, ok: &dyn Fn(&ServerMsg) -> bool| -> TestResult {
        let end = time::deadline_after(Duration::from_secs(20));
        loop {
            let m = c.read(Some(end)).map_err(|e| fail(what, e))?;
            if let Ok(s) = ServerMsg::decode(&m.data)
                && ok(&s)
            {
                return Ok(());
            }
        }
    };
    let (app, ctl, client) = start(f)?;
    send(&client, ClientMsg::Start { req: 1, op: fileops::job::Op::Copy, sources: vec![alloc::format!("{home}/a.txt")], dest: alloc::format!("{home}/cil") })?;
    wait(&client, "konflikt", &|s| matches!(s, ServerMsg::Status(st) if st.phase == Phase::Conflict))?;
    // "Restart": proces pryc bez rozhodnuti.
    let _ = app.process.kill();
    let _ = app.process.wait_exit(Some(time::deadline_after(Duration::from_secs(5))));
    drop((ctl, client));
    let (app, ctl, client) = start(f)?;
    send(&client, ClientMsg::Subscribe)?;
    let job = alloc::rc::Rc::new(core::cell::Cell::new(0u64));
    let j2 = job.clone();
    wait(&client, "uloha po obnove", &move |s| match s {
        ServerMsg::Status(st) if st.phase == Phase::Conflict && st.conflict.as_ref().is_some_and(|c| c.target.ends_with("/cil/a.txt")) => {
            j2.set(st.job);
            true
        }
        _ => false,
    })?;
    send(&client, ClientMsg::Decide { job: job.get(), resolution: Resolution::Rename, all: false })?;
    wait(&client, "dokonceni", &|s| matches!(s, ServerMsg::Status(st) if st.phase == Phase::Done))?;
    let copy = f.read(&alloc::format!("{home}/cil/a (2).txt"), 0, 16).map_err(|e| fail("kopie po obnove", e))?;
    check!(copy == b"a.txt", "obsah kopie po obnove");
    let left = f.list(&alloc::format!("{home}/apps/org.magnumopus.files/ulohy")).map_err(|e| fail("slozka stavu", e))?;
    check!(left.is_empty(), alloc::format!("stav hotove ulohy smazan: {:?}", left.iter().map(|e| e.name.clone()).collect::<Vec<_>>()));
    drop((ctl, client));
    let _ = app.process.wait_exit(Some(time::deadline_after(Duration::from_secs(5))));
    let _ = app.process.kill();
    f.delete_dataset(home).map_err(|e| fail("uklid", e))?;
    Ok(())
}

/// Sluzba "otevri cestu" (init, `libs/openwith`): slozka se otevre ve
/// spravci souboru, soubor bez aplikace spravce ukaze, neexistujici cesta
/// je chyba. Smycku supervisoru tu nahrazuje test (`open::poll`).
fn open_service(startup: &Startup, s: &mut super::gui::State, f: &FsClient) -> TestResult {
    use openwith::msg::{Reply, request};
    const SESSION: u64 = 0xfe11;
    let startup_less = |what: &str, req: &[u8]| -> Result<Reply, String> {
        let ep = crate::open::endpoint(SESSION).map_err(|e| fail("kanal sluzby", e))?;
        let (mine, theirs) = Channel::create().map_err(|e| fail("kanal", e))?;
        ep.write(req, vec![theirs.into_handle()]).map_err(|e| fail(what, e))?;
        let end = time::deadline_after(Duration::from_secs(20));
        loop {
            crate::open::poll_test();
            if let Ok(m) = mine.try_read() {
                return Reply::decode(&m.data).ok_or_else(|| alloc::format!("{what}: neplatna odpoved"));
            }
            check!(time::now() < end, alloc::format!("{what}: bez odpovedi"));
            time::sleep(Duration::from_millis(20));
        }
    };
    crate::open::test_session(SESSION, f.connect_as(UID, &[], &Profile::Standard.user_mounts()).map_err(|e| fail("relace", e))?.1).map_err(|e| fail("relace sluzby", e))?;
    crate::open::set_caller(s.gui.channel(b"actions system").map_err(|e| fail("kanal akci", e))?);
    let w = wait_window(&s.inspect, "org.magnumopus.files")?.id;
    let r = (|| -> TestResult {
        let dir = startup_less("slozka", &request(&alloc::format!("{HOME}/Projekty")))?;
        check!(dir == Reply::Opened(String::from("org.magnumopus.files")), alloc::format!("slozka: {dir:?}"));
        wait_tree(&s.inspect, w, "slozka otevrena sluzbou", |t| row(t, "plan.md").is_some())?;
        let file = startup_less("soubor", &request(&alloc::format!("{HOME}/a.txt")))?;
        check!(file == Reply::Revealed(String::from("org.magnumopus.files")), alloc::format!("soubor: {file:?}"));
        wait_tree(&s.inspect, w, "soubor ukazany ve spravci", |t| row(t, "a.txt").is_some_and(|n| n.state & guiproto::a11y::state::SELECTED != 0))?;
        // Jen aplikace (hola cesta v shellu): soubor bez aplikace se
        // neukaze, odpoved "neni aplikace".
        let app = startup_less("jen aplikace", &openwith::msg::request_app(&alloc::format!("{HOME}/a.txt")))?;
        check!(app == Reply::NoApp(String::from("txt")), alloc::format!("jen aplikace: {app:?}"));
        let none = startup_less("neexistujici", &request(&alloc::format!("{HOME}/neni.txt")))?;
        check!(matches!(&none, Reply::Failed(m) if m.contains("NOT_FOUND")), alloc::format!("neexistujici: {none:?}"));
        shell_start(startup, s, SESSION)
    })();
    crate::open::close_session(SESSION);
    r
}

/// Shell pres sluzbu: `start` = soubor bez aplikace ukaze spravce souboru
/// (poznamka v terminalu), hola cesta = jen poznamka "neni aplikace"
/// a souhrn obsahu (spravce se nevola). Okno terminalu jako v testu
/// terminalu, relace sluzby je testovaci (`session`).
fn shell_start(startup: &Startup, s: &mut super::gui::State, session: u64) -> TestResult {
    const NAME: &str = "gui-term (otevri)";
    let conn = crate::terminal::TERMINALS.lock().client_with_open(NAME, session).map_err(|e| fail("kanal terminalu", e))?;
    let app = s.gui.launch_extra(startup, "gui-term", vec![StartHandle::user(conn.into_handle())]).map_err(|e| fail("spusteni gui-term", e))?;
    let pump = || {
        crate::terminal::TERMINALS.lock().poll(startup);
        crate::open::poll_test();
    };
    let text = |t: &[Node]| t.iter().filter(|n| n.role == Role::Terminal).map(|n| n.value.clone()).collect::<Vec<_>>().join("\n");
    let r = (|| -> TestResult {
        let w = wait_window(&s.inspect, "org.magnumopus.terminal")?.id;
        let t = wait_tree(&s.inspect, w, "uvitani terminalu", |t| {
            pump();
            text(t).contains("Napište help")
        })?;
        let term = t.iter().find(|n| n.role == Role::Terminal).ok_or("terminal chybi")?.id;
        let line = |l: String| ask(&s.inspect, InspectRequest::Act { window: w, node: term, action: actions::SET_VALUE, value: l });
        line(alloc::format!("start {HOME}/a.txt"))?;
        wait_tree(&s.inspect, w, "start: soubor ukazany ve spravci", |t| {
            pump();
            text(t).contains("Pro typ .txt není aplikace. Soubor ukazuje org.magnumopus.files.")
        })?;
        line(alloc::format!("{HOME}/a.txt"))?;
        wait_tree(&s.inspect, w, "hola cesta: bez aplikace souhrn", |t| {
            pump();
            let v = text(t);
            v.lines().any(|l| l.trim() == "Pro typ .txt není aplikace.") && v.contains("obsah souboru")
        })?;
        Ok(())
    })();
    super::gui::close_terminal(startup, s, app, NAME)?;
    r
}

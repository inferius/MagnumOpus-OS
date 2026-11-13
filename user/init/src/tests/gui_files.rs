//! Spravce souboru v systemu: okno `gui-files` s relaci FS uzivatele
//! a pomocny proces `files-copy`. Prochazeni, kopie pres schranku okna
//! jako uloha (okamzita kopie FS), konflikt "ponechat obe", kos, slozka
//! s 10 000 polozkami (cas otevreni, jen viditelne radky v pristupnosti)
//! a snimek obrazovky pro kontrolu vzhledu.

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

use super::gui::{QEMU_SHOT, ask, has, node, settle, wait_tree, wait_window, with};
use super::{TestResult, devices, fail};
use crate::check;

const HOME: &str = "/home/souboryt";
const UID: u64 = 1005;
const BIG: usize = 10_000;

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
    Params { cwd: HOME.as_bytes().to_vec(), args: vec![], env: vec![] }.start_handle().map_err(|e| fail("parametry", e))
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
        let r = run(s, f);
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

fn run(s: &mut super::gui::State, f: &FsClient) -> TestResult {
    let w = wait_window(&s.inspect, "org.magnumopus.files")?.id;
    // Domov: polozky, mista a nahled vybraneho souboru.
    let t = wait_tree(&s.inspect, w, "vypis domova", |t| row(t, "a.txt").is_some() && row(t, "Projekty").is_some())?;
    check!(has(&t, Role::ListItem, "Projekty") && has(&t, Role::ListItem, "Koš"), "mista v postrannim panelu");
    check!(t.iter().any(|n| n.role == Role::ColumnHeader && n.name.starts_with("Velikost")), "zahlavi Velikost");
    select(s, w, "a.txt")?;
    wait_tree(&s.inspect, w, "nahled textu", |t| has(t, Role::Label, "obsah souboru"))?;
    // Kopirovat do Projektu: uloha files-copy (okamzita kopie FS).
    press(s, w, "Kopírovat")?;
    let t = wait_tree(&s.inspect, w, "misto Projekty", |t| has(t, Role::ListItem, "Projekty"))?;
    act(s, w, node(&t, Role::ListItem, "Projekty")?)?;
    wait_tree(&s.inspect, w, "vypis Projektu", |t| row(t, "plan.md").is_some())?;
    press(s, w, "Vložit (1)")?;
    wait_tree(&s.inspect, w, "kopie v Projektech", |t| row(t, "a.txt").is_some())?;
    let copied = f.read(&alloc::format!("{HOME}/Projekty/a.txt"), 0, 64).map_err(|e| fail("cteni kopie", e))?;
    check!(copied == b"obsah souboru\n", "obsah kopie");
    // Znovu: konflikt, ponechat obe.
    press(s, w, "Vložit (1)")?;
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
    // Snimek vzhledu (xtask porizuje pres QMP po znacce).
    select(s, w, "a.txt")?;
    wait_tree(&s.inspect, w, "nahled pred snimkem", |t| has(t, Role::Label, "obsah souboru"))?;
    settle(&s.inspect)?;
    time::sleep(Duration::from_millis(1500));
    println!("{QEMU_SHOT}soubory");
    time::sleep(Duration::from_millis(1500));
    Ok(())
}

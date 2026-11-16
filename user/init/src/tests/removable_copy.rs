//! Kopirovani a presun mezi domovem a vymennym mediem pres `files-copy`
//! (stroj Souboru, docs/cs/media.md): kopie ctenim a zapisem (medium
//! nema okamzitou kopii), prubeh, konflikt jmen, presun z media domu,
//! preruseni (pad procesu) a pokracovani. Vola ho test `vymenna media`
//! dokud je flash disk FAT pripojeny.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use fileops::job::{Op, Phase, Resolution};
use fileops::proto::{ClientMsg, ServerMsg, Status};
use fsproto::perm::{Acl, EVERYONE, Grant, Profile, grant, rights};
use fsproto::{FsClient, kind};
use libos::params::Params;
use libos::spawn::StartHandle;
use libos::{Channel, Startup, time};

use super::{TestResult, fail};
use crate::check;

const USER: u64 = 1000;
const HOME: &str = "/home/mediakopie";

/// Bajt testovaciho souboru na pozici `i` (obsah se generuje a overuje po
/// castech: init nedrzi megabajty v pameti, heap initu se nezmensuje).
fn byte(i: usize, seed: u8) -> u8 {
    (i as u32).wrapping_mul(2654435761).rotate_left(seed as u32) as u8
}

const PART: usize = 32 * 1024;

/// Zapise soubor `n` bajtu po castech.
fn write_file(f: &FsClient, path: &str, n: usize, seed: u8) -> TestResult {
    f.create(path, kind::FILE).map_err(|e| fail("soubor", e))?;
    let mut off = 0;
    while off < n {
        let part: Vec<u8> = (off..(off + PART).min(n)).map(|i| byte(i, seed)).collect();
        f.write(path, off as u64, &part).map_err(|e| fail("zapis", e))?;
        off += part.len();
    }
    Ok(())
}

/// Soubor ma presne `n` bajtu obsahu `seed` (cteni po castech).
fn same(fs: &FsClient, path: &str, n: usize, seed: u8) -> Result<bool, String> {
    let mut off = 0;
    loop {
        let part = fs.read(path, off as u64, PART).map_err(|e| fail(path, e))?;
        if part.is_empty() {
            return Ok(off == n);
        }
        if part.iter().enumerate().any(|(k, &b)| b != byte(off + k, seed)) {
            return Ok(false);
        }
        off += part.len();
    }
}

struct Copier {
    app: crate::apps::App,
    _ctl: Channel,
    client: Channel,
}

fn start(s: &Startup, f: &FsClient) -> Result<Copier, String> {
    let (ctl, theirs) = Channel::create().map_err(|e| fail("kanal", e))?;
    let user = f.connect_as(USER, &[], &Profile::Standard.user_mounts()).map_err(|e| fail("relace uzivatele", e))?.1;
    let p = Params { cwd: HOME.as_bytes().to_vec(), args: vec![], env: vec![], flags: 0, ..Default::default() }.start_handle().map_err(|e| fail("parametry", e))?;
    let app = crate::apps::spawn_in(s, "files-copy", vec![StartHandle::user(user.into_channel().into_handle()), StartHandle::user(theirs.into_handle()), p], None)
        .map_err(|e| fail("files-copy", e))?;
    let (mine, client) = Channel::create().map_err(|e| fail("kanal", e))?;
    ctl.write(b"client", vec![mine.into_handle()]).map_err(|e| fail("klient", e))?;
    Ok(Copier { app, _ctl: ctl, client })
}

impl Copier {
    fn send(&self, m: ClientMsg) -> TestResult {
        self.client.write(&m.encode(), Vec::new()).map_err(|e| fail("zprava files-copy", e))
    }

    /// Ceka na stav, pro ktery `ok` vrati true; vraci ho a vsechny stavy cestou.
    fn wait(&self, what: &str, ok: impl Fn(&Status) -> bool) -> Result<(Status, Vec<Status>), String> {
        let end = time::deadline_after(Duration::from_secs(120));
        let mut seen = Vec::new();
        loop {
            let m = self.client.read(Some(end)).map_err(|e| fail(what, e))?;
            if let Ok(ServerMsg::Status(st)) = ServerMsg::decode(&m.data) {
                if st.phase == Phase::Done && st.errors > 0 {
                    return Err(format!("{what}: uloha skoncila s chybou {:?}", st.last_error));
                }
                if ok(&st) {
                    return Ok((st, seen));
                }
                seen.push(st);
            }
        }
    }

    fn kill(self) {
        let _ = self.app.process.kill();
        let _ = self.app.process.wait_exit(Some(time::deadline_after(Duration::from_secs(5))));
    }
}

/// Domov testovaciho uzivatele (dataset s pravy jen pro nej). Prechod
/// pres `/` a `/home` uzivatel potrebuje; puvodni popis opravneni se vrati
/// na konci testu ([`run`]), jinak by ho videly dalsi sady (ucty).
fn prepare(f: &FsClient) -> TestResult {
    let _ = f.create("/home", kind::DIR);
    let _ = f.delete_dataset(HOME);
    let walk = Acl { flags: 0, grants: vec![Grant { who: EVERYONE, rights: rights::TRAVERSE, flags: grant::THIS }] };
    f.set_acl("/", Some(&walk)).map_err(|e| fail("acl /", e))?;
    f.set_acl("/home", Some(&walk)).map_err(|e| fail("acl /home", e))?;
    f.create_dataset(HOME).map_err(|e| fail("domov", e))?;
    f.set_owner(HOME, USER).map_err(|e| fail("vlastnik", e))?;
    let own = Acl { flags: 0, grants: vec![Grant { who: USER, rights: rights::ALL, flags: grant::BOTH }] };
    f.set_acl(HOME, Some(&own)).map_err(|e| fail("acl", e))
}

/// `medium` = jmeno media FAT v `/media`. Popis opravneni `/` a `/home`
/// vrati i po chybe.
pub fn run(s: &Startup, f: &FsClient, medium: &str) -> TestResult {
    let saved: Vec<(&str, Option<Acl>)> = ["/", "/home"].iter().map(|p| (*p, f.permissions(p).ok().and_then(|x| x.acl))).collect();
    let r = run_inner(s, f, medium);
    let _ = f.delete_dataset(HOME);
    for (p, acl) in &saved {
        if let Err(e) = f.set_acl(p, acl.as_ref()) {
            return Err(fail(&format!("vraceni opravneni {p}"), e));
        }
    }
    r
}

const BIG: usize = 3 << 20;
const HUGE: usize = 8 << 20;

fn run_inner(s: &Startup, f: &FsClient, medium: &str) -> TestResult {
    prepare(f)?;
    for (name, n, seed) in [("velky.bin", BIG, 3), ("obrovsky.bin", HUGE, 7)] {
        let p = format!("{HOME}/{name}");
        write_file(f, &p, n, seed)?;
        f.set_owner(&p, USER).map_err(|e| fail("vlastnik", e))?;
    }
    let (_, user) = f.connect_as(USER, &[], &Profile::Standard.user_mounts()).map_err(|e| fail("relace uzivatele", e))?;
    let dest = format!("/media/{medium}/Zkopírováno");
    user.create(&dest, kind::DIR).map_err(|e| fail("slozka na mediu", e))?;

    // 1. Kopie domov -> medium (cteni a zapis): obsah sedi.
    let c = start(s, f)?;
    c.send(ClientMsg::Start { req: 1, op: Op::Copy, sources: vec![format!("{HOME}/velky.bin")], dest: dest.clone() })?;
    // (Prubeh po castech overuje krok 4: velky soubor, stav "bezi" s bajty.)
    c.wait("kopie na medium", |st| st.phase == Phase::Done)?;
    check!(same(&user, &format!("{dest}/velky.bin"), BIG, 3)?, "obsah na mediu");

    // 2. Stejne jmeno: konflikt, "ponechat obe" = nove jmeno.
    c.send(ClientMsg::Start { req: 2, op: Op::Copy, sources: vec![format!("{HOME}/velky.bin")], dest: dest.clone() })?;
    let (st, _) = c.wait("konflikt na mediu", |st| st.phase == Phase::Conflict)?;
    c.send(ClientMsg::Decide { job: st.job, resolution: Resolution::Rename, all: false })?;
    c.wait("po konfliktu", |x| x.job == st.job && x.phase == Phase::Done)?;
    check!(user.stat(&format!("{dest}/velky (2).bin")).is_ok(), "druha kopie pod novym jmenem");

    // 3. Presun medium -> domov: na mediu zmizi, doma je.
    c.send(ClientMsg::Start { req: 3, op: Op::Move, sources: vec![format!("{dest}/velky (2).bin")], dest: String::from(HOME) })?;
    c.wait("presun z media", |st| st.op == Op::Move && st.phase == Phase::Done)?;
    check!(user.stat(&format!("{dest}/velky (2).bin")).is_err(), "na mediu po presunu neni");
    check!(same(&user, &format!("{HOME}/velky (2).bin"), BIG, 3)?, "doma po presunu");

    // 4. Preruseni: pad uprostred kopie, novy proces ulohu dokonci.
    c.send(ClientMsg::Start { req: 4, op: Op::Copy, sources: vec![format!("{HOME}/obrovsky.bin")], dest: dest.clone() })?;
    c.wait("kopie bezi", |st| st.phase == Phase::Running && st.done_bytes > 0)?;
    c.kill();
    let c = start(s, f)?;
    c.send(ClientMsg::Subscribe)?;
    c.wait("dokonceni po preruseni", |st| st.op == Op::Copy && st.phase == Phase::Done && st.dest == dest)?;
    check!(same(&user, &format!("{dest}/obrovsky.bin"), HUGE, 7)?, "obsah po preruseni a obnove");
    let tmp: Vec<String> = user.list(&dest).map_err(|e| fail("vypis", e))?.into_iter().map(|e| e.name).filter(|n| n.starts_with('.')).collect();
    check!(tmp.is_empty(), format!("docasne soubory po obnove: {tmp:?}").as_str());
    c.kill();
    // Na mediu nechat misto (dalsi kroky testu, kontrola na hostiteli).
    for n in ["velky.bin", "obrovsky.bin"] {
        let _ = user.remove(&format!("{dest}/{n}"));
    }
    Ok(())
}

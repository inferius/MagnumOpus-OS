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

fn data(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| (i as u32).wrapping_mul(2654435761).rotate_left(seed as u32) as u8).collect()
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

/// Domov testovaciho uzivatele (dataset s pravy jen pro nej).
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

fn read_all(fs: &FsClient, path: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let part = fs.read(path, out.len() as u64, 32 * 1024).map_err(|e| fail(path, e))?;
        if part.is_empty() {
            return Ok(out);
        }
        out.extend(part);
    }
}

/// `medium` = jmeno media FAT v `/media`.
pub fn run(s: &Startup, f: &FsClient, medium: &str) -> TestResult {
    prepare(f)?;
    let big = data(3 << 20, 3);
    let huge = data(8 << 20, 7);
    for (name, d) in [("velky.bin", &big), ("obrovsky.bin", &huge)] {
        let p = format!("{HOME}/{name}");
        f.create(&p, kind::FILE).map_err(|e| fail("soubor", e))?;
        f.write(&p, 0, d).map_err(|e| fail("zapis", e))?;
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
    check!(read_all(&user, &format!("{dest}/velky.bin"))? == big, "obsah na mediu");

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
    check!(read_all(&user, &format!("{HOME}/velky (2).bin"))? == big, "doma po presunu");

    // 4. Preruseni: pad uprostred kopie, novy proces ulohu dokonci.
    c.send(ClientMsg::Start { req: 4, op: Op::Copy, sources: vec![format!("{HOME}/obrovsky.bin")], dest: dest.clone() })?;
    c.wait("kopie bezi", |st| st.phase == Phase::Running && st.done_bytes > 0)?;
    c.kill();
    let c = start(s, f)?;
    c.send(ClientMsg::Subscribe)?;
    c.wait("dokonceni po preruseni", |st| st.op == Op::Copy && st.phase == Phase::Done && st.dest == dest)?;
    check!(read_all(&user, &format!("{dest}/obrovsky.bin"))? == huge, "obsah po preruseni a obnove");
    let tmp: Vec<String> = user.list(&dest).map_err(|e| fail("vypis", e))?.into_iter().map(|e| e.name).filter(|n| n.starts_with('.')).collect();
    check!(tmp.is_empty(), format!("docasne soubory po obnove: {tmp:?}").as_str());
    c.kill();
    let _ = f.delete_dataset(HOME);
    Ok(())
}

//! fsd: sluzba souboroveho systemu.
//!
//! Startovni zprava (kind USER, v tomto poradi):
//! 1. kanal k ovladaci disku (protokol `blockproto`),
//! 2. kanal pro klienta (protokol `fsproto`).
//!
//! Prvni zprava na bootstrap kanalu: konfigurace, slova oddelena mezerou
//! (`debug` = povolit testovaci operace). Sluzba pak:
//! - obsluhuje pozadavky klienta,
//! - kazdych `COMMIT_INTERVAL` ulozi zmeny commitem (fsync klienta je
//!   trvaly okamzite pres intent log),
//! - kdyz je necinna, defragmentuje po malych krocich,
//! - posila disku TRIM pro uvolnene bloky.
//!
//! Kdyz klient kanal zavre, ulozi zmeny a skonci.

#![no_std]
#![no_main]

extern crate alloc;

mod blkdev;
mod serve;
mod serve_hist;
mod serve_perm;
mod serve_v2;

use alloc::vec::Vec;
use core::time::Duration;

use blockproto::BlockClient;
use fs::Fs;
use libos::{Channel, Error, Signals, Startup, println, time};

use crate::blkdev::BlkDev;
use crate::serve::Service;
use crate::serve_v2::{MAX_SESSIONS, Session, SessionOp};

/// Jak casto ukladat zmeny commitem.
const COMMIT_INTERVAL: Duration = Duration::from_secs(5);
/// Jak casto delat kroky historie (automaticke snapshoty, expirace, kos).
const HISTORY_INTERVAL: Duration = Duration::from_secs(60);
/// Kolik bloku prepsat jednim krokem defragmentace v necinnosti.
const DEFRAG_BUDGET: u64 = 256;

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    let mut handles = startup.handles.into_iter();
    let (Some(block), Some(client)) = (handles.next(), handles.next()) else {
        println!("fsd: chybi handly ve startovni zprave");
        return 1;
    };
    let config = startup
        .bootstrap
        .read(None)
        .map(|m| m.data)
        .unwrap_or_default();
    let debug = core::str::from_utf8(&config)
        .unwrap_or("")
        .split_whitespace()
        .any(|w| w == "debug");
    let client = Channel::from_handle(client);
    let disk = match BlockClient::new(Channel::from_handle(block)) {
        Ok(c) => BlkDev::new(c),
        Err(e) => {
            println!("fsd: disk neodpovida: {e}");
            return 2;
        }
    };
    let mut fs = match Fs::mount(disk) {
        Ok(f) => f,
        Err(e) => {
            println!("fsd: souborovy system nejde pripojit: {e}");
            return 3;
        }
    };
    fs.set_discard(true);
    let i = fs.info();
    println!(
        "fsd: pripojeno '{}': {} MiB, volno {} MiB, generace {}, soucty {:?}",
        i.label,
        i.block_count * i.block_size as u64 / (1024 * 1024),
        i.free_blocks * i.block_size as u64 / (1024 * 1024),
        i.generation,
        i.algo
    );
    if i.fallback > 0 {
        println!(
            "fsd: VAROVANI: nejnovejsi stav byl poskozeny, pouzit stav o {} generaci starsi",
            i.fallback
        );
    }
    if i.replayed > 0 {
        println!("fsd: z intent logu prehrano {} operaci", i.replayed);
    }
    if i.unreadable_superblocks > 0 && !i.newest_uncertain {
        println!(
            "fsd: VAROVANI: {} slotu superbloku nejde precist (vadny sektor?) - nejnovejsi stav je jisty, pokracuji",
            i.unreadable_superblocks
        );
    }
    // Kazdy duvod zvlast (muze jich platit vic najednou).
    if i.newest_uncertain {
        println!(
            "fsd: VAROVANI: nejde precist slot, kde by lezel novejsi stav - nejnovejsi stav nejde bezpecne urcit, svazek je jen pro cteni"
        );
    }
    if i.log_unreadable {
        println!("fsd: VAROVANI: intent log nejde precist cely - log zachovan pro dalsi pokus, svazek je jen pro cteni");
    }
    if i.read_only && !i.newest_uncertain && !i.log_unreadable {
        println!(
            "fsd: VAROVANI: svazek je jen pro cteni (disk nezarucuje trvanlivost zapisu nebo neznama vlastnost formatu)"
        );
    }
    let mut svc = Service {
        fs: Some(fs),
        debug,
        temp_counter: 0,
        commit_requested: false,
        session_counter: 0,
    };
    serve(&mut svc, client);
    if let Some(fs) = svc.fs.as_mut()
        && let Err(e) = fs.commit()
    {
        println!("fsd: zaverecny commit selhal: {e}");
        return 4;
    }
    0
}

/// Obsluha relaci: prvni (ze startovni zpravy) je hlavni - jeji zavreni
/// sluzbu ukonci; dalsi vznikaji pres `CONNECT`.
fn serve(svc: &mut Service, client: Channel) {
    let mut sessions: Vec<Session> = alloc::vec![Session::control(client)];
    let mut next_commit = time::deadline_after(COMMIT_INTERVAL);
    let mut next_history = time::deadline_after(HISTORY_INTERVAL);
    // Defragmentovat jen po zmenach (jinak by se system prochazel stale dokola).
    let mut idle = false;
    let mut changed = false;
    let mut running = true;
    while running {
        // V necinnosti (po commitu bez novych pozadavku) defragmentovat.
        let mut deadline = if idle { time::now() } else { next_commit.min(next_history) };
        for s in &sessions {
            if let Some(w) = &s.watch {
                deadline = deadline.min(w.deadline);
            }
        }
        let mask = Signals::READABLE | Signals::PEER_CLOSED;
        let items: Vec<(&libos::Handle, Signals)> =
            sessions.iter().map(|s| (s.channel.handle(), mask)).collect();
        match libos::wait_many(&items, Some(deadline)) {
            Ok(_) => {
                let mut i = 0;
                while i < sessions.len() {
                    match serve_session(svc, &mut sessions, i) {
                        Some(true) => {
                            idle = false;
                            changed = true;
                            i += 1;
                        }
                        Some(false) => i += 1,
                        None => {
                            // Relace skoncila: zavrit jeji handly.
                            let mut s = sessions.remove(i);
                            svc.close_session(&mut s);
                            if i == 0 {
                                running = false;
                                break;
                            }
                        }
                    }
                }
            }
            Err(Error::TimedOut) => {}
            Err(e) => {
                println!("fsd: chyba cekani: {e}");
                break;
            }
        }
        if running {
            background(svc, &mut next_commit, &mut idle, &mut changed);
            history(svc, &mut next_history);
            poll_watches(svc, &mut sessions);
        }
    }
    for mut s in sessions {
        svc.close_session(&mut s);
    }
}

/// Zpracuje vsechny cekajici pozadavky relace `i`. `Some(true)` = neco
/// zpracovala, `None` = relace skoncila (kanal zavreny).
fn serve_session(svc: &mut Service, sessions: &mut Vec<Session>, i: usize) -> Option<bool> {
    let mut did = false;
    loop {
        let msg = match sessions[i].channel.try_read() {
            Ok(m) => m,
            Err(Error::ShouldWait) => return Some(did),
            Err(_) => return None,
        };
        did = true;
        // Klient poslal dalsi pozadavek, zatimco cekal na `JOURNAL_WATCH`:
        // nejdriv odpoved na cekani (ted, i kdyz prazdna) - poradi odpovedi
        // musi sedet s poradim pozadavku.
        if let Some(w) = sessions[i].watch.take()
            && let Some(res) = svc.poll_watch(&sessions[i], &w, true)
            && let Err(Error::PeerClosed) = sessions[i].channel.write(&fsproto::reply(res), Vec::new())
        {
            return None;
        }
        let (reply, handles, new_session, ops, watch) = match svc.handle(&mut sessions[i], &msg.data) {
            Ok(r) => (fsproto::reply(Ok(r.body)), r.handles, r.new_session, r.session_ops, r.watch),
            Err(e) => (fsproto::reply(Err(e)), Vec::new(), None, Vec::new(), None),
        };
        if let Some(w) = watch {
            // Odpoved az po dalsim potvrzeni zmen nebo po limitu.
            sessions[i].watch = Some(w);
            return Some(did);
        }
        // Zmeny jinych relaci (ridici relace: GRANT/REVOKE).
        let mut reply = reply;
        for op in ops {
            let (id, ok) = match &op {
                SessionOp::Grant(id, _) | SessionOp::Revoke(id, _) => (*id, sessions.iter().any(|s| s.id == *id && !s.control)),
            };
            if !ok {
                reply = fsproto::reply(Err(Error::NotFound));
                continue;
            }
            let s = sessions.iter_mut().find(|s| s.id == id).unwrap();
            match op {
                SessionOp::Grant(_, m) => {
                    s.mounts.retain(|x| x.name != m.name);
                    s.mounts.push(m);
                }
                SessionOp::Revoke(_, name) => s.mounts.retain(|x| x.name != name),
            }
        }
        let (reply, handles) = match new_session {
            // Prilis mnoho relaci: novy kanal zahodit, klientovi chyba.
            Some(_) if sessions.len() >= MAX_SESSIONS => (fsproto::reply(Err(Error::NoResources)), Vec::new()),
            Some(s) => {
                sessions.push(s);
                (reply, handles)
            }
            None => (reply, handles),
        };
        // Chyba odeslani odpovedi nesmi ukoncit sluzbu (a s ni svazek bez
        // zaverecneho commitu): kdyz se odpoved nevejde, posle se misto ni
        // chyba. Konci se jen pri zavrenem kanalu.
        match sessions[i].channel.write(&reply, handles) {
            Ok(()) => {}
            Err(Error::PeerClosed) => return None,
            Err(e) => {
                println!("fsd: odpoved nejde odeslat ({e}), posilam chybu");
                let fallback = fsproto::reply(Err(Error::BufferTooSmall));
                if let Err(Error::PeerClosed) = sessions[i].channel.write(&fallback, Vec::new()) {
                    return None;
                }
            }
        }
    }
}

/// Odpovi na odlozena `JOURNAL_WATCH`, kde uz jsou zaznamy nebo uplynul limit.
fn poll_watches(svc: &mut Service, sessions: &mut [Session]) {
    let now = time::now();
    for s in sessions.iter_mut() {
        let Some(w) = s.watch.take() else { continue };
        match svc.poll_watch(s, &w, now >= w.deadline) {
            None => s.watch = Some(w),
            Some(res) => {
                // Zavreny kanal: relaci ukonci pristi pruchod smycky.
                let _ = s.channel.write(&fsproto::reply(res), Vec::new());
            }
        }
    }
}

/// Kroky historie podle nastaveni datasetu: automaticke snapshoty,
/// expirace souboru, uklid kose (jako system).
fn history(svc: &mut Service, next: &mut u64) {
    if time::now() < *next {
        return;
    }
    *next = time::deadline_after(HISTORY_INTERVAL);
    let Some(fs) = svc.fs.as_mut() else { return };
    if fs.info().read_only {
        return;
    }
    let now = crate::serve::fs_now();
    if let Err(e) = fs.auto_snapshot_tick(now) {
        println!("fsd: automaticke snapshoty: {e}");
    }
    if let Err(e) = fs.expire_tick(now) {
        println!("fsd: expirace souboru: {e}");
    }
    if let Err(e) = fs.trash_tick(now) {
        println!("fsd: uklid kose: {e}");
    }
}

/// Automaticky commit a defragmentace v necinnosti.
fn background(svc: &mut Service, next_commit: &mut u64, idle: &mut bool, changed: &mut bool) {
    // Klient prave ulozil vse sam: dalsi automaticky commit az za interval.
    if core::mem::take(&mut svc.commit_requested) {
        *next_commit = time::deadline_after(COMMIT_INTERVAL);
    }
    let Some(fs) = svc.fs.as_mut() else { return };
    if time::now() >= *next_commit {
        if let Err(e) = fs.commit() {
            println!("fsd: commit selhal: {e}");
        }
        *next_commit = time::deadline_after(COMMIT_INTERVAL);
        *idle = *changed;
    } else if *idle {
        match fs.defrag_step(DEFRAG_BUDGET) {
            Ok(r) if r.done => {
                *idle = false;
                *changed = false;
            }
            Ok(_) => {}
            Err(e) => {
                println!("fsd: defragmentace: {e}");
                *idle = false;
            }
        }
    }
}

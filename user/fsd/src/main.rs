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

use alloc::vec::Vec;
use core::time::Duration;

use blockproto::BlockClient;
use fs::Fs;
use libos::{Channel, Error, Startup, println, time};

use crate::blkdev::BlkDev;
use crate::serve::Service;

/// Jak casto ukladat zmeny commitem.
const COMMIT_INTERVAL: Duration = Duration::from_secs(5);
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
    let mut svc = Service {
        fs: Some(fs),
        debug,
    };
    serve(&mut svc, &client);
    if let Some(fs) = svc.fs.as_mut()
        && let Err(e) = fs.commit()
    {
        println!("fsd: zaverecny commit selhal: {e}");
        return 4;
    }
    0
}

fn serve(svc: &mut Service, client: &Channel) {
    let mut next_commit = time::deadline_after(COMMIT_INTERVAL);
    // Defragmentovat jen po zmenach (jinak by se system prochazel stale dokola).
    let mut idle = false;
    let mut changed = false;
    loop {
        // V necinnosti (po commitu bez novych pozadavku) defragmentovat.
        let deadline = if idle { time::now() } else { next_commit };
        match client.read(Some(deadline)) {
            Ok(msg) => {
                idle = false;
                changed = true;
                let reply = fsproto::reply(svc.handle(&msg.data));
                // Klient prave ulozil vse sam: dalsi automaticky commit az
                // za cely interval.
                if msg.data.get(..4) == Some(&fsproto::op::COMMIT.to_le_bytes()[..]) {
                    next_commit = time::deadline_after(COMMIT_INTERVAL);
                }
                if client.write(&reply, Vec::new()).is_err() {
                    return;
                }
            }
            Err(Error::TimedOut) => {}
            Err(Error::PeerClosed) => return,
            Err(e) => {
                println!("fsd: chyba kanalu: {e}");
                return;
            }
        }
        let Some(fs) = svc.fs.as_mut() else { continue };
        if time::now() >= next_commit {
            if let Err(e) = fs.commit() {
                println!("fsd: commit selhal: {e}");
            }
            next_commit = time::deadline_after(COMMIT_INTERVAL);
            idle = changed;
        } else if idle {
            match fs.defrag_step(DEFRAG_BUDGET) {
                Ok(r) if r.done => {
                    idle = false;
                    changed = false;
                }
                Ok(_) => {}
                Err(e) => {
                    println!("fsd: defragmentace: {e}");
                    idle = false;
                }
            }
        }
    }
}

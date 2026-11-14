//! Mereni rezie spawnu (DL-4b, `docs/cs/dynamicke-linkovani.md`): spoustec
//! (staticky program s relaci libd a listkem, `spawn::Options::libd`)
//! spusti N programu se zavadecem (`dltest`) za sebou a zmeri:
//!
//! - cas `spawn` programu se zavadecem (vcetne relace a listku pro dite),
//! - cas samotneho `libd::child` (`Connect` a `ADOPT`, bez cekani),
//! - cas `spawn` statickeho programu (sam sebe bez prace) - bez libd,
//! - cas obehu k libd (`Stats`) = co pridaval DL-4a kazdemu spawnu.
//!
//! Handly: initfs (`kind::INITFS`) a kanal (prvni `USER`). Z kanalu cte
//! pocty (`u32`), na kazdy odpovi `[n, spawn_dyn, cekani_dyn, narrow,
//! spawn_static, libd_rtt, chyby]` v nanosekundach (soucty, `u64` LE);
//! konci se zavrenim kanalu (spoustec tak prezije i pad libd - test obnovy).
//! Bez kanalu (spusten jako staticky program pro mereni) hned skonci.
//!
//! Test poradi (`hold`, `go`): `hold` spusti `dltest-rs` v cekacim rezimu
//! (relace a listek pro nej vzniknou bez cekani: `Connect` a `ADOPT`)
//! a odpovi `ok`, az dite nahraje knihovny; `go` mu preda pokyn k `dlopen`
//! a odpovi jeho navratovym kodem (8 B). Init mezitim muze zabit libd
//! drive, nez zpracuje `ADOPT`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::time::Duration;

use libos::initfs::Initfs;
use libos::{Channel, Startup, time};

libos::entry!(main);

fn main(mut s: Startup) -> i64 {
    if s.handles.is_empty() {
        return 0;
    }
    let ch = Channel::from_handle(s.handles.remove(0));
    let Ok(fs) = Initfs::open(&s) else { return 2 };
    let (Some(dynamic), Some(own)) = (fs.find("dltest"), fs.find("dlspawn")) else { return 3 };
    let mut held: Option<(libos::Process, Channel)> = None;
    while let Ok(m) = ch.read(None) {
        let out = match m.data.as_slice() {
            b"hold" => hold(&fs, &mut held),
            b"go" => go(&mut held),
            d => {
                let n = d.get(..4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                batch(n, dynamic, own)
            }
        };
        let _ = ch.write(&out, Vec::new());
    }
    0
}

/// Spusti `dltest-rs` v cekacim rezimu; `ok` az nahraje knihovny.
fn hold(fs: &Initfs, held: &mut Option<(libos::Process, Channel)>) -> Vec<u8> {
    let Some(image) = fs.find("dltest-rs") else { return b"chybi dltest-rs".to_vec() };
    let Ok((ours, theirs)) = Channel::create() else { return b"kanal".to_vec() };
    match libos::spawn("dltest-rs", image, alloc::vec![libos::spawn::StartHandle::user(theirs.into_handle())]) {
        Ok((p, _c)) => match ours.read(Some(time::deadline_after(Duration::from_secs(20)))) {
            Ok(_) => {
                *held = Some((p, ours));
                b"ok".to_vec()
            }
            Err(_) => b"dite neohlasilo start".to_vec(),
        },
        Err(_) => b"spawn".to_vec(),
    }
}

/// Pokyn diteti a jeho navratovy kod.
fn go(held: &mut Option<(libos::Process, Channel)>) -> Vec<u8> {
    let Some((p, c)) = held.take() else { return (-1i64).to_le_bytes().to_vec() };
    let _ = c.write(b"go", Vec::new());
    let code = p.wait_exit(Some(time::deadline_after(Duration::from_secs(60)))).unwrap_or(-2);
    code.to_le_bytes().to_vec()
}

/// `n` programu se zavadecem a statickych; vysledek ke zprave.
fn batch(n: u32, dynamic: &[u8], own: &[u8]) -> Vec<u8> {
    let deadline = || Some(time::deadline_after(Duration::from_secs(20)));
    let (mut spawn_dyn, mut wait_dyn, mut narrow, mut spawn_static, mut rtt, mut errors) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    for _ in 0..n {
        let t0 = time::now();
        let r = libos::spawn("dltest", dynamic, Vec::new());
        let t1 = time::now();
        spawn_dyn += t1 - t0;
        match r {
            Ok((p, _c)) => {
                if p.wait_exit(deadline()).map_or(true, |c| c & 0xFF != 0) {
                    errors += 1;
                }
                wait_dyn += time::now() - t1;
            }
            Err(e) => {
                libos::println!("dlspawn: spawn dltest: {e}");
                errors += 1;
            }
        }
        // Samotne `libd::child` (relace a listek pro dalsiho potomka).
        let t2 = time::now();
        match libos::libd::child() {
            Ok(_) => narrow += time::now() - t2,
            Err(_) => errors += 1,
        }
        // Staticky program: zadna relace s libd, zadny NARROW.
        let t3 = time::now();
        match libos::spawn("dlspawn", own, Vec::new()) {
            Ok((p, _c)) => {
                spawn_static += time::now() - t3;
                let _ = p.wait_exit(deadline());
            }
            Err(_) => errors += 1,
        }
        // Obeh k libd: to pridaval DL-4a kazdemu spawnu (Connect).
        let t4 = time::now();
        match libos::libd::stats() {
            Ok(_) => rtt += time::now() - t4,
            Err(_) => errors += 1,
        }
    }
    let mut out = Vec::new();
    for v in [u64::from(n), spawn_dyn, wait_dyn, narrow, spawn_static, rtt, errors] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

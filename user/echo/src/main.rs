//! echo: jednoduchy server pro testy IPC.
//!
//! Cte zpravy z bootstrap kanalu (od rodice) a odpovida:
//! - `ping:<text>` -> `pong:<text>`
//! - `signal` + jeden handle (event) -> signalizuje ho, odpovi `ok`
//! - `exit:<cislo>` -> ukonci proces s timto kodem
//! - `crash` -> zapis na adresu 0 (test ukonceni procesu pri vyjimce)
//! - `overflow` -> nekonecna rekurze (test ochranne oblasti pod
//!   zasobnikem: proces skonci vypadkem stranky, nic jineho neprepise)
//! - `sleep` -> ceka navzdy (test PROCESS_KILL)
//! - `killjob:<kod>` -> `JOB_KILL` na vlastni job (test: volajici skonci
//!   take); kdyby prezil, odpovi `prezil`
//! - `alloc:<bajty>` -> vytvori objekt pameti, odpovi `ok` nebo
//!   `err:<JMENO_CHYBY>` (test rozpoctu jobu)
//! - `tp-set:<hex>` -> (aarch64) zapise TPIDR_EL0, odpovi `ok`;
//!   `tp-get` -> odpovi `tp:<hex>` (test, ze registr neprosakuje mezi
//!   procesy); na x86_64 `err:NOT_SUPPORTED`
//! - `fence-hold` + handle plotu -> drzi ho (producent), odpovi `ok`;
//!   `fence-signal:<v>` / `fence-fail:<v>` -> posune drzeny plot nebo
//!   nahlasi chybu, odpovi `ok` nebo `err:<JMENO_CHYBY>`
//! - `fence-wait:<v>` + handle plotu (jen `WAIT`) -> ceka na hodnotu
//!   (termin 5 s), odpovi `ok` nebo `err:<JMENO_CHYBY>`
//!
//! Konec kanalu u rodice = konec procesu s kodem 0.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use libos::{Error, Startup, println};

libos::entry!(main);

/// Rekurze bez konce: kazde volani zabere kus zasobniku (pole pres
/// `black_box`, aby ho prekladac neodstranil).
#[allow(unconditional_recursion, reason = "zamerne preteceni zasobniku (test)")]
fn overflow(depth: u64) -> u64 {
    let pad = core::hint::black_box([depth; 64]);
    overflow(depth + 1) + pad[(depth % 64) as usize]
}

fn main(startup: Startup) -> i64 {
    let channel = startup.bootstrap;
    // Plot drzeny jako producent (`fence-hold`).
    let mut fence: Option<libos::Fence> = None;
    loop {
        let msg = match channel.read(None) {
            Ok(m) => m,
            Err(Error::PeerClosed) => return 0,
            Err(e) => {
                println!("echo: chyba cteni: {e}");
                return 1;
            }
        };
        let text = core::str::from_utf8(&msg.data).unwrap_or("");
        if let Some(rest) = text.strip_prefix("ping:") {
            let mut reply = Vec::from(&b"pong:"[..]);
            reply.extend_from_slice(rest.as_bytes());
            if channel.write(&reply, Vec::new()).is_err() {
                return 1;
            }
        } else if text == "signal" {
            let Some(h) = msg.handles.into_iter().next() else {
                return 2;
            };
            let event = libos::Event::from_handle(h);
            if event.signal().is_err() || channel.write(b"ok", Vec::new()).is_err() {
                return 3;
            }
        } else if let Some(code) = text.strip_prefix("exit:") {
            return code.parse().unwrap_or(-1);
        } else if text == "overflow" {
            return overflow(0) as i64;
        } else if text == "crash" {
            // SAFETY: zamerne neplatny zapis - test, ze jadro proces ukonci.
            unsafe { core::ptr::write_volatile(core::ptr::null_mut::<u64>(), 1) };
        } else if let Some(size) = text.strip_prefix("alloc:") {
            let reply = match libos::Vmo::create(size.parse().unwrap_or(0)) {
                Ok(_) => alloc::string::String::from("ok"),
                Err(e) => alloc::format!("err:{}", e.name()),
            };
            if channel.write(reply.as_bytes(), Vec::new()).is_err() {
                return 4;
            }
        } else if let Some(code) = text.strip_prefix("killjob:") {
            let code = code.parse::<i64>().unwrap_or(1);
            if let Ok(job) = libos::job::current() {
                let _ = job.kill(code);
            }
            // Sem se vlakno nedostane: konci pri navratu ze syscallu.
            let _ = channel.write(b"prezil", Vec::new());
        } else if let Some(v) = text.strip_prefix("tp-set:") {
            let reply = match u64::from_str_radix(v, 16) {
                Ok(v) if write_tp(v) => alloc::string::String::from("ok"),
                Ok(_) => alloc::string::String::from("err:NOT_SUPPORTED"),
                Err(_) => alloc::string::String::from("err:INVALID_ARGS"),
            };
            if channel.write(reply.as_bytes(), Vec::new()).is_err() {
                return 5;
            }
        } else if text == "tp-get" {
            let reply = match read_tp() {
                Some(v) => alloc::format!("tp:{v:x}"),
                None => alloc::string::String::from("err:NOT_SUPPORTED"),
            };
            if channel.write(reply.as_bytes(), Vec::new()).is_err() {
                return 6;
            }
        } else if text == "fence-hold" {
            fence = msg.handles.into_iter().next().map(libos::Fence::from_handle);
            if channel.write(b"ok", Vec::new()).is_err() {
                return 7;
            }
        } else if let Some((op, v)) = text.split_once(':')
            && op.starts_with("fence-")
        {
            let v = v.parse::<u64>().unwrap_or(0);
            let r = match op {
                "fence-signal" => fence.as_ref().map_or(Err(Error::BadState), |f| f.signal(v)),
                "fence-fail" => fence.as_ref().map_or(Err(Error::BadState), |f| f.fail(v)),
                "fence-wait" => match msg.handles.into_iter().next() {
                    Some(h) => libos::Fence::from_handle(h)
                        .wait(v, Some(libos::time::deadline_after(core::time::Duration::from_secs(5)))),
                    None => Err(Error::InvalidArgs),
                },
                _ => Err(Error::NotSupported),
            };
            let reply = match r {
                Ok(()) => alloc::string::String::from("ok"),
                Err(e) => alloc::format!("err:{}", e.name()),
            };
            if channel.write(reply.as_bytes(), Vec::new()).is_err() {
                return 8;
            }
        } else if text == "sleep" {
            libos::time::sleep(core::time::Duration::from_secs(3600));
        } else {
            println!("echo: neznama zprava {text:?}");
        }
    }
}

/// Zapise registr ukazatele vlakna (jen aarch64: TPIDR_EL0 smi program
/// zapsat sam).
fn write_tp(v: u64) -> bool {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: TPIDR_EL0 je registr pro program; libos ho nepouziva.
        unsafe { core::arch::asm!("msr tpidr_el0, {}", in(reg) v, options(nomem, nostack, preserves_flags)) };
        true
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = v;
        false
    }
}

fn read_tp() -> Option<u64> {
    #[cfg(target_arch = "aarch64")]
    {
        let v: u64;
        // SAFETY: cteni TPIDR_EL0 z EL0 je povolene.
        unsafe { core::arch::asm!("mrs {}, tpidr_el0", out(reg) v, options(nomem, nostack, preserves_flags)) };
        Some(v)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        None
    }
}

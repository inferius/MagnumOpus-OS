//! echo: jednoduchy server pro testy IPC.
//!
//! Cte zpravy z bootstrap kanalu (od rodice) a odpovida:
//! - `ping:<text>` -> `pong:<text>`
//! - `signal` + jeden handle (event) -> signalizuje ho, odpovi `ok`
//! - `exit:<cislo>` -> ukonci proces s timto kodem
//! - `crash` -> zapis na adresu 0 (test ukonceni procesu pri vyjimce)
//! - `sleep` -> ceka navzdy (test PROCESS_KILL)
//! - `killjob:<kod>` -> `JOB_KILL` na vlastni job (test: volajici skonci
//!   take); kdyby prezil, odpovi `prezil`
//! - `alloc:<bajty>` -> vytvori objekt pameti, odpovi `ok` nebo
//!   `err:<JMENO_CHYBY>` (test rozpoctu jobu)
//!
//! Konec kanalu u rodice = konec procesu s kodem 0.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use libos::{Error, Startup, println};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    let channel = startup.bootstrap;
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
        } else if text == "sleep" {
            libos::time::sleep(core::time::Duration::from_secs(3600));
        } else {
            println!("echo: neznama zprava {text:?}");
        }
    }
}

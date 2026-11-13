//! timed: sluzba casu.
//!
//! Jediny proces (krome init), ktery smi nastavovat realny cas: dostane od
//! init handle na Resource s pravem `SET_CLOCK` (bez `MANAGE`, tedy bez
//! pristupu k hardwaru).
//!
//! Start: handly `kind::USER` = [Resource s `SET_CLOCK`]. Prvni zprava od
//! rodice na bootstrap kanalu = nastaveni casu jako text (viz
//! `libos::time::zone::Settings`, napr. `zone = Europe/Prague\nrtc = local`).
//! Sluzba nastaveni precte, pri `rtc = local` opravi cas ze startu (RTC bylo
//! v mistnim case) a odpovi `ok` nebo `err:<popis>`.
//!
//! Dalsi zpravy od rodice:
//! - `set` + 12 B (sekundy i64, nanosekundy u32, LE) + 1 B (1 = zapsat i do
//!   RTC) -> nastavi cas, odpovi `ok:hw` / `ok` / `err:<kod>`,
//! - `ntp` + handle kanalu -> pripoji klienta synchronizace (`ntpd`), odpovi
//!   `ok`,
//! - `status` -> `ok:` + stav ([`sync::Status`] jako text).
//!
//! Klient synchronizace smi jen navrhovat korekce (`sync`, viz [`sync`]);
//! sluzba je overi a provede plynule (`CLOCK_ADJUST`). Proces, ktery
//! zpracovava data ze site, tak hodiny primo menit nemuze.
//!
//! Konec kanalu u rodice = konec procesu.

#![no_std]
#![no_main]

extern crate alloc;

mod sync;

use alloc::borrow::Cow;
use alloc::format;
use alloc::vec::Vec;

use libos::hw::Resource;
use libos::time::{self, WallTime, zone::Settings};
use libos::{Channel, Error, Signals, Startup, println};

libos::entry!(main);

/// Nejvic pripojenych klientu synchronizace.
const MAX_SYNC_CLIENTS: usize = 4;

fn main(mut startup: Startup) -> i64 {
    let channel = startup.bootstrap;
    if startup.handles.is_empty() {
        println!("timed: chybi Resource s pravem SET_CLOCK");
        let _ = channel.write(b"err:chybi Resource", Vec::new());
        return 1;
    }
    let clock = Resource::from_handle(startup.handles.remove(0));

    let mut settings = match channel.read(None) {
        Ok(msg) => match core::str::from_utf8(&msg.data).map(Settings::parse) {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                println!("timed: chybne nastaveni casu ({e}), pouzivam UTC");
                Settings::default()
            }
            Err(_) => {
                println!("timed: nastaveni neni UTF-8, pouzivam UTC");
                Settings::default()
            }
        },
        Err(_) => return 1,
    };
    let reply: Cow<str> = match time::apply_rtc_local(&clock, &settings) {
        Ok(Some(t)) => {
            println!("timed: RTC je v mistnim case, cas opraven o pasmo {}", settings.zone.display());
            log_local(&settings, t);
            "ok".into()
        }
        Ok(None) => {
            if let Some(t) = time::wall() {
                log_local(&settings, t);
            }
            "ok".into()
        }
        Err(Error::NotSupported) => {
            println!("timed: realny cas neni znamy");
            "ok".into()
        }
        Err(e) => {
            println!("timed: oprava casu selhala: {e}");
            format!("err:{e}").into()
        }
    };
    if channel.write(reply.as_bytes(), Vec::new()).is_err() {
        return 1;
    }

    let mut sync = sync::Sync::new();
    let mut clients: Vec<Channel> = Vec::new();
    loop {
        let mut items: Vec<(&libos::Handle, Signals)> = Vec::with_capacity(1 + clients.len());
        items.push((channel.handle(), Signals::READABLE | Signals::PEER_CLOSED));
        for c in &clients {
            items.push((c.handle(), Signals::READABLE | Signals::PEER_CLOSED));
        }
        let observed = match libos::wait_many(&items, None) {
            Ok((_, o)) => o,
            Err(e) => {
                println!("timed: cekani selhalo: {e}");
                return 1;
            }
        };
        drop(items);
        // Klienti synchronizace (odzadu, aby slo odebirat).
        for i in (0..clients.len()).rev() {
            let s = observed[i + 1];
            if s.contains(Signals::READABLE) {
                while let Ok(msg) = clients[i].try_read() {
                    let reply = sync.handle(&clock, &settings, &msg.data);
                    let _ = clients[i].write(&reply, Vec::new());
                }
            } else if s.contains(Signals::PEER_CLOSED) {
                println!("timed: klient synchronizace skoncil");
                clients.swap_remove(i);
            }
        }
        if !observed[0].intersects(Signals::READABLE | Signals::PEER_CLOSED) {
            continue;
        }
        let msg = match channel.try_read() {
            Ok(m) => m,
            Err(Error::ShouldWait) => continue,
            Err(Error::PeerClosed) => return 0,
            Err(e) => {
                println!("timed: chyba cteni: {e}");
                return 1;
            }
        };
        let reply: Cow<str> = if let Some(rest) = msg.data.strip_prefix(b"set") {
            if rest.len() == 13 {
                let secs = i64::from_le_bytes(rest[0..8].try_into().unwrap());
                let nanos = u32::from_le_bytes(rest[8..12].try_into().unwrap());
                let t = WallTime { secs, nanos };
                match time::set(&clock, t, &settings, rest[12] == 1) {
                    Ok(true) => "ok:hw".into(),
                    Ok(false) => "ok".into(),
                    Err(e) => format!("err:{e}").into(),
                }
            } else {
                "err:neznama zprava".into()
            }
        } else if msg.data == b"ntp" {
            match msg.handles.into_iter().next() {
                Some(h) if clients.len() < MAX_SYNC_CLIENTS => {
                    clients.push(Channel::from_handle(h));
                    "ok".into()
                }
                Some(_) => "err:prilis mnoho klientu".into(),
                None => "err:chybi kanal".into(),
            }
        } else if let Some(rest) = msg.data.strip_prefix(b"settings ") {
            // Nastaveni ze system.toml (init po pripojeni FS).
            match core::str::from_utf8(rest) {
                Ok(t) => match Settings::parse(t) {
                    Ok(s) => {
                        settings = s;
                        "ok".into()
                    }
                    Err(e) => format!("err:{e}").into(),
                },
                Err(_) => "err:neni UTF-8".into(),
            }
        } else if msg.data == b"status" {
            format!("ok:{}", sync.status(&clock)).into()
        } else {
            "err:neznama zprava".into()
        };
        if channel.write(reply.as_bytes(), Vec::new()).is_err() {
            return 1;
        }
    }
}

fn log_local(settings: &Settings, t: WallTime) {
    let l = settings.zone.to_local(t);
    let c = l.civil;
    println!(
        "timed: mistni cas {:04}-{:02}-{:02} {:02}:{:02}:{:02} {} (UTC{}, pasmo {})",
        c.year,
        c.month,
        c.day,
        c.hour,
        c.minute,
        c.second,
        l.offset.abbrev,
        libos::time::zone::FormatOffset(l.offset.secs),
        settings.zone.display()
    );
}

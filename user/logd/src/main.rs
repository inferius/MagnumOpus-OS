//! logd: sluzba logu a zaznamu padu.
//!
//! Startovni zprava (kind USER, v tomto poradi):
//! 1. Resource s pravem `DIAG` (bez rozsahu hardwaru): `LOG_READ`,
//!    `CRASH_RECORD`,
//! 2. kanal relace FS a 3. jeji listek (relace prezije pad fsd); relace
//!    vidi `/log` (= `/system/log`) a `/pady` (= `/system/pady`).
//!
//! Prace:
//! - **cislo bootu**: `/log/boot-count` (+1 pri kazdem startu),
//! - **zaznam padu jadra** z minuleho behu (pstore): ulozi ho do
//!   `/pady/<boot>-jadro-<id>.crash` a v jadre smaze,
//! - **trvaly log**: kazdou sekundu precte novy log jadra (v nem jsou
//!   i zpravy vsech sluzeb) do vlastni fronty v RAM a pripise ho do
//!   `/log/boot-<cislo>.log`. Kdyz FS nejde (pad fsd, plny disk), drzi
//!   frontu v RAM (nejvys [`QUEUE_MAX`], pak zahodi nejstarsi a zapise
//!   o tom zpravu) a zkousi znovu,
//! - **rotace**: nejvic [`KEEP_BOOTS`] logu, kazdy nejvys [`FILE_MAX`],
//! - **pady procesu** (zprava `proc-crash` od supervisoru): vypis s duvodem
//!   konce a poslednimi radky logu procesu do `/pady`,
//! - **balicek pro vyvojare** (zprava `report`): `report.rs`.
//!
//! Na bootstrap kanalu: init posle `start`, logd odpovi `ok <cislo bootu>`.
//! Dalsi zpravy: `proc-crash <jmeno> <pid> <popis konce>` (bez odpovedi),
//! `report` (odpoved `ok <cesta>\n<nahled>` nebo `chyba <text>`).

#![no_std]
#![no_main]

extern crate alloc;

mod report;

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use fsproto::{FsClient, kind};
use libos::hw::Resource;
use libos::{Channel, Error, Result, Startup, println, time};

libos::entry!(main);

/// Jak casto cist log jadra a zapisovat na disk.
const TICK: Duration = Duration::from_secs(1);
/// Nejvic nezapsaneho logu v RAM (kdyz FS nejde).
const QUEUE_MAX: usize = 4 << 20;
/// Kolik logu bootu drzet.
pub const KEEP_BOOTS: u64 = 20;
/// Nejvetsi soubor logu jednoho bootu.
const FILE_MAX: u64 = 32 << 20;
/// Poslednich bajtu logu v RAM pro vypis pady procesu a balicek.
const RECENT_MAX: usize = 512 << 10;
/// Zapis na FS po kusech.
const WRITE_CHUNK: usize = 32 << 10;

pub struct Logd {
    diag: Resource,
    pub fs: FsClient,
    pub boot: u64,
    /// Pozice v logu jadra (pocet prectenych bajtu proudu).
    cursor: u64,
    /// Nezapsany log.
    queue: VecDeque<u8>,
    /// Kolik bajtu fronta zahodila (preteceni pri nedostupnem FS).
    dropped: u64,
    /// Posledni log (vypisy pad procesu, balicek).
    pub recent: VecDeque<u8>,
    /// Delka souboru logu tohoto bootu.
    written: u64,
    fs_ok: bool,
}

fn main(startup: Startup) -> i64 {
    let mut h = startup.handles.into_iter();
    let (Some(res), Some(fs), Some(lease)) = (h.next(), h.next(), h.next()) else {
        println!("logd: chybi handly ve startovni zprave");
        return 1;
    };
    let ctl = startup.bootstrap;
    let mut d = Logd {
        diag: Resource::from_handle(res),
        fs: FsClient::with_lease(Channel::from_handle(fs), Channel::from_handle(lease)),
        boot: 0,
        cursor: 0,
        queue: VecDeque::new(),
        dropped: 0,
        recent: VecDeque::new(),
        written: 0,
        fs_ok: true,
    };
    // Cekat na `start` od initu (az ma pripravene slozky).
    match ctl.read(Some(time::deadline_after(Duration::from_secs(30)))) {
        Ok(m) if m.data == b"start" => {}
        _ => {
            println!("logd: init neposlal start");
            return 2;
        }
    }
    d.boot = d.next_boot().unwrap_or_else(|e| {
        println!("logd: cislo bootu nejde ulozit ({e}), pouzivam 0");
        0
    });
    d.save_kernel_crash();
    d.rotate();
    let _ = ctl.write(format!("ok {}", d.boot).as_bytes(), Vec::new());
    println!("logd: log bootu {} do /system/log/{}", d.boot, d.log_name());
    loop {
        match ctl.read(Some(time::deadline_after(TICK))) {
            Ok(m) => d.message(&ctl, &m.data),
            Err(Error::TimedOut) | Err(Error::ShouldWait) => {}
            Err(Error::PeerClosed) => {
                // Init konci (vypinani): dopsat log a skoncit.
                d.pump();
                return 0;
            }
            Err(_) => {}
        }
        d.pump();
    }
}

/// Nazev s cislem zarovnanym na 6 mist (razeni podle jmena = podle bootu).
pub fn boot_tag(n: u64) -> String {
    format!("{n:06}")
}

impl Logd {
    pub fn log_name(&self) -> String {
        format!("boot-{}.log", boot_tag(self.boot))
    }

    fn next_boot(&self) -> Result<u64> {
        let prev = self
            .fs
            .read("/log/boot-count", 0, 32)
            .ok()
            .and_then(|b| core::str::from_utf8(&b).ok().and_then(|s| s.trim().parse::<u64>().ok()))
            .unwrap_or(0);
        let n = prev + 1;
        match self.fs.create("/log/boot-count", kind::FILE) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(e),
        }
        let text = format!("{n:<20}\n");
        self.fs.write("/log/boot-count", 0, text.as_bytes())?;
        Ok(n)
    }

    /// Zaznam padu jadra z minuleho behu do `/pady` a smazani v jadre.
    fn save_kernel_crash(&mut self) {
        let cap = self.diag.crash_record(abi::syscall::crash_op::CAPACITY, &mut []).unwrap_or(0);
        if cap == 0 {
            return;
        }
        let mut buf = vec![0u8; cap];
        let n = match self.diag.crash_record(abi::syscall::crash_op::READ, &mut buf) {
            Ok(0) => return,
            Ok(n) => n,
            Err(e) => {
                println!("logd: zaznam padu nejde precist: {e}");
                return;
            }
        };
        let Some((h, text, crc)) = crashrec::parse(&buf[..n]) else {
            return;
        };
        let id = crashrec::crash_id(crc);
        let id = core::str::from_utf8(&id).unwrap_or("?");
        let name = format!("/pady/{}-jadro-{id}.crash", boot_tag(self.boot.saturating_sub(1)));
        let body = crashrec::render(&h, text, crc);
        match write_file(&self.fs, &name, body.as_bytes()) {
            Ok(()) => {
                let _ = self.diag.crash_record(abi::syscall::crash_op::CLEAR, &mut []);
                println!(
                    "logd: zaznam padu jadra #{id} ({}) ulozen do /system{name}",
                    crashrec::field_str(&h.reason)
                );
            }
            // Zaznam zustava v RAM: zkusi se pri dalsim startu.
            Err(e) => println!("logd: zaznam padu jadra #{id} nejde ulozit: {e}"),
        }
    }

    /// Smaze nejstarsi logy bootu nad [`KEEP_BOOTS`] (a jejich balicky).
    fn rotate(&self) {
        let Ok(list) = self.fs.list("/log") else { return };
        let mut boots: Vec<(u64, String)> = list
            .into_iter()
            .filter_map(|e| {
                let n = e.name.strip_prefix("boot-")?.strip_suffix(".log")?.parse().ok()?;
                Some((n, e.name))
            })
            .collect();
        boots.sort();
        while boots.len() as u64 > KEEP_BOOTS {
            let (_, name) = boots.remove(0);
            let _ = self.fs.remove(&format!("/log/{name}"));
        }
    }

    fn message(&mut self, ctl: &Channel, data: &[u8]) {
        let text = core::str::from_utf8(data).unwrap_or("");
        let mut w = text.splitn(3, ' ');
        match w.next() {
            Some("proc-crash") => {
                let Some(name) = w.next() else { return };
                let why = w.next().unwrap_or("");
                // Nejdriv dotahnout posledni log (vypis pady jadrem).
                self.pump();
                self.save_proc_crash(name, why);
            }
            Some("report") => {
                self.pump();
                let reply = match report::build(self) {
                    Ok((path, preview)) => format!("ok {path}\n{preview}"),
                    Err(e) => format!("chyba {e}"),
                };
                let _ = ctl.write(reply.as_bytes(), Vec::new());
            }
            _ => println!("logd: neznama zprava: {text}"),
        }
    }

    /// Vypis pady procesu: duvod konce a radky logu, ktere se ho tykaji
    /// (`[jmeno pid]` od procesu, `proces pid (jmeno)` od jadra - vyjimka
    /// CPU, ukazatel zasobniku a zpetna stopa).
    fn save_proc_crash(&self, name: &str, why: &str) {
        let recent: Vec<u8> = self.recent.iter().copied().collect();
        let text = String::from_utf8_lossy(&recent);
        let tag = format!("[{name} ");
        let kern = format!("({name})");
        let mut lines: Vec<&str> = text
            .lines()
            .filter(|l| l.contains(&tag) || l.contains(&kern))
            .collect();
        let skip = lines.len().saturating_sub(200);
        lines.drain(..skip);
        let mut body = format!(
            "MagnumOpus: pad procesu\nproces: {name}\nkonec:  {why}\nboot:   {}\n\
             adresy prelozi: cargo xtask symbolize <tento soubor> (program z target/user)\n--- posledni radky logu procesu ---\n",
            self.boot
        );
        for l in lines {
            body.push_str(l);
            body.push('\n');
        }
        let mut n = 1;
        let path = loop {
            let p = format!("/pady/{}-{name}-{n}.txt", boot_tag(self.boot));
            if self.fs.stat(&p).is_err() {
                break p;
            }
            n += 1;
        };
        match write_file(&self.fs, &path, body.as_bytes()) {
            Ok(()) => println!("logd: pad procesu {name} zapsan do /system{path}"),
            Err(e) => println!("logd: pad procesu {name} nejde zapsat: {e}"),
        }
    }

    /// Precte novy log jadra a zapise frontu na disk.
    fn pump(&mut self) {
        let mut buf = vec![0u8; 64 << 10];
        loop {
            let (start, n) = match self.diag.log_read(self.cursor, &mut buf) {
                Ok(r) => r,
                Err(e) => {
                    println!("logd: cteni logu jadra selhalo: {e}");
                    break;
                }
            };
            if start > self.cursor {
                let lost = format!("[logd: {} B logu jadra se ztratilo (pretekl buffer)]\n", start - self.cursor);
                self.push(lost.as_bytes());
            }
            if n == 0 {
                break;
            }
            self.cursor = start + n as u64;
            let chunk = buf[..n].to_vec();
            self.push(&chunk);
        }
        self.flush();
    }

    fn push(&mut self, data: &[u8]) {
        self.queue.extend(data.iter().copied());
        if self.queue.len() > QUEUE_MAX {
            let over = self.queue.len() - QUEUE_MAX;
            self.queue.drain(..over);
            self.dropped += over as u64;
        }
        self.recent.extend(data.iter().copied());
        if self.recent.len() > RECENT_MAX {
            let over = self.recent.len() - RECENT_MAX;
            self.recent.drain(..over);
        }
    }

    fn flush(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let path = format!("/log/{}", self.log_name());
        if self.written >= FILE_MAX {
            // Plny soubor: dal jen RAM (posledni log) a fronta se zahazuje.
            self.dropped += self.queue.len() as u64;
            self.queue.clear();
            return;
        }
        if self.written == 0 {
            match self.fs.create(&path, kind::FILE) {
                Ok(()) | Err(Error::AlreadyExists) => {}
                Err(e) => return self.fs_down(e),
            }
        }
        if self.dropped > 0 {
            let note = format!("[logd: {} B logu zahozeno (FS nedostupny nebo soubor plny)]\n", self.dropped);
            if self.fs.write(&path, self.written, note.as_bytes()).is_ok() {
                self.written += note.len() as u64;
                self.dropped = 0;
            }
        }
        while !self.queue.is_empty() && self.written < FILE_MAX {
            let n = self.queue.len().min(WRITE_CHUNK);
            let chunk: Vec<u8> = self.queue.iter().take(n).copied().collect();
            match self.fs.write(&path, self.written, &chunk) {
                Ok(()) => {
                    self.written += n as u64;
                    self.queue.drain(..n);
                    if !self.fs_ok {
                        self.fs_ok = true;
                        println!("logd: FS zase funguje, log zapsan");
                    }
                }
                Err(e) => return self.fs_down(e),
            }
        }
    }

    fn fs_down(&mut self, e: Error) {
        if self.fs_ok {
            self.fs_ok = false;
            println!("logd: zapis logu selhal ({e}), drzim ho v RAM a zkusim znovu");
        }
    }
}

/// Zapise cely soubor (vytvori ho; existujici prepise od zacatku).
pub fn write_file(fs: &FsClient, path: &str, data: &[u8]) -> Result<()> {
    match fs.create(path, kind::FILE) {
        Ok(()) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(e),
    }
    for (i, chunk) in data.chunks(WRITE_CHUNK).enumerate() {
        fs.write(path, (i * WRITE_CHUNK) as u64, chunk)?;
    }
    Ok(())
}

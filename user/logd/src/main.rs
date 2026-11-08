//! logd: sluzba logu a zaznamu padu.
//!
//! Startovni zprava (kind USER, v tomto poradi):
//! 1. Resource s pravem `DIAG` (bez rozsahu hardwaru): `LOG_READ`,
//!    `CRASH_RECORD`,
//! 2. kanal relace FS a 3. jeji listek (relace prezije pad fsd); relace
//!    vidi `/log` (= `/system/log`) a `/pady` (= `/system/pady`),
//! 4. (nepovinne) kanal k oddilu MO-LOGY na flashce (`blockproto`): kopie
//!    logu, padu a balicku na FAT32, ktery precte Windows (`fatlog.rs`).
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
//! - **balicek pro vyvojare** (zprava `report`): `report.rs`,
//! - **opakovane radky**: stejny radek (bez casu a CPU) vic nez
//!   [`REPEAT_KEEP`]x za sebou se zapise jednou a pak `... (opakovano N x)`.
//!
//! Na bootstrap kanalu: init posle `start`, logd odpovi `ok <cislo bootu>`.
//! Dalsi zpravy: `proc-crash <jmeno> <pid> <popis konce>` (bez odpovedi),
//! `report` (odpoved `ok <cesta>\n<nahled>` nebo `chyba <text>`).

#![no_std]
#![no_main]

extern crate alloc;

mod fatlog;
mod report;
mod shots;

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
/// Kolik stejnych radku za sebou zapsat, nez se dalsi jen spocitaji.
pub const REPEAT_KEEP: u32 = 3;
/// Jak casto kopirovat log bootu na FAT oddil flashky.
const FAT_MIRROR: Duration = Duration::from_secs(30);

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
    /// Kopie na FAT oddil flashky (zive USB).
    pub fat: Option<fatlog::FatLog>,
    fat_handle: Option<libos::Handle>,
    last_mirror: u64,
    /// Nedokonceny radek z posledniho cteni logu.
    partial: Vec<u8>,
    /// Posledni radek (bez casu a CPU) a kolikrat se opakoval.
    last_key: Vec<u8>,
    repeats: u32,
    /// Ztracene radky podle jadra (uz ohlasene).
    lost_lines: u64,
    /// Kanal od kompozitoru s obrazky diagnostickeho snimku (`shots.rs`).
    shots: Option<Channel>,
    /// Revize sestaveni z logu jadra (jmena obrazku snimku).
    build: String,
    /// Dalsi misto obrazku na MO-LOGY.
    shot_slot: u64,
    /// Nejvic obrazku v `/log` (`snimek.conf`).
    shot_keep: usize,
}

fn main(startup: Startup) -> i64 {
    let mut h = startup.handles.into_iter();
    let (Some(res), Some(fs), Some(lease), logy) = (h.next(), h.next(), h.next(), h.next()) else {
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
        fat: None,
        fat_handle: logy,
        last_mirror: 0,
        partial: Vec::new(),
        last_key: Vec::new(),
        repeats: 0,
        lost_lines: 0,
        shots: None,
        build: String::new(),
        shot_slot: 0,
        shot_keep: 10,
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
    if let Some(h) = d.fat_handle.take() {
        d.fat = blockproto::BlockClient::new(Channel::from_handle(h))
            .ok()
            .and_then(|c| fatlog::FatLog::open(c, d.boot));
    }
    d.apply_levels();
    d.save_kernel_crash();
    d.rotate();
    let _ = ctl.write(format!("ok {}", d.boot).as_bytes(), Vec::new());
    println!("logd: log bootu {} do /system/log/{}", d.boot, d.log_name());
    loop {
        match ctl.read(Some(time::deadline_after(TICK))) {
            // Kanal pro obrazky diagnostickeho snimku (init, viz `shots.rs`).
            Ok(m) if m.data == b"snapshots" => {
                if let Some(h) = m.handles.into_iter().next() {
                    d.set_shots(Channel::from_handle(h));
                }
            }
            Ok(m) => d.message(&ctl, &m.data),
            Err(Error::TimedOut) | Err(Error::ShouldWait) => {}
            Err(Error::PeerClosed) => {
                // Init konci (vypinani): dopsat log, kopie na flashku a konec.
                d.pump();
                d.mirror();
                return 0;
            }
            Err(_) => {}
        }
        d.pump();
        d.poll_shots();
        // Prvni kopie hned po startu, pak kazdych FAT_MIRROR.
        if d.last_mirror == 0 || time::now().saturating_sub(d.last_mirror) >= FAT_MIRROR.as_nanos() as u64 {
            d.mirror();
        }
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

    /// Urovne logu z `/log/urovne.conf` (`modul = uroven` po radcich, `#`
    /// komentar); chybi-li, vytvori ho s vysvetlenim.
    fn apply_levels(&self) {
        const PATH: &str = "/log/urovne.conf";
        const DEFAULT: &str = "# Urovne logu jadra a sluzeb (nacte logd pri startu, za behu prikaz loglevel).
# modul jadra (napr. sched, arch::x86_64::apic) nebo jmeno procesu = error|warn|info|debug|trace
# Vychozi potlacene sekce (jen info a vys): syscall, object::process, smp::clock_sync.
# priklad:
# virtio-blk = warn
";
        let text = match self.fs.stat(PATH) {
            Ok(st) => self.fs.read(PATH, 0, st.size.min(16 << 10) as usize).unwrap_or_default(),
            Err(_) => {
                let _ = write_file(&self.fs, PATH, DEFAULT.as_bytes());
                return;
            }
        };
        for line in String::from_utf8_lossy(&text).lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((t, l)) = line.split_once('=') else { continue };
            let mut req = format!("{}={}", t.trim(), l.trim()).into_bytes();
            match self.diag.log_control(abi::syscall::log_op::SET, &mut req) {
                Ok(_) => {}
                Err(e) => println!("logd: urovne.conf: '{line}': {e}"),
            }
        }
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
        if let Some(f) = self.fat.as_mut() {
            f.crash(&format!("boot {}: pad jadra #{id}", self.boot.saturating_sub(1)), &body);
        }
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
                self.shots_crash(name);
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
    fn save_proc_crash(&mut self, name: &str, why: &str) {
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
        if let Some(f) = self.fat.as_mut() {
            f.crash(&format!("boot {}: pad procesu {name}", self.boot), &body);
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
            let (start, n, lost) = match self.diag.log_read(self.cursor, &mut buf) {
                Ok(r) => r,
                Err(e) => {
                    println!("logd: cteni logu jadra selhalo: {e}");
                    break;
                }
            };
            if lost > self.lost_lines || start > self.cursor {
                let note = format!(
                    "[logd: ztraceno {} radku logu jadra (pretekl kruhovy buffer, {} B)]\n",
                    lost - self.lost_lines,
                    start.saturating_sub(self.cursor)
                );
                self.lost_lines = lost;
                self.push(note.as_bytes());
            }
            if n == 0 {
                break;
            }
            self.cursor = start + n as u64;
            let chunk = buf[..n].to_vec();
            self.push_lines(&chunk);
        }
        self.flush();
    }

    /// Cele radky z logu jadra; opakovane stejne radky slouci.
    fn push_lines(&mut self, data: &[u8]) {
        self.partial.extend_from_slice(data);
        let Some(end) = self.partial.iter().rposition(|&c| c == b'\n') else { return };
        let whole: Vec<u8> = self.partial.drain(..=end).collect();
        for line in whole.split_inclusive(|&c| c == b'\n') {
            if self.build.is_empty()
                && let Some(b) = shots::build_from_line(line)
            {
                self.build = b;
            }
            let key = line_key(line);
            if key == self.last_key.as_slice() {
                self.repeats += 1;
                if self.repeats >= REPEAT_KEEP {
                    continue;
                }
            } else {
                self.flush_repeats();
                self.last_key = key.to_vec();
            }
            self.push(line);
        }
    }

    /// Souhrn potlacenych opakovani predchoziho radku.
    fn flush_repeats(&mut self) {
        if self.repeats >= REPEAT_KEEP {
            let n = self.repeats + 1 - REPEAT_KEEP;
            let note = format!("    ... predchozi radek opakovan jeste {n}x\n");
            self.push(note.as_bytes());
        }
        self.repeats = 0;
    }

    /// Kopie logu bootu na FAT oddil flashky.
    pub fn mirror(&mut self) {
        self.last_mirror = time::now();
        if self.fat.is_none() {
            return;
        }
        // Log bootu: zapsana cast ze souboru (konec) a nezapsana fronta.
        let path = format!("/log/{}", self.log_name());
        let size = self.fs.stat(&path).map(|s| s.size).unwrap_or(0);
        let mut data = Vec::new();
        let mut at = size.saturating_sub(fatlog::LOG_TAIL as u64);
        while at < size {
            match self.fs.read(&path, at, ((size - at) as usize).min(WRITE_CHUNK)) {
                Ok(c) if !c.is_empty() => {
                    at += c.len() as u64;
                    data.extend_from_slice(&c);
                }
                _ => break,
            }
        }
        data.extend(self.queue.iter().copied());
        let boot = self.boot;
        if let Some(f) = self.fat.as_mut() {
            f.boot_log(boot, &data);
        }
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

/// Radek bez casu a CPU (`[    1.234] cpu2 `) pro porovnani opakovani.
fn line_key(line: &[u8]) -> &[u8] {
    let rest = match line.iter().position(|&c| c == b']') {
        Some(i) if line.first() == Some(&b'[') => &line[i + 1..],
        _ => line,
    };
    let rest = rest.strip_prefix(b" ").unwrap_or(rest);
    match rest.strip_prefix(b"cpu") {
        Some(r) => {
            let i = r.iter().position(|&c| c == b' ').unwrap_or(0);
            &r[i..]
        }
        None => rest,
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

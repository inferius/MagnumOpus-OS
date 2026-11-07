//! Protokol souboroveho systemu (verze 1): zpravy po kanalu mezi klientem a
//! sluzbou `fsd`.
//!
//! Pozadavek: `op` u32 a parametry v poradi podle operace. Odpoved: `status`
//! i32 (0 nebo kod chyby), rezerva u32 a vysledek. Cisla little-endian,
//! retezec = delka u16 + UTF-8, data = zbytek zpravy. Cesty jsou absolutni
//! (`/adresar/soubor`), jmena se porovnavaji bez ohledu na velikost pismen.
//!
//! Jedno cteni/zapis prenese nejvys `MAX_IO` bajtu; [`FsClient`] vetsi
//! pozadavky rozdeli. Popis operaci: `docs/cs/souborovy-system.md`.

#![no_std]

extern crate alloc;

mod codec;
pub mod perm;
pub mod v2;

use alloc::string::String;
use alloc::vec::Vec;

use libos::time::WallTime;
use libos::{Channel, Error, Result};

pub use codec::{Reader, Writer};

/// Nejvic bajtu dat v jedne zprave (rezerva pro hlavicku v limitu kanalu).
pub const MAX_IO: usize = 32 * 1024;
/// Nejvetsi telo odpovedi (stranka vypisu se do nej musi vejit).
pub const MAX_REPLY: usize = 60 * 1024;

/// Kody operaci.
pub mod op {
    pub const INFO: u32 = 1;
    pub const STAT: u32 = 2;
    pub const LIST: u32 = 3;
    pub const READ: u32 = 4;
    pub const WRITE: u32 = 5;
    pub const CREATE: u32 = 6;
    pub const REMOVE: u32 = 7;
    pub const RENAME: u32 = 8;
    pub const TRUNCATE: u32 = 9;
    pub const COMMIT: u32 = 10;
    pub const FSYNC: u32 = 11;
    pub const SNAPSHOT: u32 = 12;
    pub const SNAPSHOTS: u32 = 13;
    pub const DELETE_SNAPSHOT: u32 = 14;
    pub const SNAP_STAT: u32 = 15;
    pub const SNAP_LIST: u32 = 16;
    pub const SNAP_READ: u32 = 17;
    pub const DEFRAG: u32 = 18;
    pub const FRAGMENTS: u32 = 19;
    /// Kontrola konzistence (jen v rezimu `debug`: dlouha operace nad celym
    /// svazkem, nesmi ji spoustet kazdy klient).
    pub const CHECK: u32 = 20;
    /// Stranka vypisu adresare: (cesta, kurzor str, nejvic u32) -> dalsi u8,
    /// pocet u32, polozky. Kurzor = jmeno posledni polozky predchozi stranky
    /// (prazdny = od zacatku); stranka zacina prvni polozkou za nim
    /// v poradi vypisu (jmena bez ohledu na velikost pismen). Sluzba vrati
    /// tolik polozek, kolik se vejde do jedne zpravy; `dalsi` = 1, kdyz
    /// nejaka zbyla. `LIST` vraci `BUFFER_TOO_SMALL`, kdyz se cely vypis do
    /// zpravy nevejde.
    ///
    /// Zmena adresare behem vypisu: polozka, ktera existuje po celou dobu,
    /// se vypise prave jednou; nova polozka za kurzorem se objevi, pred nim
    /// ne; smazana zmizi. Nikdy zdvojeni ani vynechani existujici polozky
    /// (na rozdil od strankovani podle indexu).
    pub const LIST_PAGE: u32 = 21;
    /// Totez pro snapshot: (snapshot, cesta, kurzor str, nejvic u32).
    pub const SNAP_LIST_PAGE: u32 = 22;
    /// Jen pro testy (sluzba spustena v rezimu `debug`): zahodi neulozeny
    /// stav a pripoji system znovu, jako po vypadku proudu.
    pub const SIMULATE_POWER_LOSS: u32 = 100;
}

/// Druh objektu.
pub mod kind {
    pub const FILE: u8 = 1;
    pub const DIR: u8 = 2;
    /// Symbolicky odkaz (zatim jen ve vypisu a STAT; vytvareni prijde s
    /// protokolem v2).
    pub const SYMLINK: u8 = 3;
    /// Vstup do jineho datasetu (slozka s vlastni historii).
    pub const DATASET: u8 = 4;
}

/// Metadata objektu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stat {
    pub oid: u64,
    pub kind: u8,
    pub size: u64,
    /// Casy: sekundy od 1970 UTC (i64) + nanosekundy (u32), na drate 12 B.
    pub mtime: WallTime,
    pub ctime: WallTime,
    /// Cas vytvoreni.
    pub btime: WallTime,
    /// Vlastnik (0 = system).
    pub owner: u64,
    /// Ucinna prava volajiciho (`perm::rights`).
    pub rights: u8,
    /// U adresare: soucet velikosti souboru v podstromu (okamzite).
    pub tree_bytes: u64,
    pub tree_files: u64,
    pub tree_dirs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub kind: u8,
    pub oid: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Info {
    pub label: String,
    pub block_size: u32,
    pub block_count: u64,
    pub free_blocks: u64,
    pub deferred_blocks: u64,
    pub generation: u64,
    /// 1 = XXH3-128, 2 = BLAKE3.
    pub algo: u8,
    pub fallback: u64,
    pub replayed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub id: u64,
    pub generation: u64,
    pub name: String,
}

/// Cas na drate: i64 sekund + u32 nanosekund.
pub fn put_time(w: &mut Writer, t: WallTime) {
    w.u64(t.secs as u64).u32(t.nanos);
}

/// Cas z dratu; nanosekundy >= 10^9 = `INVALID_ARGS`.
pub fn get_time(r: &mut Reader) -> Result<WallTime> {
    let secs = r.u64()? as i64;
    let nanos = r.u32()?;
    if nanos >= 1_000_000_000 {
        return Err(Error::InvalidArgs);
    }
    Ok(WallTime { secs, nanos })
}

impl Stat {
    pub fn put(&self, w: &mut Writer) {
        w.u64(self.oid)
            .u8(self.kind)
            .u64(self.size);
        for t in [self.mtime, self.ctime, self.btime] {
            put_time(w, t);
        }
        w.u64(self.owner).u8(self.rights);
        w.u64(self.tree_bytes)
            .u64(self.tree_files)
            .u64(self.tree_dirs);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            oid: r.u64()?,
            kind: r.u8()?,
            size: r.u64()?,
            mtime: get_time(r)?,
            ctime: get_time(r)?,
            btime: get_time(r)?,
            owner: r.u64()?,
            rights: r.u8()?,
            tree_bytes: r.u64()?,
            tree_files: r.u64()?,
            tree_dirs: r.u64()?,
        })
    }
}

impl Info {
    pub fn put(&self, w: &mut Writer) {
        w.str(&self.label)
            .u32(self.block_size)
            .u64(self.block_count)
            .u64(self.free_blocks);
        w.u64(self.deferred_blocks)
            .u64(self.generation)
            .u8(self.algo)
            .u64(self.fallback)
            .u64(self.replayed);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            label: r.str()?,
            block_size: r.u32()?,
            block_count: r.u64()?,
            free_blocks: r.u64()?,
            deferred_blocks: r.u64()?,
            generation: r.u64()?,
            algo: r.u8()?,
            fallback: r.u64()?,
            replayed: r.u64()?,
        })
    }
}

/// Zakoduje odpoved.
pub fn reply(result: Result<Writer>) -> Vec<u8> {
    let mut out = Writer::new();
    match result {
        Ok(body) => {
            out.i32(0).u32(0);
            out.bytes(&body.finish());
        }
        Err(e) => {
            out.i32(e.code()).u32(0);
        }
    }
    out.finish()
}

/// Klient souboroveho systemu.
pub struct FsClient {
    channel: Channel,
}

impl FsClient {
    pub fn new(channel: Channel) -> Self {
        Self { channel }
    }

    /// Kanal relace (napr. pro predani relace jinemu procesu).
    pub fn into_channel(self) -> Channel {
        self.channel
    }

    /// Posle pozadavek, vrati telo odpovedi.
    fn call(&self, req: Writer) -> Result<Vec<u8>> {
        self.call_with_handles(req).map(|(b, _)| b)
    }

    /// Posle pozadavek, vrati telo odpovedi i predane handly.
    fn call_with_handles(&self, req: Writer) -> Result<(Vec<u8>, Vec<libos::Handle>)> {
        self.channel.write(&req.finish(), Vec::new())?;
        let msg = self.channel.read(None)?;
        let mut r = Reader::new(&msg.data);
        let status = r.i32()?;
        r.u32()?;
        if status != 0 {
            return Err(Error::from_code(status as i64).unwrap_or(Error::Corrupted));
        }
        Ok((r.rest().to_vec(), msg.handles))
    }

    fn req(op: u32) -> Writer {
        let mut w = Writer::new();
        w.u32(op);
        w
    }

    fn path_op(&self, op: u32, path: &str) -> Result<Vec<u8>> {
        let mut w = Self::req(op);
        w.str(path);
        self.call(w)
    }

    pub fn info(&self) -> Result<Info> {
        Info::get(&mut Reader::new(&self.call(Self::req(op::INFO))?))
    }

    pub fn stat(&self, path: &str) -> Result<Stat> {
        Stat::get(&mut Reader::new(&self.path_op(op::STAT, path)?))
    }

    /// Cely vypis adresare po strankach (adresar, jehoz vypis se nevejde do
    /// jedne zpravy). Vysledek drzi klient cely v pameti.
    pub fn list(&self, path: &str) -> Result<Vec<DirEntry>> {
        self.list_pages(None, path)
    }

    fn list_pages(&self, snap: Option<&str>, path: &str) -> Result<Vec<DirEntry>> {
        let mut out = Vec::new();
        loop {
            let mut w = match snap {
                Some(s) => {
                    let mut w = Self::req(op::SNAP_LIST_PAGE);
                    w.str(s);
                    w
                }
                None => Self::req(op::LIST_PAGE),
            };
            let after = out.last().map_or("", |e: &DirEntry| e.name.as_str());
            w.str(path).str(after).u32(u32::MAX);
            let body = self.call(w)?;
            let mut r = Reader::new(&body);
            let more = r.u8()? != 0;
            let page = decode_list(r.rest())?;
            let empty = page.is_empty();
            out.extend(page);
            if !more || empty {
                return Ok(out);
            }
        }
    }

    /// Precte az `len` bajtu od `offset` (mene na konci souboru).
    pub fn read(&self, path: &str, offset: u64, len: usize) -> Result<Vec<u8>> {
        self.read_impl(op::READ, None, path, offset, len)
    }

    fn read_impl(
        &self,
        op: u32,
        snap: Option<&str>,
        path: &str,
        offset: u64,
        len: usize,
    ) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        while out.len() < len {
            let n = (len - out.len()).min(MAX_IO);
            let mut w = Self::req(op);
            if let Some(s) = snap {
                w.str(s);
            }
            w.str(path).u64(offset + out.len() as u64).u32(n as u32);
            let part = self.call(w)?;
            let got = part.len();
            out.extend_from_slice(&part);
            if got < n {
                break;
            }
        }
        Ok(out)
    }

    /// Zapise data od `offset` (soubor se pripadne zvetsi).
    pub fn write(&self, path: &str, offset: u64, data: &[u8]) -> Result<()> {
        for (i, chunk) in data.chunks(MAX_IO).enumerate() {
            let mut w = Self::req(op::WRITE);
            w.str(path).u64(offset + (i * MAX_IO) as u64).bytes(chunk);
            self.call(w)?;
        }
        Ok(())
    }

    pub fn create(&self, path: &str, kind: u8) -> Result<()> {
        let mut w = Self::req(op::CREATE);
        w.str(path).u8(kind);
        self.call(w).map(|_| ())
    }

    pub fn remove(&self, path: &str) -> Result<()> {
        self.path_op(op::REMOVE, path).map(|_| ())
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        let mut w = Self::req(op::RENAME);
        w.str(from).str(to);
        self.call(w).map(|_| ())
    }

    pub fn truncate(&self, path: &str, size: u64) -> Result<()> {
        let mut w = Self::req(op::TRUNCATE);
        w.str(path).u64(size);
        self.call(w).map(|_| ())
    }

    /// Ulozi vse atomicky (commit).
    pub fn commit(&self) -> Result<()> {
        self.call(Self::req(op::COMMIT)).map(|_| ())
    }

    /// Ucini dosavadni zmeny trvalymi (rychle, pres intent log).
    pub fn fsync(&self) -> Result<()> {
        self.call(Self::req(op::FSYNC)).map(|_| ())
    }

    pub fn snapshot(&self, name: &str) -> Result<u64> {
        Reader::new(&self.path_op(op::SNAPSHOT, name)?).u64()
    }

    pub fn snapshots(&self) -> Result<Vec<Snapshot>> {
        let body = self.call(Self::req(op::SNAPSHOTS))?;
        let mut r = Reader::new(&body);
        let n = r.u32()?;
        (0..n)
            .map(|_| {
                Ok(Snapshot {
                    id: r.u64()?,
                    generation: r.u64()?,
                    name: r.str()?,
                })
            })
            .collect()
    }

    pub fn delete_snapshot(&self, name: &str) -> Result<()> {
        self.path_op(op::DELETE_SNAPSHOT, name).map(|_| ())
    }

    pub fn snapshot_stat(&self, snap: &str, path: &str) -> Result<Stat> {
        let mut w = Self::req(op::SNAP_STAT);
        w.str(snap).str(path);
        Stat::get(&mut Reader::new(&self.call(w)?))
    }

    pub fn snapshot_list(&self, snap: &str, path: &str) -> Result<Vec<DirEntry>> {
        self.list_pages(Some(snap), path)
    }

    pub fn snapshot_read(
        &self,
        snap: &str,
        path: &str,
        offset: u64,
        len: usize,
    ) -> Result<Vec<u8>> {
        self.read_impl(op::SNAP_READ, Some(snap), path, offset, len)
    }

    /// Defragmentuje soubor; vraci pocet prepsanych bloku.
    pub fn defrag(&self, path: &str) -> Result<u64> {
        Reader::new(&self.path_op(op::DEFRAG, path)?).u64()
    }

    /// Pocet souvislych useku souboru (1 = nefragmentovany).
    pub fn fragments(&self, path: &str) -> Result<u64> {
        Reader::new(&self.path_op(op::FRAGMENTS, path)?).u64()
    }

    /// Kontrola konzistence; vraci popisy nalezenych problemu (nejvys tolik,
    /// kolik se vejde do odpovedi; celkovy pocet v `CheckResult`).
    pub fn check(&self, scrub: bool) -> Result<Vec<String>> {
        Ok(self.check_full(scrub)?.problems)
    }

    /// Kontrola konzistence se vsemi udaji.
    pub fn check_full(&self, scrub: bool) -> Result<CheckResult> {
        let mut w = Self::req(op::CHECK);
        w.u8(scrub as u8);
        let body = self.call(w)?;
        let mut r = Reader::new(&body);
        let generation = r.u64()?;
        let total = r.u64()?;
        let live_not_checked = r.u8()? != 0;
        let n = r.u32()?;
        let problems = (0..n).map(|_| r.str()).collect::<Result<_>>()?;
        Ok(CheckResult {
            generation,
            total,
            live_not_checked,
            problems,
        })
    }

    /// Jen pro testy: zahodi neulozeny stav a pripoji znovu. Vraci pocet
    /// operaci prehranych z intent logu.
    pub fn simulate_power_loss(&self) -> Result<u64> {
        Reader::new(&self.call(Self::req(op::SIMULATE_POWER_LOSS))?).u64()
    }
}

pub(crate) fn decode_list(body: &[u8]) -> Result<Vec<DirEntry>> {
    let mut r = Reader::new(body);
    let n = r.u32()?;
    (0..n)
        .map(|_| {
            Ok(DirEntry {
                kind: r.u8()?,
                oid: r.u64()?,
                name: r.str()?,
            })
        })
        .collect()
}

/// Vysledek kontroly konzistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    /// Zkontrolovana (ulozena) generace.
    pub generation: u64,
    /// Celkovy pocet problemu (i nevypsanych).
    pub total: u64,
    /// V pameti sluzby jsou nezkontrolovane zmeny (svazek jen pro cteni).
    pub live_not_checked: bool,
    pub problems: Vec<String>,
}

/// Zakoduje stranku vypisu ze zacatku `entries` (polozky za kurzorem):
/// nejvic `max`, dokud se vejdou do `budget` bajtu (aspon jedna). Vraci
/// pocet zakodovanych.
pub fn encode_list_page(w: &mut Writer, entries: &[DirEntry], max: usize, budget: usize) -> usize {
    let mut size = 5;
    let mut n = 0;
    for e in entries.iter().take(max) {
        let item = 1 + 8 + 2 + e.name.len();
        if n > 0 && size + item > budget {
            break;
        }
        size += item;
        n += 1;
    }
    w.u8((n < entries.len()) as u8);
    encode_list(w, &entries[..n]);
    n
}

/// Zakoduje seznam polozek adresare (pro sluzbu).
pub fn encode_list(w: &mut Writer, entries: &[DirEntry]) {
    w.u32(entries.len() as u32);
    for e in entries {
        w.u8(e.kind).u64(e.oid).str(&e.name);
    }
}

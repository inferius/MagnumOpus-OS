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

use alloc::string::String;
use alloc::vec::Vec;

use libos::{Channel, Error, Result};

pub use codec::{Reader, Writer};

/// Nejvic bajtu dat v jedne zprave (rezerva pro hlavicku v limitu kanalu).
pub const MAX_IO: usize = 32 * 1024;

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
    pub const CHECK: u32 = 20;
    /// Jen pro testy (sluzba spustena v rezimu `debug`): zahodi neulozeny
    /// stav a pripoji system znovu, jako po vypadku proudu.
    pub const SIMULATE_POWER_LOSS: u32 = 100;
}

/// Druh objektu.
pub mod kind {
    pub const FILE: u8 = 1;
    pub const DIR: u8 = 2;
}

/// Metadata objektu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stat {
    pub oid: u64,
    pub kind: u8,
    pub size: u64,
    pub mtime: u64,
    pub ctime: u64,
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

impl Stat {
    pub fn put(&self, w: &mut Writer) {
        w.u64(self.oid)
            .u8(self.kind)
            .u64(self.size)
            .u64(self.mtime)
            .u64(self.ctime);
        w.u64(self.tree_bytes)
            .u64(self.tree_files)
            .u64(self.tree_dirs);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            oid: r.u64()?,
            kind: r.u8()?,
            size: r.u64()?,
            mtime: r.u64()?,
            ctime: r.u64()?,
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

    /// Posle pozadavek, vrati telo odpovedi.
    fn call(&self, req: Writer) -> Result<Vec<u8>> {
        self.channel.write(&req.finish(), Vec::new())?;
        let msg = self.channel.read(None)?;
        let mut r = Reader::new(&msg.data);
        let status = r.i32()?;
        r.u32()?;
        if status != 0 {
            return Err(Error::from_code(status as i64).unwrap_or(Error::Corrupted));
        }
        Ok(r.rest().to_vec())
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

    pub fn list(&self, path: &str) -> Result<Vec<DirEntry>> {
        decode_list(&self.path_op(op::LIST, path)?)
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
        let mut w = Self::req(op::SNAP_LIST);
        w.str(snap).str(path);
        decode_list(&self.call(w)?)
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

    /// Kontrola konzistence; vraci popisy nalezenych problemu.
    pub fn check(&self, scrub: bool) -> Result<Vec<String>> {
        let mut w = Self::req(op::CHECK);
        w.u8(scrub as u8);
        let body = self.call(w)?;
        let mut r = Reader::new(&body);
        let n = r.u32()?;
        (0..n).map(|_| r.str()).collect()
    }

    /// Jen pro testy: zahodi neulozeny stav a pripoji znovu. Vraci pocet
    /// operaci prehranych z intent logu.
    pub fn simulate_power_loss(&self) -> Result<u64> {
        Reader::new(&self.call(Self::req(op::SIMULATE_POWER_LOSS))?).u64()
    }
}

fn decode_list(body: &[u8]) -> Result<Vec<DirEntry>> {
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

/// Zakoduje seznam polozek adresare (pro sluzbu).
pub fn encode_list(w: &mut Writer, entries: &[DirEntry]) {
    w.u32(entries.len() as u32);
    for e in entries {
        w.u8(e.kind).u64(e.oid).str(&e.name);
    }
}

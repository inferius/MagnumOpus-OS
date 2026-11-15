//! Protokol v2: relace, handly, podminene ulozeni, zamky.
//!
//! Ramec zprav je stejny jako ve v1 (op u32 + parametry, odpoved status
//! i32 + rezerva u32 + vysledek); operace v1 (cesty) dal plati. Nove:
//!
//! - **Relace:** `CONNECT` vrati v odpovedi novy kanal (handle zpravy) -
//!   kazdy klient ma vlastni relaci s vlastnimi handly. Zavreni kanalu
//!   zavre vsechny handly relace (sirotci zmizi, zamky se uvolni).
//! - **Handly:** `OPEN` vrati cislo handlu, `Stat` a `Revision`; dalsi
//!   operace pracuji s handlem (soubor prezije prejmenovani i smazani).
//!   Handle plati jen v relaci, ktera ho otevrela.
//! - **Zapis vraci pocet prijatych bajtu** (`accepted`): prijato = v bufferu
//!   sluzby, trvale az po `FSYNC`. Zapis rozdeleny do vic zprav = kazda
//!   zprava samostatny zapis; klient po prvni chybe nepokracuje a vi, kolik
//!   predchozich zprav bylo prijato.
//! - **Podminene ulozeni:** `REPLACE_BEGIN(cesta, ocekavana revize)` ->
//!   handle docasneho souboru; zapisy `WRITE_H`; `REPLACE_COMMIT` nahradi
//!   cil jen pri ocekavane revizi, jinak `CONFLICT` a nic se nezmeni
//!   (docasny soubor zustane - klient muze ulozit vedle, nebo potvrdit
//!   s `force`); `REPLACE_ABORT` docasny soubor zahodi.

use alloc::string::String;
use alloc::vec::Vec;

use libos::{Channel, Error, Result};

use svcclient::Kind;

use crate::recover::Rec;
use crate::{DirEntry, FsClient, Reader, Stat, Writer, decode_list};

/// Verze protokolu, kterou tento klient mluvi.
pub const VERSION: u32 = 3;

/// Kody operaci v2.
pub mod op {
    /// -> verze u32, max_io u32
    pub const HELLO: u32 = 30;
    /// -> (novy kanal relace v handlech odpovedi)
    pub const CONNECT: u32 = 31;
    /// cesta, pristup u8, sdileni u8, priznaky u8 -> handle u64, Stat, Revision
    pub const OPEN: u32 = 32;
    pub const CLOSE: u32 = 33;
    /// handle, offset u64, delka u32 -> data
    pub const READ_H: u32 = 34;
    /// handle, offset u64, data -> prijato u32
    pub const WRITE_H: u32 = 35;
    /// handle, velikost u64
    pub const SET_LEN: u32 = 36;
    /// handle -> Stat, Revision
    pub const STAT_H: u32 = 37;
    /// handle, kurzor str, nejvic u32 -> dalsi u8, polozky
    pub const LIST_H: u32 = 38;
    /// handle, od u64, delka u64 (0 = do konce), priznaky u8 (lock::*)
    pub const LOCK: u32 = 39;
    /// handle, od u64, delka u64
    pub const UNLOCK: u32 = 40;
    /// handle, zapnout u8
    pub const DELETE_ON_CLOSE: u32 = 41;
    /// z str, do str
    pub const RENAME_REPLACE: u32 = 42;
    /// cesta, ma_revizi u8, Revision -> handle docasneho souboru
    pub const REPLACE_BEGIN: u32 = 43;
    /// handle, vynutit u8 -> Revision noveho obsahu | CONFLICT
    pub const REPLACE_COMMIT: u32 = 44;
    /// handle
    pub const REPLACE_ABORT: u32 = 45;
    /// Trvaly handle po restartu sluzby: dataset u64, oid u64, generace
    /// u64, pristup u8, sdileni u8 -> handle u64, Stat, Revision (jako
    /// `OPEN`). Prava se overi znovu; objekt s jinou generaci = `NOT_FOUND`.
    pub const REOPEN: u32 = 118;
    /// Zamek POSIX (`fcntl` F_SETLK/F_GETLK, `flock`): handle u64, prikaz
    /// u8 ([`posix_lock`]), trida u8, druh u8, start u64, delka u64 (0 = do
    /// konce) -> vysledek u8 (`DONE`, `CONFLICT` + druh u8, start u64,
    /// delka u64; `DEADLOCK`). Vlastnik `fcntl` = relace, `flock` = handle.
    pub const POSIX_LOCK: u32 = 119;
    /// Soubor jako proud bajtu (stdin/stdout potomka `posix_spawn`):
    /// handle u64, rezim u8 ([`stream_mode`]), pozice u64 -> kanal proudu
    /// (handle ve zprave). fsd si otevre vlastni handle tehoz souboru,
    /// takze proud prezije zavreni relace klienta; konci zavrenim kanalu
    /// (zapis) nebo koncem souboru (cteni). Restart fsd proud ukonci.
    pub const OPEN_STREAM: u32 = 120;
}

/// Rezimy `OPEN_STREAM`.
pub mod stream_mode {
    /// fsd posila obsah od pozice do konce souboru, pak kanal zavre.
    pub const READ: u8 = 0;
    /// Data prijata z kanalu zapisuje od pozice dal.
    pub const WRITE: u8 = 1;
    /// Data prijata z kanalu pripojuje na konec souboru.
    pub const APPEND: u8 = 2;
}

/// Prikazy, tridy a vysledky `POSIX_LOCK`.
pub mod posix_lock {
    /// Zjistit konflikt (`F_GETLK`).
    pub const TEST: u8 = 0;
    /// Zamknout bez cekani (`F_SETLK`).
    pub const SET: u8 = 1;
    /// Zamknout, klient bude cekat (`F_SETLKW`): pri konfliktu se ulozi
    /// cekani a odhali cyklus (`DEADLOCK`).
    pub const SET_WAIT: u8 = 2;
    /// Odemknout (`F_UNLCK`).
    pub const UNLOCK: u8 = 3;
    /// Trida `fcntl` (vlastnik relace) a `flock` (vlastnik handle).
    pub const FCNTL: u8 = 0;
    pub const FLOCK: u8 = 1;
    /// Druh: sdileny / vyhradni.
    pub const READ: u8 = 0;
    pub const WRITE: u8 = 1;
    /// Vysledek.
    pub const DONE: u8 = 0;
    pub const CONFLICT: u8 = 1;
    pub const DEADLOCK: u8 = 2;
}

/// Vysledek [`crate::FsClient::posix_lock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PosixLock {
    Done,
    /// Cizi zamek v konfliktu: vyhradni?, start, delka (0 = do konce).
    Conflict { write: bool, start: u64, len: u64 },
    Deadlock,
}

/// Pristup a sdileni pri `OPEN` (stejne bity jako `fs::access`/`fs::share`).
pub mod access {
    pub const READ: u8 = 1;
    pub const WRITE: u8 = 2;
    pub const DELETE: u8 = 4;
    /// Spusteni (odmitne ho dataset se zakazem spousteni).
    pub const EXECUTE: u8 = 8;
}

pub mod share {
    pub const READ: u8 = 1;
    pub const WRITE: u8 = 2;
    pub const DELETE: u8 = 4;
    pub const ALL: u8 = 7;
}

/// Priznaky `OPEN`.
pub mod open_flags {
    /// Vytvorit soubor, kdyz neexistuje.
    pub const CREATE: u8 = 1;
    /// S `CREATE`: chyba `ALREADY_EXISTS`, kdyz uz existuje (`O_EXCL`).
    pub const EXCL: u8 = 2;
    /// Zkratit na 0.
    pub const TRUNCATE: u8 = 4;
    /// S `CREATE`: vytvorit adresar.
    pub const DIR: u8 = 8;
}

/// Priznaky `CONNECT`.
pub mod connect_flags {
    /// Nova relace je ridici (smi `CONNECT_AS`, granty); jen z ridici
    /// relace.
    pub const CONTROL: u32 = 1;
}

/// Priznaky `LOCK`.
pub mod lock {
    pub const EXCLUSIVE: u8 = 1;
    pub const MANDATORY: u8 = 2;
}

/// Revize objektu (viz `fs::Revision`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Revision {
    pub dataset: u64,
    pub oid: u64,
    pub generation: u64,
    pub version: u64,
}

impl Revision {
    pub fn put(&self, w: &mut Writer) {
        w.u64(self.dataset).u64(self.oid).u64(self.generation).u64(self.version);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            dataset: r.u64()?,
            oid: r.u64()?,
            generation: r.u64()?,
            version: r.u64()?,
        })
    }
}

/// Trvala identita otevreneho objektu: s ni ho jina relace otevre znovu
/// ([`FsClient::open_identity`], operace `REOPEN`). Dedeni fd pri
/// spusteni procesu (`posix_spawn`): potomek ma vlastni relaci.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub dataset: u64,
    pub oid: u64,
    pub generation: u64,
    pub access: u8,
    pub share: u8,
}

/// Otevreny soubor u klienta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opened {
    pub handle: u64,
    pub stat: Stat,
    pub revision: Revision,
}

impl FsClient {
    /// Verze protokolu sluzby (`VERSION` = umi tento modul).
    pub fn hello(&self) -> Result<u32> {
        let body = self.call(Self::req(op::HELLO))?;
        Reader::new(&body).u32()
    }

    /// Nova relace (vlastni handly, nezavisla na teto).
    pub fn connect(&self) -> Result<FsClient> {
        self.connect_with(0)
    }

    /// (Ridici relace) Dalsi ridici relace (init ji drzi s listkem,
    /// adapter sluzby svou).
    pub fn connect_control(&self) -> Result<FsClient> {
        self.connect_with(connect_flags::CONTROL)
    }

    fn connect_with(&self, flags: u32) -> Result<FsClient> {
        let mut w = Self::req(op::CONNECT);
        w.u32(flags);
        let (_, mut handles) = self.call_with_handles(w)?;
        let h = handles.pop().ok_or(Error::Corrupted)?;
        Ok(FsClient::new(Channel::from_handle(h)))
    }

    /// Otevre `path` (priznaky `open_flags`). Cislo handlu je cislo
    /// knihovny (trvaly handle, prezije restart sluzby - viz `recover`).
    pub fn open(&self, path: &str, access: u8, share: u8, flags: u8) -> Result<Opened> {
        let mut w = Self::req(op::OPEN);
        w.str(path).u8(access).u8(share).u8(flags);
        // Otevreni bez vytvoreni a zkraceni jde zopakovat.
        let kind = if flags & (open_flags::CREATE | open_flags::TRUNCATE) != 0 { Kind::Effect } else { Kind::Idempotent };
        let body = self.call_as(kind, w)?;
        self.adopt(&body, access, share)
    }

    /// Trvala identita handlu `local` (pro predani jine relaci). Docasny
    /// soubor podmineneho ulozeni ani mrtvy handle predat nejde.
    pub fn identity(&self, local: u64) -> Result<Identity> {
        let rec = self.handles.lock().get(local)?;
        if rec.dead {
            return Err(Error::BadHandle);
        }
        if rec.replace {
            return Err(Error::BadState);
        }
        let v = rec.revision;
        Ok(Identity { dataset: v.dataset, oid: v.oid, generation: v.generation, access: rec.access, share: rec.share })
    }

    /// Otevre objekt podle identity z jine relace (`REOPEN`): fsd overi
    /// prava a pripojene cesty teto relace znovu, sirotka (smazany soubor
    /// s otevrenym handlem) otevre jen vlastnik.
    pub fn open_identity(&self, id: &Identity) -> Result<Opened> {
        let mut w = Self::req(op::REOPEN);
        w.u64(id.dataset).u64(id.oid).u64(id.generation).u8(id.access).u8(id.share);
        let body = self.call_as(Kind::Idempotent, w)?;
        self.adopt(&body, id.access, id.share)
    }

    /// Zapise odpoved `OPEN`/`REOPEN` (handle, Stat, Revision) mezi handly
    /// knihovny.
    fn adopt(&self, body: &[u8], access: u8, share: u8) -> Result<Opened> {
        let mut r = Reader::new(body);
        let fsd = r.u64()?;
        let stat = Stat::get(&mut r)?;
        let revision = Revision::get(&mut r)?;
        let handle = self.handles.lock().insert(Rec {
            fsd,
            generation: self.conn.generation(),
            revision,
            access,
            share,
            replace: false,
            locked: false,
            delete_on_close: false,
            dead: false,
        });
        Ok(Opened { handle, stat, revision })
    }

    /// Je handle z jine instance sluzby, nez s jakou mluvi spojeni?
    fn stale(&self, local: u64) -> bool {
        self.handles
            .lock()
            .get(local)
            .is_ok_and(|r| r.generation != self.conn.generation())
    }

    /// Cislo handlu u bezici instance; po restartu sluzby ho otevre znovu.
    fn resolve(&self, local: u64) -> Result<u64> {
        let rec = self.handles.lock().get(local)?;
        if rec.dead {
            return Err(Error::BadHandle);
        }
        if rec.generation == self.conn.generation() {
            return Ok(rec.fsd);
        }
        if rec.replace {
            // Docasny soubor podmineneho ulozeni patril stare instanci.
            self.handles.lock().update(local, |r| r.dead = true);
            return Err(Error::ServiceRestarted);
        }
        let mut w = Self::req(op::REOPEN);
        w.u64(rec.revision.dataset)
            .u64(rec.revision.oid)
            .u64(rec.revision.generation)
            .u8(rec.access)
            .u8(rec.share);
        let fsd = match self.call_as(Kind::Idempotent, w).and_then(|b| Reader::new(&b).u64()) {
            Ok(h) => h,
            Err(e) => {
                self.handles.lock().update(local, |r| r.dead = true);
                return Err(e);
            }
        };
        let generation = self.conn.generation();
        self.handles.lock().update(local, |r| {
            r.fsd = fsd;
            r.generation = generation;
        });
        if rec.delete_on_close {
            let mut w = Self::req(op::DELETE_ON_CLOSE);
            w.u64(fsd).u8(1);
            let _ = self.call_as(Kind::Effect, w);
        }
        if rec.locked {
            // Zamky stare instance zanikly: aplikace je musi ziskat znovu.
            self.handles.lock().update(local, |r| r.locked = false);
            return Err(Error::ServiceRestarted);
        }
        Ok(fsd)
    }

    /// Operace s handlem `local`; `build(cislo u sluzby)` sestavi
    /// pozadavek. Po restartu sluzby handle otevre znovu; `Idempotent`
    /// operaci zopakuje, ostatni vrati `SERVICE_RESTARTED`.
    fn h_call(&self, local: u64, kind: Kind, build: impl Fn(u64) -> Writer) -> Result<Vec<u8>> {
        for _ in 0..3 {
            let fsd = self.resolve(local)?;
            // Cislo handlu patri jedne instanci: nikdy neopakovat stejne
            // bajty na nove (proto Effect na urovni spojeni).
            match self.call_as(Kind::Effect, build(fsd)) {
                Err(Error::ServiceRestarted | Error::BadHandle) if self.stale(local) => {
                    if kind == Kind::Effect {
                        return Err(Error::ServiceRestarted);
                    }
                }
                r => return r,
            }
        }
        Err(Error::ServiceRestarted)
    }

    pub fn close(&self, h: u64) -> Result<()> {
        let rec = self.handles.lock().get(h)?;
        let r = if rec.dead || rec.generation != self.conn.generation() {
            // Handle zanikl se starou instanci sluzby.
            Ok(Vec::new())
        } else {
            let mut w = Self::req(op::CLOSE);
            w.u64(rec.fsd);
            match self.call_as(Kind::Effect, w) {
                Err(Error::ServiceRestarted | Error::BadHandle) if self.stale(h) => Ok(Vec::new()),
                r => r,
            }
        };
        self.handles.lock().remove(h);
        r.map(|_| ())
    }

    fn h_op(&self, opcode: u32, h: u64, kind: Kind) -> Result<Vec<u8>> {
        self.h_call(h, kind, |fsd| {
            let mut w = Self::req(opcode);
            w.u64(fsd);
            w
        })
    }

    /// Precte az `len` bajtu (po kusech `MAX_IO`).
    pub fn read_h(&self, h: u64, offset: u64, len: usize) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        while out.len() < len {
            let n = (len - out.len()).min(crate::MAX_IO);
            let at = offset + out.len() as u64;
            let chunk = self.h_call(h, Kind::Idempotent, |fsd| {
                let mut w = Self::req(op::READ_H);
                w.u64(fsd).u64(at).u32(n as u32);
                w
            })?;
            let got = chunk.len();
            out.extend_from_slice(&chunk);
            if got < n {
                break;
            }
        }
        Ok(out)
    }

    /// Zapise `data` po kusech. `Ok(n)` = prijato vse (`n == data.len()`);
    /// chyba = `(prijato pred chybou, chyba)` - klient vi presne, kolik
    /// predchozich kusu sluzba prijala.
    pub fn write_h(&self, h: u64, offset: u64, data: &[u8]) -> core::result::Result<usize, (usize, Error)> {
        let mut done = 0;
        for chunk in data.chunks(crate::MAX_IO) {
            let at = offset + done as u64;
            let accepted = self
                .h_call(h, Kind::Effect, |fsd| {
                    let mut w = Self::req(op::WRITE_H);
                    w.u64(fsd).u64(at).bytes(chunk);
                    w
                })
                .and_then(|b| Reader::new(&b).u32())
                .map_err(|e| (done, e))? as usize;
            done += accepted;
            if accepted < chunk.len() {
                return Err((done, Error::IoError));
            }
        }
        Ok(done)
    }

    pub fn set_len(&self, h: u64, size: u64) -> Result<()> {
        self.h_call(h, Kind::Effect, |fsd| {
            let mut w = Self::req(op::SET_LEN);
            w.u64(fsd).u64(size);
            w
        })
        .map(|_| ())
    }

    pub fn stat_h(&self, h: u64) -> Result<(Stat, Revision)> {
        let body = self.h_op(op::STAT_H, h, Kind::Idempotent)?;
        let mut r = Reader::new(&body);
        Ok((Stat::get(&mut r)?, Revision::get(&mut r)?))
    }

    /// Cely vypis otevreneho adresare (po strankach s kurzorem).
    pub fn list_h(&self, h: u64) -> Result<Vec<DirEntry>> {
        let mut out: Vec<DirEntry> = Vec::new();
        loop {
            let after = String::from(out.last().map_or("", |e| e.name.as_str()));
            let body = self.h_call(h, Kind::Idempotent, |fsd| {
                let mut w = Self::req(op::LIST_H);
                w.u64(fsd).str(&after).u32(u32::MAX);
                w
            })?;
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

    pub fn lock(&self, h: u64, start: u64, len: u64, flags: u8) -> Result<()> {
        self.h_call(h, Kind::Effect, |fsd| {
            let mut w = Self::req(op::LOCK);
            w.u64(fsd).u64(start).u64(len).u8(flags);
            w
        })?;
        self.handles.lock().update(h, |r| r.locked = true);
        Ok(())
    }

    /// Soubor jako proud bajtu (viz [`op::OPEN_STREAM`]). Bez opakovani po
    /// restartu sluzby (proud by zacal znovu).
    pub fn open_stream(&self, h: u64, mode: u8, offset: u64) -> Result<Channel> {
        let fsd = self.resolve(h)?;
        let mut w = Self::req(op::OPEN_STREAM);
        w.u64(fsd).u8(mode).u64(offset);
        let (_, mut handles) = self.call_with_handles(w)?;
        handles.pop().map(Channel::from_handle).ok_or(Error::Corrupted)
    }

    /// Zamek POSIX (viz [`op::POSIX_LOCK`]).
    pub fn posix_lock(&self, h: u64, cmd: u8, class: u8, write: bool, start: u64, len: u64) -> Result<PosixLock> {
        let body = self.h_call(h, Kind::Effect, |fsd| {
            let mut w = Self::req(op::POSIX_LOCK);
            w.u64(fsd).u8(cmd).u8(class).u8(u8::from(write)).u64(start).u64(len);
            w
        })?;
        if cmd != posix_lock::TEST && cmd != posix_lock::UNLOCK {
            self.handles.lock().update(h, |r| r.locked = true);
        }
        let mut r = Reader::new(&body);
        Ok(match r.u8()? {
            posix_lock::DONE => PosixLock::Done,
            posix_lock::CONFLICT => PosixLock::Conflict { write: r.u8()? != 0, start: r.u64()?, len: r.u64()? },
            _ => PosixLock::Deadlock,
        })
    }

    pub fn unlock(&self, h: u64, start: u64, len: u64) -> Result<()> {
        self.h_call(h, Kind::Effect, |fsd| {
            let mut w = Self::req(op::UNLOCK);
            w.u64(fsd).u64(start).u64(len);
            w
        })
        .map(|_| ())
    }

    pub fn delete_on_close(&self, h: u64, on: bool) -> Result<()> {
        self.h_call(h, Kind::Effect, |fsd| {
            let mut w = Self::req(op::DELETE_ON_CLOSE);
            w.u64(fsd).u8(on as u8);
            w
        })?;
        self.handles.lock().update(h, |r| r.delete_on_close = on);
        Ok(())
    }

    pub fn rename_replace(&self, from: &str, to: &str) -> Result<()> {
        let mut w = Self::req(op::RENAME_REPLACE);
        w.str(from).str(to);
        self.call(w).map(|_| ())
    }

    /// Zacne podminene ulozeni `path`; `expected = None` = soubor nesmi
    /// existovat. Vraci handle docasneho souboru pro `write_h`.
    pub fn replace_begin(&self, path: &str, expected: Option<Revision>) -> Result<u64> {
        let mut w = Self::req(op::REPLACE_BEGIN);
        w.str(path).u8(expected.is_some() as u8);
        expected.unwrap_or_default().put(&mut w);
        let fsd = Reader::new(&self.call(w)?).u64()?;
        Ok(self.handles.lock().insert(Rec {
            fsd,
            generation: self.conn.generation(),
            revision: Revision::default(),
            access: access::READ | access::WRITE,
            share: 0,
            replace: true,
            locked: false,
            delete_on_close: false,
            dead: false,
        }))
    }

    /// Dokonci podminene ulozeni; `CONFLICT` = cil se zmenil (docasny soubor
    /// zustava, jde potvrdit s `force` nebo zahodit `replace_abort`).
    pub fn replace_commit(&self, h: u64, force: bool) -> Result<Revision> {
        let body = self.h_call(h, Kind::Effect, |fsd| {
            let mut w = Self::req(op::REPLACE_COMMIT);
            w.u64(fsd).u8(force as u8);
            w
        })?;
        let rev = Revision::get(&mut Reader::new(&body))?;
        // Potvrzene ulozeni handle zavrelo.
        self.handles.lock().remove(h);
        Ok(rev)
    }

    pub fn replace_abort(&self, h: u64) -> Result<()> {
        let gone = |c: &Self| c.stale(h) || c.handles.lock().get(h).is_ok_and(|r| r.dead);
        let r = match self.h_op(op::REPLACE_ABORT, h, Kind::Effect) {
            // Docasny soubor stare instance: zahodi ho uklid sluzby.
            Err(Error::ServiceRestarted | Error::BadHandle) if gone(self) => Ok(Vec::new()),
            r => r,
        };
        self.handles.lock().remove(h);
        r.map(|_| ())
    }

    /// Cele podminene ulozeni najednou: zacatek, zapis, potvrzeni. Pri
    /// `CONFLICT` docasny soubor zahodi a vrati `CONFLICT`.
    pub fn save_if(&self, path: &str, expected: Option<Revision>, data: &[u8]) -> Result<Revision> {
        let h = self.replace_begin(path, expected)?;
        if let Err((_, e)) = self.write_h(h, 0, data) {
            let _ = self.replace_abort(h);
            return Err(e);
        }
        match self.replace_commit(h, false) {
            Ok(r) => Ok(r),
            Err(e) => {
                let _ = self.replace_abort(h);
                Err(e)
            }
        }
    }
}

/// Jmeno docasneho souboru podmineneho ulozeni (ve stejnem adresari).
pub fn temp_name(base: &str, handle: u64) -> String {
    alloc::format!(".~{base}.{handle}.tmp")
}

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

use crate::{DirEntry, FsClient, Reader, Stat, Writer, decode_list};

/// Verze protokolu, kterou tento klient mluvi.
pub const VERSION: u32 = 2;

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
}

/// Pristup a sdileni pri `OPEN` (stejne bity jako `fs::access`/`fs::share`).
pub mod access {
    pub const READ: u8 = 1;
    pub const WRITE: u8 = 2;
    pub const DELETE: u8 = 4;
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

/// Otevreny soubor u klienta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opened {
    pub handle: u64,
    pub stat: Stat,
    pub revision: Revision,
}

impl FsClient {
    /// Verze protokolu sluzby (2 = umi tento modul).
    pub fn hello(&self) -> Result<u32> {
        let body = self.call(Self::req(op::HELLO))?;
        Reader::new(&body).u32()
    }

    /// Nova relace (vlastni handly, nezavisla na teto).
    pub fn connect(&self) -> Result<FsClient> {
        let mut w = Self::req(op::CONNECT);
        w.u32(0);
        let (_, mut handles) = self.call_with_handles(w)?;
        let h = handles.pop().ok_or(Error::Corrupted)?;
        Ok(FsClient::new(Channel::from_handle(h)))
    }

    /// Otevre `path` (priznaky `open_flags`).
    pub fn open(&self, path: &str, access: u8, share: u8, flags: u8) -> Result<Opened> {
        let mut w = Self::req(op::OPEN);
        w.str(path).u8(access).u8(share).u8(flags);
        let body = self.call(w)?;
        let mut r = Reader::new(&body);
        Ok(Opened {
            handle: r.u64()?,
            stat: Stat::get(&mut r)?,
            revision: Revision::get(&mut r)?,
        })
    }

    pub fn close(&self, h: u64) -> Result<()> {
        self.h_op(op::CLOSE, h).map(|_| ())
    }

    fn h_op(&self, opcode: u32, h: u64) -> Result<Vec<u8>> {
        let mut w = Self::req(opcode);
        w.u64(h);
        self.call(w)
    }

    /// Precte az `len` bajtu (po kusech `MAX_IO`).
    pub fn read_h(&self, h: u64, offset: u64, len: usize) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        while out.len() < len {
            let n = (len - out.len()).min(crate::MAX_IO);
            let mut w = Self::req(op::READ_H);
            w.u64(h).u64(offset + out.len() as u64).u32(n as u32);
            let chunk = self.call(w)?;
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
            let mut w = Self::req(op::WRITE_H);
            w.u64(h).u64(offset + done as u64).bytes(chunk);
            let accepted = self
                .call(w)
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
        let mut w = Self::req(op::SET_LEN);
        w.u64(h).u64(size);
        self.call(w).map(|_| ())
    }

    pub fn stat_h(&self, h: u64) -> Result<(Stat, Revision)> {
        let body = self.h_op(op::STAT_H, h)?;
        let mut r = Reader::new(&body);
        Ok((Stat::get(&mut r)?, Revision::get(&mut r)?))
    }

    /// Cely vypis otevreneho adresare (po strankach s kurzorem).
    pub fn list_h(&self, h: u64) -> Result<Vec<DirEntry>> {
        let mut out: Vec<DirEntry> = Vec::new();
        loop {
            let mut w = Self::req(op::LIST_H);
            let after = out.last().map_or("", |e| e.name.as_str());
            w.u64(h).str(after).u32(u32::MAX);
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

    pub fn lock(&self, h: u64, start: u64, len: u64, flags: u8) -> Result<()> {
        let mut w = Self::req(op::LOCK);
        w.u64(h).u64(start).u64(len).u8(flags);
        self.call(w).map(|_| ())
    }

    pub fn unlock(&self, h: u64, start: u64, len: u64) -> Result<()> {
        let mut w = Self::req(op::UNLOCK);
        w.u64(h).u64(start).u64(len);
        self.call(w).map(|_| ())
    }

    pub fn delete_on_close(&self, h: u64, on: bool) -> Result<()> {
        let mut w = Self::req(op::DELETE_ON_CLOSE);
        w.u64(h).u8(on as u8);
        self.call(w).map(|_| ())
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
        Reader::new(&self.call(w)?).u64()
    }

    /// Dokonci podminene ulozeni; `CONFLICT` = cil se zmenil (docasny soubor
    /// zustava, jde potvrdit s `force` nebo zahodit `replace_abort`).
    pub fn replace_commit(&self, h: u64, force: bool) -> Result<Revision> {
        let mut w = Self::req(op::REPLACE_COMMIT);
        w.u64(h).u8(force as u8);
        Revision::get(&mut Reader::new(&self.call(w)?))
    }

    pub fn replace_abort(&self, h: u64) -> Result<()> {
        self.h_op(op::REPLACE_ABORT, h).map(|_| ())
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

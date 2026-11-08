//! Vlastni stav `agentd` v `/system/agent` (relace FS `agentd` ho vidi jako
//! `/state`): tajemstvi stroje, auditni log, utrata a zasifrovane klice.
//!
//! Uzivatele ani agent sem nevidi (prava FS a cesta je na seznamu
//! "neopusti pocitac"). Soubory:
//!
//! - `machine.key` - 32 nahodnych bajtu (klic pro odvozeni klicu uzivatelu),
//! - `audit/<uzivatel>.log` - radky `agentproto::audit::Record`,
//! - `spend/<uzivatel>` - `den utrata` (mikrodolary),
//! - `keys/<uzivatel>.<poskytovatel>` - zasifrovany API klic.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agentproto::audit::Record;
use agentproto::price::Spend;
use agentproto::secret::{self, Secret};
use fsproto::{FsClient, kind};
use libos::{Error, Result};

const ROOT: &str = "/state";

pub struct State {
    fs: FsClient,
    machine: [u8; 32],
    /// Dalsi poradove cislo auditu podle uzivatele.
    seq: BTreeMap<u64, u64>,
}

fn ensure_dir(fs: &FsClient, path: &str) -> Result<()> {
    match fs.create(path, kind::DIR) {
        Ok(()) | Err(Error::AlreadyExists) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Cely obsah souboru (`NOT_FOUND`, kdyz neni).
pub fn read_all(fs: &FsClient, path: &str, max: u64) -> Result<Vec<u8>> {
    let st = fs.stat(path)?;
    if st.size > max {
        return Err(Error::OutOfRange);
    }
    fs.read(path, 0, st.size as usize)
}

/// Prepise soubor celym obsahem (vytvori ho, kdyz neni).
pub fn write_all(fs: &FsClient, path: &str, data: &[u8]) -> Result<()> {
    match fs.create(path, kind::FILE) {
        Ok(()) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(e),
    }
    fs.truncate(path, 0)?;
    if !data.is_empty() {
        fs.write(path, 0, data)?;
    }
    Ok(())
}

/// Dny od 1970 (UTC) a sekundy.
pub fn now() -> (u64, i64) {
    let secs = libos::time::wall().map_or(0, |t| t.secs);
    ((secs.max(0) as u64) / 86_400, secs)
}

impl State {
    /// Otevre stav; tajemstvi stroje vytvori pri prvnim startu.
    pub fn open(fs: FsClient) -> Result<Self> {
        for d in ["audit", "spend", "keys"] {
            ensure_dir(&fs, &format!("{ROOT}/{d}"))?;
        }
        let path = format!("{ROOT}/machine.key");
        let machine = match read_all(&fs, &path, 64) {
            Ok(d) if d.len() == 32 => {
                let mut k = [0u8; 32];
                k.copy_from_slice(&d);
                k
            }
            Ok(_) => return Err(Error::Corrupted),
            Err(Error::NotFound) => {
                let mut k = [0u8; 32];
                libos::random::fill_seeded(&mut k)?;
                write_all(&fs, &path, &k)?;
                fs.fsync()?;
                k
            }
            Err(e) => return Err(e),
        };
        Ok(Self { fs, machine, seq: BTreeMap::new() })
    }

    fn audit_path(user: u64) -> String {
        format!("{ROOT}/audit/{user}.log")
    }

    /// Prida zaznam auditu (poradove cislo doplni). Chyba zapisu se
    /// zaloguje, ale volani neodmitne (audit nesmi zablokovat uzivatele);
    /// prave proto je vraci volajicimu.
    pub fn audit(&mut self, user: u64, mut r: Record) -> Result<u64> {
        let path = Self::audit_path(user);
        let size = match self.fs.stat(&path) {
            Ok(st) => st.size,
            Err(Error::NotFound) => {
                self.fs.create(&path, kind::FILE)?;
                0
            }
            Err(e) => return Err(e),
        };
        let next = match self.seq.get(&user) {
            Some(n) => *n,
            None => {
                // Prvni zapis od startu: pokracovat za poslednim radkem.
                let lines = if size == 0 { 0 } else { self.fs.read(&path, 0, size as usize)?.iter().filter(|&&b| b == b'\n').count() };
                lines as u64 + 1
            }
        };
        r.seq = next;
        self.fs.write(&path, size, r.line().as_bytes())?;
        self.seq.insert(user, next + 1);
        Ok(next)
    }

    /// Zaznamy uzivatele od poradi `from` (nejvic `max`, nejvys 48 KiB);
    /// vraci radky a poradi dalsiho.
    pub fn audit_page(&self, user: u64, from: u64, max: u32) -> Result<(Vec<String>, u64)> {
        let data = match read_all(&self.fs, &Self::audit_path(user), 64 << 20) {
            Ok(d) => d,
            Err(Error::NotFound) => return Ok((Vec::new(), from)),
            Err(e) => return Err(e),
        };
        let text = String::from_utf8_lossy(&data);
        let mut out = Vec::new();
        let mut bytes = 0;
        let mut next = from;
        for line in text.lines() {
            let Ok(r) = Record::parse(line) else { continue };
            if r.seq < from {
                continue;
            }
            if out.len() >= max as usize || bytes + line.len() > 48 * 1024 {
                break;
            }
            bytes += line.len();
            next = r.seq + 1;
            out.push(String::from(line));
        }
        Ok((out, next))
    }

    /// Utrata uzivatele dnes (relace zacina s 0).
    pub fn day_spend(&self, user: u64) -> Spend {
        let (day, _) = now();
        let text = read_all(&self.fs, &format!("{ROOT}/spend/{user}"), 256).unwrap_or_default();
        let text = String::from_utf8_lossy(&text);
        let mut it = text.split_whitespace().map(|v| v.parse::<u64>().unwrap_or(0));
        let (d, micros) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
        Spend { session: 0, day: if d == day { micros } else { 0 }, day_index: day }
    }

    /// Pricte cenu volani k denni utrate uzivatele; vraci novou denni utratu.
    pub fn add_spend(&mut self, user: u64, cost: u64) -> Result<u64> {
        let mut s = self.day_spend(user);
        s.day = s.day.saturating_add(cost);
        write_all(&self.fs, &format!("{ROOT}/spend/{user}"), format!("{} {}\n", s.day_index, s.day).as_bytes())?;
        Ok(s.day)
    }

    /// Ulozi API klic zasifrovany pro uzivatele a poskytovatele.
    pub fn store_key(&mut self, user: u64, provider: &str, key: &[u8]) -> Result<()> {
        if !provider.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') || provider.is_empty() {
            return Err(Error::InvalidArgs);
        }
        let mut nonce = [0u8; 12];
        libos::random::fill_seeded(&mut nonce)?;
        let sealed = secret::seal(&self.machine, user, provider, &nonce, key);
        write_all(&self.fs, &format!("{ROOT}/keys/{user}.{provider}"), &sealed)?;
        self.fs.fsync()
    }

    /// API klic uzivatele (`NOT_FOUND` = jeste ho nezadal).
    pub fn load_key(&self, user: u64, provider: &str) -> Result<Secret> {
        let sealed = read_all(&self.fs, &format!("{ROOT}/keys/{user}.{provider}"), 4096)?;
        secret::open(&self.machine, user, provider, &sealed)
    }
}

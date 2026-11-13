//! Uloha kopirovani nebo presunu po malych krocich.
//!
//! Kazde volani [`Job::step`] udela omezenou praci (jedna okamzita kopie,
//! jedno prejmenovani nebo jeden blok dat [`CHUNK`]), takze proces mezi
//! kroky stihne zpravy (pozastavit, zrusit, rozhodnuti konfliktu).
//!
//! Polozka se sklada do docasneho jmena vedle cile
//! (`.<jmeno>.kopie-<uloha>`) a az hotova se prejmenuje na cil: cil nikdy
//! neni polovicni. Po preruseni (i po restartu systemu) se z ulozeneho
//! stavu ([`Job::encode`]) pokracuje: okamzita kopie se zopakuje, kopie
//! ctenim pokracuje od posledniho ulozeneho mista.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use wire::{Reader, Writer};

use crate::path::{is_within, join, name, numbered};
use crate::{Error, Kind, Result, Vfs};

/// Blok kopie ctenim a zapisem.
pub const CHUNK: usize = 256 * 1024;
/// Ulozit stav nejpozdeji po tolika zkopirovanych bajtech.
pub const SAVE_EVERY: u64 = 8 * 1024 * 1024;
/// Nejvic zapamatovanych chyb (dalsi se jen pocitaji).
pub const MAX_ERRORS: usize = 64;
/// Verze ulozeneho stavu.
const STATE_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Copy,
    Move,
}

/// Rozhodnuti konfliktu (cil uz existuje).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Puvodni cil do kose, na jeho misto nova polozka.
    Overwrite,
    Skip,
    /// Nova polozka s volnym jmenem (`a (2).txt`).
    Rename,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Running,
    Paused,
    /// Ceka na rozhodnuti konfliktu ([`Job::decide`]).
    Conflict,
    Done,
    Canceled,
}

/// Konflikt: `target` uz existuje.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub source: String,
    pub target: String,
}

/// Vysledek kroku.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Neco se udelalo; volat znovu.
    Worked,
    /// Ceka na rozhodnuti.
    Conflict(Conflict),
    /// Nic k praci (pozastaveno, hotovo, zruseno).
    Idle,
}

/// Rozpracovana polozka.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cur {
    src: String,
    target: String,
    tmp: String,
    /// Cil existuje a ma se prepsat (puvodni do kose az pri dokonceni).
    replace: bool,
    /// Bajty polozky pro prubeh, kdyz jdou celou polozkou najednou.
    bytes: u64,
    mode: Mode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    /// Okamzita kopie nebo prejmenovani.
    Start,
    /// Kopie ctenim a zapisem: fronta relativnich cest (`""` = polozka
    /// sama) a soubor, ktery se prave kopiruje (cesta, posun, velikost).
    Fallback { queue: Vec<String>, file: Option<(String, u64, u64)> },
    /// Obsah je v docasnem jmenu; zbyva prejmenovat (a zdroj do kose).
    Finish,
}

/// Uloha kopirovani / presunu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: u64,
    pub op: Op,
    pub sources: Vec<String>,
    /// Cilova slozka.
    pub dest: String,
    /// Rozhodnuti "pro vsechny" (dalsi konflikty se neptaji).
    pub policy: Option<Resolution>,
    pub phase: Phase,
    /// Index dalsi polozky `sources`.
    next: usize,
    cur: Option<Cur>,
    conflict: Option<Conflict>,
    pub done_items: u32,
    pub skipped: u32,
    pub done_bytes: u64,
    /// `None` = nezname (nekde chybi souhrn slozky), prubeh po polozkach.
    pub total_bytes: Option<u64>,
    prepared: bool,
    /// Chyby polozek (zdroj, chyba); nejvic [`MAX_ERRORS`].
    pub errors: Vec<(String, Error)>,
    pub error_count: u32,
    unsaved: u64,
    item_done: bool,
}

impl Job {
    pub fn new(id: u64, op: Op, sources: Vec<String>, dest: String) -> Self {
        Self {
            id,
            op,
            sources,
            dest,
            policy: None,
            phase: Phase::Running,
            next: 0,
            cur: None,
            conflict: None,
            done_items: 0,
            skipped: 0,
            done_bytes: 0,
            total_bytes: None,
            prepared: false,
            errors: Vec::new(),
            error_count: 0,
            unsaved: 0,
            item_done: false,
        }
    }

    pub fn total_items(&self) -> u32 {
        self.sources.len() as u32
    }

    /// Prubeh v procentech (po bajtech, kdyz jsou zname, jinak po
    /// polozkach).
    pub fn percent(&self) -> u8 {
        let p = match self.total_bytes {
            Some(t) if t > 0 => self.done_bytes.min(t) * 100 / t,
            _ if !self.sources.is_empty() => (self.done_items + self.skipped + self.error_count).min(self.total_items()) as u64 * 100 / self.sources.len() as u64,
            _ => 100,
        };
        p as u8
    }

    /// Prave cekajici konflikt.
    pub fn conflict(&self) -> Option<&Conflict> {
        self.conflict.as_ref()
    }

    /// Docasne jmeno polozky teto ulohy (spravce ho ve vypisu skryje).
    pub fn temp_name(id: u64, item: &str) -> String {
        format!(".{item}.kopie-{id}")
    }

    /// Je jmeno docasne jmeno nejake ulohy (`.x.kopie-<cislo>`)?
    pub fn is_temp_name(n: &str) -> bool {
        n.starts_with('.') && n.rsplit_once(".kopie-").is_some_and(|(a, d)| a.len() > 1 && !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
    }

    pub fn pause(&mut self) {
        if self.phase == Phase::Running {
            self.phase = Phase::Paused;
        }
    }

    pub fn resume(&mut self) {
        if self.phase == Phase::Paused {
            self.phase = if self.conflict.is_some() { Phase::Conflict } else { Phase::Running };
        }
    }

    /// Zrusit: rozpracovana polozka (docasne jmeno) se smaze, hotove
    /// polozky zustanou.
    pub fn cancel(&mut self, fs: &mut impl Vfs) {
        if let Some(c) = self.cur.take() {
            let _ = fs.remove_tree(&c.tmp);
        }
        self.conflict = None;
        self.phase = Phase::Canceled;
        self.item_done = true;
    }

    /// Rozhodnuti cekajiciho konfliktu; `all` = i pro dalsi konflikty.
    pub fn decide(&mut self, fs: &mut impl Vfs, r: Resolution, all: bool) {
        let Some(c) = self.conflict.take() else { return };
        if all {
            self.policy = Some(r);
        }
        if self.phase == Phase::Conflict {
            self.phase = Phase::Running;
        }
        self.resolve(fs, c.source, c.target, r);
    }

    /// Po nacteni ulozeneho stavu: rozpracovana okamzita kopie se zopakuje
    /// (docasne jmeno pryc); konflikt se zepta znovu.
    pub fn resumed(&mut self, fs: &mut impl Vfs) {
        if let Some(c) = &self.cur
            && c.mode == Mode::Start
        {
            let _ = fs.remove_tree(&c.tmp);
        }
        if self.conflict.is_some() && self.phase == Phase::Running {
            self.phase = Phase::Conflict;
        }
    }

    /// Ma se ted ulozit stav (konec polozky nebo [`SAVE_EVERY`] bajtu)?
    /// Pred ulozenim `Vfs::sync`.
    pub fn should_save(&self) -> bool {
        self.item_done || self.unsaved >= SAVE_EVERY
    }

    /// Stav ulozen.
    pub fn saved(&mut self) {
        self.item_done = false;
        self.unsaved = 0;
    }

    /// Jeden krok prace.
    pub fn step(&mut self, fs: &mut impl Vfs) -> Step {
        match self.phase {
            Phase::Paused | Phase::Done | Phase::Canceled => return Step::Idle,
            Phase::Conflict => return self.conflict.clone().map_or(Step::Idle, Step::Conflict),
            Phase::Running => {}
        }
        if !self.prepared {
            self.prepare(fs);
            return Step::Worked;
        }
        if self.cur.is_none() {
            if self.next >= self.sources.len() {
                let _ = fs.sync();
                self.phase = Phase::Done;
                self.item_done = true;
                return Step::Idle;
            }
            let src = self.sources[self.next].clone();
            self.next += 1;
            self.plan(fs, src);
            return match &self.conflict {
                Some(c) => Step::Conflict(c.clone()),
                None => Step::Worked,
            };
        }
        self.work(fs);
        Step::Worked
    }

    /// Celkova velikost (souhrny slozek z FS, nic se neprochazi).
    fn prepare(&mut self, fs: &mut impl Vfs) {
        let mut total = Some(0u64);
        for s in &self.sources {
            total = match (total, fs.stat(s).ok().and_then(|m| m.bytes())) {
                (Some(t), Some(b)) => Some(t + b),
                _ => None,
            };
        }
        self.total_bytes = total;
        self.prepared = true;
    }

    fn fail(&mut self, fs: &mut impl Vfs, src: &str, e: Error) {
        if let Some(c) = self.cur.take() {
            let _ = fs.remove_tree(&c.tmp);
        }
        self.error_count += 1;
        if self.errors.len() < MAX_ERRORS {
            self.errors.push((String::from(src), e));
        }
        self.item_done = true;
    }

    /// Nova polozka: kontroly, konflikt, rozpracovany stav.
    fn plan(&mut self, fs: &mut impl Vfs, src: String) {
        let item = String::from(name(&src));
        if item.is_empty() || is_within(&self.dest, &src) {
            // Slozka do sebe sama (nebo koren): nejde.
            return self.fail(fs, &src.clone(), Error::InvalidArgs);
        }
        if let Err(e) = fs.stat(&src) {
            return self.fail(fs, &src.clone(), e);
        }
        let target = join(&self.dest, &item);
        if target.eq_ignore_ascii_case(&src) {
            match self.op {
                // Presun na stejne misto: nic.
                Op::Move => {
                    self.done_items += 1;
                    self.item_done = true;
                }
                // Kopie do stejne slozky: rovnou volne jmeno (bez dotazu).
                Op::Copy => self.resolve(fs, src, target, Resolution::Rename),
            }
            return;
        }
        match fs.stat(&target) {
            Err(Error::NotFound) => self.begin(fs, src, target, false),
            Err(e) => self.fail(fs, &src.clone(), e),
            Ok(_) => match self.policy {
                Some(r) => self.resolve(fs, src, target, r),
                None => {
                    self.conflict = Some(Conflict { source: src, target });
                    self.phase = Phase::Conflict;
                }
            },
        }
    }

    fn resolve(&mut self, fs: &mut impl Vfs, src: String, target: String, r: Resolution) {
        match r {
            Resolution::Skip => {
                self.skipped += 1;
                self.item_done = true;
            }
            Resolution::Overwrite => self.begin(fs, src, target, true),
            Resolution::Rename => {
                let is_dir = fs.stat(&src).is_ok_and(|m| m.kind == Kind::Dir);
                let item = String::from(name(&target));
                for n in 2..10_000u32 {
                    let cand = join(&self.dest, &numbered(&item, n, is_dir));
                    match fs.stat(&cand) {
                        Err(Error::NotFound) => return self.begin(fs, src, cand, false),
                        Err(e) => return self.fail(fs, &src.clone(), e),
                        Ok(_) => {}
                    }
                }
                self.fail(fs, &src.clone(), Error::AlreadyExists);
            }
        }
    }

    fn begin(&mut self, fs: &mut impl Vfs, src: String, target: String, replace: bool) {
        let bytes = fs.stat(&src).ok().and_then(|m| m.bytes()).unwrap_or(0);
        let tmp = join(&self.dest, &Self::temp_name(self.id, name(&target)));
        // Zbytek predchoziho pokusu (nemel by byt).
        let _ = fs.remove_tree(&tmp);
        self.cur = Some(Cur { src, target, tmp, replace, bytes, mode: Mode::Start });
    }

    fn work(&mut self, fs: &mut impl Vfs) {
        let Some(mut c) = self.cur.take() else { return };
        let r = match &mut c.mode {
            Mode::Start => self.start(fs, &mut c),
            Mode::Fallback { .. } => self.fallback(fs, &mut c),
            Mode::Finish => self.finish(fs, &c).map(|_| true),
        };
        match r {
            Ok(true) => {}
            Ok(false) => self.cur = Some(c),
            Err(e) => {
                self.cur = Some(c.clone());
                self.fail(fs, &c.src, e);
            }
        }
    }

    /// Okamzita kopie / prejmenovani. `Ok(true)` = polozka hotova.
    fn start(&mut self, fs: &mut impl Vfs, c: &mut Cur) -> Result<bool> {
        if self.op == Op::Move && fs.dataset_of(&c.src)? == fs.dataset_of(&self.dest)? {
            // Presun v jednom datasetu: prejmenovani (okamzite).
            if c.replace {
                trash_existing(fs, &c.target)?;
            }
            fs.rename(&c.src, &c.target)?;
            self.item_finished(c.bytes);
            return Ok(true);
        }
        match fs.clone_path(&c.src, &c.tmp) {
            Ok(()) => {
                self.done_bytes += c.bytes;
                self.unsaved += c.bytes;
                c.bytes = 0;
                c.mode = Mode::Finish;
                Ok(false)
            }
            Err(Error::NotSupported) => {
                c.mode = Mode::Fallback { queue: alloc::vec![String::new()], file: None };
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    /// Kopie ctenim a zapisem (jeden krok).
    fn fallback(&mut self, fs: &mut impl Vfs, c: &mut Cur) -> Result<bool> {
        let Mode::Fallback { queue, file } = &mut c.mode else { return Ok(false) };
        if let Some((rel, off, size)) = file {
            let data = fs.read(&sub(&c.src, rel), *off, CHUNK.min((*size - *off) as usize))?;
            if data.is_empty() && *off < *size {
                // Zdroj se zkratil: kopie konci tim, co je.
                *file = None;
                return Ok(false);
            }
            fs.write(&sub(&c.tmp, rel), *off, &data)?;
            *off += data.len() as u64;
            self.done_bytes += data.len() as u64;
            self.unsaved += data.len() as u64;
            if *off >= *size {
                *file = None;
            }
            return Ok(false);
        }
        let Some(rel) = queue.pop() else {
            c.bytes = 0;
            c.mode = Mode::Finish;
            return Ok(false);
        };
        let s = sub(&c.src, &rel);
        let t = sub(&c.tmp, &rel);
        let m = fs.stat(&s)?;
        match m.kind {
            Kind::Dir => {
                fs.mkdir(&t)?;
                let mut kids = fs.list(&s)?;
                kids.sort_by(|a, b| b.0.cmp(&a.0));
                for (n, _) in kids {
                    queue.push(if rel.is_empty() { n } else { format!("{rel}/{n}") });
                }
            }
            Kind::File => {
                fs.create_file(&t)?;
                if m.size > 0 {
                    *file = Some((rel, 0, m.size));
                }
            }
            Kind::Link => fs.clone_path(&s, &t)?,
        }
        Ok(false)
    }

    fn finish(&mut self, fs: &mut impl Vfs, c: &Cur) -> Result<()> {
        match fs.stat(&c.tmp) {
            Ok(_) => {
                if c.replace {
                    trash_existing(fs, &c.target)?;
                }
                fs.rename(&c.tmp, &c.target)?;
            }
            // Po obnove: prejmenovani uz probehlo pred prerusenim.
            Err(Error::NotFound) if fs.stat(&c.target).is_ok() => {}
            Err(e) => return Err(e),
        }
        if self.op == Op::Move {
            trash_existing(fs, &c.src)?;
        }
        self.item_finished(c.bytes);
        Ok(())
    }

    fn item_finished(&mut self, bytes: u64) {
        self.done_bytes += bytes;
        self.done_items += 1;
        self.item_done = true;
    }

    /// Ulozeny stav (pro obnovu po preruseni).
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(STATE_VERSION).u64(self.id).u8(matches!(self.op, Op::Move) as u8).str(&self.dest);
        put_strs(&mut w, &self.sources);
        put_res(&mut w, self.policy);
        w.u8(match self.phase {
            Phase::Running => 0,
            Phase::Paused => 1,
            Phase::Conflict => 2,
            Phase::Done => 3,
            Phase::Canceled => 4,
        });
        w.u32(self.next as u32).u32(self.done_items).u32(self.skipped).u64(self.done_bytes);
        w.u8(self.total_bytes.is_some() as u8).u64(self.total_bytes.unwrap_or(0)).u8(self.prepared as u8);
        w.u32(self.error_count).u32(self.errors.len() as u32);
        for (p, e) in &self.errors {
            w.str(p).u64(e.code() as i64 as u64);
        }
        match &self.conflict {
            Some(c) => {
                w.u8(1).str(&c.source).str(&c.target);
            }
            None => {
                w.u8(0);
            }
        }
        match &self.cur {
            None => {
                w.u8(0);
            }
            Some(c) => {
                w.u8(1).str(&c.src).str(&c.target).str(&c.tmp).u8(c.replace as u8).u64(c.bytes);
                match &c.mode {
                    Mode::Start => {
                        w.u8(0);
                    }
                    Mode::Fallback { queue, file } => {
                        w.u8(1);
                        put_strs(&mut w, queue);
                        match file {
                            Some((p, off, size)) => {
                                w.u8(1).str(p).u64(*off).u64(*size);
                            }
                            None => {
                                w.u8(0);
                            }
                        }
                    }
                    Mode::Finish => {
                        w.u8(2);
                    }
                }
            }
        }
        w.finish()
    }

    /// Precte ulozeny stav. Neplatna data = `INVALID_ARGS` (stav z disku
    /// je nedoveryhodny vstup). Rozpracovana polozka po obnove: okamzita
    /// kopie se zopakuje (docasne jmeno se smaze), kopie ctenim pokracuje.
    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        if r.u8()? != STATE_VERSION {
            return Err(Error::NotSupported);
        }
        let id = r.u64()?;
        let op = match r.u8()? {
            0 => Op::Copy,
            1 => Op::Move,
            _ => return Err(Error::InvalidArgs),
        };
        let dest = r.str()?;
        let sources = get_strs(&mut r)?;
        let policy = get_res(&mut r)?;
        let phase = match r.u8()? {
            0 => Phase::Running,
            1 => Phase::Paused,
            2 => Phase::Conflict,
            3 => Phase::Done,
            4 => Phase::Canceled,
            _ => return Err(Error::InvalidArgs),
        };
        let next = r.u32()? as usize;
        let (done_items, skipped, done_bytes) = (r.u32()?, r.u32()?, r.u64()?);
        let has_total = r.u8()? != 0;
        let total = r.u64()?;
        let prepared = r.u8()? != 0;
        let error_count = r.u32()?;
        let n = r.u32()? as usize;
        if n > MAX_ERRORS || next > sources.len() {
            return Err(Error::InvalidArgs);
        }
        let mut errors = Vec::with_capacity(n);
        for _ in 0..n {
            let p = r.str()?;
            let e = Error::from_code(r.u64()? as i64).unwrap_or(Error::IoError);
            errors.push((p, e));
        }
        let conflict = match r.u8()? {
            0 => None,
            1 => Some(Conflict { source: r.str()?, target: r.str()? }),
            _ => return Err(Error::InvalidArgs),
        };
        let cur = match r.u8()? {
            0 => None,
            1 => {
                let (src, target, tmp) = (r.str()?, r.str()?, r.str()?);
                let replace = r.u8()? != 0;
                let bytes = r.u64()?;
                let mode = match r.u8()? {
                    0 => Mode::Start,
                    1 => {
                        let queue = get_strs(&mut r)?;
                        let file = match r.u8()? {
                            0 => None,
                            1 => {
                                let (p, off, size) = (r.str()?, r.u64()?, r.u64()?);
                                if off > size {
                                    return Err(Error::InvalidArgs);
                                }
                                Some((p, off, size))
                            }
                            _ => return Err(Error::InvalidArgs),
                        };
                        Mode::Fallback { queue, file }
                    }
                    2 => Mode::Finish,
                    _ => return Err(Error::InvalidArgs),
                };
                Some(Cur { src, target, tmp, replace, bytes, mode })
            }
            _ => return Err(Error::InvalidArgs),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(Self {
            id,
            op,
            sources,
            dest,
            policy,
            phase,
            next,
            cur,
            conflict,
            done_items,
            skipped,
            done_bytes,
            total_bytes: has_total.then_some(total),
            prepared,
            errors,
            error_count,
            unsaved: 0,
            item_done: false,
        })
    }
}

/// Cesta `rel` uvnitr `base` (`""` = `base`).
fn sub(base: &str, rel: &str) -> String {
    if rel.is_empty() { String::from(base) } else { join(base, rel) }
}

/// Puvodni cil do kose; uz neexistujici cil nevadi.
fn trash_existing(fs: &mut impl Vfs, target: &str) -> Result<()> {
    match fs.trash(target) {
        Ok(()) | Err(Error::NotFound) => Ok(()),
        Err(e) => Err(e),
    }
}

const MAX_LIST: usize = 1 << 20;

fn put_strs(w: &mut Writer, v: &[String]) {
    w.u32(v.len() as u32);
    for s in v {
        w.str(s);
    }
}

fn get_strs(r: &mut Reader) -> Result<Vec<String>> {
    let n = r.u32()? as usize;
    if n > MAX_LIST {
        return Err(Error::InvalidArgs);
    }
    let mut v = Vec::with_capacity(n.min(1024));
    for _ in 0..n {
        v.push(r.str()?);
    }
    Ok(v)
}

pub(crate) fn put_res(w: &mut Writer, r: Option<Resolution>) {
    w.u8(match r {
        None => 0,
        Some(Resolution::Overwrite) => 1,
        Some(Resolution::Skip) => 2,
        Some(Resolution::Rename) => 3,
    });
}

pub(crate) fn get_res(r: &mut Reader) -> Result<Option<Resolution>> {
    Ok(match r.u8()? {
        0 => None,
        1 => Some(Resolution::Overwrite),
        2 => Some(Resolution::Skip),
        3 => Some(Resolution::Rename),
        _ => return Err(Error::InvalidArgs),
    })
}

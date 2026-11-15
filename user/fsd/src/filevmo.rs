//! Objekty pameti souboru pro `mmap` (fsproto v2 `MAP_VMO`, docs/cs/souborovy-system.md,
//! kapitola Soubory jako pamet).
//!
//! Soubor se mapuje po **oknech** (`WINDOW`, zarovnane): kazde okno je
//! zvetsitelny objekt pameti s kapacitou `WINDOW` a velikosti podle souboru
//! (strankove zaokrouhlene). Vsechny procesy dostanou stejne okno - sdilene
//! mapovani ma v kazdem procesu tytez ramce. Mapovani rozsahu = vic oken za
//! sebou v jedne rezervaci klienta; soubor muze mit libovolnou velikost,
//! limit plati na namapovany rozsah (kvota relace `SESSION_QUOTA`).
//!
//! **Zdroj pravdy:** pro nactene okno je obsah v objektu pameti, ne ve FS.
//! `READ_H`, `WRITE_H`, `SET_LEN` a velikost v `STAT_H` jdou pres zaznam
//! souboru (`Entry`), takze `read()`/`write()` a mapovani jsou koherentni
//! v obou smerech. Nenactena okna ctou a pisou primo FS. Pred kazdou jinou
//! operaci (cesta, snapshot, kopie, commit, `fsync`...) se spinave stranky
//! zapisou zpet (`flush_all`) a po ni se zjisti cizi zmena souboru (verze
//! objektu) a okna se znovu nactou (`refresh_all`).
//!
//! **Zapis zpet:** kazde okno je od nacteni sledovane (`VMO_DIRTY`);
//! spinave stranky se zapisou pres interni handle (`Fs::pin_h`, drzi objekt
//! i po zavreni fd a smazani jmena) pri `SYNC_VMO` (`msync`, `fsync`),
//! `UNMAP_VMO` (`munmap`), zavreni handlu, konci relace, pred cizi operaci
//! a nejpozdeji po `WRITEBACK` (hlavni smycka).
//!
//! Pamet oken se pocita do rozpoctu fsd (vytvari je fsd), ne do jobu
//! klienta. Sdilene mapovani drzi okno (`pins`), dokud ho klient neuvolni
//! (`UNMAP_VMO`) nebo neskonci relace; zaznamy bez drzeni se zahazuji od
//! nejstarsiho pouziti nad `CACHE_MAX`. Soukrome mapovani je klon okna
//! (`VMO_CLONE`) v pameti klienta a okno nedrzi.
//!
//! Pozdeji (pager v jadre, plneni po strankach pri vypadku) se zmeni jen
//! plneni oken zde: klient dal dostane handle okna a nic dalsiho.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use core::time::Duration;

use fs::Fs;
use libos::{Error, Result, Vmo};

use crate::blkdev::BlkDev;

/// Velikost okna souboru (zarovnani i kapacita objektu pameti okna).
pub const WINDOW: u64 = fsproto::v2::WINDOW;
/// Nejvyssi cislo okna (soubor do 2^20 * 16 MiB = 16 TiB).
pub const MAX_WINDOWS: u64 = 1 << 20;
/// Nejvic bajtu oken, ktera jedna relace drzi sdilene namapovana.
pub const SESSION_QUOTA: u64 = 1024 * 1024 * 1024;
/// Nejvic bajtu oken ve fsd; nad tim se zahodi nejstarsi zaznamy bez drzeni.
pub const CACHE_MAX: u64 = 512 * 1024 * 1024;
/// Nejdele, jak stare jsou spinave stranky, nez se zapisou zpet.
pub const WRITEBACK: Duration = Duration::from_secs(30);
/// Kolik indexu spinavych stranek cist jednim `VMO_DIRTY`.
const DIRTY_BATCH: usize = 512;

/// Klic souboru: (dataset, oid).
pub type Key = (u64, u64);

struct Window {
    vmo: Vmo,
    /// Adresa trvaleho mapovani okna ve fsd (cela kapacita `WINDOW`).
    addr: u64,
    /// Aktualni velikost objektu (cele stranky).
    len: u64,
}

impl Window {
    fn bytes(&self) -> &mut [u8] {
        // SAFETY: [addr, addr + WINDOW) je trvale namapovany objekt okna
        // s pravem zapisu (odmapuje se az v `Drop`); pristup jen do `len`
        // (velikost objektu) hlida volajici. Soucasny zapis klienta do
        // tychz stranek je zamer (sdilena pamet), ne datovy zavod v Rustu:
        // fsd s obsahem pracuje jen jako s bajty.
        unsafe { core::slice::from_raw_parts_mut(self.addr as *mut u8, WINDOW as usize) }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        let _ = libos::sys::vm_unmap(libos::rt::self_process(), self.addr, WINDOW);
    }
}

/// Zaznam namapovaneho souboru.
pub struct Entry {
    /// Interni handle FS (`Fs::pin_h`): zapis zpet i po zavreni fd klienta.
    fh: fs::Handle,
    /// Velikost souboru (zdroj pravdy, dokud zaznam existuje).
    size: u64,
    /// Verze objektu ve FS po posledni vlastni zmene (cizi zmena = jina).
    version: u64,
    windows: BTreeMap<u64, Window>,
    /// Kolik sdilenych mapovani (relace, okno) zaznam drzi.
    pins: u32,
    used: u64,
}

/// Zaznamy namapovanych souboru.
#[derive(Default)]
pub struct FileVmos {
    map: BTreeMap<Key, Entry>,
    /// Soucet velikosti nactenych oken.
    bytes: u64,
    clock: u64,
}

fn page() -> u64 {
    libos::vm::page_size()
}

/// Velikost objektu okna od `start` pro soubor o velikosti `size`.
fn window_len(size: u64, start: u64) -> u64 {
    size.saturating_sub(start).min(WINDOW).div_ceil(page()) * page()
}

/// Vsechny spinave stranky okna (indexy); vycisti je.
pub fn take_dirty(vmo: &Vmo) -> Result<Vec<u64>> {
    let mut out = Vec::new();
    let mut buf = [0u64; DIRTY_BATCH];
    loop {
        let n = libos::sys::vmo_dirty(vmo.handle().raw(), &mut buf)?;
        out.extend_from_slice(&buf[..n]);
        if n < DIRTY_BATCH {
            return Ok(out);
        }
    }
}

/// Precte z FS `buf.len()` bajtu od `off`; za koncem souboru nuly.
fn read_fs(fs: &mut Fs<BlkDev>, h: fs::Handle, off: u64, buf: &mut [u8]) -> Result<()> {
    let mut done = 0usize;
    while done < buf.len() {
        let n = (buf.len() - done).min(fsproto::VMO_IO);
        let d = fs.read_h(h, off + done as u64, n)?;
        buf[done..done + d.len()].copy_from_slice(&d);
        if d.len() < n {
            buf[done + d.len()..].fill(0);
            return Ok(());
        }
        done += n;
    }
    Ok(())
}

impl Entry {
    /// Nastavi velikost objektu nactenych oken podle `size` (zmenseni =
    /// pristup za koncem v mapovanich je `SIGBUS`).
    fn fit(&mut self, bytes: &mut u64) -> Result<()> {
        for (&idx, w) in self.windows.iter_mut() {
            let want = window_len(self.size, idx * WINDOW);
            if want != w.len {
                w.vmo.set_size(want)?;
                *bytes = *bytes - w.len + want;
                w.len = want;
            }
        }
        Ok(())
    }

    /// Zapise spinave stranky oken do FS (jen bajty pod velikosti souboru)
    /// a srovna velikost ve FS. Stranka, kterou se zapsat nepodarilo, se
    /// znovu oznaci (zapis do ni) - zkusi se priste. Vraci pocet stranek.
    fn flush(&mut self, fs: &mut Fs<BlkDev>) -> Result<u64> {
        let pg = page();
        let mut first_err = None;
        let mut count = 0u64;
        for (&idx, w) in self.windows.iter() {
            let pages = take_dirty(&w.vmo)?;
            count += pages.len() as u64;
            let start = idx * WINDOW;
            let mut i = 0;
            while i < pages.len() {
                // Souvisly beh stranek.
                let mut j = i + 1;
                while j < pages.len() && pages[j] == pages[j - 1] + 1 {
                    j += 1;
                }
                let lo = pages[i] * pg;
                let hi = ((pages[j - 1] + 1) * pg).min(self.size.saturating_sub(start)).min(w.len);
                let mut at = lo;
                while at < hi {
                    let n = (hi - at).min(fsproto::VMO_IO as u64);
                    let chunk = &w.bytes()[at as usize..(at + n) as usize];
                    if let Err(e) = fs.write_h(self.fh, start + at, chunk) {
                        first_err.get_or_insert(e);
                        // Znovu oznacit: zapis tehoz bajtu vyvola vypadek
                        // sledovani (stranka je po vycisteni jen pro cteni).
                        let mut p = at - at % pg;
                        while p < hi {
                            let b = w.bytes();
                            // SAFETY: p < len objektu; volatile, aby se zapis
                            // stejne hodnoty nevypustil.
                            unsafe { core::ptr::write_volatile(&mut b[p as usize], core::ptr::read_volatile(&b[p as usize])) };
                            p += pg;
                        }
                        break;
                    }
                    at += n;
                }
                i = j;
            }
        }
        if let Some(e) = first_err {
            return Err(e);
        }
        if fs.stat_h(self.fh)?.size != self.size {
            fs.set_len_h(self.fh, self.size)?;
        }
        self.version = fs.revision_h(self.fh)?.version;
        Ok(count)
    }

    /// Cizi zmena ve FS (verze objektu jina nez po vlastni zmene): velikost
    /// a nactena okna znovu z FS.
    fn refresh(&mut self, fs: &mut Fs<BlkDev>, bytes: &mut u64) -> Result<()> {
        let rev = fs.revision_h(self.fh)?;
        if rev.version == self.version {
            return Ok(());
        }
        self.size = fs.stat_h(self.fh)?.size;
        self.fit(bytes)?;
        for (&idx, w) in self.windows.iter() {
            let start = idx * WINDOW;
            let n = self.size.saturating_sub(start).min(w.len) as usize;
            read_fs(fs, self.fh, start, &mut w.bytes()[..n])?;
            w.bytes()[n..w.len as usize].fill(0);
            // Obsah je z FS: neni co zapisovat zpet.
            take_dirty(&w.vmo)?;
        }
        self.version = rev.version;
        Ok(())
    }

    /// Nacte okno `idx` (objekt, trvale mapovani ve fsd, obsah z FS,
    /// sledovani zapisu).
    fn load(&mut self, fs: &mut Fs<BlkDev>, idx: u64, bytes: &mut u64) -> Result<&Window> {
        if !self.windows.contains_key(&idx) {
            let start = idx * WINDOW;
            let len = window_len(self.size, start);
            let vmo = Vmo::create_resizable(len, WINDOW)?;
            let me = libos::rt::self_process();
            let flags = libos::abi::vm::MAP_READ | libos::abi::vm::MAP_WRITE;
            let addr = libos::sys::vm_map(me, vmo.handle().raw(), 0, WINDOW, flags, 0)?;
            let w = Window { vmo, addr, len };
            let n = self.size.saturating_sub(start).min(len) as usize;
            read_fs(fs, self.fh, start, &mut w.bytes()[..n])?;
            // Prvni volani zapne sledovani (a vrati nactene stranky).
            take_dirty(&w.vmo)?;
            *bytes += len;
            self.windows.insert(idx, w);
        }
        self.windows.get(&idx).ok_or(Error::NotFound)
    }

    /// Cteni pres zaznam (`READ_H`): nactena okna z pameti, ostatni z FS.
    pub fn read(&mut self, fs: &mut Fs<BlkDev>, off: u64, n: usize) -> Result<Vec<u8>> {
        let end = off.saturating_add(n as u64).min(self.size);
        if off >= end {
            return Ok(Vec::new());
        }
        let mut out = alloc::vec![0u8; (end - off) as usize];
        let mut at = off;
        while at < end {
            let idx = at / WINDOW;
            let wend = ((idx + 1) * WINDOW).min(end);
            let dst = &mut out[(at - off) as usize..(wend - off) as usize];
            match self.windows.get(&idx) {
                Some(w) => {
                    let s = (at - idx * WINDOW) as usize;
                    dst.copy_from_slice(&w.bytes()[s..s + dst.len()]);
                }
                None => read_fs(fs, self.fh, at, dst)?,
            }
            at = wend;
        }
        Ok(out)
    }

    /// Zapis pres zaznam (`WRITE_H`): nactena okna do pameti (spinave
    /// stranky jdou zpet pozdeji), ostatni rovnou do FS.
    pub fn write(&mut self, fs: &mut Fs<BlkDev>, off: u64, data: &[u8], bytes: &mut u64) -> Result<()> {
        let end = off.checked_add(data.len() as u64).ok_or(Error::InvalidArgs)?;
        if end > self.size {
            self.size = end;
            self.fit(bytes)?;
        }
        let mut direct = false;
        let mut at = off;
        while at < end {
            let idx = at / WINDOW;
            let wend = ((idx + 1) * WINDOW).min(end);
            let src = &data[(at - off) as usize..(wend - off) as usize];
            match self.windows.get(&idx) {
                Some(w) => {
                    let s = (at - idx * WINDOW) as usize;
                    w.bytes()[s..s + src.len()].copy_from_slice(src);
                }
                None => {
                    fs.write_h(self.fh, at, src)?;
                    direct = true;
                }
            }
            at = wend;
        }
        if direct {
            self.version = fs.revision_h(self.fh)?.version;
        }
        Ok(())
    }

    /// Zmena velikosti (`SET_LEN`, uz provedena ve FS): okna se zmensi nebo
    /// zvetsi, zbytek posledni stranky za koncem se vynuluje.
    pub fn set_len(&mut self, fs: &mut Fs<BlkDev>, size: u64, bytes: &mut u64) -> Result<()> {
        self.size = size;
        self.fit(bytes)?;
        let pg = page();
        if size % pg != 0
            && let Some(w) = self.windows.get(&(size / WINDOW))
        {
            let s = (size % WINDOW) as usize;
            let e = (s as u64).next_multiple_of(pg).min(w.len) as usize;
            w.bytes()[s..e].fill(0);
        }
        self.version = fs.revision_h(self.fh)?.version;
        Ok(())
    }

    pub fn size(&self) -> u64 {
        self.size
    }
}

impl FileVmos {
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Zaznam souboru, pokud existuje (soubor je namapovany).
    pub fn entry(&mut self, key: Key) -> Option<(&mut Entry, &mut u64)> {
        let e = self.map.get_mut(&key)?;
        Some((e, &mut self.bytes))
    }

    /// Zaznam souboru za handlem klienta `h` (zalozi ho: interni handle,
    /// velikost a verze z FS).
    pub fn open(&mut self, fs: &mut Fs<BlkDev>, key: Key, h: fs::Handle) -> Result<&mut Entry> {
        self.clock += 1;
        if !self.map.contains_key(&key) {
            let fh = fs.pin_h(h)?;
            let size = fs.stat_h(fh)?.size;
            let version = fs.revision_h(fh)?.version;
            self.map.insert(key, Entry { fh, size, version, windows: BTreeMap::new(), pins: 0, used: 0 });
        }
        let e = self.map.get_mut(&key).ok_or(Error::NotFound)?;
        e.used = self.clock;
        Ok(e)
    }

    /// Okno `idx` zaznamu `key` (nacte ho); vraci objekt a velikost souboru.
    pub fn window(&mut self, fs: &mut Fs<BlkDev>, key: Key, idx: u64) -> Result<(&Vmo, u64)> {
        if idx >= MAX_WINDOWS {
            return Err(Error::OutOfRange);
        }
        let e = self.map.get_mut(&key).ok_or(Error::NotFound)?;
        let size = e.size;
        let w = e.load(fs, idx, &mut self.bytes)?;
        Ok((&w.vmo, size))
    }

    /// Delka okna `idx` (pro kvotu relace).
    pub fn window_bytes(&self, key: Key, idx: u64) -> u64 {
        self.map.get(&key).and_then(|e| e.windows.get(&idx)).map_or(0, |w| w.len)
    }

    pub fn pin(&mut self, key: Key) {
        if let Some(e) = self.map.get_mut(&key) {
            e.pins += 1;
        }
    }

    pub fn unpin(&mut self, key: Key) {
        if let Some(e) = self.map.get_mut(&key) {
            e.pins = e.pins.saturating_sub(1);
        }
    }

    /// Zapise zpet jeden soubor.
    pub fn flush(&mut self, fs: &mut Fs<BlkDev>, key: Key) -> Result<()> {
        match self.map.get_mut(&key) {
            Some(e) => e.flush(fs).map(|_| ()),
            None => Ok(()),
        }
    }

    /// Zapise zpet vsechny soubory (pred cizi operaci, periodicky, pred
    /// koncem). Chyba jednoho souboru nezastavi ostatni. Vraci pocet
    /// zapsanych stranek.
    pub fn flush_all(&mut self, fs: &mut Fs<BlkDev>) -> Result<u64> {
        let mut first = Ok(0);
        for e in self.map.values_mut() {
            match e.flush(fs) {
                Ok(n) => {
                    if let Ok(t) = &mut first {
                        *t += n;
                    }
                }
                Err(err) => {
                    if first.is_ok() {
                        first = Err(err);
                    }
                }
            }
        }
        first
    }

    /// Po cizi operaci: soubory zmenene mimo zaznam znovu nacist; zaznamy
    /// bez drzeni nad `CACHE_MAX` zahodit.
    pub fn refresh_all(&mut self, fs: &mut Fs<BlkDev>) {
        let mut gone = Vec::new();
        for (&k, e) in self.map.iter_mut() {
            if let Err(err) = e.refresh(fs, &mut self.bytes) {
                gone.push((k, err));
            }
        }
        // Zanikl objekt, nebo cely dataset (handle do smazaneho datasetu
        // hlasi i jine chyby)?
        let ids = if gone.is_empty() { Vec::new() } else { fs.dataset_ids().unwrap_or_default() };
        let gone: Vec<(Key, bool)> =
            gone.into_iter().map(|(k, err)| (k, matches!(err, Error::NotFound | Error::BadHandle) || !ids.contains(&k.0))).collect();
        // Objekt zanikl (smazany dataset): okna na 0 - klienti dostanou pri
        // pristupu vyjimku za koncem (`SIGBUS`) misto starych dat. Jina
        // chyba (I/O): zaznam pryc, mapovani klientu zustanou.
        for (k, vanished) in gone {
            if vanished {
                self.shrink(k);
            }
            self.drop_entry(fs, k);
        }
        self.evict(fs);
    }

    /// Zmensi okna zaznamu na 0 (objekt zanikl, svazek se odpojuje).
    fn shrink(&mut self, k: Key) {
        if let Some(e) = self.map.get_mut(&k) {
            for w in e.windows.values_mut() {
                if w.vmo.set_size(0).is_ok() {
                    self.bytes -= w.len;
                    w.len = 0;
                }
            }
        }
    }

    /// Konec fsd (odpojeni svazku): vse zapsat zpet, okna zmensit na 0
    /// a interni handly zavrit. Vraci prvni chybu zapisu.
    pub fn close_all(&mut self, fs: &mut Fs<BlkDev>) -> Result<u64> {
        let r = self.flush_all(fs);
        let keys: Vec<Key> = self.map.keys().copied().collect();
        for k in keys {
            self.shrink(k);
            self.drop_entry(fs, k);
        }
        r
    }

    /// Zahodi nejstarsi zaznamy bez drzeni nad `CACHE_MAX` (po zapisu zpet).
    pub fn evict(&mut self, fs: &mut Fs<BlkDev>) {
        while self.bytes > CACHE_MAX {
            let Some((&k, _)) = self.map.iter().filter(|(_, e)| e.pins == 0).min_by_key(|(_, e)| e.used) else { return };
            if let Some(e) = self.map.get_mut(&k)
                && e.flush(fs).is_err()
            {
                // Data by se ztratila: nechat, zkusi se priste.
                return;
            }
            self.drop_entry(fs, k);
        }
    }

    fn drop_entry(&mut self, fs: &mut Fs<BlkDev>, k: Key) {
        if let Some(e) = self.map.remove(&k) {
            self.bytes -= e.windows.values().map(|w| w.len).sum::<u64>();
            let _ = fs.close(e.fh);
        }
    }

    /// Prehled pro diagnostiku: (soubory, nactena okna, bajty oken).
    pub fn stats(&self) -> (usize, usize, u64) {
        (self.map.len(), self.map.values().map(|e| e.windows.len()).sum(), self.bytes)
    }
}

/// Operace, ktere s objekty pameti souboru pocitaji samy (ctou a pisou pres
/// zaznam nebo obsah souboru nectou); pred ostatnimi se zapisuje zpet
/// a po nich se hleda cizi zmena (`Service::handle`).
pub fn aware(opcode: u32) -> bool {
    use fsproto::v2::op;
    matches!(
        opcode,
        op::HELLO
            | op::CONNECT
            | op::CLOSE
            | op::READ_H
            | op::WRITE_H
            | op::SET_LEN
            | op::STAT_H
            | op::LIST_H
            | op::LOCK
            | op::UNLOCK
            | op::POSIX_LOCK
            | op::WHOAMI
            | op::MAP_VMO
            | op::UNMAP_VMO
            | op::SYNC_VMO
            | op::ADOPT_VMO
            | op::READ_VMO
            | op::SOCKET_BIND
            | op::SOCKET_CONNECT
    )
}

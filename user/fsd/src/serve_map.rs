//! Soubory jako pamet (fsproto v2 `MAP_VMO`, `UNMAP_VMO`, `SYNC_VMO`,
//! `READ_VMO`) a napojeni handlovych operaci na zaznamy namapovanych
//! souboru (`filevmo.rs`): dokud zaznam existuje, je zdrojem pravdy pro
//! obsah i velikost souboru.

use alloc::vec::Vec;

use fsproto::Reader;
use fsproto::v2::{map_flags, op};
use libos::{Error, Handle, Result, Rights};

use crate::filevmo::{Key, SESSION_QUOTA};
use crate::serve::Service;
use crate::serve_v2::{Reply, Session};

impl Service {
    /// Operace mapovani; `None` = neni jeho. `handles` = handly pozadavku.
    pub fn handle_map(&mut self, sess: &mut Session, opcode: u32, r: &mut Reader, handles: &mut Vec<Handle>) -> Option<Result<Reply>> {
        Some(match opcode {
            op::MAP_VMO => self.map_vmo(sess, r),
            op::UNMAP_VMO => (|| {
                let key = (r.u64()?, r.u64()?);
                let idx = r.u64()?;
                self.unmap_window(sess, key, idx)?;
                Ok(Reply::body(fsproto::Writer::new()))
            })(),
            op::SYNC_VMO => (|| {
                let key = (r.u64()?, r.u64()?);
                let durable = r.u8()? != 0;
                // Relace soubor nedrzi: mapovani je z doby pred restartem
                // fsd (zaznam zanikl) - klient to musi vedet (msync selze).
                if !sess.mapped.keys().any(|&(d, o, _)| (d, o) == key) {
                    return Err(Error::ServiceRestarted);
                }
                let fs = self.fs.as_mut().ok_or(Error::IoError)?;
                self.vmos.flush(fs, key)?;
                if durable {
                    fs.fsync()?;
                }
                Ok(Reply::body(fsproto::Writer::new()))
            })(),
            op::ADOPT_VMO => (|| {
                let h = r.u64()?;
                let idx = r.u64()?;
                if !sess.handles.contains(&h) {
                    return Err(Error::BadHandle);
                }
                let vmo = libos::Vmo::from_handle(handles.pop().ok_or(Error::InvalidArgs)?);
                let n = self.adopt_window(h, idx, &vmo)?;
                let mut w = fsproto::Writer::new();
                w.u32(n);
                Ok(Reply::body(w))
            })(),
            op::READ_VMO => (|| {
                let h = r.u64()?;
                if !sess.handles.contains(&h) {
                    return Err(Error::BadHandle);
                }
                let (off, n) = (r.u64()?, (r.u32()? as usize).min(fsproto::VMO_IO));
                let vmo = libos::Vmo::from_handle(handles.pop().ok_or(Error::InvalidArgs)?);
                let data = self.read_handle(h, off, n)?;
                copy_to_vmo(&vmo, &data)?;
                let mut w = fsproto::Writer::new();
                w.u32(data.len() as u32);
                Ok(Reply::body(w))
            })(),
            _ => return None,
        })
    }

    /// Zaznam namapovaneho souboru za handlem `h` (jen kdyz nejaky je).
    pub fn mapped_key(&mut self, h: u64) -> Option<Key> {
        if self.vmos.is_empty() {
            return None;
        }
        let rev = self.fs.as_mut()?.revision_h(h).ok()?;
        let key = (rev.dataset, rev.oid);
        self.vmos.entry(key).map(|_| key)
    }

    /// `READ_H`/`READ_VMO`: namapovany soubor pres zaznam (prava handlu
    /// overi FS), jinak primo FS.
    pub fn read_handle(&mut self, h: u64, off: u64, n: usize) -> Result<Vec<u8>> {
        let key = self.mapped_key(h);
        let fs = self.fs.as_mut().ok_or(Error::IoError)?;
        match key.and_then(|k| self.vmos.entry(k)) {
            Some((e, _)) => {
                fs.check_access_h(h, fs::access::READ)?;
                e.read(fs, off, n)
            }
            None => fs.read_h(h, off, n),
        }
    }

    /// `WRITE_H`: namapovany soubor pres zaznam, jinak primo FS.
    pub fn write_handle(&mut self, h: u64, off: u64, data: &[u8]) -> Result<()> {
        let key = self.mapped_key(h);
        let fs = self.fs.as_mut().ok_or(Error::IoError)?;
        match key.and_then(|k| self.vmos.entry(k)) {
            Some((e, bytes)) => {
                fs.check_access_h(h, fs::access::WRITE)?;
                e.write(fs, off, data, bytes)
            }
            None => fs.write_h(h, off, data),
        }
    }

    /// `SET_LEN`: ve FS (prava a zamky handlu), pak okna zaznamu.
    pub fn set_len_handle(&mut self, h: u64, size: u64) -> Result<()> {
        let key = self.mapped_key(h);
        let fs = self.fs.as_mut().ok_or(Error::IoError)?;
        fs.set_len_h(h, size)?;
        match key.and_then(|k| self.vmos.entry(k)) {
            Some((e, bytes)) => e.set_len(fs, size, bytes),
            None => Ok(()),
        }
    }

    /// Velikost souboru za handlem: namapovany ze zaznamu, jinak `None`.
    pub fn mapped_size(&mut self, h: u64) -> Option<u64> {
        let key = self.mapped_key(h)?;
        self.vmos.entry(key).map(|(e, _)| e.size())
    }

    /// `CLOSE`: zapsat zpet soubor zavirance handlu (POSIX: data `mmap`
    /// jsou po `close` v souboru, i kdyz mapovani trva).
    pub fn flush_handle(&mut self, h: u64) {
        if let Some(key) = self.mapped_key(h)
            && let Some(fs) = self.fs.as_mut()
        {
            let _ = self.vmos.flush(fs, key);
        }
    }

    /// Konec relace: uvolnit jeji drzeni oken a zapsat soubory zpet.
    pub fn release_mapped(&mut self, sess: &mut Session) {
        let held = core::mem::take(&mut sess.mapped);
        sess.mapped_bytes = 0;
        let Some(fs) = self.fs.as_mut() else { return };
        let mut last = None;
        for (d, o, _) in held.into_keys() {
            self.vmos.unpin((d, o));
            if last != Some((d, o)) {
                let _ = self.vmos.flush(fs, (d, o));
                last = Some((d, o));
            }
        }
        self.vmos.evict(fs);
    }

    /// `MAP_VMO`: okno souboru (`filevmo.rs`). Zapis jen pro handle se
    /// zapisem a sdilene mapovani (soukrome = klon u klienta).
    fn map_vmo(&mut self, sess: &mut Session, r: &mut Reader) -> Result<Reply> {
        let h = r.u64()?;
        let idx = r.u64()?;
        let flags = r.u8()?;
        if !sess.handles.contains(&h) {
            return Err(Error::BadHandle);
        }
        let shared = flags & map_flags::SHARED != 0;
        let write = flags & map_flags::WRITE != 0;
        if write && !shared {
            return Err(Error::InvalidArgs);
        }
        let fs = self.fs.as_mut().ok_or(Error::IoError)?;
        let st = fs.stat_h(h)?;
        if st.kind != fs::Kind::File {
            return Err(Error::WrongType);
        }
        fs.check_access_h(h, fs::access::READ)?;
        if write {
            fs.check_access_h(h, fs::access::WRITE)?;
        }
        let rev = fs.revision_h(h)?;
        let key = (rev.dataset, rev.oid);
        self.vmos.open(fs, key, h)?;
        let (vmo, size) = self.vmos.window(fs, key, idx)?;
        let mut rights = Rights::BASIC | Rights::READ | Rights::MAP;
        if write {
            rights = rights | Rights::WRITE;
        }
        let handle = vmo.handle().duplicate(Some(rights))?;
        // Sdilene mapovani okno drzi (kvota relace podle skutecne delky).
        let wkey = (key.0, key.1, idx);
        if shared && !sess.mapped.contains_key(&wkey) {
            let len = self.vmos.window_bytes(key, idx);
            if sess.mapped_bytes.saturating_add(len) > SESSION_QUOTA {
                return Err(Error::NoMemory);
            }
            sess.mapped.insert(wkey, len);
            sess.mapped_bytes += len;
            self.vmos.pin(key);
        }
        self.vmos.evict(fs);
        let mut w = fsproto::Writer::new();
        w.u64(size).u64(key.0).u64(key.1);
        Ok(Reply { body: w, handles: alloc::vec![handle], new_session: None, session_ops: Vec::new(), watch: None })
    }

    /// `ADOPT_VMO` (obnova po padu fsd, Q5): spinave stranky okna stare
    /// instance zapise jako zapis klienta s handlem `h` (pravo zapisu
    /// overi FS); nic za velikosti souboru. Vraci pocet stranek.
    fn adopt_window(&mut self, h: u64, idx: u64, old: &libos::Vmo) -> Result<u32> {
        use crate::filevmo::{MAX_WINDOWS, WINDOW, take_dirty};
        if idx >= MAX_WINDOWS {
            return Err(Error::OutOfRange);
        }
        let fs = self.fs.as_mut().ok_or(Error::IoError)?;
        if fs.stat_h(h)?.kind != fs::Kind::File {
            return Err(Error::WrongType);
        }
        fs.check_access_h(h, fs::access::WRITE)?;
        let rev = fs.revision_h(h)?;
        let key = (rev.dataset, rev.oid);
        self.vmos.open(fs, key, h)?;
        let pages = take_dirty(old)?;
        let size = old.size()?.min(WINDOW);
        if pages.is_empty() || size == 0 {
            return Ok(0);
        }
        let page = libos::vm::page_size();
        let me = libos::rt::self_process();
        let addr = libos::sys::vm_map(me, old.handle().raw(), 0, size, libos::abi::vm::MAP_READ, 0)?;
        // SAFETY: [addr, addr + size) je prave namapovany objekt (jen pro
        // cteni); po zapisu se odmapuje. Obsah muze klient soubezne menit -
        // fsd ho bere jen jako bajty.
        let buf = unsafe { core::slice::from_raw_parts(addr as *const u8, size as usize) };
        let mut written = 0u32;
        let mut r = Ok(());
        if let Some((e, bytes)) = self.vmos.entry(key) {
            for p in pages {
                let at = p * page;
                let file_off = idx * WINDOW + at;
                if at >= size || file_off >= e.size() {
                    continue;
                }
                let n = page.min(size - at).min(e.size() - file_off) as usize;
                if let Err(err) = e.write(fs, file_off, &buf[at as usize..at as usize + n], bytes) {
                    r = Err(err);
                    break;
                }
                written += 1;
            }
        }
        let _ = libos::sys::vm_unmap(me, addr, size);
        r.map(|()| written)
    }

    /// `UNMAP_VMO`: relace uvolni drzeni okna; soubor se zapise zpet.
    fn unmap_window(&mut self, sess: &mut Session, key: Key, idx: u64) -> Result<()> {
        let len = sess.mapped.remove(&(key.0, key.1, idx)).ok_or(Error::NotFound)?;
        sess.mapped_bytes = sess.mapped_bytes.saturating_sub(len);
        self.vmos.unpin(key);
        let fs = self.fs.as_mut().ok_or(Error::IoError)?;
        let r = self.vmos.flush(fs, key);
        self.vmos.evict(fs);
        r
    }
}

/// Zapise `data` na zacatek VMO klienta: namapovat a kopirovat ve fsd
/// (optimalizovany kod) je radove rychlejsi nez `VMO_WRITE` (kopie pres
/// buffer jadra). Bez prava `MAP` = `VMO_WRITE`.
fn copy_to_vmo(vmo: &libos::Vmo, data: &[u8]) -> Result<()> {
    if data.is_empty() {
        return Ok(());
    }
    let page = libos::vm::page_size();
    let len = (data.len() as u64).div_ceil(page) * page;
    let me = libos::rt::self_process();
    let flags = libos::abi::vm::MAP_READ | libos::abi::vm::MAP_WRITE;
    let Ok(addr) = libos::sys::vm_map(me, vmo.handle().raw(), 0, len, flags, 0) else {
        return vmo.write(0, data);
    };
    // SAFETY: rozsah [addr, addr + len) je prave namapovany objekt pameti
    // s pravem zapisu, nikdo jiny ve fsd na nej neukazuje.
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), addr as *mut u8, data.len()) };
    libos::sys::vm_unmap(me, addr, len)
}

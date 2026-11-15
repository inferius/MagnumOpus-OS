//! Mapovani souboru pres okna objektu pameti z fsd (fsproto `MAP_VMO`,
//! docs/cs/souborovy-system.md, kapitola Soubory jako pamet).
//!
//! Rozsah souboru pokryva nekolik oken (`fsproto::v2::WINDOW`, zarovnana);
//! klient vezme rezervaci adres a kazdou cast okna do ni namapuje
//! (`MAP_IN_RESERVATION`), takze mapovani je souvisle. Soukrome mapovani =
//! klon okna (kopie pri zapisu, nic se nekopiruje predem); sdilene =
//! primo okno (tytez ramce ve vsech procesech, zapis jde do souboru).
//!
//! Sdilena mapovani si proces eviduje (`SHARED`): `munmap` celeho
//! mapovani okna uvolni (`UNMAP_VMO`, zapis zpet), `msync` je zapise zpet
//! (`SYNC_VMO`). Castecny `munmap` okna nechava drzena do konce relace.
//!
//! Obnova po padu fsd (rozhodnuti Q5): zapisovatelne sdilene mapovani drzi
//! handly svych oken a identitu souboru. Kdyz `msync` nebo `munmap` zjisti
//! novou instanci fsd, soubor se znovu otevre se zapisem (prava overi nova
//! instance), stara okna se predaji (`ADOPT_VMO`: fsd zapise jejich
//! spinave stranky jako zapis klienta) a rozsah se premapuje na nova okna.
//! Bez prava zapisu se zmeny zahodi a `msync` vrati `SERVICE_RESTARTED`.

use alloc::vec::Vec;

use fsproto::v2::{Identity, WINDOW, map_flags};
use libos::sync::Mutex;

use crate::{Error, Result};

/// Sdilene mapovani souboru v procesu.
struct Shared {
    addr: u64,
    len: u64,
    /// Offset souboru na zacatku mapovani.
    off: u64,
    prot: u32,
    file: (u64, u64),
    /// Okna `first..first + count`.
    first: u64,
    count: u64,
    /// Zapisovatelne: identita souboru a handly oken (obnova po padu fsd).
    id: Option<Identity>,
    wins: Vec<libos::Vmo>,
    /// Generace spojeni s fsd pri namapovani (jina = fsd mezitim spadl).
    generation: u64,
    /// Zmeny se po padu fsd nepodarilo predat (bez prava zapisu): `msync`
    /// vraci `SERVICE_RESTARTED`.
    lost: bool,
}

static SHARED: Mutex<Vec<Shared>> = Mutex::new(Vec::new());

/// Namapuje `len` bajtu souboru (handle relace `local`) od `off` (zarovnane
/// na stranku) s pravy `prot`; `addr` 0 = kamkoli, jinak presne tam (rozsah
/// uz musi byt volny nebo cely v rezervaci volajiciho). Vraci adresu.
pub(crate) fn map(local: u64, off: u64, len: u64, prot: u32, addr: u64, private: bool) -> Result<u64> {
    let s = crate::fs::session()?;
    let write = prot & abi::vm::MAP_WRITE != 0;
    let mut flags = 0;
    if !private {
        flags |= map_flags::SHARED;
        if write {
            flags |= map_flags::WRITE;
        }
    }
    let end = off.checked_add(len).ok_or(Error::InvalidArgs)?;
    let (first, last) = (off / WINDOW, (end - 1) / WINDOW);
    // Rezervace: vlastni (addr 0), nebo volajiciho (cela v rezervaci),
    // jinak nova na pevne adrese.
    let (base, own) = if addr == 0 {
        (libos::vm::reserve(len, 0)?, true)
    } else if in_reservation(addr, len) {
        (addr, false)
    } else {
        (libos::vm::reserve_at(addr, len, 0)?, true)
    };
    let me = libos::rt::self_process();
    let fixed = prot | abi::vm::MAP_FIXED | abi::vm::MAP_IN_RESERVATION;
    // Soubor (dataset, oid) a pocet drzenych oken (pro uklid po chybe).
    let mut file = None;
    let mut held = 0u64;
    let keep = !private && write;
    let id = if keep { Some(s.identity(local)?) } else { None };
    let mut wins = Vec::new();
    let r = (|| {
        for idx in first..=last {
            let win = s.map_vmo(local, idx, flags)?;
            file = Some(win.file);
            if !private {
                held += 1;
            }
            let ws = idx * WINDOW;
            let lo = off.max(ws);
            let hi = end.min(ws + WINDOW);
            let at = base + (lo - off);
            if private {
                // Klon jen casti okna pod koncem souboru (snimek); za koncem
                // zustane rezervace (pristup = vyjimka jako za koncem
                // souboru u sdileneho).
                let page = libos::vm::page_size();
                let top = hi.min(win.size.div_ceil(page) * page);
                if top > lo {
                    let clone = win.vmo.clone_range(lo - ws, top - lo)?;
                    libos::sys::vm_map(me, clone.handle().raw(), 0, top - lo, fixed, at)?;
                }
            } else {
                libos::sys::vm_map(me, win.vmo.handle().raw(), lo - ws, hi - lo, fixed, at)?;
            }
            if keep {
                wins.push(win.vmo);
            }
        }
        Ok(())
    })();
    match (r, file) {
        (Ok(()), Some(file)) => {
            if !private {
                let generation = s.conn().generation();
                SHARED.lock().push(Shared { addr: base, len, off, prot, file, first, count: last - first + 1, id, wins, generation, lost: false });
            }
            Ok(base)
        }
        (r, file) => {
            // Uklid: namapovane casti pryc, rezervaci vratit, okna uvolnit
            // (jen ta, ktera nedrzi jine mapovani procesu).
            let _ = libos::vm::release(base, len);
            if !own {
                let _ = libos::vm::reserve_at(base, len, 0);
            }
            if let Some(file) = file {
                release_windows(s, file, first, held);
            }
            Err(r.err().unwrap_or(Error::InvalidArgs))
        }
    }
}

/// Uvolni okna `first..first + count` souboru, ktera nepokryva jine
/// sdilene mapovani procesu (relace drzi okno jednou).
fn release_windows(s: &fsproto::FsClient, file: (u64, u64), first: u64, count: u64) {
    let used: Vec<(u64, u64)> = SHARED.lock().iter().filter(|m| m.file == file).map(|m| (m.first, m.count)).collect();
    for idx in first..first + count {
        if !used.iter().any(|&(f, c)| f <= idx && idx < f + c) {
            let _ = s.unmap_vmo(file, idx);
        }
    }
}

/// Je cely rozsah jedna rezervace (bez mapovani)?
fn in_reservation(addr: u64, len: u64) -> bool {
    libos::vm::query(addr).is_ok_and(|r| r.kind == abi::vm::region::RESERVED && r.start <= addr && addr + len <= r.start + r.len)
}

/// `munmap(addr, len)` probehl: sdilena mapovani, ktera rozsah cela
/// pokryva, se uvolni (`UNMAP_VMO`, zapis zpet).
pub fn unmapped(addr: u64, len: u64) {
    let gone: Vec<Shared> = {
        let mut list = SHARED.lock();
        let mut out = Vec::new();
        let mut i = 0;
        while i < list.len() {
            let m = &list[i];
            if addr <= m.addr && m.addr + m.len <= addr + len {
                out.push(list.swap_remove(i));
            } else {
                i += 1;
            }
        }
        out
    };
    let Ok(s) = crate::fs::session() else { return };
    for m in gone {
        // Zapisovatelne mapovani z doby pred padem fsd: zmeny predat nove
        // instanci (bez premapovani - rozsah uz je pryc).
        if let Some(id) = &m.id
            && restarted(s, &m)
        {
            for (i, w) in m.wins.iter().enumerate() {
                let _ = s.adopt_window(id, m.first + i as u64, w);
            }
        }
        release_windows(s, m.file, m.first, m.count);
    }
}

/// Patri mapovani stare instanci fsd? (Zjisti se dotazem: spojeni si
/// novou instanci vsimne az pri volani.)
fn restarted(s: &fsproto::FsClient, m: &Shared) -> bool {
    matches!(s.sync_vmo(m.file, false), Err(Error::ServiceRestarted)) || s.conn().generation() != m.generation
}

/// Obnova po padu fsd: stara okna predat (`ADOPT_VMO`) a rozsah premapovat
/// na nova. Bez prava zapisu = `SERVICE_RESTARTED`, zmeny se zahodi.
fn recover(s: &fsproto::FsClient, m: &mut Shared) -> Result<()> {
    let id = m.id.ok_or(Error::ServiceRestarted)?;
    let me = libos::rt::self_process();
    let fixed = m.prot | abi::vm::MAP_FIXED | abi::vm::MAP_IN_RESERVATION;
    for i in 0..m.count as usize {
        let idx = m.first + i as u64;
        let win = match s.recover_window(&id, idx, &m.wins[i]) {
            Ok(w) => w,
            Err(e) => {
                if e == Error::ServiceRestarted {
                    m.id = None;
                    m.wins.clear();
                    m.lost = true;
                }
                return Err(e);
            }
        };
        // Cast mapovani v okne: mapovani -> rezervace (adresy zustanou)
        // a do ni nove okno.
        let ws = idx * WINDOW;
        let lo = m.off.max(ws);
        let hi = (m.off + m.len).min(ws + WINDOW);
        let at = m.addr + (lo - m.off);
        libos::vm::decommit(at, hi - lo)?;
        libos::sys::vm_map(me, win.vmo.handle().raw(), lo - ws, hi - lo, fixed, at)?;
        m.wins[i] = win.vmo;
        m.file = win.file;
    }
    m.generation = s.conn().generation();
    Ok(())
}

/// `msync(addr, len)`: soubory sdilenych mapovani v rozsahu zapsat zpet;
/// `durable` = i trvale (`MS_SYNC`). Rozsah bez sdileneho mapovani souboru
/// = uspech (anonymni pamet nema kam).
pub fn sync(addr: u64, len: u64, durable: bool) -> Result<()> {
    let mut list = SHARED.lock();
    let overlap = |m: &Shared| m.addr < addr + len && addr < m.addr + m.len;
    if !list.iter().any(overlap) {
        return Ok(());
    }
    let s = crate::fs::session()?;
    let mut done: Vec<(u64, u64)> = Vec::new();
    for m in list.iter_mut().filter(|m| overlap(m)) {
        if m.lost {
            return Err(Error::ServiceRestarted);
        }
        let r = s.sync_vmo(m.file, durable);
        let stale = matches!(r, Err(Error::ServiceRestarted)) || s.conn().generation() != m.generation;
        if stale && m.id.is_some() {
            // Nova instance fsd: kazde mapovani zvlast (i dalsi tehoz souboru).
            recover(s, m)?;
            s.sync_vmo(m.file, durable)?;
        } else if stale {
            // Jen pro cteni: nic k predani (okno stare instance zustane).
        } else if !done.contains(&m.file) {
            r?;
        }
        done.push(m.file);
    }
    Ok(())
}

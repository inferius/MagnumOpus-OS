//! Adresni prostor procesu: vlastni spodni polovina, sdilene jadro.
//!
//! Spodni polovina obsahuje jen mapovani objektu pameti ([`MemoryObject`]).
//! Jadro do pameti procesu nikdy nepristupuje pres adresy userspace: pro
//! kopirovani dat (argumenty syscallu) prelozi adresu podle seznamu
//! mapovani na fyzickou stranku objektu a kopiruje pres primou mapu. Diky
//! tomu nemuze vzniknout page fault v jadre a SMAP muze byt trvale zapnuty.
//!
//! Kazde mapovani se strhne z rozpoctu jadra jobu procesu: zaznam mapovani
//! a horni odhad tabulek stranek, ktere pro nej mohou vzniknout.
//!
//! Stranky na pozadani: mapovani namapuje jen stranky objektu, ktere uz
//! existuji; ostatni doplni vypadek (`fault.rs`): cteni nulovou strankou
//! jen pro cteni, zapis vlastnim ramcem. Kopirovani jadra z/do pameti
//! procesu vypadek nikdy nevyvola: dosud nevzniklou stranku cte jako nuly
//! a pred zapisem ji vytvori.
//!
//! Konec procesu: [`AddressSpace::teardown`] zrusi vsechna mapovani
//! a tabulky hned (ne az s posledni referenci na proces). Potom je prostor
//! "mrtvy": `map` vraci `BadState`, ostatni operace nic nenajdou.

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use abi::vm::{MAP_EXECUTE, MAP_FIXED, MAP_READ, MAP_WRITE, USER_ALLOC_BASE, USER_MAX, USER_MIN};

use super::{PhysAddr, VirtAddr, new_address_space, tlb};
use crate::arch::paging::{MapFlags, PAGE_SIZE, PageSize, PageTable, PageTableOps};
use crate::error::Error;
use crate::object::job::{Charge, Job, Pool};
use crate::object::vmo::MemoryObject;
use crate::sync::SpinLock;

mod fault;
pub use fault::Access;

struct Mapping {
    len: u64,
    vmo: Arc<MemoryObject>,
    vmo_offset: u64,
    flags: u32,
    _charge: Charge,
}

/// Pamet jadra za zaznam mapovani (uzel stromu + data).
const MAPPING_COST: u64 = 128;

/// Horni odhad tabulek stranek pro mapovani delky `len`: jedna tabulka na
/// kazdych zacatych 2 MiB (+1 za nezarovnany zacatek) a tri vyssi urovne.
fn page_table_cost(len: u64) -> u64 {
    (len.div_ceil(2 << 20) + 1 + 3) * PAGE_SIZE
}

struct Inner {
    /// `None` = prostor zruseny (`teardown`) nebo uz rusi.
    table: Option<PageTable>,
    /// Mapovani podle pocatecni adresy (neprekryvaji se).
    mappings: BTreeMap<u64, Mapping>,
}

pub struct AddressSpace {
    /// Fyzicka adresa korenove tabulky, 0 po `teardown`.
    root: AtomicU64,
    inner: SpinLock<Inner>,
    /// Job procesu (rozpocet mapovani); `None` = adresni prostor jadra/testu.
    job: Option<Arc<Job>>,
    /// Strzeni za korenovou tabulku (vrati se s `teardown`).
    root_charge: SpinLock<Charge>,
}

fn page_aligned(v: u64) -> bool {
    v.is_multiple_of(PAGE_SIZE)
}

/// Priznaky stranky mapovani s pravy `flags` (`MAP_*`) objektu `vmo`;
/// `writable = false` pro nulovou stranku (nikdy zapisovatelna).
fn page_flags(vmo: &MemoryObject, flags: u32, writable: bool) -> MapFlags {
    let mut f = MapFlags::USER | vmo.cache_flags();
    if vmo.is_dma() {
        f = f | MapFlags::DMA;
    }
    if writable && flags & MAP_WRITE != 0 {
        f = f | MapFlags::WRITE;
    }
    if flags & MAP_EXECUTE != 0 {
        f = f | MapFlags::EXEC;
    }
    f
}

impl Inner {
    fn overlaps(&self, start: u64, len: u64) -> bool {
        let end = start + len;
        // Mapovani zacinajici pred `end`; staci posledni z nich.
        self.mappings
            .range(..end)
            .next_back()
            .is_some_and(|(&s, m)| s + m.len > start)
    }

    fn find_free(&self, len: u64, hint: u64) -> Option<u64> {
        let mut candidate = if hint >= USER_MIN && page_aligned(hint) {
            hint
        } else {
            USER_ALLOC_BASE
        };
        loop {
            let end = candidate.checked_add(len)?;
            if end > USER_MAX {
                return None;
            }
            if !self.overlaps(candidate, len) {
                return Some(candidate);
            }
            // Za konec prvniho prekryvajiciho mapovani.
            let (&s, m) = self.mappings.range(..end).next_back()?;
            candidate = s + m.len;
        }
    }

    /// Mapovani obsahujici adresu `addr`.
    fn mapping_at(&self, addr: u64) -> Option<(u64, &Mapping)> {
        let (&start, m) = self.mappings.range(..=addr).next_back()?;
        (addr < start + m.len).then_some((start, m))
    }
}

impl AddressSpace {
    /// Novy prazdny prostor; korenova tabulka se strhne z rozpoctu jadra
    /// `job`.
    pub fn new(job: Option<Arc<Job>>) -> Result<Self, Error> {
        let root_charge = match &job {
            Some(j) => j.charge(Pool::Kernel, PAGE_SIZE)?,
            None => Charge::none(),
        };
        let table = new_address_space().ok_or(Error::NoMemory)?;
        Ok(Self {
            root: AtomicU64::new(table.root()),
            inner: SpinLock::new(Inner {
                table: Some(table),
                mappings: BTreeMap::new(),
            }),
            job,
            root_charge: SpinLock::new(root_charge),
        })
    }

    /// Fyzicka adresa korenove tabulky (pro `arch::switch_user_space`);
    /// `None` po `teardown`.
    pub fn root(&self) -> Option<u64> {
        match self.root.load(Ordering::Acquire) {
            0 => None,
            r => Some(r),
        }
    }

    /// Zrusi vsechna mapovani a tabulky stranek (konec procesu). Objekty
    /// pameti, pro ktere `park` vrati true, se neuvolni: vrati se volajicimu
    /// (pamet pro DMA bez IOMMU, viz `Process::finish`). Ostatni objekty
    /// a strzeni mapovani i korenove tabulky se vrati hned. Dalsi volani nic
    /// nedela.
    ///
    /// # Safety
    ///
    /// Prostor nesmi byt aktivni na zadnem CPU ani ho nesmi zadne CPU
    /// aktivovat pozdeji: vsechna vlakna procesu skoncila a pred koncem se
    /// prepnula na koren jadra (`sched::exit`), planovac pro ne koren
    /// procesu nepouzije.
    pub unsafe fn teardown(&self, park: impl Fn(&MemoryObject) -> bool) -> Vec<Arc<MemoryObject>> {
        let (table, mappings) = {
            let mut inner = self.inner.lock();
            let Some(table) = inner.table.take() else {
                return Vec::new();
            };
            self.root.store(0, Ordering::Release);
            (table, core::mem::take(&mut inner.mappings))
        };
        // Zaznamy TLB a cache prekladu (i spekulativni) zahodit na vsech
        // CPU, nez se tabulky a stranky vrati alokatoru.
        tlb::shootdown();
        // SAFETY: predano volajicim (prostor neni a nebude aktivni); TLB
        // vyprazdnene vyse.
        unsafe { table.destroy() };
        let mut parked = Vec::new();
        for (_, m) in mappings {
            if park(&m.vmo) && !parked.iter().any(|p| Arc::ptr_eq(p, &m.vmo)) {
                parked.push(m.vmo.clone());
            }
            // `m` (Arc na objekt a strzeni mapovani) se uvolni tady.
        }
        drop(core::mem::take(&mut *self.root_charge.lock()));
        parked
    }

    /// Velikost stranky tohoto adresniho prostoru (syscall `VM_PAGE_SIZE`):
    /// na tuto hodnotu musi byt zarovnane adresy a delky mapovani. Zatim
    /// vsude ramec jadra (4 KiB); az bude mit adresni prostor vlastni
    /// granuli (aarch64: 16 KiB pro nativni procesy), vrati ji tady
    /// a `page_aligned` a mapovani ji pouziji misto `PAGE_SIZE`.
    pub fn page_size(&self) -> u64 {
        PAGE_SIZE
    }

    /// Namapuje `len` bajtu objektu `vmo` od `vmo_offset`. S `MAP_FIXED` presne
    /// na `addr`, jinak na volne misto (`addr` je napoveda, 0 = kdekoli).
    pub fn map(
        &self,
        vmo: Arc<MemoryObject>,
        vmo_offset: u64,
        len: u64,
        flags: u32,
        addr: u64,
    ) -> Result<u64, Error> {
        let known = MAP_READ | MAP_WRITE | MAP_EXECUTE | MAP_FIXED;
        if flags & !known != 0 || flags & MAP_READ == 0 {
            return Err(Error::InvalidArgs);
        }
        // W^X: stranka nesmi byt zaroven zapisovatelna a spustitelna.
        if flags & MAP_WRITE != 0 && flags & MAP_EXECUTE != 0 {
            return Err(Error::InvalidArgs);
        }
        if len == 0 || !page_aligned(len) || !page_aligned(vmo_offset) {
            return Err(Error::InvalidArgs);
        }
        match vmo_offset.checked_add(len) {
            Some(end) if end <= vmo.size() => {}
            _ => return Err(Error::OutOfRange),
        }

        // Tabulky stranek pro mapovani: nejvys 1 stranka na 2 MiB + 3 vyssi
        // urovne; vznikaji na pozadani programu, takze ne z rezervy jadra.
        if !super::phys::user_room(len.div_ceil(2 << 20) + 3) {
            return Err(Error::NoMemory);
        }
        let charge = match &self.job {
            Some(j) => j.charge(Pool::Kernel, MAPPING_COST + page_table_cost(len))?,
            None => Charge::none(),
        };
        let mut inner = self.inner.lock();
        // Proces skoncil (prostor zruseny): nic noveho.
        if inner.table.is_none() {
            return Err(Error::BadState);
        }
        let start = if flags & MAP_FIXED != 0 {
            let end = addr.checked_add(len).ok_or(Error::InvalidArgs)?;
            if !page_aligned(addr) || addr < USER_MIN || end > USER_MAX {
                return Err(Error::InvalidArgs);
            }
            if inner.overlaps(addr, len) {
                return Err(Error::BadState);
            }
            addr
        } else {
            inner.find_free(len, addr).ok_or(Error::NoResources)?
        };

        if vmo.is_revoked() {
            return Err(Error::AccessDenied);
        }
        let map_flags = page_flags(&vmo, flags, true);
        let table = inner.table.as_mut().expect("adresni prostor je zruseny");
        for off in (0..len).step_by(PAGE_SIZE as usize) {
            // Jen existujici stranky; ostatni doplni vypadek.
            let Some(phys) = vmo.page(vmo_offset + off) else {
                continue;
            };
            if table
                .map(
                    VirtAddr::new(start + off),
                    phys,
                    PageSize::Size4K,
                    map_flags,
                )
                .is_err()
            {
                // Nedostatek pameti na tabulky: vratit, co uz je namapovane.
                for done in (0..off).step_by(PAGE_SIZE as usize) {
                    table.unmap_4k(VirtAddr::new(start + done));
                }
                drop(inner);
                tlb::shootdown();
                return Err(Error::NoMemory);
            }
        }
        inner.mappings.insert(
            start,
            Mapping {
                len,
                vmo,
                vmo_offset,
                flags,
                _charge: charge,
            },
        );
        Ok(start)
    }

    /// Zrusi mapovani. Rozsah musi presne pokryvat cela mapovani
    /// (castecne odmapovani zatim neni podporovane).
    pub fn unmap(&self, addr: u64, len: u64) -> Result<(), Error> {
        if len == 0 || !page_aligned(addr) || !page_aligned(len) {
            return Err(Error::InvalidArgs);
        }
        let end = addr.checked_add(len).ok_or(Error::InvalidArgs)?;
        let mut inner = self.inner.lock();
        // Overit, ze rozsah tvori souvislou radu celych mapovani.
        let mut pos = addr;
        let starts: Vec<u64> = inner.mappings.range(addr..end).map(|(&s, _)| s).collect();
        for &s in &starts {
            if s != pos {
                return Err(Error::NotFound);
            }
            pos += inner.mappings[&s].len;
        }
        if pos != end || inner.mapping_at(addr).is_some_and(|(s, _)| s != addr) {
            return Err(Error::NotFound);
        }
        let mut removed = Vec::new();
        for s in starts {
            let m = inner.mappings.remove(&s).unwrap();
            let table = inner.table.as_mut().expect("adresni prostor je zruseny");
            for off in (0..m.len).step_by(PAGE_SIZE as usize) {
                table.unmap_4k(VirtAddr::new(s + off));
            }
            removed.push(m);
        }
        drop(inner);
        // Stranky (v `removed`) se smi uvolnit az kdyz je zadne CPU nema v TLB.
        tlb::shootdown();
        drop(removed);
        Ok(())
    }

    /// Zrusi vsechna mapovani objektu `vmo` (odvolani objektu). Vraci jejich
    /// pocet.
    pub fn unmap_vmo(&self, vmo: &Arc<MemoryObject>) -> usize {
        let mut inner = self.inner.lock();
        let starts: Vec<u64> = inner
            .mappings
            .iter()
            .filter(|(_, m)| Arc::ptr_eq(&m.vmo, vmo))
            .map(|(&s, _)| s)
            .collect();
        let mut removed = Vec::new();
        for s in starts {
            let m = inner.mappings.remove(&s).unwrap();
            let table = inner.table.as_mut().expect("adresni prostor je zruseny");
            for off in (0..m.len).step_by(PAGE_SIZE as usize) {
                table.unmap_4k(VirtAddr::new(s + off));
            }
            removed.push(m);
        }
        drop(inner);
        let n = removed.len();
        if n > 0 {
            // Stranky smi pryc az kdyz je zadne CPU nema v TLB.
            tlb::shootdown();
        }
        drop(removed);
        n
    }

    /// Odmapuje nulovou stranku vsude, kde mapovani objektu `vmo` ukazuje na
    /// jeho offsety `offset..offset+len` (stranky prave vznikly). Vraci, zda
    /// neco odmapoval (volajici pak vyprazdni TLB vsech CPU).
    pub fn drop_zero_ptes(&self, vmo: &MemoryObject, offset: u64, len: u64) -> bool {
        let zero = super::zero_frame();
        let mut inner = self.inner.lock();
        let Inner { table, mappings } = &mut *inner;
        let Some(table) = table.as_mut() else {
            return false;
        };
        let mut any = false;
        for (&start, m) in mappings.iter() {
            if !core::ptr::eq(Arc::as_ptr(&m.vmo), vmo) {
                continue;
            }
            let lo = offset.max(m.vmo_offset);
            let hi = (offset + len).min(m.vmo_offset + m.len);
            let mut pos = lo & !(PAGE_SIZE - 1);
            while pos < hi {
                let va = VirtAddr::new(start + (pos - m.vmo_offset));
                if table.translate(va) == Some(zero) {
                    table.unmap_4k(va);
                    any = true;
                }
                pos += PAGE_SIZE;
            }
        }
        any
    }

    /// Pred zapisem jadra do rozsahu userspace vytvori jeho dosud nevznikle
    /// stranky a odmapuje za ne nulovou stranku (tady i u ostatnich
    /// mapperu). Mapovani overi jen zbezne; plnou kontrolu dela kopirovani.
    fn commit_user_range(&self, addr: u64, len: usize) -> Result<(), Error> {
        let end = addr.checked_add(len as u64).ok_or(Error::BadAddress)?;
        let mut fresh: Vec<(Arc<MemoryObject>, u64)> = Vec::new();
        {
            let inner = self.inner.lock();
            let mut pos = addr & !(PAGE_SIZE - 1);
            while pos < end {
                let (start, m) = inner.mapping_at(pos).ok_or(Error::BadAddress)?;
                if m.flags & MAP_WRITE == 0 || m.vmo.is_physical() {
                    return Err(Error::BadAddress);
                }
                let off = m.vmo_offset + (pos - start);
                if m.vmo.commit_page(off)?.1 {
                    fresh.try_reserve(1).map_err(|_| Error::NoMemory)?;
                    fresh.push((m.vmo.clone(), off));
                }
                pos += PAGE_SIZE;
            }
        }
        for (vmo, off) in fresh {
            // Vlastni prostor zvlast: init (userboot) neni v seznamu mapperu.
            if self.drop_zero_ptes(&vmo, off, PAGE_SIZE) {
                tlb::shootdown();
            }
            vmo.forget_zero(off, PAGE_SIZE);
        }
        Ok(())
    }

    /// Prelozi rozsah userspace na kousky fyzicke pameti a zavola na ne `f`
    /// (adresa v prime mape, offset v rozsahu, delka). Overi, ze je cely
    /// rozsah namapovany s pozadovanymi pravy; jinak `BadAddress`. Dosud
    /// nevznikla stranka se pri cteni cte z nulove stranky; zapis ji musi
    /// predem vytvorit (`commit_user_range`).
    fn with_user_range(
        &self,
        addr: u64,
        len: usize,
        write: bool,
        mut f: impl FnMut(*mut u8, usize, usize),
    ) -> Result<(), Error> {
        if len == 0 {
            return Ok(());
        }
        let end = addr.checked_add(len as u64).ok_or(Error::BadAddress)?;
        if addr < USER_MIN || end > USER_MAX {
            return Err(Error::BadAddress);
        }
        let inner = self.inner.lock();
        // Nejdriv overit cely rozsah, at se pri chybe nic neprovede napul.
        let mut pos = addr;
        while pos < end {
            let (start, m) = inner.mapping_at(pos).ok_or(Error::BadAddress)?;
            // Registry zarizeni nejsou v prime mape: jako buffer syscallu
            // je nelze pouzit.
            if (write && m.flags & MAP_WRITE == 0) || m.vmo.is_physical() {
                return Err(Error::BadAddress);
            }
            pos = start + m.len;
        }
        let mut done = 0usize;
        while done < len {
            let pos = addr + done as u64;
            let (start, m) = inner.mapping_at(pos).expect("overeno vyse");
            let in_page = (pos % PAGE_SIZE) as usize;
            let n = (PAGE_SIZE as usize - in_page).min(len - done);
            let page: PhysAddr = match m.vmo.page(m.vmo_offset + (pos - start)) {
                Some(page) => page,
                // Jen pro cteni (`f` do nulove stranky nepise).
                None if !write => super::zero_frame(),
                None => return Err(Error::BadAddress),
            };
            f(
                page.to_virt().as_mut_ptr::<u8>().wrapping_add(in_page),
                done,
                n,
            );
            done += n;
        }
        Ok(())
    }

    /// Zkopiruje data z pameti procesu.
    pub fn copy_from_user(&self, addr: u64, buf: &mut [u8]) -> Result<(), Error> {
        let dst = buf.as_mut_ptr();
        self.with_user_range(addr, buf.len(), false, |src, at, n| {
            // SAFETY: `src..src+n` je namapovana stranka objektu pameti (prima
            // mapa), cil je buffer jadra; zamek adresniho prostoru brani
            // uvolneni stranek behem kopirovani.
            unsafe { core::ptr::copy_nonoverlapping(src, dst.add(at), n) }
        })
    }

    /// Zkopiruje data do pameti procesu (jen do zapisovatelnych mapovani).
    pub fn copy_to_user(&self, addr: u64, data: &[u8]) -> Result<(), Error> {
        if data.is_empty() {
            return Ok(());
        }
        self.commit_user_range(addr, data.len())?;
        let src = data.as_ptr();
        self.with_user_range(addr, data.len(), true, |dst, at, n| {
            // SAFETY: viz `copy_from_user`.
            unsafe { core::ptr::copy_nonoverlapping(src.add(at), dst, n) }
        })
    }

    /// Pocet mapovani (pro testy a ladeni).
    pub fn mapping_count(&self) -> usize {
        self.inner.lock().mappings.len()
    }

    /// Prelozi adresu podle tabulek stranek (pro testy).
    pub fn translate(&self, virt: VirtAddr) -> Option<PhysAddr> {
        self.inner.lock().table.as_ref()?.translate(virt)
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        let inner = self.inner.get_mut();
        if let Some(table) = inner.table.take() {
            // SAFETY: AddressSpace se rusi az s posledni referenci na proces;
            // jeho vlakna uz nebezi a planovac pri prepnuti na jine vlakno
            // vzdy prepne adresni prostor, takze tento neni aktivni na
            // zadnem CPU. TLB zaznamy byly zahozeny prepnutim.
            unsafe { table.destroy() };
        }
        // Mapovani (Arc na objekty pameti) se uvolni s `inner`.
    }
}

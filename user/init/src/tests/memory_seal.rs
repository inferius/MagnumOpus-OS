//! Pecet oblasti (`PROT_SEAL` ve `VM_PROTECT`, jako `mseal`): zapecetenou
//! oblast nejde odmapovat, uvolnit, zmenit jeji prava ani ji nahradit novym
//! mapovanim (ochrana RELRO a kodu knihoven, docs/cs/dynamicke-linkovani.md).

use libos::abi::vm::{MAP_FIXED, MAP_IN_RESERVATION, MAP_READ, MAP_WRITE, UNMAP_DECOMMIT, region};
use libos::{Error, Startup, Vmo, sys, vm};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("pecet: zapecetene mapovani nejde zmenit, odmapovat ani nahradit", sealed_mapping),
    ("pecet: zapecetena rezervace nejde obsadit ani uvolnit", sealed_reservation),
];

const RW: u32 = MAP_READ | MAP_WRITE;

fn me() -> u32 {
    libos::rt::self_process()
}

fn sealed_mapping(_: &Startup) -> TestResult {
    let pg = vm::page_size();
    let base = vm::alloc(4 * pg, RW).map_err(|e| fail("alloc", e))?;
    // SAFETY: prave namapovano RW.
    unsafe { core::ptr::write_volatile(base as *mut u64, 0x1234) };
    // Prostredni dve stranky zapecetit jen pro cteni (oblast se rozdeli).
    vm::seal(base + pg, 2 * pg, MAP_READ).map_err(|e| fail("pecet", e))?;
    let q = vm::query(base + pg).map_err(|e| fail("query", e))?;
    check!(q.attrs & region::ATTR_SEALED != 0, "atribut SEALED");
    check_eq!((q.flags, q.max_flags), (MAP_READ, MAP_READ), "prava a max po peceti");
    check_eq!(vm::protect(base + pg, pg, RW).err(), Some(Error::AccessDenied), "VM_PROTECT zpet na RW");
    check_eq!(vm::protect(base + pg, pg, MAP_READ).err(), Some(Error::AccessDenied), "VM_PROTECT i na stejna prava");
    check_eq!(vm::release(base + pg, pg).err(), Some(Error::AccessDenied), "VM_UNMAP");
    check_eq!(vm::release(base, 4 * pg).err(), Some(Error::AccessDenied), "VM_UNMAP pres celek");
    check_eq!(vm::decommit(base + pg, pg).err(), Some(Error::AccessDenied), "decommit");
    check_eq!(vm::seal(base + pg, pg, MAP_READ).err(), Some(Error::AccessDenied), "druha pecet");
    // Nove mapovani pres zapecetenou oblast (nahrada GOT anonymni RW pameti).
    let vmo = Vmo::create(pg).map_err(|e| fail("vmo", e))?;
    check_eq!(
        sys::vm_map(me(), vmo.handle().raw(), 0, pg, RW | MAP_FIXED, base + pg).err(),
        Some(Error::BadState),
        "VM_MAP pres zapecetenou oblast"
    );
    // Nezapecetene sousedni stranky jdou dal.
    vm::protect(base, pg, MAP_READ).map_err(|e| fail("protect souseda", e))?;
    // SAFETY: stranka je namapovana (jen pro cteni).
    check_eq!(unsafe { core::ptr::read_volatile(base as *const u64) }, 0x1234, "data souseda");
    vm::release(base + 3 * pg, pg).map_err(|e| fail("release souseda", e))?;
    Ok(())
}

fn sealed_reservation(_: &Startup) -> TestResult {
    let pg = vm::page_size();
    let base = vm::reserve(4 * pg, 0).map_err(|e| fail("rezervace", e))?;
    vm::seal(base, 2 * pg, 0).map_err(|e| fail("pecet rezervace", e))?;
    let vmo = Vmo::create(pg).map_err(|e| fail("vmo", e))?;
    check_eq!(
        sys::vm_map(me(), vmo.handle().raw(), 0, pg, RW | MAP_FIXED | MAP_IN_RESERVATION, base).err(),
        Some(Error::BadState),
        "commit do zapecetene rezervace"
    );
    check_eq!(sys::vm_unmap_flags(me(), base, pg, UNMAP_DECOMMIT).err(), Some(Error::AccessDenied), "decommit rezervace");
    check_eq!(vm::release(base, pg).err(), Some(Error::AccessDenied), "release rezervace");
    // Nezapecetena cast rezervace jde obsadit.
    sys::vm_map(me(), vmo.handle().raw(), 0, pg, RW | MAP_FIXED | MAP_IN_RESERVATION, base + 2 * pg)
        .map_err(|e| fail("commit do nezapecetene casti", e))?;
    Ok(())
}

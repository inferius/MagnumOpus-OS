//! Objekty pameti a mapovani.

use abi::vm::{MAP_EXECUTE, MAP_READ, MAP_WRITE};

use super::handle::arg_handle;
use super::{Args, current_process, user};
use crate::error::{Error, Result};
use crate::object::process::Process;
use crate::object::vmo::MemoryObject;
use crate::object::{KernelObject, Rights};

/// Najvetsi blok, ktery VMO_READ/VMO_WRITE prenese jednim volanim.
const RW_MAX: usize = 1 << 20;

/// `VMO_CREATE(size, process) -> handle`. Pamet se strhne z rozpoctu jobu
/// procesu `process` (handle s pravem `WRITE`; 0 = volajici). Rodic tak
/// vytvari pamet programu (segmenty, zasobnik) na ucet potomka.
pub fn vmo_create(a: Args) -> Result<u64> {
    let p = current_process();
    let job = if a[1] == 0 {
        p.job().clone()
    } else {
        let target = p
            .handles()
            .get_as::<Process>(arg_handle(a[1])?, Rights::WRITE)?;
        target.job().clone()
    };
    let vmo = MemoryObject::new(a[0], Some(&job))?;
    let rights = vmo.default_rights();
    Ok(p.handles().insert(vmo, rights)?.0 as u64)
}

/// `VMO_READ(handle, offset, buf_ptr, len)`.
pub fn vmo_read(a: Args) -> Result<u64> {
    let p = current_process();
    let vmo = p
        .handles()
        .get_as::<MemoryObject>(arg_handle(a[0])?, Rights::READ)?;
    let len = user::check_len(a[3], RW_MAX)?;
    let mut buf = alloc::vec![0; len];
    vmo.read(a[1], &mut buf)?;
    user::write_bytes(&p, a[2], &buf)?;
    Ok(0)
}

/// `VMO_WRITE(handle, offset, buf_ptr, len)`.
pub fn vmo_write(a: Args) -> Result<u64> {
    let p = current_process();
    let vmo = p
        .handles()
        .get_as::<MemoryObject>(arg_handle(a[0])?, Rights::WRITE)?;
    let len = user::check_len(a[3], RW_MAX)?;
    let data = user::read_bytes(&p, a[2], len)?;
    vmo.write(a[1], &data)?;
    Ok(0)
}

/// `VM_MAP(process, vmo, vmo_offset, len, flags, addr) -> addr`.
///
/// Prava: proces WRITE (meni jeho adresni prostor), objekt pameti MAP
/// a podle priznaku READ / WRITE / EXECUTE.
pub fn map(a: Args) -> Result<u64> {
    let p = current_process();
    let flags = u32::try_from(a[4]).map_err(|_| Error::InvalidArgs)?;
    let mut needed = Rights::MAP;
    if flags & MAP_READ != 0 {
        needed = needed | Rights::READ;
    }
    if flags & MAP_WRITE != 0 {
        needed = needed | Rights::WRITE;
    }
    if flags & MAP_EXECUTE != 0 {
        needed = needed | Rights::EXECUTE;
    }
    let (target, vmo) = {
        let table = p.handles();
        (
            table.get_as::<Process>(arg_handle(a[0])?, Rights::WRITE)?,
            table.get_as::<MemoryObject>(arg_handle(a[1])?, needed)?,
        )
    };
    let addr = target.address_space().map(vmo.clone(), a[2], a[3], flags, a[5])?;
    // Fyzicky objekt si pamatuje, kde je namapovany (odvolani); kdyz byl
    // mezitim odvolan, mapovani se vrati.
    if let Err(e) = vmo.register_mapper(&target) {
        let _ = target.address_space().unmap(addr, a[3]);
        return Err(e);
    }
    Ok(addr)
}

/// `VM_UNMAP(process, addr, len)`.
pub fn unmap(a: Args) -> Result<u64> {
    let p = current_process();
    let target = p
        .handles()
        .get_as::<Process>(arg_handle(a[0])?, Rights::WRITE)?;
    target.address_space().unmap(a[1], a[2])?;
    Ok(0)
}

/// `VM_PAGE_SIZE(process) -> bajty`: velikost stranky adresniho prostoru
/// procesu. Staci libovolny handle procesu (bez prav), velikost stranky
/// neni tajemstvi; rodic ji potrebuje pri nahravani programu do potomka.
pub fn page_size(a: Args) -> Result<u64> {
    let p = current_process();
    let target = p
        .handles()
        .get_as::<Process>(arg_handle(a[0])?, Rights::NONE)?;
    Ok(target.address_space().page_size())
}

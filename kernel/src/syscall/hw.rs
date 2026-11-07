//! Pristup k hardwaru pro ovladace (vyzaduje Resource s pravem MANAGE)
//! a nastaveni realneho casu (Resource s pravem SET_CLOCK).

use alloc::sync::Arc;

use super::handle::arg_handle;
use super::{Args, current_process, user};
use crate::arch::paging::PAGE_SIZE;
use crate::error::{Error, Result};
use crate::mm;
use crate::object::interrupt::Interrupt;
use crate::object::process::Process;
use crate::object::resource::Resource;
use crate::object::vmo::MemoryObject;
use crate::object::{KernelObject, Rights, charged};

fn check_resource(p: &Process, v: u64) -> Result<Arc<Resource>> {
    p.handles()
        .get_as::<Resource>(arg_handle(v)?, Rights::MANAGE)
}

/// `CLOCK_SET(resource, in_ptr, flags) -> 1/0`: nastavi realny cas.
/// Vyzaduje Resource s pravem `SET_CLOCK` (ma ho jen korenovy). Vraci 1,
/// kdyz se cas zapsal i do hardwarovych hodin (`set_flags::WRITE_HARDWARE`).
pub fn clock_set(a: Args) -> Result<u64> {
    use abi::time::{SetRequest, set_flags};
    let p = current_process();
    p.handles()
        .get_as::<Resource>(arg_handle(a[0])?, Rights::SET_CLOCK)?;
    if a[2] & !set_flags::ALL != 0 {
        return Err(Error::InvalidArgs);
    }
    let raw = user::read_bytes(&p, a[1], SetRequest::SIZE)?;
    let bytes: &[u8; SetRequest::SIZE] = raw.as_slice().try_into().map_err(|_| Error::InvalidArgs)?;
    let req = SetRequest::decode(bytes).ok_or(Error::InvalidArgs)?;
    let hw = crate::clock::set(
        req,
        a[2] & set_flags::WRITE_HARDWARE != 0,
        format_args!("[{} {}]", p.name, p.id),
    );
    Ok(hw as u64)
}

fn insert(p: &Process, object: Arc<dyn KernelObject>) -> Result<u64> {
    let rights = object.default_rights();
    Ok(p.handles().insert(object, rights)?.0 as u64)
}

/// `VMO_CREATE_CONTIGUOUS(resource, size) -> handle`.
pub fn vmo_create_contiguous(a: Args) -> Result<u64> {
    let p = current_process();
    let res = check_resource(&p, a[0])?;
    // Velikost overi `new_contiguous`; kvota se odecte az po uspechu a ve
    // skutecne velikosti (alokator pridava bloky 2^n stranek).
    let vmo = MemoryObject::new_contiguous_dma(a[1], res.domain().cloned(), Some(p.job()))?;
    res.charge_dma(vmo.allocated_bytes())?;
    insert(&p, vmo)
}

/// `VMO_CREATE_PHYSICAL(resource, phys, size) -> handle`: jen mimo RAM.
pub fn vmo_create_physical(a: Args) -> Result<u64> {
    let p = current_process();
    let res = check_resource(&p, a[0])?;
    if !mm::is_device_memory(a[1], a[2]) || !res.allows_range(a[1], a[2]) {
        return Err(Error::AccessDenied);
    }
    insert(&p, MemoryObject::new_physical(a[1], a[2], Some(p.job()))?)
}

/// `VMO_PHYS_ADDR(resource, vmo, offset) -> phys`.
pub fn vmo_phys_addr(a: Args) -> Result<u64> {
    let p = current_process();
    check_resource(&p, a[0])?;
    let vmo = p
        .handles()
        .get_as::<MemoryObject>(arg_handle(a[1])?, Rights::NONE)?;
    let page = vmo.page(a[2] & !(PAGE_SIZE - 1)).ok_or(Error::OutOfRange)?;
    Ok(page.as_u64() + (a[2] % PAGE_SIZE))
}

/// `FB_CONTROL(vmo, op, out_ptr)`: framebuffer z firmware (viz
/// `abi::syscall::FB_CONTROL`).
pub fn fb_control(a: Args) -> Result<u64> {
    use abi::syscall::fb_op;
    let p = current_process();
    let need = if a[1] == fb_op::INFO { Rights::READ } else { Rights::WRITE };
    let vmo = p.handles().get_as::<MemoryObject>(arg_handle(a[0])?, need)?;
    let ours = crate::userboot::FB_VMO.lock().as_ref().is_some_and(|f| Arc::ptr_eq(f, &vmo));
    if !ours {
        return Err(Error::WrongType);
    }
    match a[1] {
        fb_op::INFO => {
            let fb = crate::console::framebuffer().ok_or(Error::NotFound)?;
            let format = match fb.format {
                boot_info::PixelFormat::Rgb => 1,
                boot_info::PixelFormat::Bgr => 2,
                _ => 0,
            };
            let info = abi::syscall::FbInfo { width: fb.width, height: fb.height, stride: fb.stride, format };
            user::write_bytes(&p, a[2], &info.encode())?;
        }
        fb_op::CLAIM => {
            crate::log::info!("framebuffer prevzal proces {} {} (log jadra dal na seriove lince)", p.name, p.id);
            crate::console::set_claimed(true);
        }
        fb_op::RELEASE => {
            crate::console::set_claimed(false);
            crate::log::info!("framebuffer vratil proces {} {}", p.name, p.id);
        }
        _ => return Err(Error::InvalidArgs),
    }
    Ok(0)
}

/// `INTERRUPT_CREATE(resource, out_msi_ptr) -> handle`.
pub fn interrupt_create(a: Args) -> Result<u64> {
    let p = current_process();
    let res = check_resource(&p, a[0])?;
    res.charge_irq()?;
    let irq = Interrupt::new(res.device())
        .and_then(|irq| charged(irq, p.job()))
        .inspect_err(|_| res.refund_irq())?;
    let (address, data) = irq.msi_message().inspect_err(|_| res.refund_irq())?;
    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(&address.to_le_bytes());
    out[8..12].copy_from_slice(&data.to_le_bytes());
    user::write_bytes(&p, a[1], &out)?;
    insert(&p, irq)
}

/// `INTERRUPT_CREATE_LINE(resource, line) -> handle`: preruseni na stare
/// lince (ISA IRQ). Linku musi prostredek povolovat; jedna linka = jeden
/// objekt.
pub fn interrupt_create_line(a: Args) -> Result<u64> {
    let p = current_process();
    let res = check_resource(&p, a[0])?;
    let line = u32::try_from(a[1]).map_err(|_| Error::InvalidArgs)?;
    if !res.allows_line(line) {
        return Err(Error::AccessDenied);
    }
    res.charge_irq()?;
    let irq = Interrupt::new_line(line).inspect_err(|_| res.refund_irq())?;
    insert(&p, irq)
}

/// Port a sirka z argumentu, overene proti prostredku.
fn port_args(res: &Resource, port: u64, width: u64) -> Result<(u16, u8)> {
    let port = u16::try_from(port).map_err(|_| Error::InvalidArgs)?;
    let width = match width {
        1 | 2 | 4 => width as u8,
        _ => return Err(Error::InvalidArgs),
    };
    if !res.allows_ports(port, width) {
        return Err(Error::AccessDenied);
    }
    Ok((port, width))
}

/// `IOPORT_READ(resource, port, width) -> value`.
pub fn ioport_read(a: Args) -> Result<u64> {
    let p = current_process();
    let res = check_resource(&p, a[0])?;
    let (port, width) = port_args(&res, a[1], a[2])?;
    crate::arch::port_read(port, width).map(|v| v as u64)
}

/// `IOPORT_WRITE(resource, port, width, value)`.
pub fn ioport_write(a: Args) -> Result<u64> {
    let p = current_process();
    let res = check_resource(&p, a[0])?;
    let (port, width) = port_args(&res, a[1], a[2])?;
    let value = u32::try_from(a[3]).map_err(|_| Error::InvalidArgs)?;
    if width < 4 && value >> (8 * width as u32) != 0 {
        return Err(Error::InvalidArgs);
    }
    crate::arch::port_write(port, width, value).map(|_| 0)
}

/// `INTERRUPT_ACK(interrupt)`.
pub fn interrupt_ack(a: Args) -> Result<u64> {
    let p = current_process();
    let irq = p
        .handles()
        .get_as::<Interrupt>(arg_handle(a[0])?, Rights::WRITE)?;
    irq.ack();
    Ok(0)
}

/// `RESOURCE_CREATE(resource, ranges_ptr, ranges_count, irq_max, dma_max, device) -> handle`.
/// Rozsahy jsou dvojice u64 (fyzicky zacatek, delka).
pub fn resource_create(a: Args) -> Result<u64> {
    use crate::object::resource::MAX_RANGES;
    let p = current_process();
    let parent = check_resource(&p, a[0])?;
    let count = user::check_len(a[2], MAX_RANGES)?;
    let raw = user::read_bytes(&p, a[1], count * 16)?;
    let ranges = raw
        .as_chunks::<16>()
        .0
        .iter()
        .map(|c| {
            (
                u64::from_le_bytes(c[0..8].try_into().unwrap()),
                u64::from_le_bytes(c[8..16].try_into().unwrap()),
            )
        })
        .collect();
    let irq_max = u32::try_from(a[3]).map_err(|_| Error::InvalidArgs)?;
    // Zarizeni: 0 = zadne, jinak 1 << 32 | segment << 16 | sbernice << 8 |
    // zarizeni << 3 | funkce.
    let device = match a[5] {
        0 => None,
        v if v >> 32 == 1 => Some(((v >> 16) as u16, (v >> 8) as u8, v as u8)),
        _ => return Err(Error::InvalidArgs),
    };
    insert(&p, charged(parent.restrict(ranges, irq_max, a[4], device)?, p.job())?)
}

/// `RANDOM_ADD(resource, ptr, len, credit_bits)`: entropie od ovladace
/// (virtio-rng). Vyzaduje Resource s pravem `MANAGE`; kredit nejvys 8 bitu
/// na bajt.
pub fn random_add(a: Args) -> Result<u64> {
    let p = current_process();
    check_resource(&p, a[0])?;
    let len = user::check_len(a[2], abi::syscall::RANDOM_MAX)?;
    let credit = u32::try_from(a[3]).map_err(|_| Error::InvalidArgs)?;
    let data = user::read_bytes(&p, a[1], len)?;
    crate::random::add(&data, credit);
    Ok(0)
}

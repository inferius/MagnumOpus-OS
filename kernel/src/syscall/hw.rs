//! Pristup k hardwaru pro ovladace (vyzaduje Resource s pravem MANAGE).

use alloc::sync::Arc;

use super::handle::arg_handle;
use super::{Args, current_process, user};
use crate::error::{Error, Result};
use crate::mm;
use crate::object::interrupt::Interrupt;
use crate::object::process::Process;
use crate::object::resource::Resource;
use crate::object::vmo::MemoryObject;
use crate::object::{KernelObject, Rights};

fn check_resource(p: &Process, v: u64) -> Result<()> {
    p.handles()
        .get_as::<Resource>(arg_handle(v)?, Rights::MANAGE)
        .map(|_| ())
}

fn insert(p: &Process, object: Arc<dyn KernelObject>) -> Result<u64> {
    let rights = object.default_rights();
    Ok(p.handles().insert(object, rights)?.0 as u64)
}

/// `VMO_CREATE_CONTIGUOUS(resource, size) -> handle`.
pub fn vmo_create_contiguous(a: Args) -> Result<u64> {
    let p = current_process();
    check_resource(&p, a[0])?;
    insert(&p, MemoryObject::new_contiguous(a[1])?)
}

/// `VMO_CREATE_PHYSICAL(resource, phys, size) -> handle`: jen mimo RAM.
pub fn vmo_create_physical(a: Args) -> Result<u64> {
    let p = current_process();
    check_resource(&p, a[0])?;
    if !mm::is_device_memory(a[1], a[2]) {
        return Err(Error::AccessDenied);
    }
    insert(&p, MemoryObject::new_physical(a[1], a[2])?)
}

/// `VMO_PHYS_ADDR(resource, vmo, offset) -> phys`.
pub fn vmo_phys_addr(a: Args) -> Result<u64> {
    let p = current_process();
    check_resource(&p, a[0])?;
    let vmo = p
        .handles()
        .get_as::<MemoryObject>(arg_handle(a[1])?, Rights::NONE)?;
    let page = vmo
        .page(a[2] & !(abi::vm::PAGE_SIZE - 1))
        .ok_or(Error::OutOfRange)?;
    Ok(page.as_u64() + (a[2] % abi::vm::PAGE_SIZE))
}

/// `INTERRUPT_CREATE(resource, out_msi_ptr) -> handle`.
pub fn interrupt_create(a: Args) -> Result<u64> {
    let p = current_process();
    check_resource(&p, a[0])?;
    let irq = Interrupt::new()?;
    let (address, data) = irq.msi_message();
    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(&address.to_le_bytes());
    out[8..12].copy_from_slice(&data.to_le_bytes());
    user::write_bytes(&p, a[1], &out)?;
    insert(&p, irq)
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

//! Kopirovani argumentu syscallu z a do pameti procesu.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::object::process::Process;

/// Precte `len` bajtu z adresy `addr` procesu.
pub fn read_bytes(p: &Process, addr: u64, len: usize) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    buf.try_reserve_exact(len).map_err(|_| Error::NoMemory)?;
    buf.resize(len, 0);
    p.address_space().copy_from_user(addr, &mut buf)?;
    Ok(buf)
}

pub fn write_bytes(p: &Process, addr: u64, data: &[u8]) -> Result<()> {
    p.address_space().copy_to_user(addr, data)
}

pub fn read_u32s(p: &Process, addr: u64, count: usize) -> Result<Vec<u32>> {
    let bytes = read_bytes(p, addr, count.checked_mul(4).ok_or(Error::InvalidArgs)?)?;
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect())
}

pub fn write_u32s(p: &Process, addr: u64, values: &[u32]) -> Result<()> {
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    write_bytes(p, addr, &bytes)
}

pub fn write_u64(p: &Process, addr: u64, value: u64) -> Result<()> {
    write_bytes(p, addr, &value.to_le_bytes())
}

/// Precte jmeno (UTF-8, max `abi::syscall::NAME_MAX` bajtu).
pub fn read_name(p: &Process, addr: u64, len: u64) -> Result<String> {
    if len as usize > abi::syscall::NAME_MAX {
        return Err(Error::InvalidArgs);
    }
    let bytes = read_bytes(p, addr, len as usize)?;
    String::from_utf8(bytes).map_err(|_| Error::InvalidArgs)
}

/// Pomocna kontrola delky zadane procesem.
pub fn check_len(len: u64, max: usize) -> Result<usize> {
    if len as usize > max || len > max as u64 {
        Err(Error::InvalidArgs)
    } else {
        Ok(len as usize)
    }
}

#[allow(dead_code, reason = "pro budouci syscally se strukturami")]
pub fn zeroed(len: usize) -> Vec<u8> {
    vec![0; len]
}

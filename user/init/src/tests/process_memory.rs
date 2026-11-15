//! Pamet jineho procesu (`PROCESS_MEMORY`, syscall 77, Windows vrstva W2):
//! kopie pres jadro, prava `READ_MEMORY`/`WRITE_MEMORY`, meze.

use alloc::vec;

use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Error, Handle, Process, Rights, Startup, Vmo};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("pamet procesu: cteni a zapis jineho procesu (PROCESS_MEMORY)", read_write_other),
    ("pamet procesu: prava, mapovani jen pro cteni, nenamapovano, skonceny proces", rights_and_limits),
];

/// Prazdny proces (bez vlakna) s namapovanym objektem pameti; vraci
/// (proces, objekt, adresa v procesu).
fn target(len: u64, flags: u32) -> Result<(Process, Vmo, u64), alloc::string::String> {
    let job = libos::job::current().map_err(|e| fail("job", e))?;
    let p = Process::create(&job, "pamet-cil").map_err(|e| fail("proces", e))?;
    let vmo = Vmo::create(len).map_err(|e| fail("objekt pameti", e))?;
    let at = p.map(&vmo, 0, len, flags, 0).map_err(|e| fail("mapovani do cile", e))?;
    Ok((p, vmo, at))
}

fn read_write_other(_: &Startup) -> TestResult {
    // Vic nez jeden blok jadra (64 KiB): kopie po castech.
    let len = 3 * 64 * 1024 + 4096;
    let (p, vmo, at) = target(len as u64, MAP_READ | MAP_WRITE)?;
    let data: alloc::vec::Vec<u8> = (0..len).map(|i| (i * 7 + i / 251) as u8).collect();
    check_eq!(p.write_memory(at, &data).map_err(|e| fail("zapis", e))?, len, "zapsano bajtu");
    let mut back = vec![0u8; len];
    vmo.read(0, &mut back).map_err(|e| fail("cteni objektu", e))?;
    check!(back == data, "obsah objektu po zapisu do jineho procesu");
    let mut got = vec![0u8; len];
    check_eq!(p.read_memory(at, &mut got).map_err(|e| fail("cteni", e))?, len, "precteno bajtu");
    check!(got == data, "cteni z jineho procesu");
    // Posun uprostred stranky a kratky usek.
    let mut small = [0u8; 10];
    check_eq!(p.read_memory(at + 4093, &mut small).map_err(|e| fail("kratke cteni", e))?, 10, "kratke cteni");
    check!(small[..] == data[4093..4103], "kratke cteni pres hranici stranky");
    // Vlastni proces pres handle z jadra (SELF_PROCESS ma vychozi prava).
    let me = Process::from_handle(Handle::from_raw(libos::rt::self_process()));
    let src = [5u8, 6, 7, 8];
    let mut dst = [0u8; 4];
    let r = me.read_memory(src.as_ptr() as u64, &mut dst);
    core::mem::forget(me);
    check_eq!(r.map_err(|e| fail("cteni vlastni pameti", e))?, 4, "vlastni proces");
    check_eq!(dst, src, "obsah vlastni pameti");
    Ok(())
}

fn rights_and_limits(_: &Startup) -> TestResult {
    let (p, _vmo, at) = target(8192, MAP_READ | MAP_WRITE)?;
    let mut buf = [0u8; 16];
    // Handle bez prav pameti (napr. z vyctu procesu): ACCESS_DENIED.
    let plain = p
        .handle()
        .duplicate(Some(Rights::BASIC | Rights::READ | Rights::WRITE | Rights::MANAGE))
        .map_err(|e| fail("duplikat", e))?;
    let plain = Process::from_handle(plain);
    check_eq!(plain.read_memory(at, &mut buf).err(), Some(Error::AccessDenied), "cteni bez READ_MEMORY");
    check_eq!(plain.write_memory(at, &buf).err(), Some(Error::AccessDenied), "zapis bez WRITE_MEMORY");
    let ro = Process::from_handle(p.handle().duplicate(Some(Rights::BASIC | Rights::READ_MEMORY)).map_err(|e| fail("duplikat", e))?);
    check!(ro.read_memory(at, &mut buf).is_ok(), "cteni jen s READ_MEMORY");
    check_eq!(ro.write_memory(at, &buf).err(), Some(Error::AccessDenied), "zapis jen s READ_MEMORY");
    // Nenamapovano a mimo pamet programu.
    check_eq!(p.read_memory(at + 8192, &mut buf).err(), Some(Error::BadAddress), "za koncem mapovani");
    check_eq!(p.read_memory(u64::MAX - 4, &mut buf).err(), Some(Error::BadAddress), "preteceni adresy");
    // Mapovani jen pro cteni: zapis nejde, cteni ano.
    let (q, _v2, at2) = target(4096, MAP_READ)?;
    check!(q.read_memory(at2, &mut buf).is_ok(), "cteni mapovani jen pro cteni");
    check_eq!(q.write_memory(at2, &buf).err(), Some(Error::BadAddress), "zapis do mapovani jen pro cteni");
    // Skonceny proces (prostor zrusen).
    q.kill().map_err(|e| fail("kill", e))?;
    let _ = q.wait_exit(Some(libos::time::deadline_after(core::time::Duration::from_secs(5))));
    check_eq!(q.read_memory(at2, &mut buf).err(), Some(Error::BadState), "skonceny proces");
    check_eq!(
        libos::sys::process_memory_read(p.handle().raw(), at, &mut []).map_err(|e| fail("nulova delka", e))?,
        0,
        "nulova delka"
    );
    Ok(())
}

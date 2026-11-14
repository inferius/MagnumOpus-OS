//! Stranky na pozadani pres skutecne vypadky: velky objekt, nulova
//! stranka, soubezne vypadky, strop commitu.

use alloc::vec::Vec;
use core::time::Duration;

use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::Mapping;
use libos::{Error, Startup, Vmo, time};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("stranky na pozadani: 256 MiB objekt, cteni nul a zapis", big_lazy),
    ("nulova stranka: zapis jinym mapovanim i VMO_WRITE je videt", zero_page_coherence),
    ("stranky na pozadani: 4 vlakna soubezne na stejnych strankach", concurrent_faults),
    ("slib vetsi nez pamet -> NO_MEMORY", commit_over_ram),
];

const MIB: u64 = 1024 * 1024;

fn page() -> u64 {
    libos::vm::page_size()
}

fn big_lazy(_: &Startup) -> TestResult {
    let size = 256 * MIB;
    let vmo = Vmo::create(size).map_err(|e| fail("256 MiB", e))?;
    let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE).map_err(|e| fail("mapovani", e))?;
    // Cteni po 1 MiB: nuly (nulova stranka, zadne ramce).
    for off in (0..size).step_by(MIB as usize) {
        check_eq!(map.read::<u64>(off + 8), 0, "cteni nevznikle stranky");
    }
    // Zapis do kazde stranky prvnich 4 MiB a zpet.
    for off in (0..4 * MIB).step_by(page() as usize) {
        map.write::<u64>(off, off ^ 0x5A5A);
    }
    for off in (0..4 * MIB).step_by(page() as usize) {
        check_eq!(map.read::<u64>(off), off ^ 0x5A5A, "zapsana hodnota");
    }
    // Tataz data pres jadro (VMO_READ), nezapsane dal nuly.
    let mut b = [0u8; 8];
    vmo.read(page(), &mut b).map_err(|e| fail("VMO_READ", e))?;
    check_eq!(u64::from_le_bytes(b), page() ^ 0x5A5A, "VMO_READ zapsane stranky");
    vmo.read(200 * MIB, &mut b).map_err(|e| fail("VMO_READ", e))?;
    check_eq!(u64::from_le_bytes(b), 0, "VMO_READ nevznikle stranky");
    Ok(())
}

fn zero_page_coherence(_: &Startup) -> TestResult {
    let vmo = Vmo::create(2 * page()).map_err(|e| fail("objekt", e))?;
    let rw = Mapping::new(&vmo, 0, 2 * page(), MAP_READ | MAP_WRITE).map_err(|e| fail("RW", e))?;
    let ro = Mapping::new(&vmo, 0, 2 * page(), MAP_READ).map_err(|e| fail("RO", e))?;
    // RO cte nulovou stranku, pak zapis pres RW: RO musi videt novou hodnotu.
    check_eq!(ro.read::<u64>(16), 0, "pred zapisem");
    rw.write::<u64>(16, 42);
    check_eq!(ro.read::<u64>(16), 42, "druhe mapovani po zapisu vypadkem");
    // Druha stranka: RO cte nuly, zapis jadrem (VMO_WRITE).
    check_eq!(ro.read::<u64>(page() + 8), 0, "druha stranka pred zapisem");
    vmo.write(page() + 8, &7u64.to_le_bytes()).map_err(|e| fail("VMO_WRITE", e))?;
    check_eq!(ro.read::<u64>(page() + 8), 7, "mapovani po VMO_WRITE");
    check_eq!(rw.read::<u64>(page() + 8), 7, "RW po VMO_WRITE");
    Ok(())
}

fn concurrent_faults(_: &Startup) -> TestResult {
    const THREADS: u64 = 4;
    let len = 4 * MIB;
    let vmo = Vmo::create(len).map_err(|e| fail("objekt", e))?;
    let map = Mapping::new(&vmo, 0, len, MAP_READ | MAP_WRITE).map_err(|e| fail("mapovani", e))?;
    let (base, step) = (map.addr(), page());
    let mut workers = Vec::new();
    for t in 0..THREADS {
        let w = libos::thread::spawn("vypadky", move || {
            // Kazde vlakno pise na svuj bajt v kazde strance: vsechna
            // vlakna tak padaji na tytez stranky naraz.
            for off in (0..len).step_by(step as usize) {
                // SAFETY: `base..base+len` je namapovane pro zapis po celou
                // dobu (mapovani zrusime az po `join`); bajty vlaken se
                // neprekryvaji.
                unsafe { core::ptr::write_volatile((base + off + t) as *mut u8, t as u8 + 1) };
            }
        })
        .map_err(|e| fail("vlakno", e))?;
        workers.push(w);
    }
    for w in workers {
        w.join(Some(time::deadline_after(Duration::from_secs(20))))
            .map_err(|e| fail("join", e))?;
    }
    for off in (0..len).step_by(step as usize) {
        for t in 0..THREADS {
            check_eq!(map.read::<u8>(off + t), t as u8 + 1, "bajt vlakna");
        }
    }
    Ok(())
}

fn commit_over_ram(_: &Startup) -> TestResult {
    // 512 GiB se slibit neda (rozpocet initu je bez limitu, rozhoduje RAM).
    check_eq!(Vmo::create(512 * 1024 * MIB).err(), Some(Error::NoMemory), "slib nad RAM");
    // Po odmitnuti jde dal normalne alokovat.
    check!(Vmo::create(MIB).is_ok(), "mala alokace po odmitnuti");
    Ok(())
}

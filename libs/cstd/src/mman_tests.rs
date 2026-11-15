//! Testy mmap na hostiteli: kontrola argumentu a mapovani souboru.

use crate::errno::{EBADF, EINVAL};
use crate::mman::*;

const PAGE: u64 = 4096;

#[test]
fn argument_checks() {
    let anon = MAP_PRIVATE | MAP_ANONYMOUS;
    let r = check(0, 5000, PROT_READ | PROT_WRITE, anon, -1, 0, PAGE).unwrap();
    assert_eq!((r.len, r.addr, r.file), (8192, None, None), "delka na cele stranky");
    assert_eq!(check(0, 0, PROT_READ, anon, -1, 0, PAGE), Err(EINVAL));
    assert_eq!(check(0, 10, PROT_READ, MAP_ANONYMOUS, -1, 0, PAGE), Err(EINVAL), "chybi SHARED/PRIVATE");
    assert_eq!(check(0x1001, 10, PROT_READ, anon | MAP_FIXED, -1, 0, PAGE), Err(EINVAL), "nezarovnana pevna adresa");
    let f = check(0x10000, 10, PROT_READ, anon | MAP_FIXED, -1, 0, PAGE).unwrap();
    assert_eq!((f.addr, f.replace), (Some(0x10000), true));
    assert_eq!(check(0, 10, PROT_READ, MAP_PRIVATE, -1, 0, PAGE), Err(EBADF), "soubor bez fd");
    assert_eq!(check(0, 10, PROT_READ, MAP_PRIVATE, 3, 100, PAGE), Err(EINVAL), "offset mimo stranku");
    // Sdileny zapis: rozhodne `map` podle druhu fd (pametovy fd ano, soubor ENOTSUP).
    assert!(check(0, 10, PROT_READ | PROT_WRITE, MAP_SHARED, 3, 0, PAGE).unwrap().shared);
    assert!(check(0, 10, PROT_READ, MAP_SHARED, 3, 0, PAGE).unwrap().shared);
}

#[test]
fn file_mapping_reads_content() {
    let p = std::env::temp_dir().join(format!("cstd-mmap-{}", std::process::id()));
    std::fs::write(&p, b"obsah souboru").unwrap();
    let path = p.to_str().unwrap().as_bytes().to_vec();
    let fd = crate::sys::open(&path, &crate::unistd::open_flags(0).unwrap()).unwrap();
    // SAFETY: platne argumenty; vysledek se jen cte.
    let m = unsafe { mmap(core::ptr::null_mut(), 13, PROT_READ, MAP_PRIVATE, fd, 0) };
    assert_ne!(m, MAP_FAILED);
    // SAFETY: namapovano aspon 13 bajtu.
    let got = unsafe { core::slice::from_raw_parts(m as *const u8, 4096) };
    assert_eq!(&got[..13], b"obsah souboru");
    assert!(got[13..].iter().all(|&b| b == 0), "zbytek stranky nuly");
    // SAFETY: mapovani z mmap.
    assert_eq!(unsafe { munmap(m, 13) }, 0);
    crate::sys::close(fd).unwrap();
    std::fs::remove_file(p).unwrap();
}

//! Host testy parseru ELF (`cargo test -p elf`) proti souborum z clang
//! a ld.lld (`tests/fixtures/`, zdroj a `gen.sh` tamtez).

extern crate std;

use std::vec::Vec;

use crate::dynamic::Dynamic;
use crate::reloc::{Kind, Relr};
use crate::symbols::{gnu_hash, kind, sysv_hash};
use crate::{ElfError, ElfFile, ElfType, Machine};

macro_rules! fixture {
    ($name:literal) => {
        include_bytes!(concat!("../tests/fixtures/", $name))
    };
}

const X64_LIB: &[u8] = fixture!("x86_64-libfix.so");
const X64_RELR: &[u8] = fixture!("x86_64-libfix-relr.so");
const X64_PIE: &[u8] = fixture!("x86_64-main.pie");
const X64_TEXTREL: &[u8] = fixture!("x86_64-textrel.so");
const A64_LIB: &[u8] = fixture!("aarch64-libfix.so");
const A64_RELR: &[u8] = fixture!("aarch64-libfix-relr.so");
const A64_PIE: &[u8] = fixture!("aarch64-main.pie");

fn open(data: &[u8], m: Machine) -> (ElfFile<'_>, Dynamic) {
    let elf = ElfFile::parse_object(data, m).expect("hlavicka");
    let d = elf.dynamic().expect("dynamicka cast").expect("PT_DYNAMIC");
    (elf, d)
}

/// Druhy relokaci RELA s jmeny symbolu.
fn relocs<'a>(elf: &ElfFile<'a>, d: &Dynamic) -> Vec<(Kind, &'a str, u64, i64)> {
    let syms = elf.symbols(d).unwrap();
    elf.relas(d)
        .unwrap()
        .map(|r| {
            let r = r.unwrap();
            let name = if r.sym == 0 { "" } else { syms.get(r.sym as usize).unwrap().name };
            (r.kind, name, r.offset, r.addend)
        })
        .collect()
}

#[test]
fn hash_functions_match_reference() {
    assert_eq!(gnu_hash(""), 5381);
    assert_eq!(gnu_hash("printf"), 0x156b_2bb8);
    assert_eq!(sysv_hash("printf"), 0x0779_05a6);
}

#[test]
fn shared_library_headers() {
    for (data, m) in [(X64_LIB, Machine::X86_64), (A64_LIB, Machine::Aarch64)] {
        let (elf, d) = open(data, m);
        assert_eq!(elf.elf_type, ElfType::Dyn);
        assert_eq!(elf.interp().unwrap(), None, "knihovna nema PT_INTERP");
        let tls = elf.tls().unwrap().expect("PT_TLS");
        assert_eq!((tls.file_size, tls.mem_size), (4, 4), "tls_counter = 7");
        assert_eq!(elf.tls_data(&tls).unwrap(), &7u32.to_le_bytes());
        assert!(elf.relro().unwrap().is_some(), "plne RELRO");
        let names: Vec<&str> = d.needed().iter().map(|&o| elf.dyn_str(&d, o).unwrap()).collect();
        assert_eq!(names, ["libdep.so.1"]);
        assert_eq!(elf.dyn_str(&d, d.soname.unwrap()).unwrap(), "libfix.so.1");
        assert_eq!(elf.dyn_str(&d, d.runpath.unwrap()).unwrap(), "$ORIGIN");
        assert!(d.bind_now() && !d.pie());
        for s in elf.load_segments() {
            assert_eq!(s.unwrap().align, 0x10000, "segmenty po 64 KiB");
        }
    }
}

#[test]
fn symbol_lookup_gnu_and_sysv() {
    for (data, m) in [(X64_LIB, Machine::X86_64), (A64_LIB, Machine::Aarch64)] {
        let (elf, d) = open(data, m);
        // Knihovna ma oba hashe: GNU, pak jen SysV.
        let mut only_sysv = d.clone();
        only_sysv.gnu_hash = None;
        assert!(d.hash.is_some() && d.gnu_hash.is_some());
        for d in [&d, &only_sysv] {
            let syms = elf.symbols(d).unwrap();
            let (_, f) = syms.lookup("exported_fn").unwrap().expect("exported_fn");
            assert_eq!(f.kind, kind::FUNC);
            assert!(f.is_defined() && f.value != 0);
            let (_, v) = syms.lookup("exported_var").unwrap().expect("exported_var");
            assert_eq!((v.kind, v.size), (kind::OBJECT, 4));
            assert_eq!(elf.at_vaddr(v.value, 4).unwrap(), &42u32.to_le_bytes());
            let (_, t) = syms.lookup("tls_counter").unwrap().expect("tls_counter");
            assert!(t.is_tls());
            assert_eq!(syms.lookup("hidden_fn").unwrap(), None, "skryty symbol");
            assert_eq!(syms.lookup("dep_call").unwrap(), None, "nedefinovany symbol neni export");
            assert_eq!(syms.lookup("neexistuje").unwrap(), None);
        }
    }
    // Jen GNU hash (RELR varianta).
    let (elf, d) = open(X64_RELR, Machine::X86_64);
    assert!(d.hash.is_none());
    assert!(elf.symbols(&d).unwrap().lookup("exported_fn").unwrap().is_some());
}

#[test]
fn relocations_x86_64() {
    let (elf, d) = open(X64_LIB, Machine::X86_64);
    let r = relocs(&elf, &d);
    let count = |k: Kind| r.iter().filter(|x| x.0 == k).count();
    assert_eq!(count(Kind::Relative), 2, "{r:?}");
    assert!(r.contains(&(Kind::Absolute, "exported_var", r.iter().find(|x| x.0 == Kind::Absolute).unwrap().2, 0)));
    assert!(r.iter().any(|x| x.0 == Kind::GlobDat && x.1 == "dep_value"));
    assert!(r.iter().any(|x| x.0 == Kind::TlsModule && x.1 == "tls_counter"));
    assert!(r.iter().any(|x| x.0 == Kind::TlsOffset && x.1 == "tls_counter"));
    assert!(r.iter().any(|x| x.0 == Kind::JumpSlot && x.1 == "dep_call"));
    assert!(r.iter().any(|x| x.0 == Kind::JumpSlot && x.1 == "__tls_get_addr"), "general-dynamic TLS");
    // RELATIVE miri do pointer_table (3 ukazatele) a hodnota = baze + addend.
    let syms = elf.symbols(&d).unwrap();
    let (_, table) = syms.lookup("pointer_table").unwrap().unwrap();
    for x in r.iter().filter(|x| x.0 == Kind::Relative) {
        assert!(x.2 >= table.value && x.2 < table.value + 24);
    }
    let rel = elf.relas(&d).unwrap().map(Result::unwrap).find(|x| x.kind == Kind::Relative).unwrap();
    assert_eq!(rel.value(0x7000_0000, 0), Some(0x7000_0000 + rel.addend as u64));
}

#[test]
fn relocations_aarch64() {
    let (elf, d) = open(A64_LIB, Machine::Aarch64);
    let r = relocs(&elf, &d);
    assert_eq!(r.iter().filter(|x| x.0 == Kind::Relative).count(), 2, "{r:?}");
    assert!(r.iter().any(|x| x.0 == Kind::Absolute && x.1 == "exported_var"));
    assert!(r.iter().any(|x| x.0 == Kind::TlsDesc && x.1 == "tls_counter"), "TLSDESC je na aarch64 vychozi");
    assert!(r.iter().any(|x| x.0 == Kind::JumpSlot && x.1 == "dep_call"));
    assert!(r.iter().all(|x| !matches!(x.0, Kind::Unknown(_))));
}

#[test]
fn relr_matches_relative_relocations() {
    for (data, m) in [(X64_RELR, Machine::X86_64), (A64_RELR, Machine::Aarch64)] {
        let (elf, d) = open(data, m);
        assert!(d.relr.size > 0, "DT_RELR");
        let r = relocs(&elf, &d);
        assert!(r.iter().all(|x| x.0 != Kind::Relative), "relativni jsou v RELR");
        let syms = elf.symbols(&d).unwrap();
        let (_, table) = syms.lookup("pointer_table").unwrap().unwrap();
        let addrs: Vec<u64> = elf.relrs(&d).unwrap().map(Result::unwrap).collect();
        assert_eq!(addrs.len(), 2, "{addrs:x?}");
        for a in addrs {
            assert!(a >= table.value && a < table.value + 24);
            // Puvodni slovo = adresa cile (addend RELR je v pameti).
            let w = u64::from_le_bytes(elf.at_vaddr(a, 8).unwrap().try_into().unwrap());
            assert!(w != 0);
        }
    }
}

#[test]
fn relr_decoder() {
    let enc = |w: &[u64]| -> Vec<u8> { w.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let dec = |b: &[u8]| -> Vec<u64> { Relr::decode(b).map(Result::unwrap).collect() };
    // Adresa, bitmapa (bity 1 a 3 = slova +0 a +16 za adresou), pak bitmapa
    // s nejvyssim bitem (o 63 slov dal, slovo 62).
    let b = enc(&[0x1000, (1 << 1) | (1 << 3) | 1, (1 << 63) | 1]);
    assert_eq!(dec(&b), [0x1000, 0x1008, 0x1018, 0x1008 + 63 * 8 + 62 * 8]);
    // Prazdna bitmapa taky posune o 63 slov.
    let b = enc(&[0x1000, 1, 3]);
    assert_eq!(dec(&b), [0x1000, 0x1008 + 63 * 8]);
    // Bitmapa na zacatku je chyba.
    assert!(Relr::decode(&enc(&[3])).next().unwrap().is_err());
}

#[test]
fn pie_program() {
    for (data, m) in [(X64_PIE, Machine::X86_64), (A64_PIE, Machine::Aarch64)] {
        let (elf, d) = open(data, m);
        assert_eq!(elf.elf_type, ElfType::Dyn);
        assert_eq!(elf.interp().unwrap(), Some("/system/lib/ld.so"));
        assert!(d.pie() && d.bind_now());
        let names: Vec<&str> = d.needed().iter().map(|&o| elf.dyn_str(&d, o).unwrap()).collect();
        assert_eq!(names, ["libfix.so.1"]);
        assert!(elf.entry != 0);
    }
    // Staticky zavadec (ET_EXEC) PIE odmitne.
    assert!(matches!(ElfFile::parse(X64_PIE), Err(ElfError::Unsupported(_))));
}

#[test]
fn rejects_textrel_and_wrong_machine() {
    let elf = ElfFile::parse_object(X64_TEXTREL, Machine::X86_64).unwrap();
    assert_eq!(elf.dynamic().err(), Some(ElfError::TextRelocations));
    assert!(matches!(ElfFile::parse_object(A64_LIB, Machine::X86_64), Err(ElfError::Unsupported(_))));
    assert!(matches!(ElfFile::parse_object(b"\x7fELF", Machine::X86_64), Err(ElfError::TooShort)));
}

/// Projde vse, co zavadec cte; na vadnem vstupu jen chyby, zadna panika.
fn walk(data: &[u8], m: Machine) {
    let Ok(elf) = ElfFile::parse_object(data, m) else { return };
    let _ = elf.interp();
    let _ = elf.tls().map(|t| t.map(|t| elf.tls_data(&t)));
    let _ = elf.relro();
    for s in elf.load_segments().flatten() {
        let _ = elf.segment_data(&s);
    }
    let Ok(Some(d)) = elf.dynamic() else { return };
    for &o in d.needed() {
        let _ = elf.dyn_str(&d, o);
    }
    if let Ok(syms) = elf.symbols(&d) {
        for i in 0..syms.len().min(64) {
            let _ = syms.get(i);
        }
        for n in ["exported_fn", "tls_counter", "x"] {
            let _ = syms.lookup(n);
        }
    }
    if let Ok(r) = elf.relas(&d) {
        for x in r.take(1000) {
            let _ = x;
        }
    }
    if let Ok(r) = elf.relrs(&d) {
        for x in r.take(10_000) {
            let _ = x;
        }
    }
}

#[test]
fn damaged_input_does_not_panic() {
    for (data, m) in [(X64_LIB, Machine::X86_64), (A64_RELR, Machine::Aarch64), (X64_PIE, Machine::X86_64)] {
        // Zkraceni.
        for len in (0..data.len()).step_by(13) {
            walk(&data[..len], m);
        }
        // Nahodne zmenene bajty (deterministicky LCG).
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..3000 {
            let mut d = data.to_vec();
            for _ in 0..4 {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let i = (x >> 33) as usize % d.len();
                d[i] = (x >> 16) as u8;
            }
            walk(&d, m);
        }
    }
}

/// Modul "nahrany" do pameti (segmenty na `vaddr`, bss nuly) jako ho vidi
/// zavadec: stejne udaje jako ze souboru.
#[test]
fn loaded_module_view() {
    for (data, m) in [(X64_LIB, Machine::X86_64), (A64_RELR, Machine::Aarch64)] {
        let file = ElfFile::parse_object(data, m).unwrap();
        let span = file.load_segments().map(|s| s.unwrap()).map(|s| s.vaddr + s.mem_size).max().unwrap();
        let mut mem = std::vec![0u8; span as usize];
        for s in file.load_segments() {
            let s = s.unwrap();
            mem[s.vaddr as usize..(s.vaddr + s.file_size) as usize].copy_from_slice(file.segment_data(&s));
        }
        let elf = ElfFile::parse_loaded(&mem, m).unwrap();
        let d = elf.dynamic().unwrap().unwrap();
        assert_eq!(elf.dyn_str(&d, d.soname.unwrap()).unwrap(), "libfix.so.1");
        let syms = elf.symbols(&d).unwrap();
        let (_, v) = syms.lookup("exported_var").unwrap().unwrap();
        assert_eq!(elf.at_vaddr(v.value, 4).unwrap(), &42u32.to_le_bytes());
        assert_eq!(elf.relas(&d).unwrap().count(), file.relas(&file.dynamic().unwrap().unwrap()).unwrap().count());
        let tls = elf.tls().unwrap().unwrap();
        assert_eq!(elf.tls_data(&tls).unwrap(), &7u32.to_le_bytes());
        assert!(elf.at_vaddr(span, 1).is_err(), "mimo modul");
    }
}

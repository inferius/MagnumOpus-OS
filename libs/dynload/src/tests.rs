//! Testy logiky zavadece na hostiteli.

use super::*;

const M4: Option<TlsModule> = Some(TlsModule { size: 4, align: 4 });

#[test]
fn tls_x86_64_matches_linker_local_exec() {
    // Program s 4 B (align 4): lld pro local-exec dava TP - 4.
    let l = tls_layout(Machine::X86_64, &[M4], 0);
    assert_eq!(l.offsets, [Some(-4)]);
    assert_eq!((l.tp, l.size, l.align), (64, 80, 16));
    // Program bez TLS, knihovny 4 B a 24 B s align 8.
    let l = tls_layout(Machine::X86_64, &[None, M4, Some(TlsModule { size: 24, align: 8 })], 0);
    assert_eq!(l.offsets, [None, Some(-4), Some(-32)]);
    // Vsechna data pod TP a v bloku.
    assert!(l.tp >= 32 && l.tp % 16 == 0);
}

#[test]
fn tls_aarch64_matches_linker_local_exec() {
    // lld: TP + zarovnane(16, align).
    let l = tls_layout(Machine::Aarch64, &[M4], 0);
    assert_eq!(l.offsets, [Some(16)]);
    assert_eq!(l.tp, 0);
    let l = tls_layout(Machine::Aarch64, &[M4, Some(TlsModule { size: 8, align: 64 })], 0);
    assert_eq!(l.offsets, [Some(16), Some(64)]);
    assert_eq!((l.size, l.align), (128, 64));
}

#[test]
fn relocation_values() {
    let d = Def { base: 0x10_0000, value: 0x40, tls: None };
    assert_eq!(compute(Kind::Relative, 0x5000, 0x10, None, None, 0), Ok(Apply::Word(0x5010)));
    assert_eq!(compute(Kind::GlobDat, 0x5000, 0, Some(d), None, 0), Ok(Apply::Word(0x10_0040)));
    assert_eq!(compute(Kind::Absolute, 0x5000, 8, Some(d), None, 0), Ok(Apply::Word(0x10_0048)));
    // Slaby nedefinovany symbol = 0.
    assert_eq!(compute(Kind::JumpSlot, 0x5000, 0, None, None, 0), Ok(Apply::Word(0)));
    assert_eq!(compute(Kind::IRelative, 0x5000, 0x80, None, None, 0), Ok(Apply::CallIfunc(0x5080)));
    let t = Def { base: 0x10_0000, value: 4, tls: Some((2, -32)) };
    assert_eq!(compute(Kind::TlsModule, 0, 0, Some(t), None, 0), Ok(Apply::Word(2)));
    assert_eq!(compute(Kind::TlsOffset, 0, 8, Some(t), None, 0), Ok(Apply::Word(12)));
    assert_eq!(compute(Kind::TlsTpOffset, 0, 0, Some(t), None, 0), Ok(Apply::Word((-28i64) as u64)));
    assert_eq!(compute(Kind::TlsDesc, 0, 0, Some(t), None, 0x777), Ok(Apply::Desc(0x777, (-28i64) as u64)));
    // TLS relokace bez symbolu = vlastni modul.
    assert_eq!(compute(Kind::TlsModule, 0, 0, None, Some((1, -4)), 0), Ok(Apply::Word(1)));
    assert!(compute(Kind::TlsModule, 0, 0, None, None, 0).is_err());
    assert!(compute(Kind::Copy, 0, 0, Some(d), None, 0).is_err());
    assert!(compute(Kind::Unknown(99), 0, 0, None, None, 0).is_err());
}

#[test]
fn library_paths() {
    assert_eq!(system_name("libdltest.so.1"), Some("libdltest.so.1"));
    assert_eq!(system_name("/system/lib/libx.so.2"), Some("libx.so.2"));
    assert_eq!(system_name("/home/a/libx.so"), None);
    assert_eq!(system_name("../libx.so"), None);
    assert_eq!(system_name(""), None);
    assert_eq!(system_name("/system/lib/"), None);
}

#[test]
fn tls_surplus_after_modules() {
    // Rezerva nesmi zmenit posuny modulu (program local-exec).
    let x = tls_layout(Machine::X86_64, &[M4], STATIC_TLS_SURPLUS);
    assert_eq!(x.offsets, [Some(-4)]);
    let (start, len) = x.surplus;
    assert_eq!(len, STATIC_TLS_SURPLUS);
    assert!(start <= -4 - len as i64 && start % 64 == 0, "rezerva pod modulem: {start}");
    assert!(x.tp as i64 + start >= 0, "rezerva v bloku");
    let a = tls_layout(Machine::Aarch64, &[M4], STATIC_TLS_SURPLUS);
    assert_eq!(a.offsets, [Some(16)]);
    assert!(a.surplus.0 >= 20 && a.surplus.0 % 64 == 0);
    assert!(a.size >= a.surplus.0 as u64 + STATIC_TLS_SURPLUS);
}

#[test]
fn surplus_x86_64_below_modules() {
    let l = tls_layout(Machine::X86_64, &[M4], 256);
    let mut s = Surplus::new(Machine::X86_64, &l, 64);
    let a = s.alloc(4, 4).unwrap();
    // Pod modulem programu (-4) a v rezerve.
    assert!(a < -4 && a >= l.surplus.0);
    let b = s.alloc(16, 16).unwrap();
    assert!(b + 16 <= a && b % 16 == 0);
    assert_eq!(s.used(), (l.surplus.0 + 256 - b) as u64);
    // Zbytek nestaci, zarovnani vetsi nez TP nejde.
    assert_eq!(s.alloc(512, 8), None);
    assert_eq!(s.alloc(4, 128), None);
    // Rezervu jde vycerpat presne.
    let free = 256 - s.used();
    assert!(s.alloc(free, 1).is_some());
    assert_eq!(s.used(), 256);
    assert_eq!(s.alloc(1, 1), None);
}

#[test]
fn surplus_aarch64_above_modules() {
    let l = tls_layout(Machine::Aarch64, &[M4], 128);
    let mut s = Surplus::new(Machine::Aarch64, &l, 4096);
    let a = s.alloc(8, 8).unwrap();
    assert!(a >= 16 + 4 && a == l.surplus.0);
    let b = s.alloc(64, 64).unwrap();
    assert!(b >= a + 8 && b % 64 == 0);
    assert!(b as u64 + 64 <= l.size);
    assert_eq!(s.alloc(128, 1), None);
    assert_eq!(s.used(), (b + 64 - l.surplus.0) as u64);
}

#[test]
fn tls_reserve_bounds() {
    assert_eq!(tls_reserve(None), STATIC_TLS_SURPLUS);
    assert_eq!(tls_reserve(Some(8192)), 8192);
    assert_eq!(tls_reserve(Some(1 << 30)), MAX_STATIC_TLS_RESERVE);
    assert_eq!(tls_reserve(Some(0)), 0);
}

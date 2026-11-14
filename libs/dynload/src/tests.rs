//! Testy logiky zavadece na hostiteli.

use super::*;

const M4: Option<TlsModule> = Some(TlsModule { size: 4, align: 4 });

#[test]
fn tls_x86_64_matches_linker_local_exec() {
    // Program s 4 B (align 4): lld pro local-exec dava TP - 4.
    let l = tls_layout(Machine::X86_64, &[M4]);
    assert_eq!(l.offsets, [Some(-4)]);
    assert_eq!((l.tp, l.size, l.align), (16, 32, 16));
    // Program bez TLS, knihovny 4 B a 24 B s align 8.
    let l = tls_layout(Machine::X86_64, &[None, M4, Some(TlsModule { size: 24, align: 8 })]);
    assert_eq!(l.offsets, [None, Some(-4), Some(-32)]);
    // Vsechna data pod TP a v bloku.
    assert!(l.tp >= 32 && l.tp % 16 == 0);
}

#[test]
fn tls_aarch64_matches_linker_local_exec() {
    // lld: TP + zarovnane(16, align).
    let l = tls_layout(Machine::Aarch64, &[M4]);
    assert_eq!(l.offsets, [Some(16)]);
    assert_eq!(l.tp, 0);
    let l = tls_layout(Machine::Aarch64, &[M4, Some(TlsModule { size: 8, align: 64 })]);
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
    assert_eq!(initfs_path("libdltest.so.1").as_deref(), Some("lib/libdltest.so.1"));
    assert_eq!(initfs_path("/system/lib/libx.so.2").as_deref(), Some("lib/libx.so.2"));
    assert_eq!(initfs_path("/home/a/libx.so"), None);
    assert_eq!(initfs_path("../libx.so"), None);
    assert_eq!(initfs_path(""), None);
}

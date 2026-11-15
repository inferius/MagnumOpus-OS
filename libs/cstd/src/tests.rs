//! Testy logiky cstd na hostiteli (symboly C se tu neexportuji).

use crate::errno::{self, EACCES, EEXIST, EINVAL, ENOENT, ESPIPE};
use crate::stdio::{Mode, parse_mode};
use std::io;

#[test]
fn fopen_modes() {
    assert_eq!(parse_mode(b"r"), Some(Mode { read: true, ..Mode::default() }));
    assert_eq!(parse_mode(b"rb+"), Some(Mode { read: true, write: true, ..Mode::default() }));
    let w = parse_mode(b"w").unwrap();
    assert!(w.write && w.truncate && w.create && !w.read && !w.append);
    let a = parse_mode(b"a+").unwrap();
    assert!(a.append && a.read && a.create && !a.truncate);
    let x = parse_mode(b"wx").unwrap();
    assert!(x.create_new);
}

#[test]
fn fopen_bad_modes() {
    assert_eq!(parse_mode(b""), None);
    assert_eq!(parse_mode(b"q"), None);
    assert_eq!(parse_mode(b"rx"), None);
    assert_eq!(parse_mode(b"rz"), None);
}

#[test]
fn errno_from_io() {
    let k = |k| errno::from_io(&io::Error::from(k));
    assert_eq!(k(io::ErrorKind::NotFound), ENOENT);
    assert_eq!(k(io::ErrorKind::PermissionDenied), EACCES);
    assert_eq!(k(io::ErrorKind::AlreadyExists), EEXIST);
    assert_eq!(k(io::ErrorKind::InvalidInput), EINVAL);
    assert_eq!(k(io::ErrorKind::NotSeekable), ESPIPE);
}

#[test]
fn errno_is_per_thread() {
    errno::set(ENOENT);
    let other = std::thread::spawn(|| {
        assert_eq!(errno::get(), 0);
        errno::set(EINVAL);
        errno::get()
    })
    .join()
    .unwrap();
    assert_eq!(other, EINVAL);
    assert_eq!(errno::get(), ENOENT);
    // SAFETY: ukazatel na errno tohoto vlakna.
    assert_eq!(unsafe { *errno::__errno_location() }, ENOENT);
}

#[test]
fn messages_are_c_strings() {
    for e in [0, ENOENT, EINVAL, 9999] {
        let m = errno::message(e);
        assert_eq!(m.last(), Some(&0));
        assert!(!m[..m.len() - 1].contains(&0));
    }
}

#[test]
fn file_roundtrip_through_c_api() {
    use crate::stdio::*;
    let dir = std::env::temp_dir().join(format!("cstd-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.txt");
    let cpath = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
    // SAFETY: platne retezce C a FILE z fopen.
    unsafe {
        let f = fopen(cpath.as_ptr().cast(), c"w".as_ptr().cast());
        assert!(!f.is_null());
        assert_eq!(fputs(c"radek 1\nradek 2\n".as_ptr().cast(), f), 0);
        assert_eq!(fputc(b'x' as i32, f), b'x' as i32);
        assert_eq!(ftell(f), 17);
        assert_eq!(fclose(f), 0);

        let f = fopen(cpath.as_ptr().cast(), c"r".as_ptr().cast());
        let mut buf = [0u8; 32];
        assert!(!fgets(buf.as_mut_ptr(), 32, f).is_null());
        assert_eq!(&buf[..9], b"radek 1\n\0");
        assert_eq!(fgetc(f), b'r' as i32);
        assert_eq!(ungetc(b'R' as i32, f), b'R' as i32);
        assert_eq!(ftell(f), 8);
        let mut rest = [0u8; 32];
        let n = fread(rest.as_mut_ptr(), 1, 32, f);
        assert_eq!(&rest[..n], b"Radek 2\nx");
        assert_eq!(feof(f), 1);
        rewind(f);
        assert_eq!(feof(f), 0);
        assert_eq!(fgetc(f), b'r' as i32);
        assert_eq!(fclose(f), 0);

        let missing = std::ffi::CString::new(dir.join("nic").to_str().unwrap()).unwrap();
        assert!(fopen(missing.as_ptr().cast(), c"r".as_ptr().cast()).is_null());
        assert_eq!(errno::get(), ENOENT);
        assert_eq!(remove(cpath.as_ptr().cast()), 0);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Cisla v `include/errno.h` = konstanty v `src/errno.rs`.
#[test]
fn errno_header_matches() {
    let header = include_str!("../include/errno.h");
    let rust = include_str!("errno.rs");
    let mut count = 0;
    for line in header.lines() {
        let mut it = line.split_whitespace();
        let (Some("#define"), Some(name), Some(val)) = (it.next(), it.next(), it.next()) else { continue };
        let Ok(n) = val.parse::<i32>() else { continue };
        assert!(rust.contains(&format!("pub const {name}: c_int = {n};")), "{name} = {n} chybi v errno.rs");
        count += 1;
    }
    let consts = rust.lines().filter(|l| l.starts_with("pub const E")).count();
    assert_eq!(count, consts, "errno.h a errno.rs maji ruzny pocet cisel");
}

#[test]
fn langinfo_header_matches() {
    use crate::locale::{langinfo, nl};
    let h = include_str!("../include/langinfo.h");
    for (name, v) in [("CODESET", nl::CODESET), ("D_FMT", nl::D_FMT), ("T_FMT", nl::T_FMT), ("RADIXCHAR", nl::RADIXCHAR), ("DAY_1", nl::DAY_1), ("ABDAY_1", nl::ABDAY_1), ("MON_1", nl::MON_1), ("ABMON_1", nl::ABMON_1), ("CRNCYSTR", nl::CRNCYSTR)] {
        assert!(h.contains(&format!("#define {name} {v}\n")), "{name}");
    }
    assert_eq!(langinfo(nl::CODESET, true), b"UTF-8\0");
    assert_eq!(langinfo(nl::D_FMT, false), b"%m/%d/%y\0");
    assert_eq!(langinfo(nl::DAY_1 + 6, false), b"Saturday\0");
    assert_eq!(langinfo(nl::ABMON_1 + 11, false), b"Dec\0");
    assert_eq!(langinfo(999, false), b"\0");
}

#[test]
fn sched_policy_rules() {
    use crate::pthread::{SCHED_FIFO, SCHED_OTHER, SCHED_RR, sched_set_check};
    assert_eq!(sched_set_check(SCHED_OTHER, Some(0)), 0);
    assert_eq!(sched_set_check(SCHED_FIFO, Some(10)), crate::errno::EPERM);
    assert_eq!(sched_set_check(SCHED_RR, Some(1)), crate::errno::EPERM);
    assert_eq!(sched_set_check(SCHED_OTHER, Some(5)), crate::errno::EINVAL);
    assert_eq!(sched_set_check(42, Some(0)), crate::errno::EINVAL);
    assert_eq!(sched_set_check(SCHED_OTHER, None), crate::errno::EINVAL);
    let h = include_str!("../include/sched.h");
    for (n, v) in [("SCHED_OTHER", SCHED_OTHER), ("SCHED_FIFO", SCHED_FIFO), ("SCHED_RR", SCHED_RR)] {
        assert!(h.contains(&format!("#define {n} {v}\n")), "{n}");
    }
}

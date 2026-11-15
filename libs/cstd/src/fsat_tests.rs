//! Testy `fsat.rs` na hostiteli (logika cest, pathconf, sendfile, hlavicky).

use crate::fsat::*;

#[test]
fn join_at_paths() {
    assert_eq!(join_at(Some(b"/a/b"), b"c").unwrap(), b"/a/b/c");
    assert_eq!(join_at(Some(b"/"), b"c").unwrap(), b"/c");
    assert_eq!(join_at(Some(b"/a/"), b"c/d").unwrap(), b"/a/c/d");
    // absolutni cesta adresar ignoruje
    assert_eq!(join_at(Some(b"/a"), b"/x").unwrap(), b"/x");
    assert_eq!(join_at(None, b"rel").unwrap(), b"rel");
    assert_eq!(join_at(Some(b"/a"), b""), Err(crate::errno::ENOENT));
}

#[test]
fn pathconf_values() {
    assert_eq!(conf(pc::PATH_MAX), Ok(4096));
    assert_eq!(conf(pc::NAME_MAX), Ok(255));
    assert_eq!(conf(99), Err(crate::errno::EINVAL));
}

#[test]
fn headers_match() {
    let fcntl = include_str!("../include/fcntl.h");
    assert!(fcntl.contains(&format!("#define AT_REMOVEDIR {AT_REMOVEDIR:#x}\n")));
    assert!(fcntl.contains(&format!("#define AT_SYMLINK_NOFOLLOW {AT_SYMLINK_NOFOLLOW:#x}\n")));
    let unistd = include_str!("../include/unistd.h");
    for (n, v) in [("LINK_MAX", pc::LINK_MAX), ("NAME_MAX", pc::NAME_MAX), ("PATH_MAX", pc::PATH_MAX), ("PIPE_BUF", pc::PIPE_BUF)] {
        assert!(unistd.contains(&format!("#define _PC_{n} {v}\n")), "{n}");
    }
    let limits = include_str!("../include/limits.h");
    assert!(limits.contains(&format!("#define PATH_MAX {PATH_MAX}\n")));
    assert!(limits.contains(&format!("#define NAME_MAX {NAME_MAX}\n")));
}

#[test]
fn sendfile_copies_with_and_without_offset() {
    use crate::unistd::{close, lseek, open};
    let dir = std::env::temp_dir().join(format!("cstd-sendfile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("a");
    let dst = dir.join("b");
    std::fs::write(&src, b"0123456789").unwrap();
    let c = |p: &std::path::Path| std::ffi::CString::new(p.to_str().unwrap()).unwrap();
    let (cs, cd) = (c(&src), c(&dst));
    unsafe {
        let i = open(cs.as_ptr().cast(), crate::unistd::O_RDONLY, 0);
        let o = open(cd.as_ptr().cast(), crate::unistd::O_WRONLY | crate::unistd::O_CREAT | crate::unistd::O_TRUNC, 0o644);
        assert!(i >= 0 && o >= 0);
        let mut off: i64 = 2;
        assert_eq!(sendfile(o, i, &mut off, 3), 3);
        assert_eq!(off, 5);
        // pozice vstupu se s offsetem nehnula: dalsi kopie od zacatku
        assert_eq!(sendfile(o, i, core::ptr::null_mut(), 100), 10);
        assert_eq!(lseek(i, 0, 1), 10);
        close(i);
        close(o);
    }
    assert_eq!(std::fs::read(&dst).unwrap(), b"2340123456789");
    std::fs::remove_dir_all(&dir).unwrap();
}

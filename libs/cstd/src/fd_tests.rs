//! Testy casti 2a na hostiteli: priznaky `open`, `st_mode`, vypis
//! adresare a pravidla cisel fd (nad nahradni vrstvou `sys::host`).

use crate::dirent::with_dots;
use crate::errno::{EBADF, EINVAL, ESPIPE};
use crate::stat::{S_IFDIR, S_IFREG, mode_of};
use crate::sys::{self, Entry, Kind, Seek, Stat};
use crate::unistd::{O_APPEND, O_CLOEXEC, O_CREAT, O_EXCL, O_RDWR, O_TRUNC, O_WRONLY, open_flags};

#[test]
fn open_flag_decoding() {
    let o = open_flags(O_WRONLY | O_CREAT | O_TRUNC).unwrap();
    assert!(o.write && !o.read && o.create && o.trunc && !o.excl);
    assert_eq!(o.status(), sys::status::WRONLY);
    let o = open_flags(O_RDWR | O_APPEND | O_CLOEXEC | O_EXCL | O_CREAT).unwrap();
    assert!(o.read && o.write && o.append && o.cloexec && o.excl);
    assert_eq!(o.status(), sys::status::RDWR | sys::status::APPEND);
    assert!(open_flags(3).is_none(), "O_ACCMODE 3 je neplatny");
}

#[test]
fn st_mode_from_rights() {
    let mut s = Stat::stream(Kind::File);
    s.readable = true;
    s.writable = false;
    assert_eq!(mode_of(&s), S_IFREG | 0o444);
    s.writable = true;
    assert_eq!(mode_of(&s), S_IFREG | 0o644);
    s.kind = Kind::Dir;
    assert_eq!(mode_of(&s), S_IFDIR | 0o755);
}

#[test]
fn readdir_has_dots_first() {
    let e = |n: &[u8]| Entry { name: n.to_vec(), kind: Kind::File, ino: 7 };
    let list = with_dots(vec![e(b"a"), e(b"."), e(b"b")]);
    let names: Vec<_> = list.iter().map(|x| x.name.clone()).collect();
    assert_eq!(names, vec![b".".to_vec(), b"..".to_vec(), b"a".to_vec(), b"b".to_vec()]);
}

fn temp(name: &str) -> Vec<u8> {
    let p = std::env::temp_dir().join(format!("cstd-fd-{}-{name}", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p.to_str().unwrap().as_bytes().to_vec()
}

/// Cislo fd jen pro tento test (zadny jiny test tak vysoko nepristane).
const HIGH: core::ffi::c_int = 4000;

#[test]
fn fd_numbers_dup_and_shared_offset() {
    let path = temp("dup");
    let fd = sys::open(&path, &open_flags(O_RDWR | O_CREAT).unwrap()).unwrap();
    assert!(fd >= 3, "0-2 jsou stdio");
    assert_eq!(sys::write(fd, b"abcdef").unwrap(), 6);
    // dup sdili pozici (POSIX: stejny popis otevreni).
    let d = sys::dup(fd, 0, false).unwrap();
    assert_ne!(d, fd);
    assert_eq!(sys::seek(d, Seek::Set(2)).unwrap(), 2);
    let mut b = [0u8; 2];
    assert_eq!(sys::read(fd, &mut b).unwrap(), 2);
    assert_eq!(&b, b"cd");
    // dup2 na konkretni cislo, F_DUPFD od minima. Vysoka cisla: testy bezi
    // soubezne nad jednou tabulkou fd a nova cisla se berou od nejnizsiho
    // volneho, takze sem jiny test nesahne.
    assert_eq!(sys::dup2(fd, HIGH, true).unwrap(), HIGH);
    assert!(sys::cloexec(HIGH).unwrap());
    let above = sys::dup(fd, HIGH, false).unwrap();
    assert!(above > HIGH);
    sys::close(above).unwrap();
    // pread nemeni pozici.
    let mut c = [0u8; 3];
    assert_eq!(sys::pread(fd, &mut c, 0).unwrap(), 3);
    assert_eq!(&c, b"abc");
    assert_eq!(sys::seek(fd, Seek::Cur(0)).unwrap(), 4);
    sys::close(fd).unwrap();
    // Duplikat zustava platny i po zavreni puvodniho cisla.
    assert_eq!(sys::seek(d, Seek::End(0)).unwrap(), 6);
    // Zavrene cislo uz neplati. Ne na `fd`: uvolnene nizke cislo muze hned
    // dostat soubezny test (druhe `close` by zavrelo jeho soubor).
    sys::close(HIGH).unwrap();
    assert_eq!(sys::close(HIGH), Err(EBADF));
    assert_eq!(sys::read(HIGH, &mut b), Err(EBADF));
    assert_eq!(sys::read(-1, &mut b), Err(EBADF));
    sys::close(d).unwrap();
    std::fs::remove_file(core::str::from_utf8(&path).unwrap()).unwrap();
}

#[test]
fn pipe_ends_and_access_mode() {
    let (r, w) = sys::pipe(false, false).unwrap();
    assert_eq!(sys::write(w, b"ping").unwrap(), 4);
    let mut b = [0u8; 4];
    assert_eq!(sys::read(r, &mut b).unwrap(), 4);
    assert_eq!(&b, b"ping");
    // Cteci konec nejde psat, roura nema pozici.
    assert_eq!(sys::write(r, b"x"), Err(EBADF));
    assert_eq!(sys::seek(r, Seek::Set(0)), Err(ESPIPE));
    assert_eq!(sys::fstat(r).unwrap().kind, Kind::Pipe);
    sys::close(r).unwrap();
    sys::close(w).unwrap();
    assert_eq!(sys::dup(5000, 0, false).map_err(|e| e == EBADF || e == EINVAL), Err(true));
}

#[test]
fn flock_ranges() {
    use crate::flock::{abs_range, flock};
    let l = |w: i16, s: i64, n: i64| flock { l_type: 1, l_whence: w, l_start: s, l_len: n, l_pid: 0 };
    assert_eq!(abs_range(&l(0, 10, 5), 0, 0), Ok((10, 5)));
    assert_eq!(abs_range(&l(1, -2, 4), 100, 0), Ok((98, 4)), "od aktualni pozice");
    assert_eq!(abs_range(&l(2, 0, 0), 0, 500), Ok((500, 0)), "od konce do nekonecna");
    assert_eq!(abs_range(&l(0, 10, -4), 0, 0), Ok((6, 4)), "zaporna delka = pred startem");
    assert_eq!(abs_range(&l(0, 2, -4), 0, 0), Err(EINVAL));
    assert_eq!(abs_range(&l(9, 0, 0), 0, 0), Err(EINVAL));
}

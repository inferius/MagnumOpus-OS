//! Testy `termios.rs` na hostiteli nad terminalem v pameti (`sys/host.rs`,
//! `ttyproto::Tty` 24x80): rezim, `cfmakeraw`, velikost okna, `ioctl`,
//! `SIGWINCH` a `ENOTTY` mimo terminal. Plus `ffs`.

use core::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use crate::errno::{self, EINVAL, ENOTTY};
use crate::termios::*;

/// Jeden test: terminal v pameti je sdileny (stdin = fd 0).
#[test]
fn terminal_mode_size_and_signals() {
    let mut t = core::mem::MaybeUninit::<termios>::uninit();
    // SAFETY: platne ukazatele; termios vyplni tcgetattr.
    unsafe {
        assert_eq!(tcgetattr(0, t.as_mut_ptr()), 0);
        let mut t = t.assume_init();
        assert_ne!(t.c_lflag & ttyproto::lflag::ICANON, 0, "vychozi kanonicky rezim");
        assert_eq!(t.c_cc[ttyproto::cc::VINTR], 3);
        assert_eq!(cfgetospeed(&t), ttyproto::cflag::B38400);
        // Surovy rezim a zpet.
        let cooked = t;
        cfmakeraw(&mut t);
        assert_eq!(tcsetattr(0, TCSAFLUSH, &t), 0);
        let mut got = cooked;
        assert_eq!(tcgetattr(0, &mut got), 0);
        assert_eq!(got.c_lflag & (ttyproto::lflag::ICANON | ttyproto::lflag::ECHO), 0);
        assert_eq!(got.c_oflag & ttyproto::oflag::OPOST, 0);
        assert_eq!(tcsetattr(0, 9, &t), -1);
        assert_eq!(errno::get(), EINVAL);
        // Pres ioctl (TCGETS / TCSETS).
        assert_eq!(crate::misc::ioctl(0, TCSETS, (&raw const cooked).cast_mut().cast::<c_void>()), 0);
        let mut via = t;
        assert_eq!(crate::misc::ioctl(0, TCGETS, (&raw mut via).cast::<c_void>()), 0);
        assert_ne!(via.c_lflag & ttyproto::lflag::ICANON, 0);
        assert_eq!(cfsetspeed(&mut via, ttyproto::cflag::B38400), 0);
        assert_eq!(cfsetispeed(&mut via, 0o777777), -1);
        assert_eq!(tcflush(0, TCIOFLUSH), 0);
        assert_eq!(tcdrain(1), 0);
        assert_eq!(tcflush(0, 7), -1);
        assert_eq!(tcgetpgrp(0), crate::signal::getpid());

        // Velikost okna a SIGWINCH (obsluha bezi ve vlakne hlidace).
        static SEEN: AtomicU32 = AtomicU32::new(0);
        extern "C" fn on_winch(_: crate::c_int) {
            SEEN.fetch_add(1, Ordering::SeqCst);
        }
        let _ = crate::signal::signal(crate::signal::SIGWINCH, on_winch as extern "C" fn(crate::c_int) as usize);
        let mut w = winsize::default();
        assert_eq!(crate::misc::ioctl(1, TIOCGWINSZ, (&raw mut w).cast::<c_void>()), 0);
        assert_eq!((w.ws_row, w.ws_col), (24, 80));
        let big = winsize { ws_row: 50, ws_col: 132, ws_xpixel: 0, ws_ypixel: 0 };
        assert_eq!(crate::misc::ioctl(1, TIOCSWINSZ, (&raw const big).cast_mut().cast::<c_void>()), 0);
        assert_eq!(crate::misc::ioctl(2, TIOCGWINSZ, (&raw mut w).cast::<c_void>()), 0);
        assert_eq!((w.ws_row, w.ws_col), (50, 132));
        let start = Instant::now();
        while SEEN.load(Ordering::SeqCst) == 0 && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(SEEN.load(Ordering::SeqCst), 1, "SIGWINCH po zmene velikosti");
        assert_eq!(crate::misc::ioctl(1, TIOCGWINSZ, core::ptr::null_mut::<c_void>()), -1);
        assert_eq!(errno::get(), crate::errno::EFAULT);
    }
}

#[test]
fn not_a_terminal() {
    let mut t = core::mem::MaybeUninit::<termios>::uninit();
    // SAFETY: platny ukazatel; fd 3999 neni otevreny.
    unsafe {
        assert_eq!(tcgetattr(3999, t.as_mut_ptr()), -1);
        assert_eq!(errno::get(), crate::errno::EBADF);
    }
    // Soubor neni terminal.
    let p = std::env::temp_dir().join(format!("cstd-tty-{}.txt", std::process::id()));
    let c = std::ffi::CString::new(p.to_string_lossy().as_bytes()).unwrap();
    // SAFETY: retezec C.
    let fd = unsafe { crate::unistd::open(c.as_ptr().cast(), crate::unistd::O_RDWR | crate::unistd::O_CREAT, 0o644) };
    assert!(fd >= 0);
    // SAFETY: platny ukazatel.
    unsafe {
        assert_eq!(tcgetattr(fd, t.as_mut_ptr()), -1);
        assert_eq!(errno::get(), ENOTTY);
        assert_eq!(crate::misc::ioctl(fd, TIOCGWINSZ, t.as_mut_ptr().cast::<c_void>()), -1);
        assert_eq!(errno::get(), ENOTTY);
    }
    assert_eq!(crate::unistd::close(fd), 0);
    let _ = std::fs::remove_file(p);
    assert_eq!(size_of::<termios>(), 60);
    let h = include_str!("../include/termios.h");
    for (n, v) in [("ICANON", ttyproto::lflag::ICANON), ("ECHO", ttyproto::lflag::ECHO), ("ICRNL", ttyproto::iflag::ICRNL), ("OPOST", ttyproto::oflag::OPOST), ("CS8", ttyproto::cflag::CS8)] {
        assert!(h.contains(&format!("#define {n} {v:07o}\n")), "{n}");
    }
}

#[test]
fn ffs_family() {
    use crate::misc::{ffs, ffsl, ffsll};
    assert_eq!(ffs(0), 0);
    assert_eq!(ffs(1), 1);
    assert_eq!(ffs(0x18), 4);
    assert_eq!(ffs(i32::MIN), 32);
    assert_eq!(ffsl(1 << 20), 21);
    assert_eq!(ffsll(1 << 40), 41);
    assert_eq!(ffsll(i64::MIN), 64);
}

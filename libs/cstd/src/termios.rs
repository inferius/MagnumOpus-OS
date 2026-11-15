//! `termios.h` nad ridicim kanalem terminalu (`ttyproto`, docs/cs/cstd.md,
//! kapitola termios). Terminal ma jen proces, ktery ho dostal ve startovni
//! zprave (stitek `TTY`), a jen pro fd 0-2 se stdio; jinak `ENOTTY`.
//!
//! Radkovou disciplinu dela terminal (editor radku, echo, Ctrl+C), cstd
//! jen predava rezim (`struct termios` jako glibc, 60 B). Zmena velikosti
//! okna = `SIGWINCH` z vlakna, ktere ceka na udalost terminalu (spusti ho
//! prvni `TIOCGWINSZ`/`tcgetattr`; obsluha bezi v tom vlakne jako u
//! casovace, rozhodnuti 6A).

use std::sync::atomic::{AtomicBool, Ordering};

use ttyproto::{Body, Mode, Request, Size};

use crate::{c_int, errno, sys};

pub const NCCS: usize = ttyproto::cc::NCCS;

/// `struct termios` (glibc).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct termios {
    pub c_iflag: u32,
    pub c_oflag: u32,
    pub c_cflag: u32,
    pub c_lflag: u32,
    pub c_line: u8,
    pub c_cc: [u8; NCCS],
    pub c_ispeed: u32,
    pub c_ospeed: u32,
}

/// `struct winsize`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct winsize {
    pub ws_row: u16,
    pub ws_col: u16,
    pub ws_xpixel: u16,
    pub ws_ypixel: u16,
}

pub const TCSANOW: c_int = 0;
pub const TCSADRAIN: c_int = 1;
pub const TCSAFLUSH: c_int = 2;
pub const TCIFLUSH: c_int = 0;
pub const TCOFLUSH: c_int = 1;
pub const TCIOFLUSH: c_int = 2;
pub const TCOOFF: c_int = 0;
pub const TCOON: c_int = 1;
pub const TCIOFF: c_int = 2;
pub const TCION: c_int = 3;

/// `ioctl` terminalu (cisla jako Linux).
pub const TCGETS: u64 = 0x5401;
pub const TCSETS: u64 = 0x5402;
pub const TCSETSW: u64 = 0x5403;
pub const TCSETSF: u64 = 0x5404;
pub const TCFLSH: u64 = 0x540B;
pub const TIOCGPGRP: u64 = 0x540F;
pub const TIOCSPGRP: u64 = 0x5410;
pub const TIOCGWINSZ: u64 = 0x5413;
pub const TIOCSWINSZ: u64 = 0x5414;

impl From<Mode> for termios {
    fn from(m: Mode) -> Self {
        termios {
            c_iflag: m.iflag,
            c_oflag: m.oflag,
            c_cflag: m.cflag,
            c_lflag: m.lflag,
            c_line: m.line,
            c_cc: m.cc,
            c_ispeed: m.ispeed,
            c_ospeed: m.ospeed,
        }
    }
}

impl From<termios> for Mode {
    fn from(t: termios) -> Self {
        Mode {
            iflag: t.c_iflag,
            oflag: t.c_oflag,
            cflag: t.c_cflag,
            lflag: t.c_lflag,
            line: t.c_line,
            cc: t.c_cc,
            ispeed: t.c_ispeed,
            ospeed: t.c_ospeed,
        }
    }
}

/// Je `fd` terminal? (stdio a proces ma ridici kanal terminalu.)
pub fn is_tty(fd: c_int) -> Result<(), c_int> {
    match sys::isatty(fd) {
        Ok(true) if sys::tty_available() => Ok(()),
        Ok(_) => Err(errno::ENOTTY),
        Err(e) => Err(e),
    }
}

/// Pozadavek na terminal za `fd`.
fn call(fd: c_int, req: Request) -> Result<Body, c_int> {
    is_tty(fd)?;
    let (data, handles) = sys::tty_call(&req.encode())?;
    sys::close_handles(handles);
    ttyproto::decode_reply(&req, &data).map_err(sys::abi_errno)
}

fn get_mode(fd: c_int) -> Result<Mode, c_int> {
    match call(fd, Request::GetMode)? {
        Body::Mode(m) => Ok(m),
        _ => Err(errno::EIO),
    }
}

/// `tcsetattr`: `act` = `TCSANOW`/`TCSADRAIN`/`TCSAFLUSH`. Uspech, kdyz
/// terminal provedl aspon cast zmen (POSIX); nepodporovane priznaky
/// terminal vynuluje (zjisti je nasledne `tcgetattr`).
pub fn set_mode(fd: c_int, act: c_int, t: &termios) -> Result<(), c_int> {
    let when = match act {
        TCSANOW => ttyproto::when::NOW,
        TCSADRAIN => ttyproto::when::DRAIN,
        TCSAFLUSH => ttyproto::when::FLUSH,
        _ => return Err(errno::EINVAL),
    };
    call(fd, Request::SetMode { when, mode: (*t).into() }).map(|_| ())
}

pub fn get_size(fd: c_int) -> Result<winsize, c_int> {
    match call(fd, Request::GetSize)? {
        Body::Size(s) => {
            watch_resize();
            Ok(winsize { ws_row: s.rows, ws_col: s.cols, ws_xpixel: s.xpixel, ws_ypixel: s.ypixel })
        }
        _ => Err(errno::EIO),
    }
}

pub fn set_size(fd: c_int, w: &winsize) -> Result<(), c_int> {
    let s = Size { rows: w.ws_row, cols: w.ws_col, xpixel: w.ws_xpixel, ypixel: w.ws_ypixel };
    call(fd, Request::SetSize(s)).map(|_| ())
}

static WATCHING: AtomicBool = AtomicBool::new(false);

/// Spusti (jednou) vlakno, ktere pri zmene velikosti okna posle `SIGWINCH`.
fn watch_resize() {
    if WATCHING.swap(true, Ordering::AcqRel) {
        return;
    }
    let Ok((data, mut handles)) = sys::tty_call(&Request::WatchSize.encode()) else { return };
    if ttyproto::decode_reply(&Request::WatchSize, &data).is_err() || handles.len() != 1 {
        sys::close_handles(handles);
        return;
    }
    let ev = handles.remove(0);
    let spawned = std::thread::Builder::new().name("sigwinch".into()).stack_size(128 * 1024).spawn(move || {
        while sys::tty_wait_resize(ev).is_ok() {
            crate::signal::raise(crate::signal::SIGWINCH);
        }
    });
    if spawned.is_err() {
        WATCHING.store(false, Ordering::Release);
    }
}

// --- funkce C -----------------------------------------------------------------

/// # Safety
///
/// `t` je platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn tcgetattr(fd: c_int, t: *mut termios) -> c_int {
    if t.is_null() {
        return errno::ret(Err(errno::EFAULT));
    }
    match get_mode(fd) {
        Ok(m) => {
            // SAFETY: kontrakt C.
            unsafe { t.write(m.into()) };
            watch_resize();
            0
        }
        Err(e) => errno::ret(Err(e)),
    }
}

/// # Safety
///
/// `t` je platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn tcsetattr(fd: c_int, act: c_int, t: *const termios) -> c_int {
    // SAFETY: kontrakt C.
    let Some(t) = (unsafe { t.as_ref() }) else { return errno::ret(Err(errno::EFAULT)) };
    errno::ret(set_mode(fd, act, t))
}

/// # Safety
///
/// `t` je platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn cfmakeraw(t: *mut termios) {
    // SAFETY: kontrakt C.
    if let Some(t) = unsafe { t.as_mut() } {
        let mut m: Mode = (*t).into();
        m.make_raw();
        *t = m.into();
    }
}

/// Rychlost linky (`B*`): terminal je virtualni, jen se uchova.
///
/// # Safety
///
/// `t` je platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn cfgetispeed(t: *const termios) -> u32 {
    // SAFETY: kontrakt C.
    unsafe { (*t).c_ispeed }
}

/// # Safety
///
/// `t` je platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn cfgetospeed(t: *const termios) -> u32 {
    // SAFETY: kontrakt C.
    unsafe { (*t).c_ospeed }
}

fn valid_speed(s: u32) -> bool {
    s & !ttyproto::cflag::CBAUD == 0
}

/// # Safety
///
/// `t` je platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn cfsetispeed(t: *mut termios, s: u32) -> c_int {
    if !valid_speed(s) {
        return errno::ret(Err(errno::EINVAL));
    }
    // SAFETY: kontrakt C.
    unsafe { (*t).c_ispeed = s };
    0
}

/// # Safety
///
/// `t` je platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn cfsetospeed(t: *mut termios, s: u32) -> c_int {
    if !valid_speed(s) {
        return errno::ret(Err(errno::EINVAL));
    }
    // SAFETY: kontrakt C.
    unsafe {
        (*t).c_ospeed = s;
        (*t).c_cflag = ((*t).c_cflag & !ttyproto::cflag::CBAUD) | s;
    }
    0
}

/// # Safety
///
/// `t` je platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn cfsetspeed(t: *mut termios, s: u32) -> c_int {
    // SAFETY: kontrakt C.
    unsafe {
        if cfsetospeed(t, s) != 0 {
            return -1;
        }
        cfsetispeed(t, s)
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn tcflush(fd: c_int, q: c_int) -> c_int {
    let q = match q {
        TCIFLUSH => ttyproto::queue::INPUT,
        TCOFLUSH => ttyproto::queue::OUTPUT,
        TCIOFLUSH => ttyproto::queue::BOTH,
        _ => return errno::ret(Err(errno::EINVAL)),
    };
    errno::ret(call(fd, Request::Flush(q)).map(|_| ()))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn tcdrain(fd: c_int) -> c_int {
    errno::ret(call(fd, Request::Drain).map(|_| ()))
}

/// `tcflow`: rizeni toku virtualni linka nema; terminal se jen overi.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn tcflow(fd: c_int, action: c_int) -> c_int {
    if !(TCOOFF..=TCION).contains(&action) {
        return errno::ret(Err(errno::EINVAL));
    }
    errno::ret(is_tty(fd))
}

/// `tcsendbreak`: virtualni linka break nema; terminal se jen overi.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn tcsendbreak(fd: c_int, _duration: c_int) -> c_int {
    errno::ret(is_tty(fd))
}

/// Skupina v popredi terminalu = vlastni skupina procesu (skupiny nejsou).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn tcgetpgrp(fd: c_int) -> c_int {
    match is_tty(fd) {
        Ok(()) => crate::signal::getpid(),
        Err(e) => errno::ret(Err(e)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn tcsetpgrp(fd: c_int, pgrp: c_int) -> c_int {
    if let Err(e) = is_tty(fd) {
        return errno::ret(Err(e));
    }
    if pgrp == crate::signal::getpid() { 0 } else { errno::ret(Err(errno::EPERM)) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn tcgetsid(fd: c_int) -> c_int {
    tcgetpgrp(fd)
}

/// `ioctl` terminalu; `None` = jiny pozadavek.
///
/// # Safety
///
/// `p` ukazuje na strukturu podle `req` (`termios`, `winsize`, `int`).
pub unsafe fn ioctl(fd: c_int, req: u64, p: *mut core::ffi::c_void) -> Option<Result<(), c_int>> {
    let null = p.is_null();
    Some(match req {
        TCGETS if !null => get_mode(fd).map(|m| {
            // SAFETY: kontrakt C (termios).
            unsafe { p.cast::<termios>().write_unaligned(m.into()) }
        }),
        TCSETS | TCSETSW | TCSETSF if !null => {
            let act = (req - TCSETS) as c_int;
            // SAFETY: kontrakt C (termios).
            set_mode(fd, act, &unsafe { p.cast::<termios>().read_unaligned() })
        }
        TIOCGWINSZ if !null => get_size(fd).map(|w| {
            // SAFETY: kontrakt C (winsize).
            unsafe { p.cast::<winsize>().write_unaligned(w) }
        }),
        // SAFETY: kontrakt C (winsize).
        TIOCSWINSZ if !null => set_size(fd, &unsafe { p.cast::<winsize>().read_unaligned() }),
        TIOCGPGRP if !null => is_tty(fd).map(|()| {
            // SAFETY: kontrakt C (pid_t).
            unsafe { p.cast::<c_int>().write_unaligned(crate::signal::getpid()) }
        }),
        TCFLSH => {
            let q = p as usize as c_int;
            if tcflush(fd, q) == 0 { Ok(()) } else { Err(errno::get()) }
        }
        TCGETS | TCSETS | TCSETSW | TCSETSF | TIOCGWINSZ | TIOCSWINSZ | TIOCGPGRP => Err(errno::EFAULT),
        TIOCSPGRP => is_tty(fd),
        _ => return None,
    })
}

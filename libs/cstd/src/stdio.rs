//! `stdio.h`: proudy `FILE` nad cisly fd (`sys`): stdin/stdout/stderr jsou
//! fd 0, 1, 2, soubory z `fopen` dostanou vlastni cislo (`fileno`,
//! `fdopen`), formatovany vystup a vstup po znacich a radcich.
//!
//! Buffer: stdout po radcich, stderr bez bufferu, soubory 4 KiB. Pri
//! ukonceni procesu (`exit`, navrat z `main`) se vse vyprazdni (osrt
//! `at_exit`). Kazdy `FILE` ma vlastni zamek (vlakna smi pouzivat stejny
//! proud).

use core::ffi::VaList;
use std::sync::{Mutex, MutexGuard};

use cstd_core::format::{self, Out, VaArgs};

use crate::sys::{self, R, Seek};
use crate::errno;
use crate::{c_bytes, c_char, c_int, c_long, size_t};

pub const EOF: c_int = -1;
pub const SEEK_SET: c_int = 0;
pub const SEEK_CUR: c_int = 1;
pub const SEEK_END: c_int = 2;
pub const IOFBF: c_int = 0;
pub const IOLBF: c_int = 1;
pub const IONBF: c_int = 2;
const BUF: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Buffering {
    None,
    Line,
    Full,
}

struct Stream {
    /// Cislo fd; -1 = zavreny proud.
    fd: c_int,
    buffering: Buffering,
    wbuf: Vec<u8>,
    rbuf: Vec<u8>,
    rpos: usize,
    eof: bool,
    err: bool,
    readable: bool,
    writable: bool,
    /// Orientace (`fwide`): 0 = zatim zadna, < 0 bajtova, > 0 siroka.
    orient: i8,
    /// Rozpracovany vicebajtovy znak sirokeho cteni (`fgetwc`).
    wstate: cstd_core::mb::MbState,
}

/// Proud C (`FILE *`).
pub struct FILE {
    s: Mutex<Stream>,
}

impl FILE {
    const fn new(fd: c_int, buffering: Buffering, readable: bool, writable: bool) -> Self {
        FILE {
            s: Mutex::new(Stream {
                fd,
                buffering,
                wbuf: Vec::new(),
                rbuf: Vec::new(),
                rpos: 0,
                eof: false,
                err: false,
                readable,
                writable,
                orient: 0,
                wstate: cstd_core::mb::MbState::INITIAL,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Stream> {
        self.s.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Stream {
    fn write_raw(&mut self, mut data: &[u8]) -> R<()> {
        if self.fd < 0 {
            return Err(errno::EBADF);
        }
        while !data.is_empty() {
            let n = sys::write(self.fd, data)?;
            if n == 0 {
                return Err(errno::EIO);
            }
            data = &data[n..];
        }
        Ok(())
    }

    fn flush(&mut self) -> R<()> {
        if self.wbuf.is_empty() {
            return Ok(());
        }
        let data = core::mem::take(&mut self.wbuf);
        let r = self.write_raw(&data);
        if r.is_err() {
            self.err = true;
        }
        r
    }

    /// Zahodi prectena, ale nespotrebovana data (pred zapisem a posunem);
    /// u souboru vrati pozici zpet (proud bez pozice je jen zahodi).
    fn drop_read(&mut self) -> R<()> {
        let unread = self.rbuf.len() - self.rpos;
        self.rbuf.clear();
        self.rpos = 0;
        if unread > 0 {
            match sys::seek(self.fd, Seek::Cur(-(unread as i64))) {
                Ok(_) | Err(errno::ESPIPE) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn write(&mut self, data: &[u8]) -> R<()> {
        if !self.writable {
            self.err = true;
            return Err(errno::EBADF);
        }
        self.drop_read()?;
        match self.buffering {
            Buffering::None => {
                self.flush()?;
                let r = self.write_raw(data);
                if r.is_err() {
                    self.err = true;
                }
                r
            }
            Buffering::Line => {
                self.wbuf.extend_from_slice(data);
                if data.contains(&b'\n') { self.flush() } else { Ok(()) }
            }
            Buffering::Full => {
                self.wbuf.extend_from_slice(data);
                if self.wbuf.len() >= BUF { self.flush() } else { Ok(()) }
            }
        }
    }

    /// Doplni buffer cteni; `Ok(0)` = konec.
    fn fill(&mut self) -> R<usize> {
        if !self.readable || self.fd < 0 {
            return Err(errno::EBADF);
        }
        self.flush()?;
        // Pred ctenim stdin vyprazdnit stdout (vyzva bez konce radku).
        if self.fd == 0 {
            flush_std_out();
        }
        let mut buf = vec![0u8; BUF];
        let n = sys::read(self.fd, &mut buf)?;
        buf.truncate(n);
        self.rbuf = buf;
        self.rpos = 0;
        Ok(n)
    }

    fn getc(&mut self) -> Option<u8> {
        if self.rpos >= self.rbuf.len() {
            match self.fill() {
                Ok(0) => {
                    self.eof = true;
                    return None;
                }
                Ok(_) => {}
                Err(e) => {
                    self.err = true;
                    errno::set(e);
                    return None;
                }
            }
        }
        let c = self.rbuf[self.rpos];
        self.rpos += 1;
        Some(c)
    }

    fn read(&mut self, out: &mut [u8]) -> usize {
        let mut done = 0;
        while done < out.len() {
            if self.rpos >= self.rbuf.len() {
                match self.fill() {
                    Ok(0) => {
                        self.eof = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        self.err = true;
                        errno::set(e);
                        break;
                    }
                }
            }
            let n = (out.len() - done).min(self.rbuf.len() - self.rpos);
            out[done..done + n].copy_from_slice(&self.rbuf[self.rpos..self.rpos + n]);
            self.rpos += n;
            done += n;
        }
        done
    }

    fn seek(&mut self, pos: Seek) -> R<u64> {
        self.flush()?;
        self.drop_read()?;
        self.eof = false;
        self.wstate = cstd_core::mb::MbState::INITIAL;
        sys::seek(self.fd, pos)
    }

    fn tell(&mut self) -> R<u64> {
        let p = sys::seek(self.fd, Seek::Cur(0))?;
        let p = p as i64 - (self.rbuf.len() - self.rpos) as i64 + self.wbuf.len() as i64;
        u64::try_from(p).map_err(|_| errno::EINVAL)
    }
}

/// Zamceny proud pro siroky vstup a vystup (`wchar::wio`).
pub(crate) struct Locked<'a>(MutexGuard<'a, Stream>);

impl Locked<'_> {
    /// Dalsi bajt (`None` = konec nebo chyba, priznaky nastavene).
    pub(crate) fn getc(&mut self) -> Option<u8> {
        self.0.getc()
    }

    /// Vrati bajty zpet pred dalsi cteni (`ungetwc`).
    pub(crate) fn unget(&mut self, bytes: &[u8]) {
        let s = &mut *self.0;
        if s.rpos >= bytes.len() {
            s.rpos -= bytes.len();
            let at = s.rpos;
            s.rbuf[at..at + bytes.len()].copy_from_slice(bytes);
        } else {
            let rest = s.rbuf.split_off(s.rpos);
            s.rbuf.clear();
            s.rbuf.extend_from_slice(bytes);
            s.rbuf.extend_from_slice(&rest);
            s.rpos = 0;
        }
        s.eof = false;
    }

    pub(crate) fn write(&mut self, data: &[u8]) -> R<()> {
        self.0.write(data)
    }

    /// `fwide`: kladny `mode` = siroka, zaporny = bajtova, 0 = jen dotaz.
    /// Orientace se nastavi jen jednou.
    pub(crate) fn orient(&mut self, mode: c_int) -> c_int {
        if self.0.orient == 0 && mode != 0 {
            self.0.orient = if mode > 0 { 1 } else { -1 };
        }
        self.0.orient as c_int
    }

    pub(crate) fn wstate(&mut self) -> &mut cstd_core::mb::MbState {
        &mut self.0.wstate
    }

    pub(crate) fn set_error(&mut self) {
        self.0.err = true;
    }
}

/// Zamkne proud (`None` = nulovy ukazatel).
///
/// # Safety
///
/// `f` je nulovy nebo platny `FILE *`.
pub(crate) unsafe fn lock_file<'a>(f: *mut FILE) -> Option<Locked<'a>> {
    // SAFETY: kontrakt funkce.
    unsafe { file(f) }.map(|f| {
        ensure_exit_hook();
        Locked(f.lock())
    })
}

/// `stdout` jako `FILE *`.
pub fn stdout_ptr() -> *mut FILE {
    &STDOUT as *const FILE as *mut FILE
}

static STDIN: FILE = FILE::new(0, Buffering::Full, true, false);
static STDOUT: FILE = FILE::new(1, Buffering::Line, false, true);
static STDERR: FILE = FILE::new(2, Buffering::None, false, true);

/// `stdin` jako `FILE *` (pro `scanf`).
pub fn stdin_ptr() -> *mut FILE {
    &STDIN as *const FILE as *mut FILE
}

/// Ukazatel na `FILE` jako globalni promenna C (`extern FILE *stdout`).
#[repr(transparent)]
pub struct FilePtr(*const FILE);
// SAFETY: ukazuje na statiku se zamkem.
unsafe impl Sync for FilePtr {}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub static stdin: FilePtr = FilePtr(&STDIN);
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub static stdout: FilePtr = FilePtr(&STDOUT);
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub static stderr: FilePtr = FilePtr(&STDERR);

/// Otevrene soubory (`fopen`) pro `fflush(NULL)` a vyprazdneni pri konci.
static OPEN: Mutex<Vec<usize>> = Mutex::new(Vec::new());

fn flush_std_out() {
    let _ = STDOUT.lock().flush();
}

/// Vyprazdni vsechny proudy (`fflush(NULL)`, konec procesu).
pub fn flush_all() {
    flush_std_out();
    let _ = STDERR.lock().flush();
    let open = OPEN.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for p in open {
        // SAFETY: v seznamu jsou jen otevrene FILE (odebira je fclose).
        let _ = unsafe { &*(p as *const FILE) }.lock().flush();
    }
}

extern "C" fn flush_at_exit() {
    flush_all();
}

/// Zaregistruje vyprazdneni pri ukonceni procesu (jednou).
pub(crate) fn ensure_exit_hook() {
    #[cfg(target_os = "magnumopus")]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            std::os::magnumopus::rt::at_exit(flush_at_exit);
        });
    }
    #[cfg(not(target_os = "magnumopus"))]
    let _ = flush_at_exit;
}

/// Rezim `fopen` (`r`, `w`, `a`, `+`, `b`, `x`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mode {
    pub read: bool,
    pub write: bool,
    pub append: bool,
    pub truncate: bool,
    pub create: bool,
    pub create_new: bool,
}

pub fn parse_mode(m: &[u8]) -> Option<Mode> {
    let (first, rest) = m.split_first()?;
    let plus = rest.contains(&b'+');
    let excl = rest.contains(&b'x');
    if rest.iter().any(|c| !matches!(c, b'+' | b'b' | b'x' | b't' | b'e')) {
        return None;
    }
    let mut o = match first {
        b'r' => Mode { read: true, write: plus, ..Mode::default() },
        b'w' => Mode { write: true, read: plus, truncate: true, create: true, ..Mode::default() },
        b'a' => Mode { write: true, read: plus, append: true, create: true, ..Mode::default() },
        _ => return None,
    };
    if excl {
        if *first == b'r' {
            return None;
        }
        o.create_new = true;
    }
    Some(o)
}

/// # Safety
///
/// `f` je platny `FILE *` (nebo NULL).
unsafe fn file<'a>(f: *mut FILE) -> Option<&'a FILE> {
    // SAFETY: predano volajicim.
    unsafe { f.as_ref() }
}

impl Mode {
    /// Priznaky `open` pro rezim `fopen`.
    pub fn open_flags(&self) -> sys::OpenFlags {
        sys::OpenFlags {
            read: self.read,
            write: self.write,
            append: self.append,
            create: self.create,
            excl: self.create_new,
            trunc: self.truncate,
            ..sys::OpenFlags::default()
        }
    }
}

/// Novy proud nad cislem fd (zaregistruje ho pro vyprazdneni).
fn new_stream(fd: c_int, read: bool, write: bool) -> *mut FILE {
    ensure_exit_hook();
    // Terminal (stdio) po radcich, jinak plny buffer.
    let buffering = if sys::isatty(fd).unwrap_or(false) { Buffering::Line } else { Buffering::Full };
    let ptr = Box::into_raw(Box::new(FILE::new(fd, buffering, read, write)));
    OPEN.lock().unwrap_or_else(|e| e.into_inner()).push(ptr as usize);
    ptr
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fopen(path: *const c_char, mode: *const c_char) -> *mut FILE {
    // SAFETY: retezce C.
    let (p, m) = unsafe { (c_bytes(path), c_bytes(mode)) };
    let Some(mode) = parse_mode(m) else {
        errno::set(errno::EINVAL);
        return core::ptr::null_mut();
    };
    match sys::open(p, &mode.open_flags()) {
        Ok(fd) => new_stream(fd, mode.read, mode.write),
        Err(e) => {
            errno::set(e);
            core::ptr::null_mut()
        }
    }
}

/// Proud nad existujicim cislem fd (`fdopen`); rezim musi sedet s pravy fd.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fdopen(fd: c_int, mode: *const c_char) -> *mut FILE {
    // SAFETY: retezec C.
    let Some(mode) = parse_mode(unsafe { c_bytes(mode) }) else {
        errno::set(errno::EINVAL);
        return core::ptr::null_mut();
    };
    let st = match sys::status(fd) {
        Ok(s) => s & sys::status::ACCMODE,
        Err(e) => {
            errno::set(e);
            return core::ptr::null_mut();
        }
    };
    let ok_read = !mode.read || st != sys::status::WRONLY;
    let ok_write = !mode.write || st != sys::status::RDONLY;
    if !ok_read || !ok_write {
        errno::set(errno::EINVAL);
        return core::ptr::null_mut();
    }
    new_stream(fd, mode.read, mode.write)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fileno(f: *mut FILE) -> c_int {
    // SAFETY: kontrakt C.
    let Some(file) = (unsafe { file(f) }) else {
        errno::set(errno::EBADF);
        return -1;
    };
    let fd = file.lock().fd;
    if fd < 0 {
        errno::set(errno::EBADF);
    }
    fd
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fclose(f: *mut FILE) -> c_int {
    // SAFETY: kontrakt C.
    let Some(file) = (unsafe { file(f) }) else { return EOF };
    let r = {
        let mut s = file.lock();
        let r = s.flush();
        let fd = core::mem::replace(&mut s.fd, -1);
        let c = if fd >= 0 { sys::close(fd) } else { Ok(()) };
        r.and(c)
    };
    let mut open = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = open.iter().position(|p| *p == f as usize) {
        open.remove(i);
        drop(open);
        // SAFETY: `f` vytvoril `fopen`/`fdopen` (Box), uz neni v seznamu.
        drop(unsafe { Box::from_raw(f) });
    }
    match r {
        Ok(()) => 0,
        Err(e) => {
            errno::set(e);
            EOF
        }
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fflush(f: *mut FILE) -> c_int {
    // SAFETY: kontrakt C.
    match unsafe { file(f) } {
        None => {
            flush_all();
            0
        }
        Some(file) => match file.lock().flush() {
            Ok(()) => 0,
            Err(e) => {
                errno::set(e);
                EOF
            }
        },
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fwrite(p: *const u8, size: size_t, n: size_t, f: *mut FILE) -> size_t {
    // SAFETY: kontrakt C.
    let Some(file) = (unsafe { file(f) }) else { return 0 };
    let Some(total) = size.checked_mul(n) else { return 0 };
    if total == 0 {
        return 0;
    }
    ensure_exit_hook();
    // SAFETY: `p` ma `size * n` bajtu (kontrakt C).
    let data = unsafe { core::slice::from_raw_parts(p, total) };
    match file.lock().write(data) {
        Ok(()) => n,
        Err(e) => {
            errno::set(e);
            0
        }
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fread(p: *mut u8, size: size_t, n: size_t, f: *mut FILE) -> size_t {
    // SAFETY: kontrakt C.
    let Some(file) = (unsafe { file(f) }) else { return 0 };
    let Some(total) = size.checked_mul(n) else { return 0 };
    if total == 0 {
        return 0;
    }
    // SAFETY: `p` ma misto pro `size * n` bajtu.
    let out = unsafe { core::slice::from_raw_parts_mut(p, total) };
    file.lock().read(out) / size
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fputc(c: c_int, f: *mut FILE) -> c_int {
    // SAFETY: kontrakt C.
    let Some(file) = (unsafe { file(f) }) else { return EOF };
    ensure_exit_hook();
    match file.lock().write(&[c as u8]) {
        Ok(()) => c & 0xFF,
        Err(_) => EOF,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn putc(c: c_int, f: *mut FILE) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { fputc(c, f) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn putchar(c: c_int) -> c_int {
    // SAFETY: stdout je platny FILE.
    unsafe { fputc(c, stdout.0 as *mut FILE) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fputs(s: *const c_char, f: *mut FILE) -> c_int {
    // SAFETY: kontrakt C.
    let (Some(file), b) = (unsafe { file(f) }, unsafe { c_bytes(s) }) else { return EOF };
    ensure_exit_hook();
    match file.lock().write(b) {
        Ok(()) => 0,
        Err(_) => EOF,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn puts(s: *const c_char) -> c_int {
    ensure_exit_hook();
    // SAFETY: kontrakt C.
    let b = unsafe { c_bytes(s) };
    let mut out = STDOUT.lock();
    match out.write(b).and_then(|_| out.write(b"\n")) {
        Ok(()) => 0,
        Err(_) => EOF,
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fgetc(f: *mut FILE) -> c_int {
    // SAFETY: kontrakt C.
    let Some(file) = (unsafe { file(f) }) else { return EOF };
    file.lock().getc().map_or(EOF, c_int::from)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn getc(f: *mut FILE) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { fgetc(f) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn getchar() -> c_int {
    STDIN.lock().getc().map_or(EOF, c_int::from)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn ungetc(c: c_int, f: *mut FILE) -> c_int {
    // SAFETY: kontrakt C.
    let Some(file) = (unsafe { file(f) }) else { return EOF };
    if c == EOF {
        return EOF;
    }
    let mut s = file.lock();
    if s.rpos > 0 {
        s.rpos -= 1;
        let at = s.rpos;
        s.rbuf[at] = c as u8;
    } else {
        s.rbuf.insert(0, c as u8);
    }
    s.eof = false;
    c & 0xFF
}

/// Precte radek vcetne `\n`, nejvys `n - 1` znaku, a zakonci nulou.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fgets(buf: *mut c_char, n: c_int, f: *mut FILE) -> *mut c_char {
    // SAFETY: kontrakt C.
    let Some(file) = (unsafe { file(f) }) else { return core::ptr::null_mut() };
    if n <= 0 || buf.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: `buf` ma aspon `n` bajtu.
    let out = unsafe { core::slice::from_raw_parts_mut(buf, n as usize) };
    let mut s = file.lock();
    let mut i = 0;
    while i + 1 < out.len() {
        match s.getc() {
            Some(c) => {
                out[i] = c;
                i += 1;
                if c == b'\n' {
                    break;
                }
            }
            None => break,
        }
    }
    if i == 0 {
        return core::ptr::null_mut();
    }
    out[i] = 0;
    buf
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fseek(f: *mut FILE, off: c_long, whence: c_int) -> c_int {
    // SAFETY: kontrakt C.
    let Some(file) = (unsafe { file(f) }) else { return -1 };
    let pos = match whence {
        SEEK_SET if off >= 0 => Seek::Set(off as u64),
        SEEK_CUR => Seek::Cur(off),
        SEEK_END => Seek::End(off),
        _ => {
            errno::set(errno::EINVAL);
            return -1;
        }
    };
    match file.lock().seek(pos) {
        Ok(_) => 0,
        Err(e) => {
            errno::set(e);
            -1
        }
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn ftell(f: *mut FILE) -> c_long {
    // SAFETY: kontrakt C.
    let Some(file) = (unsafe { file(f) }) else { return -1 };
    match file.lock().tell() {
        Ok(p) => c_long::try_from(p).unwrap_or(c_long::MAX),
        Err(e) => {
            errno::set(e);
            -1
        }
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fseeko(f: *mut FILE, off: i64, whence: c_int) -> c_int {
    // SAFETY: kontrakt C (off_t = long = 64 bitu).
    unsafe { fseek(f, off, whence) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn ftello(f: *mut FILE) -> i64 {
    // SAFETY: kontrakt C.
    unsafe { ftell(f) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn rewind(f: *mut FILE) {
    // SAFETY: kontrakt C.
    unsafe {
        fseek(f, 0, SEEK_SET);
        clearerr(f);
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn feof(f: *mut FILE) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { file(f) }.map_or(0, |f| f.lock().eof as c_int)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn ferror(f: *mut FILE) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { file(f) }.map_or(0, |f| f.lock().err as c_int)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn clearerr(f: *mut FILE) {
    // SAFETY: kontrakt C.
    if let Some(file) = unsafe { file(f) } {
        let mut s = file.lock();
        s.eof = false;
        s.err = false;
    }
}

/// Rezim bufferu (`_IOFBF`, `_IOLBF`, `_IONBF`); vlastni buffer volajiciho
/// se nepouzije (proud ma svuj).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn setvbuf(f: *mut FILE, _buf: *mut c_char, mode: c_int, _size: size_t) -> c_int {
    // SAFETY: kontrakt C.
    let Some(file) = (unsafe { file(f) }) else { return -1 };
    let b = match mode {
        IOFBF => Buffering::Full,
        IOLBF => Buffering::Line,
        IONBF => Buffering::None,
        _ => return -1,
    };
    let mut s = file.lock();
    let _ = s.flush();
    s.buffering = b;
    0
}

/// `setbuf`: NULL = bez bufferu, jinak plne bufferovani (vlastni buffer
/// volajiciho se nepouzije, jako u `setvbuf`).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn setbuf(f: *mut FILE, buf: *mut c_char) {
    let mode = if buf.is_null() { IONBF } else { IOFBF };
    // SAFETY: kontrakt C.
    let _ = unsafe { setvbuf(f, buf, mode, 0) };
}

/// Vystup formatovani do proudu (po kusech, ne po bajtech).
struct FileOut {
    buf: Vec<u8>,
}

impl Out for FileOut {
    fn put(&mut self, b: u8) {
        self.buf.push(b);
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn vfprintf(f: *mut FILE, fmt: *const c_char, mut ap: VaList) -> c_int {
    // SAFETY: kontrakt C.
    let (Some(file), fmt) = (unsafe { file(f) }, unsafe { c_bytes(fmt) }) else { return -1 };
    ensure_exit_hook();
    let mut out = FileOut { buf: Vec::new() };
    let r = format::format_enc(&mut out, fmt, &mut VaArgs(&mut ap), crate::locale::ctype());
    // Vystup do chyby prevodu (`%ls` se znakem mimo kodovani) se zapise.
    match file.lock().write(&out.buf) {
        Ok(()) if r.is_err() => {
            errno::set(errno::EILSEQ);
            -1
        }
        Ok(()) => c_int::try_from(out.buf.len()).unwrap_or(c_int::MAX),
        Err(e) => {
            errno::set(e);
            -1
        }
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fprintf(f: *mut FILE, fmt: *const c_char, args: ...) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { vfprintf(f, fmt, args) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn vprintf(fmt: *const c_char, ap: VaList) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { vfprintf(stdout.0 as *mut FILE, fmt, ap) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn printf(fmt: *const c_char, args: ...) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { vfprintf(stdout.0 as *mut FILE, fmt, args) }
}

/// `vasprintf` (GNU/BSD, libc++): vystup do noveho bufferu z `malloc`
/// (uvolni volajici `free`). Chyba = -1, `*out` je pak nulovy.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn vasprintf(out: *mut *mut c_char, fmt: *const c_char, mut ap: VaList) -> c_int {
    if out.is_null() || fmt.is_null() {
        errno::set(errno::EINVAL);
        return -1;
    }
    // SAFETY: retezec C.
    let f = unsafe { c_bytes(fmt) };
    let mut buf = FileOut { buf: Vec::new() };
    let r = format::format_enc(&mut buf, f, &mut VaArgs(&mut ap), crate::locale::ctype());
    // SAFETY: `out` je platny (kontrakt C).
    unsafe { *out = core::ptr::null_mut() };
    if r.is_err() {
        errno::set(errno::EILSEQ);
        return -1;
    }
    let Ok(n) = c_int::try_from(buf.buf.len()) else {
        errno::set(errno::EOVERFLOW);
        return -1;
    };
    // SAFETY: malloc z cstd-core; kopie `n` bajtu a nula do nove pameti.
    unsafe {
        let p = cstd_core::stdlib::malloc(buf.buf.len() + 1);
        if p.is_null() {
            errno::set(errno::ENOMEM);
            return -1;
        }
        core::ptr::copy_nonoverlapping(buf.buf.as_ptr(), p, buf.buf.len());
        *p.add(buf.buf.len()) = 0;
        *out = p;
    }
    n
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn asprintf(out: *mut *mut c_char, fmt: *const c_char, args: ...) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { vasprintf(out, fmt, args) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn perror(s: *const c_char) {
    let e = errno::get();
    let msg = errno::message(e);
    let mut err = STDERR.lock();
    if !s.is_null() {
        // SAFETY: retezec C.
        let p = unsafe { c_bytes(s) };
        if !p.is_empty() {
            let _ = err.write(p);
            let _ = err.write(b": ");
        }
    }
    let _ = err.write(&msg[..msg.len() - 1]);
    let _ = err.write(b"\n");
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn remove(path: *const c_char) -> c_int {
    // SAFETY: retezec C.
    let p = unsafe { c_bytes(path) };
    let r = match sys::stat(p) {
        Ok(st) if st.kind == sys::Kind::Dir => sys::rmdir(p),
        _ => sys::unlink(p),
    };
    errno::ret(r)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn rename(from: *const c_char, to: *const c_char) -> c_int {
    // SAFETY: retezce C.
    let (a, b) = unsafe { (c_bytes(from), c_bytes(to)) };
    errno::ret(sys::rename(a, b))
}

/// Precte radek az po `delim` vcetne (`getdelim`); buffer `*line` zvetsi
/// pres `realloc`. Vraci delku bez nuly, -1 = konec nebo chyba.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn getdelim(line: *mut *mut c_char, cap: *mut size_t, delim: c_int, f: *mut FILE) -> isize {
    // SAFETY: kontrakt C.
    let Some(file) = (unsafe { file(f) }) else { return -1 };
    if line.is_null() || cap.is_null() {
        errno::set(errno::EINVAL);
        return -1;
    }
    let mut out = Vec::new();
    {
        let mut s = file.lock();
        while let Some(c) = s.getc() {
            out.push(c);
            if c_int::from(c) == delim {
                break;
            }
        }
    }
    if out.is_empty() {
        return -1;
    }
    // SAFETY: `*line` je NULL nebo z malloc, `*cap` jeho velikost.
    unsafe {
        if (*line).is_null() || *cap < out.len() + 1 {
            let p = cstd_core::stdlib::realloc((*line).cast(), out.len() + 1).cast::<c_char>();
            if p.is_null() {
                errno::set(errno::ENOMEM);
                return -1;
            }
            *line = p;
            *cap = out.len() + 1;
        }
        core::ptr::copy_nonoverlapping(out.as_ptr(), *line, out.len());
        *(*line).add(out.len()) = 0;
    }
    out.len() as isize
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn getline(line: *mut *mut c_char, cap: *mut size_t, f: *mut FILE) -> isize {
    // SAFETY: kontrakt C.
    unsafe { getdelim(line, cap, c_int::from(b'\n'), f) }
}

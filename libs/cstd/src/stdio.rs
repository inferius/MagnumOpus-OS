//! `stdio.h`: proudy `FILE` nad std (stdin/stdout/stderr = stdio procesu,
//! soubory = `std::fs::File`), formatovany vystup a vstup po znacich.
//!
//! Buffer: stdout po radcich, stderr bez bufferu, soubory 4 KiB. Pri
//! ukonceni procesu (`exit`, navrat z `main`) se vse vyprazdni (osrt
//! `at_exit`). Kazdy `FILE` ma vlastni zamek (vlakna smi pouzivat stejny
//! proud).

use core::ffi::VaList;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::{Mutex, MutexGuard};

use cstd_core::format::{self, Out, VaArgs};

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

enum Target {
    Stdin,
    Stdout,
    Stderr,
    File(fs::File),
    Closed,
}

struct Stream {
    target: Target,
    buffering: Buffering,
    wbuf: Vec<u8>,
    rbuf: Vec<u8>,
    rpos: usize,
    eof: bool,
    err: bool,
    readable: bool,
    writable: bool,
}

/// Proud C (`FILE *`).
pub struct FILE {
    s: Mutex<Stream>,
}

impl FILE {
    const fn std(target: Target, buffering: Buffering, readable: bool, writable: bool) -> Self {
        FILE {
            s: Mutex::new(Stream {
                target,
                buffering,
                wbuf: Vec::new(),
                rbuf: Vec::new(),
                rpos: 0,
                eof: false,
                err: false,
                readable,
                writable,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Stream> {
        self.s.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Stream {
    fn write_raw(&mut self, data: &[u8]) -> io::Result<()> {
        match &mut self.target {
            Target::Stdout => {
                let mut o = io::stdout().lock();
                o.write_all(data)?;
                o.flush()
            }
            Target::Stderr => io::stderr().lock().write_all(data),
            Target::File(f) => f.write_all(data),
            Target::Stdin | Target::Closed => Err(io::Error::from(io::ErrorKind::Unsupported)),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
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
    /// u souboru vrati pozici zpet.
    fn drop_read(&mut self) -> io::Result<()> {
        let unread = self.rbuf.len() - self.rpos;
        self.rbuf.clear();
        self.rpos = 0;
        if unread > 0
            && let Target::File(f) = &mut self.target
        {
            f.seek(SeekFrom::Current(-(unread as i64)))?;
        }
        Ok(())
    }

    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        if !self.writable {
            self.err = true;
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
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
    fn fill(&mut self) -> io::Result<usize> {
        if !self.readable {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        self.flush()?;
        // Pred ctenim stdin vyprazdnit stdout (vyzva bez konce radku).
        if matches!(self.target, Target::Stdin) {
            flush_std_out();
        }
        let mut buf = vec![0u8; BUF];
        let n = match &mut self.target {
            Target::Stdin => io::stdin().lock().read(&mut buf)?,
            Target::File(f) => f.read(&mut buf)?,
            _ => return Err(io::Error::from(io::ErrorKind::Unsupported)),
        };
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
                    errno::set(errno::from_io(&e));
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
                        errno::set(errno::from_io(&e));
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

    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.flush()?;
        self.drop_read()?;
        self.eof = false;
        match &mut self.target {
            Target::File(f) => f.seek(pos),
            _ => Err(io::Error::from(io::ErrorKind::NotSeekable)),
        }
    }
}

static STDIN: FILE = FILE::std(Target::Stdin, Buffering::Full, true, false);
static STDOUT: FILE = FILE::std(Target::Stdout, Buffering::Line, false, true);
static STDERR: FILE = FILE::std(Target::Stderr, Buffering::None, false, true);

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

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fopen(path: *const c_char, mode: *const c_char) -> *mut FILE {
    // SAFETY: retezce C.
    let (p, m) = unsafe { (c_bytes(path), c_bytes(mode)) };
    let Some(mode) = parse_mode(m) else {
        errno::set(errno::EINVAL);
        return core::ptr::null_mut();
    };
    let Ok(p) = core::str::from_utf8(p) else {
        errno::set(errno::EINVAL);
        return core::ptr::null_mut();
    };
    let r = fs::OpenOptions::new()
        .read(mode.read)
        .write(mode.write && !mode.append)
        .append(mode.append)
        .truncate(mode.truncate)
        .create(mode.create && !mode.create_new)
        .create_new(mode.create_new)
        .open(p);
    match r {
        Ok(f) => {
            ensure_exit_hook();
            let file = Box::new(FILE::std(Target::File(f), Buffering::Full, mode.read, mode.write));
            let ptr = Box::into_raw(file);
            OPEN.lock().unwrap_or_else(|e| e.into_inner()).push(ptr as usize);
            ptr
        }
        Err(e) => {
            errno::set(errno::from_io(&e));
            core::ptr::null_mut()
        }
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fclose(f: *mut FILE) -> c_int {
    // SAFETY: kontrakt C.
    let Some(file) = (unsafe { file(f) }) else { return EOF };
    let r = {
        let mut s = file.lock();
        let r = s.flush();
        s.target = Target::Closed;
        r
    };
    let mut open = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = open.iter().position(|p| *p == f as usize) {
        open.remove(i);
        drop(open);
        // SAFETY: `f` vytvoril `fopen` (Box), uz neni v seznamu.
        drop(unsafe { Box::from_raw(f) });
    }
    match r {
        Ok(()) => 0,
        Err(e) => {
            errno::set(errno::from_io(&e));
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
                errno::set(errno::from_io(&e));
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
            errno::set(errno::from_io(&e));
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
        SEEK_SET if off >= 0 => SeekFrom::Start(off as u64),
        SEEK_CUR => SeekFrom::Current(off),
        SEEK_END => SeekFrom::End(off),
        _ => {
            errno::set(errno::EINVAL);
            return -1;
        }
    };
    match file.lock().seek(pos) {
        Ok(_) => 0,
        Err(e) => errno::fail(&e),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn ftell(f: *mut FILE) -> c_long {
    // SAFETY: kontrakt C.
    let Some(file) = (unsafe { file(f) }) else { return -1 };
    let mut s = file.lock();
    let buffered_read = (s.rbuf.len() - s.rpos) as i64;
    let buffered_write = s.wbuf.len() as i64;
    let pos = match &mut s.target {
        Target::File(f) => f.stream_position(),
        _ => Err(io::Error::from(io::ErrorKind::NotSeekable)),
    };
    match pos {
        Ok(p) => p as i64 - buffered_read + buffered_write,
        Err(e) => i64::from(errno::fail(&e)),
    }
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
    format::format(&mut out, fmt, &mut VaArgs(&mut ap));
    match file.lock().write(&out.buf) {
        Ok(()) => c_int::try_from(out.buf.len()).unwrap_or(c_int::MAX),
        Err(e) => errno::fail(&e),
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
    let Ok(p) = core::str::from_utf8(unsafe { c_bytes(path) }) else {
        errno::set(errno::EINVAL);
        return -1;
    };
    let r = match fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => fs::remove_dir(p),
        _ => fs::remove_file(p),
    };
    match r {
        Ok(()) => 0,
        Err(e) => errno::fail(&e),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn rename(from: *const c_char, to: *const c_char) -> c_int {
    // SAFETY: retezce C.
    let (a, b) = unsafe { (c_bytes(from), c_bytes(to)) };
    let (Ok(a), Ok(b)) = (core::str::from_utf8(a), core::str::from_utf8(b)) else {
        errno::set(errno::EINVAL);
        return -1;
    };
    match fs::rename(a, b) {
        Ok(()) => 0,
        Err(e) => errno::fail(&e),
    }
}

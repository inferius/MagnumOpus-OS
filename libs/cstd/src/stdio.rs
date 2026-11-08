//! `stdio.h`: `printf` a spol. Standardni vystup je log programu: text se
//! sbira po radcich (`\n` nebo plny buffer) a predava funkci z
//! [`set_output`] - v systemu log jadra pres libos. Souborove funkce
//! (`fopen`, `FILE`) zatim nejsou.

use core::ffi::VaList;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use crate::ctypes::{c_char, c_int, size_t};
use crate::format::{self, Buf, Out, VaArgs};

/// Funkce, ktera dostane jeden hotovy radek (bez `\n`).
pub type Sink = fn(&[u8]);

static SINK: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// Nastavi, kam jde standardni vystup (jednou na zacatku programu).
pub fn set_output(f: Sink) {
    SINK.store(f as *mut (), Ordering::Release);
}

fn emit(line: &[u8]) {
    let p = SINK.load(Ordering::Acquire);
    if !p.is_null() {
        // SAFETY: do SINK se uklada jen `Sink` (set_output).
        let f: Sink = unsafe { core::mem::transmute::<*mut (), Sink>(p) };
        f(line);
    }
}

/// Buffer radku sdileny vsemi vlakny (zamek: zapisy z vice vlaken se
/// nepromichaji uvnitr jednoho `printf`).
struct Line {
    buf: [u8; 256],
    len: usize,
}

static LOCK: AtomicBool = AtomicBool::new(false);
static mut LINE: Line = Line { buf: [0; 256], len: 0 };

fn with_line<R>(f: impl FnOnce(&mut Line) -> R) -> R {
    while LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    // SAFETY: LINE je dostupny jen pod zamkem LOCK.
    let r = f(unsafe { &mut *core::ptr::addr_of_mut!(LINE) });
    LOCK.store(false, Ordering::Release);
    r
}

struct LineOut<'a> {
    line: &'a mut Line,
    count: usize,
}

impl Out for LineOut<'_> {
    fn put(&mut self, b: u8) {
        self.count += 1;
        let l = &mut *self.line;
        if b == b'\n' {
            emit(&l.buf[..l.len]);
            l.len = 0;
            return;
        }
        if l.len == l.buf.len() {
            emit(&l.buf[..l.len]);
            l.len = 0;
        }
        l.buf[l.len] = b;
        l.len += 1;
    }
}

/// Vypise rozepsany radek (bez `\n` na konci).
pub fn flush() {
    with_line(|l| {
        if l.len > 0 {
            emit(&l.buf[..l.len]);
            l.len = 0;
        }
    });
}

/// Delka formatu (retezec zakonceny nulou).
unsafe fn fmt_bytes<'a>(fmt: *const c_char) -> &'a [u8] {
    // SAFETY: `fmt` je retezec zakonceny nulou (kontrakt printf).
    unsafe { core::slice::from_raw_parts(fmt, crate::string::strlen(fmt)) }
}

fn clamp(n: usize) -> c_int {
    n.min(c_int::MAX as usize) as c_int
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vprintf(fmt: *const c_char, mut ap: VaList) -> c_int {
    // SAFETY: kontrakt printf.
    let f = unsafe { fmt_bytes(fmt) };
    with_line(|line| {
        let mut out = LineOut { line, count: 0 };
        format::format(&mut out, f, &mut VaArgs(&mut ap));
        clamp(out.count)
    })
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn printf(fmt: *const c_char, args: ...) -> c_int {
    // SAFETY: kontrakt printf.
    unsafe { vprintf(fmt, args) }
}

/// Zapise nejvys `n - 1` znaku a nulu; vraci delku celeho vystupu (vetsi
/// nebo rovna `n` = oriznuto). `n == 0` nic nezapise.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vsnprintf(buf: *mut c_char, n: size_t, fmt: *const c_char, mut ap: VaList) -> c_int {
    // SAFETY: `buf` ma aspon `n` bajtu (kontrakt C); `fmt` je retezec.
    let (dst, f) = unsafe {
        let dst: &mut [u8] = if n == 0 || buf.is_null() { &mut [] } else { core::slice::from_raw_parts_mut(buf, n) };
        (dst, fmt_bytes(fmt))
    };
    let cap = dst.len().saturating_sub(1);
    let mut out = Buf { buf: &mut dst[..cap], pos: 0, total: 0 };
    format::format(&mut out, f, &mut VaArgs(&mut ap));
    let (pos, total) = (out.pos, out.total);
    if !dst.is_empty() {
        dst[pos] = 0;
    }
    clamp(total)
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn snprintf(buf: *mut c_char, n: size_t, fmt: *const c_char, args: ...) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { vsnprintf(buf, n, fmt, args) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn puts(s: *const c_char) -> c_int {
    // SAFETY: retezec zakonceny nulou.
    let s = unsafe { fmt_bytes(s) };
    with_line(|line| {
        let mut out = LineOut { line, count: 0 };
        for &b in s {
            out.put(b);
        }
        out.put(b'\n');
    });
    1
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn putchar(c: c_int) -> c_int {
    with_line(|line| LineOut { line, count: 0 }.put(c as u8));
    c & 0xFF
}

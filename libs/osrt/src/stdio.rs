//! Standardni vstup a vystup: kanaly s protokolem proudu bajtu.
//!
//! Protokol (docs/cs/rust-std.md): kazda zprava = kus bajtu (nejvys
//! `CHANNEL_MAX_DATA`), zavreni kanalu = konec proudu. Kanaly preda rodic
//! ve startovni zprave se stitky `abi::params::tag::STD*`. Bez kanalu jde
//! stdout i stderr do logu jadra (po radcich) a stdin je prazdny;
//! priznaky `abi::params::flags::STD*_NULL` vystup zahodi.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use libos::Channel;
use libos::sync::Mutex;

use crate::Result;
use crate::pipe::{read_stream, write_stream};
use crate::start::Init;

static STDIN: Init<Channel> = Init::new();
static STDOUT: Init<Channel> = Init::new();
static STDERR: Init<Channel> = Init::new();
/// Priznaky `abi::params::flags` (zahodit stdout/stderr).
static NULL: AtomicU32 = AtomicU32::new(0);

/// Zbytek posledni zpravy stdin, ktery se neveselo do bufferu ctenare.
static STDIN_REST: Mutex<(Vec<u8>, usize)> = Mutex::new((Vec::new(), 0));
/// Rozepsany radek pro log jadra (vystup bez kanalu).
static LOG_LINE: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// # Safety
///
/// Jen pri startu (viz `start::Init`).
pub(crate) unsafe fn init(stdin: Option<Channel>, stdout: Option<Channel>, stderr: Option<Channel>, flags: u32) {
    NULL.store(flags & abi::params::flags::ALL, Ordering::Relaxed);
    // SAFETY: predano volajicim.
    unsafe {
        if let Some(c) = stdin {
            STDIN.set(c);
        }
        if let Some(c) = stdout {
            STDOUT.set(c);
        }
        if let Some(c) = stderr {
            STDERR.set(c);
        }
    }
}

/// Ktery proud.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    In,
    Out,
    Err,
}

fn channel(s: Stream) -> Option<&'static Channel> {
    match s {
        Stream::In => STDIN.get(),
        Stream::Out => STDOUT.get(),
        Stream::Err => STDERR.get(),
    }
}

fn null_flag(s: Stream) -> u32 {
    match s {
        Stream::In => 0,
        Stream::Out => abi::params::flags::STDOUT_NULL,
        Stream::Err => abi::params::flags::STDERR_NULL,
    }
}

/// Zahazuje se proud (`Stdio::null` u rodice)?
pub fn is_null(s: Stream) -> bool {
    NULL.load(Ordering::Relaxed) & null_flag(s) != 0
}

/// Zapise cely `buf` do stdout/stderr. Plna fronta kanalu = kratke cekani
/// (zpetny tlak); zavreny kanal = `PeerClosed` (jako zavrena roura).
pub fn write(stream: Stream, buf: &[u8]) -> Result<usize> {
    if is_null(stream) {
        return Ok(buf.len());
    }
    match channel(stream) {
        Some(ch) => write_stream(ch, buf)?,
        None => log_write(buf),
    }
    Ok(buf.len())
}

/// Vystup bez kanalu: do logu jadra po celych radcich.
fn log_write(buf: &[u8]) {
    let mut line = LOG_LINE.lock();
    for &b in buf {
        if b == b'\n' {
            flush_line(&mut line);
        } else {
            line.push(b);
            if line.len() >= abi::syscall::DEBUG_LOG_MAX {
                flush_line(&mut line);
            }
        }
    }
}

fn flush_line(line: &mut Vec<u8>) {
    let text = alloc::string::String::from_utf8_lossy(line);
    crate::debug_log(&text);
    line.clear();
}

/// Precte az `buf.len()` bajtu ze stdin; 0 = konec proudu (nebo zadny stdin).
pub fn read(buf: &mut [u8]) -> Result<usize> {
    match STDIN.get() {
        Some(ch) => read_stream(ch, &STDIN_REST, buf),
        None => Ok(0),
    }
}

/// Handle kanalu proudu (cekani v `poll`); `None` = bez kanalu.
pub fn handle(stream: Stream) -> Option<&'static libos::Handle> {
    channel(stream).map(Channel::handle)
}

/// Ma stdin prectena, ale nevydana data?
pub fn stdin_buffered() -> bool {
    let r = STDIN_REST.lock();
    r.1 < r.0.len()
}

/// Ma proces kanal pro dany proud (jinak jde vystup do logu jadra)?
pub fn has_channel(stream: Stream) -> bool {
    channel(stream).is_some()
}

/// Jak predat proud potomkovi pri `Stdio::inherit`.
pub enum Inherit {
    /// Duplikat vlastniho kanalu (potomek pise do stejneho proudu).
    Channel(Channel),
    /// Proces proud zahazuje: potomek taky.
    Null,
    /// Bez kanalu (log jadra, prazdny stdin): potomek taky bez kanalu.
    None,
}

/// Proud pro potomka pri `Stdio::inherit`.
pub fn inherit(stream: Stream) -> Result<Inherit> {
    if is_null(stream) {
        return Ok(Inherit::Null);
    }
    match channel(stream) {
        Some(ch) => Ok(Inherit::Channel(Channel::from_handle(ch.handle().duplicate(None)?))),
        None => Ok(Inherit::None),
    }
}

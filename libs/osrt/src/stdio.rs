//! Standardni vstup a vystup: kanaly s protokolem proudu bajtu.
//!
//! Protokol (docs/cs/rust-std.md): kazda zprava = kus bajtu (nejvys
//! `CHANNEL_MAX_DATA`), zavreni kanalu = konec proudu. Kanaly preda rodic
//! ve startovni zprave se stitky `abi::params::tag::STD*`. Bez kanalu jde
//! stdout i stderr do logu jadra (po radcich) a stdin je prazdny.

use alloc::vec::Vec;
use core::time::Duration;

use abi::Error;
use libos::Channel;
use libos::sync::Mutex;

use crate::Result;
use crate::start::Init;

static STDIN: Init<Channel> = Init::new();
static STDOUT: Init<Channel> = Init::new();
static STDERR: Init<Channel> = Init::new();
/// Priznaky `abi::params::flags` (zahodit stdout/stderr).
static NULL: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Zbytek posledni zpravy stdin, ktery se neveselo do bufferu ctenare.
static STDIN_REST: Mutex<(Vec<u8>, usize)> = Mutex::new((Vec::new(), 0));
/// Rozepsany radek pro log jadra (vystup bez kanalu).
static LOG_LINE: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// # Safety
///
/// Jen pri startu (viz `start::Init`).
pub(crate) unsafe fn init(stdin: Option<Channel>, stdout: Option<Channel>, stderr: Option<Channel>, flags: u32) {
    NULL.store(flags & abi::params::flags::ALL, core::sync::atomic::Ordering::Relaxed);
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
    Out,
    Err,
}

/// Zapise cely `buf`. Plna fronta kanalu = kratke cekani a znovu (zpetny
/// tlak); zavreny kanal = `PeerClosed` (jako zavrena roura).
pub fn write(stream: Stream, buf: &[u8]) -> Result<usize> {
    let channel = match stream {
        Stream::Out => STDOUT.get(),
        Stream::Err => STDERR.get(),
    };
    let null = match stream {
        Stream::Out => abi::params::flags::STDOUT_NULL,
        Stream::Err => abi::params::flags::STDERR_NULL,
    };
    if NULL.load(core::sync::atomic::Ordering::Relaxed) & null != 0 {
        return Ok(buf.len());
    }
    let Some(channel) = channel else {
        log_write(buf);
        return Ok(buf.len());
    };
    for chunk in buf.chunks(abi::syscall::CHANNEL_MAX_DATA) {
        loop {
            match channel.write(chunk, Vec::new()) {
                Ok(()) => break,
                // Fronta prijemce je plna: pockat, az si cast precte.
                Err(Error::ShouldWait) => libos::time::sleep(Duration::from_millis(1)),
                Err(e) => return Err(e),
            }
        }
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
    let Some(channel) = STDIN.get() else {
        return Ok(0);
    };
    let mut rest = STDIN_REST.lock();
    // Prazdna zprava neni konec proudu (ten je jen zavreni kanalu).
    while rest.1 >= rest.0.len() {
        match channel.read(None) {
            Ok(msg) => *rest = (msg.data, 0),
            Err(Error::PeerClosed) => return Ok(0),
            Err(e) => return Err(e),
        }
    }
    let (data, at) = &mut *rest;
    let n = buf.len().min(data.len() - *at);
    buf[..n].copy_from_slice(&data[*at..*at + n]);
    *at += n;
    Ok(n)
}

/// Ma proces kanal pro dany proud (jinak jde vystup do logu jadra)?
pub fn has_channel(stream: Stream) -> bool {
    match stream {
        Stream::Out => STDOUT.get().is_some(),
        Stream::Err => STDERR.get().is_some(),
    }
}

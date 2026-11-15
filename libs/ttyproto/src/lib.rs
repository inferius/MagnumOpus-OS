//! Protokol rizeni terminalu (`termios.h`, docs/cs/cstd.md, kapitola
//! termios): rezim vstupu a vystupu a velikost okna.
//!
//! Proces spusteny v terminalu dostane ve startovni zprave krome proudu
//! stdin/stdout/stderr i **ridici kanal terminalu** (stitek
//! `abi::params::tag::TTY`). Data jdou dal proudy bajtu; tento kanal nese
//! jen rizeni. Radkovou disciplinu (kanonicky rezim, echo, Ctrl+C ->
//! `SIGINT`) dela strana terminalu - editor radku zustava v terminalu,
//! v surovem rezimu terminal posila bajty klaves (VT sekvence pro sipky).
//!
//! Zpravy (little-endian, `wire`): pozadavek = `u32` op + telo; odpoved
//! = `i32` stav (0 = ok, jinak kod `abi::Error`) + telo. Na kanalu je
//! rozpracovany nejvys jeden pozadavek (klient ceka na odpoved).
//!
//! | op | pozadavek | odpoved |
//! |---|---|---|
//! | `GET_MODE` 1 | - | [`Mode`] |
//! | `SET_MODE` 2 | `u8` kdy ([`when`]), [`Mode`] | [`Mode`] (co terminal skutecne nastavil) |
//! | `GET_SIZE` 3 | - | [`Size`] |
//! | `SET_SIZE` 4 | [`Size`] | - |
//! | `FLUSH` 5 | `u8` fronta ([`queue`]) | - |
//! | `DRAIN` 6 | - | - (az je vystup predany) |
//! | `WATCH_SIZE` 7 | - | - + handle udalosti: terminal ji signalizuje pri zmene velikosti (`SIGWINCH`) |
//!
//! Hodnoty priznaku a indexy `c_cc` jsou jako Linux (`struct termios`
//! glibc), takze klient v C je predava beze zmeny. Terminal nepodporovane
//! priznaky v odpovedi `SET_MODE` vynuluje.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

pub use abi::{Error, Result};
use wire::{Reader, Writer};

pub mod op {
    pub const GET_MODE: u32 = 1;
    pub const SET_MODE: u32 = 2;
    pub const GET_SIZE: u32 = 3;
    pub const SET_SIZE: u32 = 4;
    pub const FLUSH: u32 = 5;
    pub const DRAIN: u32 = 6;
    pub const WATCH_SIZE: u32 = 7;
}

/// Kdy plati novy rezim (`tcsetattr`: `TCSANOW`, `TCSADRAIN`, `TCSAFLUSH`).
pub mod when {
    pub const NOW: u8 = 0;
    pub const DRAIN: u8 = 1;
    pub const FLUSH: u8 = 2;
}

/// Ktera fronta (`tcflush`: `TCIFLUSH`, `TCOFLUSH`, `TCIOFLUSH`).
pub mod queue {
    pub const INPUT: u8 = 0;
    pub const OUTPUT: u8 = 1;
    pub const BOTH: u8 = 2;
}

/// Priznaky vstupu (`c_iflag`, jako Linux).
pub mod iflag {
    pub const IGNBRK: u32 = 0o1;
    pub const BRKINT: u32 = 0o2;
    pub const IGNPAR: u32 = 0o4;
    pub const PARMRK: u32 = 0o10;
    pub const INPCK: u32 = 0o20;
    pub const ISTRIP: u32 = 0o40;
    pub const INLCR: u32 = 0o100;
    pub const IGNCR: u32 = 0o200;
    pub const ICRNL: u32 = 0o400;
    pub const IUCLC: u32 = 0o1000;
    pub const IXON: u32 = 0o2000;
    pub const IXANY: u32 = 0o4000;
    pub const IXOFF: u32 = 0o10000;
    pub const IMAXBEL: u32 = 0o20000;
    pub const IUTF8: u32 = 0o40000;
}

/// Priznaky vystupu (`c_oflag`).
pub mod oflag {
    pub const OPOST: u32 = 0o1;
    pub const OLCUC: u32 = 0o2;
    pub const ONLCR: u32 = 0o4;
    pub const OCRNL: u32 = 0o10;
    pub const ONOCR: u32 = 0o20;
    pub const ONLRET: u32 = 0o40;
}

/// Priznaky linky (`c_cflag`); terminal je virtualni, rychlost jen pro
/// programy, ktere ji ctou.
pub mod cflag {
    pub const CSIZE: u32 = 0o60;
    pub const CS5: u32 = 0o0;
    pub const CS6: u32 = 0o20;
    pub const CS7: u32 = 0o40;
    pub const CS8: u32 = 0o60;
    pub const CSTOPB: u32 = 0o100;
    pub const CREAD: u32 = 0o200;
    pub const PARENB: u32 = 0o400;
    pub const PARODD: u32 = 0o1000;
    pub const HUPCL: u32 = 0o2000;
    pub const CLOCAL: u32 = 0o4000;
    pub const CBAUD: u32 = 0o10017;
    pub const B38400: u32 = 0o17;
}

/// Mistni priznaky (`c_lflag`).
pub mod lflag {
    pub const ISIG: u32 = 0o1;
    pub const ICANON: u32 = 0o2;
    pub const ECHO: u32 = 0o10;
    pub const ECHOE: u32 = 0o20;
    pub const ECHOK: u32 = 0o40;
    pub const ECHONL: u32 = 0o100;
    pub const NOFLSH: u32 = 0o200;
    pub const TOSTOP: u32 = 0o400;
    pub const ECHOCTL: u32 = 0o1000;
    pub const ECHOPRT: u32 = 0o2000;
    pub const ECHOKE: u32 = 0o4000;
    pub const IEXTEN: u32 = 0o100000;
}

/// Indexy `c_cc` (jako Linux).
pub mod cc {
    pub const VINTR: usize = 0;
    pub const VQUIT: usize = 1;
    pub const VERASE: usize = 2;
    pub const VKILL: usize = 3;
    pub const VEOF: usize = 4;
    pub const VTIME: usize = 5;
    pub const VMIN: usize = 6;
    pub const VSWTC: usize = 7;
    pub const VSTART: usize = 8;
    pub const VSTOP: usize = 9;
    pub const VSUSP: usize = 10;
    pub const VEOL: usize = 11;
    pub const VREPRINT: usize = 12;
    pub const VDISCARD: usize = 13;
    pub const VWERASE: usize = 14;
    pub const VLNEXT: usize = 15;
    pub const VEOL2: usize = 16;
    /// Delka pole `c_cc` (`NCCS`).
    pub const NCCS: usize = 32;
}

/// Rezim terminalu (pole `struct termios`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub iflag: u32,
    pub oflag: u32,
    pub cflag: u32,
    pub lflag: u32,
    pub line: u8,
    pub cc: [u8; cc::NCCS],
    pub ispeed: u32,
    pub ospeed: u32,
}

impl Mode {
    /// Vychozi kanonicky rezim (jako novy terminal Linuxu, UTF-8).
    pub fn cooked() -> Self {
        let mut c = [0u8; cc::NCCS];
        c[cc::VINTR] = 0x03;
        c[cc::VQUIT] = 0x1c;
        c[cc::VERASE] = 0x7f;
        c[cc::VKILL] = 0x15;
        c[cc::VEOF] = 0x04;
        c[cc::VTIME] = 0;
        c[cc::VMIN] = 1;
        c[cc::VSTART] = 0x11;
        c[cc::VSTOP] = 0x13;
        c[cc::VSUSP] = 0x1a;
        c[cc::VREPRINT] = 0x12;
        c[cc::VDISCARD] = 0x0f;
        c[cc::VWERASE] = 0x17;
        c[cc::VLNEXT] = 0x16;
        Mode {
            iflag: iflag::ICRNL | iflag::IXON | iflag::IUTF8,
            oflag: oflag::OPOST | oflag::ONLCR,
            cflag: cflag::B38400 | cflag::CS8 | cflag::CREAD,
            lflag: lflag::ISIG | lflag::ICANON | lflag::ECHO | lflag::ECHOE | lflag::ECHOK | lflag::ECHOCTL | lflag::ECHOKE | lflag::IEXTEN,
            line: 0,
            cc: c,
            ispeed: cflag::B38400,
            ospeed: cflag::B38400,
        }
    }

    /// `cfmakeraw`: surovy rezim (bez echo, bez radkove discipliny, bajt po
    /// bajtu, bez prevodu konce radku).
    pub fn make_raw(&mut self) {
        use iflag::*;
        self.iflag &= !(IGNBRK | BRKINT | PARMRK | ISTRIP | INLCR | IGNCR | ICRNL | IXON);
        self.oflag &= !oflag::OPOST;
        self.lflag &= !(lflag::ECHO | lflag::ECHONL | lflag::ICANON | lflag::ISIG | lflag::IEXTEN);
        self.cflag &= !(cflag::CSIZE | cflag::PARENB);
        self.cflag |= cflag::CS8;
        self.cc[cc::VMIN] = 1;
        self.cc[cc::VTIME] = 0;
    }

    pub fn put(&self, w: &mut Writer) {
        w.u32(self.iflag).u32(self.oflag).u32(self.cflag).u32(self.lflag).u8(self.line);
        w.bytes(&self.cc);
        w.u32(self.ispeed).u32(self.ospeed);
    }

    pub fn get(r: &mut Reader<'_>) -> Result<Self> {
        let (iflag, oflag, cflag, lflag, line) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u8()?);
        let mut c = [0u8; cc::NCCS];
        c.copy_from_slice(r.take(cc::NCCS)?);
        Ok(Mode { iflag, oflag, cflag, lflag, line, cc: c, ispeed: r.u32()?, ospeed: r.u32()? })
    }
}

/// Velikost okna (`struct winsize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
    pub xpixel: u16,
    pub ypixel: u16,
}

impl Size {
    pub fn put(&self, w: &mut Writer) {
        w.u16(self.rows).u16(self.cols).u16(self.xpixel).u16(self.ypixel);
    }

    pub fn get(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Size { rows: r.u16()?, cols: r.u16()?, xpixel: r.u16()?, ypixel: r.u16()? })
    }
}

/// Pozadavek klienta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    GetMode,
    SetMode { when: u8, mode: Mode },
    GetSize,
    SetSize(Size),
    Flush(u8),
    Drain,
    WatchSize,
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Request::GetMode => w = Writer::op(op::GET_MODE),
            Request::SetMode { when, mode } => {
                w = Writer::op(op::SET_MODE);
                w.u8(*when);
                mode.put(&mut w);
            }
            Request::GetSize => w = Writer::op(op::GET_SIZE),
            Request::SetSize(s) => {
                w = Writer::op(op::SET_SIZE);
                s.put(&mut w);
            }
            Request::Flush(q) => {
                w = Writer::op(op::FLUSH);
                w.u8(*q);
            }
            Request::Drain => w = Writer::op(op::DRAIN),
            Request::WatchSize => w = Writer::op(op::WATCH_SIZE),
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let req = match r.u32()? {
            op::GET_MODE => Request::GetMode,
            op::SET_MODE => {
                let when = r.u8()?;
                if when > when::FLUSH {
                    return Err(Error::InvalidArgs);
                }
                Request::SetMode { when, mode: Mode::get(&mut r)? }
            }
            op::GET_SIZE => Request::GetSize,
            op::SET_SIZE => Request::SetSize(Size::get(&mut r)?),
            op::FLUSH => {
                let q = r.u8()?;
                if q > queue::BOTH {
                    return Err(Error::InvalidArgs);
                }
                Request::Flush(q)
            }
            op::DRAIN => Request::Drain,
            op::WATCH_SIZE => Request::WatchSize,
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(req)
    }
}

/// Telo odpovedi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Body {
    Empty,
    Mode(Mode),
    Size(Size),
}

/// Odpoved: stav a telo (u chyby prazdne).
pub fn encode_reply(r: Result<Body>) -> Vec<u8> {
    let mut w = Writer::new();
    match r {
        Ok(b) => {
            w.i32(0);
            match b {
                Body::Empty => {}
                Body::Mode(m) => m.put(&mut w),
                Body::Size(s) => s.put(&mut w),
            }
        }
        Err(e) => {
            w.i32(e as i32);
        }
    }
    w.finish()
}

/// Precte odpoved na pozadavek `req`.
pub fn decode_reply(req: &Request, b: &[u8]) -> Result<Body> {
    let mut r = Reader::new(b);
    let status = r.i32()?;
    if status != 0 {
        return Err(Error::from_code(i64::from(status)).unwrap_or(Error::Corrupted));
    }
    let body = match req {
        Request::GetMode | Request::SetMode { .. } => Body::Mode(Mode::get(&mut r)?),
        Request::GetSize => Body::Size(Size::get(&mut r)?),
        _ => Body::Empty,
    };
    if !r.is_empty() {
        return Err(Error::Corrupted);
    }
    Ok(body)
}

/// Priznaky, ktere terminal MagnumOpus umi (ostatni `SET_MODE` vynuluje).
pub const SUPPORTED_IFLAG: u32 = iflag::ICRNL | iflag::INLCR | iflag::IGNCR | iflag::IXON | iflag::IUTF8 | iflag::BRKINT | iflag::ISTRIP;
pub const SUPPORTED_OFLAG: u32 = oflag::OPOST | oflag::ONLCR;
pub const SUPPORTED_LFLAG: u32 = lflag::ISIG | lflag::ICANON | lflag::ECHO | lflag::ECHOE | lflag::ECHOK | lflag::ECHONL | lflag::ECHOCTL | lflag::ECHOKE | lflag::IEXTEN | lflag::NOFLSH;

/// Stav strany terminalu (okno terminalu, testovaci server): rezim
/// a velikost. Odpovedi na pozadavky bez prace s kanalem (handle udalosti
/// pro `WATCH_SIZE` prida volajici).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tty {
    pub mode: Mode,
    pub size: Size,
    /// Smi klient menit velikost (`TIOCSWINSZ`)? Okno terminalu ne.
    pub client_resize: bool,
}

impl Tty {
    pub fn new(size: Size) -> Self {
        Tty { mode: Mode::cooked(), size, client_resize: false }
    }

    /// Zpracuje pozadavek; `WatchSize` jen potvrdi (udalost dodava volajici).
    pub fn handle(&mut self, req: &Request) -> Result<Body> {
        Ok(match req {
            Request::GetMode => Body::Mode(self.mode),
            Request::SetMode { mode, .. } => {
                let mut m = *mode;
                m.iflag &= SUPPORTED_IFLAG;
                m.oflag &= SUPPORTED_OFLAG;
                m.lflag &= SUPPORTED_LFLAG;
                // Linka je virtualni: vzdy 8 bitu bez parity.
                m.cflag = (m.cflag & (cflag::CBAUD | cflag::CREAD | cflag::CLOCAL | cflag::HUPCL)) | cflag::CS8;
                self.mode = m;
                Body::Mode(m)
            }
            Request::GetSize => Body::Size(self.size),
            Request::SetSize(s) => {
                if !self.client_resize {
                    return Err(Error::AccessDenied);
                }
                self.size = *s;
                Body::Empty
            }
            Request::Flush(_) | Request::Drain | Request::WatchSize => Body::Empty,
        })
    }

    /// Okno zmenilo velikost; `true` = zmena (signalizovat udalost).
    pub fn resize(&mut self, s: Size) -> bool {
        let changed = self.size != s;
        self.size = s;
        changed
    }
}

#[cfg(test)]
mod tests;

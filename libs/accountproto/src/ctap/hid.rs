//! Prenos CTAPHID (CTAP 2.1, kap. 11.2): zpravy rozdelene do hlaseni HID
//! po 64 B. Prvni paket: kanal (4 B) | prikaz s bitem 7 | delka (2 B, BE)
//! | 57 B dat. Pokracovaci: kanal | poradi 0-127 | 59 B dat.

use alloc::vec::Vec;

/// Velikost hlaseni HID FIDO.
pub const PACKET: usize = 64;
const INIT_DATA: usize = PACKET - 7;
const CONT_DATA: usize = PACKET - 5;
/// Nejdelsi zprava: 57 + 128 * 59.
pub const MAX_MESSAGE: usize = INIT_DATA + 128 * CONT_DATA;
/// Vysilaci kanal (jen pro INIT).
pub const BROADCAST: u32 = 0xFFFF_FFFF;

pub mod cmd {
    pub const PING: u8 = 0x81;
    pub const MSG: u8 = 0x83;
    pub const LOCK: u8 = 0x84;
    pub const INIT: u8 = 0x86;
    pub const WINK: u8 = 0x88;
    pub const CBOR: u8 = 0x90;
    pub const CANCEL: u8 = 0x91;
    pub const KEEPALIVE: u8 = 0xBB;
    pub const ERROR: u8 = 0xBF;
}

/// Stav v KEEPALIVE.
pub mod keepalive {
    pub const PROCESSING: u8 = 1;
    /// Klic ceka na dotek.
    pub const UP_NEEDED: u8 = 2;
}

/// Chyby v prikazu ERROR.
pub mod err {
    pub const INVALID_CMD: u8 = 0x01;
    pub const INVALID_PAR: u8 = 0x02;
    pub const INVALID_LEN: u8 = 0x03;
    pub const INVALID_SEQ: u8 = 0x04;
    pub const MSG_TIMEOUT: u8 = 0x05;
    pub const CHANNEL_BUSY: u8 = 0x06;
    pub const INVALID_CHANNEL: u8 = 0x0B;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HidError {
    /// Zprava delsi nez [`MAX_MESSAGE`].
    TooLong,
    /// Pokracovaci paket mimo poradi nebo bez prvniho.
    Sequence,
    /// Paket s jinym kanalem uprostred zpravy.
    Channel,
}

/// Rozdeli zpravu do paketu.
pub fn fragment(cid: u32, command: u8, data: &[u8]) -> Result<Vec<[u8; PACKET]>, HidError> {
    if data.len() > MAX_MESSAGE {
        return Err(HidError::TooLong);
    }
    let mut out = Vec::new();
    let mut p = [0u8; PACKET];
    p[..4].copy_from_slice(&cid.to_be_bytes());
    p[4] = command | 0x80;
    p[5..7].copy_from_slice(&(data.len() as u16).to_be_bytes());
    let first = data.len().min(INIT_DATA);
    p[7..7 + first].copy_from_slice(&data[..first]);
    out.push(p);
    for (seq, chunk) in data[first..].chunks(CONT_DATA).enumerate() {
        let mut c = [0u8; PACKET];
        c[..4].copy_from_slice(&cid.to_be_bytes());
        c[4] = seq as u8;
        c[5..5 + chunk.len()].copy_from_slice(chunk);
        out.push(c);
    }
    Ok(out)
}

/// Skladani zpravy z paketu.
#[derive(Default)]
pub struct Assembler {
    cid: u32,
    command: u8,
    need: usize,
    seq: u8,
    data: Vec<u8>,
    active: bool,
}

/// Slozena zprava.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub cid: u32,
    pub command: u8,
    pub data: Vec<u8>,
}

impl Assembler {
    /// Prida paket; `Ok(Some(zprava))`, kdyz je zprava cela.
    pub fn push(&mut self, p: &[u8; PACKET]) -> Result<Option<Message>, HidError> {
        let cid = u32::from_be_bytes([p[0], p[1], p[2], p[3]]);
        if p[4] & 0x80 != 0 {
            // Prvni paket zahodi rozpracovanou zpravu (tak to dela i klic
            // po CANCEL nebo novem INIT).
            let need = u16::from_be_bytes([p[5], p[6]]) as usize;
            if need > MAX_MESSAGE {
                self.active = false;
                return Err(HidError::TooLong);
            }
            self.cid = cid;
            self.command = p[4];
            self.need = need;
            self.seq = 0;
            self.data.clear();
            self.data.extend_from_slice(&p[7..7 + need.min(INIT_DATA)]);
            self.active = true;
        } else {
            if !self.active {
                return Err(HidError::Sequence);
            }
            if cid != self.cid {
                return Err(HidError::Channel);
            }
            if p[4] != self.seq {
                self.active = false;
                return Err(HidError::Sequence);
            }
            self.seq = self.seq.wrapping_add(1);
            let rest = (self.need - self.data.len()).min(CONT_DATA);
            self.data.extend_from_slice(&p[5..5 + rest]);
        }
        if self.data.len() >= self.need {
            self.active = false;
            return Ok(Some(Message { cid: self.cid, command: self.command, data: core::mem::take(&mut self.data) }));
        }
        Ok(None)
    }
}

/// Odpoved na INIT: nonce (8) | kanal (4) | verze protokolu | major | minor
/// | build | schopnosti.
pub fn init_response(nonce: &[u8; 8], cid: u32, caps: u8) -> Vec<u8> {
    let mut v = nonce.to_vec();
    v.extend_from_slice(&cid.to_be_bytes());
    v.extend_from_slice(&[2, 1, 0, 0, caps]);
    v
}

/// Schopnosti v odpovedi INIT.
pub mod caps {
    pub const WINK: u8 = 0x01;
    pub const CBOR: u8 = 0x04;
    /// Klic NEumi CTAP1/U2F (MSG).
    pub const NMSG: u8 = 0x08;
}

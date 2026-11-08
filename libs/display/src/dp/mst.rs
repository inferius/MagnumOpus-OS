//! Multi-Stream Transport (DP 1.2+) - **jen priprava**: kodovani
//! hlavicky a tela sideband zprav s kontrolnimi soucty a zprava
//! `LINK_ADDRESS` (zjisteni topologie). Prideleni casovych slotu (VCPI,
//! payload table) a obsluha vetvicu (branch) nejsou hotove; ovladace
//! zatim jedou jen SST a MST v prijemci nezapinaji.
//!
//! ```text
//! hlavicka: [LCT << 4 | LCR] [RAD ... (LCT/2 B)] [broadcast << 7 | path << 6 | delka tela]
//!           [SOMT << 7 | EOMT << 6 | seq << 4 | CRC4]
//! telo:     [data ...] [CRC8]
//! ```
//!
//! CRC podle `drm_dp_mst_topology.c` (licence "permission ... without
//! fee"): hlavicka polynom x^4 + x + 1 (0x13) pres nibbly, telo polynom
//! 0xD5 (x^8 + x^7 + x^6 + x^4 + x^2 + 1) pres bajty.

use alloc::vec::Vec;

use crate::{Error, Result};

/// Typy pozadavku sideband.
pub mod req {
    pub const LINK_ADDRESS: u8 = 0x01;
    pub const ENUM_PATH_RESOURCES: u8 = 0x10;
    pub const ALLOCATE_PAYLOAD: u8 = 0x11;
    pub const REMOTE_DPCD_READ: u8 = 0x20;
    pub const REMOTE_I2C_READ: u8 = 0x22;
}

/// Hlavicka sideband zpravy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Link Count Total (pocet skoku + 1).
    pub lct: u8,
    /// Link Count Remaining.
    pub lcr: u8,
    /// Relative Address: cisla portu na ceste (po nibblech).
    pub rad: [u8; 8],
    pub broadcast: bool,
    pub path_msg: bool,
    /// Delka tela vcetne CRC8.
    pub body_len: u8,
    pub somt: bool,
    pub eomt: bool,
    pub seqno: u8,
}

/// CRC4 hlavicky pres `nibbles` nibblu od zacatku `data`.
pub fn header_crc4(data: &[u8], nibbles: usize) -> u8 {
    let mut rem: u8 = 0;
    for i in 0..nibbles * 4 {
        let bit = (data[i / 8] >> (7 - i % 8)) & 1;
        rem = (rem << 1) | bit;
        if rem & 0x10 != 0 {
            rem ^= 0x13;
        }
    }
    for _ in 0..4 {
        rem <<= 1;
        if rem & 0x10 != 0 {
            rem ^= 0x13;
        }
    }
    rem & 0x0F
}

/// CRC8 tela.
pub fn body_crc8(data: &[u8]) -> u8 {
    let mut rem: u16 = 0;
    for i in 0..data.len() * 8 {
        let bit = (data[i / 8] >> (7 - i % 8)) & 1;
        rem = (rem << 1) | bit as u16;
        if rem & 0x100 != 0 {
            rem ^= 0xD5;
        }
    }
    for _ in 0..8 {
        rem <<= 1;
        if rem & 0x100 != 0 {
            rem ^= 0xD5;
        }
    }
    (rem & 0xFF) as u8
}

impl Header {
    /// Hlavicka pro prijemce pripojeneho primo (LCT 1).
    pub fn direct(body_len: u8, seqno: u8) -> Self {
        Header {
            lct: 1,
            lcr: 0,
            rad: [0; 8],
            broadcast: false,
            path_msg: false,
            body_len,
            somt: true,
            eomt: true,
            seqno,
        }
    }

    pub fn len(&self) -> usize {
        3 + self.lct as usize / 2
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        let start = out.len();
        out.push((self.lct << 4) | (self.lcr & 0x0F));
        for i in 0..self.lct as usize / 2 {
            out.push((self.rad[2 * i] << 4) | (self.rad[2 * i + 1] & 0x0F));
        }
        out.push((self.broadcast as u8) << 7 | (self.path_msg as u8) << 6 | (self.body_len & 0x3F));
        out.push((self.somt as u8) << 7 | (self.eomt as u8) << 6 | (self.seqno & 1) << 4);
        let n = out.len() - start;
        let crc = header_crc4(&out[start..], n * 2 - 1);
        out[start + n - 1] |= crc;
    }

    pub fn decode(b: &[u8]) -> Result<(Self, usize)> {
        let first = *b.first().ok_or(Error::Truncated)?;
        let lct = first >> 4;
        if lct == 0 || lct > 15 {
            return Err(Error::Invalid);
        }
        let len = 3 + lct as usize / 2;
        let h = b.get(..len).ok_or(Error::Truncated)?;
        if header_crc4(h, len * 2 - 1) != h[len - 1] & 0x0F {
            return Err(Error::BadChecksum);
        }
        let mut rad = [0u8; 8];
        for i in 0..lct as usize / 2 {
            rad[2 * i] = h[1 + i] >> 4;
            rad[2 * i + 1] = h[1 + i] & 0x0F;
        }
        let a = h[len - 2];
        let z = h[len - 1];
        Ok((
            Header {
                lct,
                lcr: first & 0x0F,
                rad,
                broadcast: a & 0x80 != 0,
                path_msg: a & 0x40 != 0,
                body_len: a & 0x3F,
                somt: z & 0x80 != 0,
                eomt: z & 0x40 != 0,
                seqno: (z >> 4) & 1,
            },
            len,
        ))
    }
}

/// Cela zprava (jeden kus: SOMT i EOMT) s telem `body` (bez CRC).
pub fn encode_message(h: &Header, body: &[u8], out: &mut Vec<u8>) -> Result<()> {
    if body.len() + 1 > 0x3F {
        return Err(Error::Invalid);
    }
    let h = Header {
        body_len: body.len() as u8 + 1,
        ..*h
    };
    h.encode(out);
    out.extend_from_slice(body);
    out.push(body_crc8(body));
    Ok(())
}

/// Rozebere zpravu: hlavicka a telo bez CRC (overene).
pub fn decode_message(b: &[u8]) -> Result<(Header, &[u8])> {
    let (h, n) = Header::decode(b)?;
    let body = b.get(n..n + h.body_len as usize).ok_or(Error::Truncated)?;
    let (data, crc) = body.split_at(body.len().checked_sub(1).ok_or(Error::Truncated)?);
    if body_crc8(data) != crc[0] {
        return Err(Error::BadChecksum);
    }
    Ok((h, data))
}

/// Pozadavek `LINK_ADDRESS` primemu prijemci (telo = jen typ).
pub fn link_address(seqno: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let _ = encode_message(&Header::direct(0, seqno), &[req::LINK_ADDRESS], &mut out);
    out
}

//! Kanal AUX DisplayPortu: kodovani transakci (nativni DPCD a I2C pres
//! AUX), opakovani pri `DEFER` a cteni EDID pres I2C pres AUX.
//!
//! ```text
//! pozadavek: [prikaz << 4 | adresa 19:16] [adresa 15:8] [adresa 7:0] [delka-1] [data zapisu]
//! odpoved:   [kod << 4] [data cteni]
//!            kod bity 1-0 nativni (0 ACK, 1 NACK, 2 DEFER),
//!                bity 3-2 I2C (0 ACK, 1 NACK, 2 DEFER)
//! ```
//!
//! Transakce nese nejvic 16 B dat. Postup opakovani a cteni EDID podle
//! `drm_dp_helper.c` (licence "permission ... without fee", MIT-like):
//! nejmene 7 pokusu pri DEFER, kratka odpoved I2C pokracuje od mista,
//! kde skoncila.

use crate::{Error, Result};

/// Nejvic bajtu dat v jedne transakci.
pub const MAX_PAYLOAD: usize = 16;
/// Pokusy pri `DEFER` (DP 1.4: nejmene 7).
pub const DEFER_RETRIES: u32 = 7;
/// Adresy I2C pres AUX: EDID a ukazatel segmentu E-DDC.
pub const I2C_EDID: u32 = 0x50;
pub const I2C_SEGMENT: u32 = 0x30;

/// Prikazy AUX.
pub mod cmd {
    pub const I2C_WRITE: u8 = 0x0;
    pub const I2C_READ: u8 = 0x1;
    pub const I2C_WRITE_STATUS_UPDATE: u8 = 0x2;
    /// Middle-Of-Transaction: I2C transakce pokracuje (bez STOP).
    pub const I2C_MOT: u8 = 0x4;
    pub const NATIVE_WRITE: u8 = 0x8;
    pub const NATIVE_READ: u8 = 0x9;
}

/// Zprava AUX.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Msg {
    pub cmd: u8,
    /// 20bitova adresa (DPCD) nebo 7bitova adresa I2C.
    pub address: u32,
    /// Delka dat (0 = jen adresa, pouziva I2C pro START/STOP).
    pub len: usize,
}

/// Vysledek transakce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Ack,
    Nack,
    Defer,
}

impl Msg {
    pub fn is_native(&self) -> bool {
        self.cmd & 0x8 != 0
    }

    pub fn is_read(&self) -> bool {
        self.cmd & 0x1 != 0 && self.cmd & 0x2 == 0
    }

    /// Hlavicka pozadavku (3 B, s delkou 4 B).
    pub fn header(&self, out: &mut [u8; 4]) -> usize {
        out[0] = (self.cmd << 4) | ((self.address >> 16) & 0x0F) as u8;
        out[1] = (self.address >> 8) as u8;
        out[2] = self.address as u8;
        if self.len == 0 {
            return 3;
        }
        out[3] = (self.len - 1) as u8;
        4
    }

    /// Cely pozadavek do `out` (hlavicka + data zapisu); vraci delku.
    pub fn encode(&self, data: &[u8], out: &mut [u8; 20]) -> usize {
        let mut h = [0u8; 4];
        let n = self.header(&mut h);
        out[..n].copy_from_slice(&h[..n]);
        if self.is_read() || self.len == 0 {
            return n;
        }
        out[n..n + self.len].copy_from_slice(&data[..self.len]);
        n + self.len
    }

    /// Rozebere pozadavek (simulace prijemce v testech): zprava a data zapisu.
    pub fn decode(tx: &[u8]) -> Option<(Msg, &[u8])> {
        if tx.len() < 3 {
            return None;
        }
        let cmd = tx[0] >> 4;
        let address = ((tx[0] as u32 & 0x0F) << 16) | (tx[1] as u32) << 8 | tx[2] as u32;
        let len = if tx.len() >= 4 { tx[3] as usize + 1 } else { 0 };
        let m = Msg { cmd, address, len };
        let data = if m.is_read() || len == 0 { &tx[0..0] } else { tx.get(4..4 + len)? };
        Some((m, data))
    }
}

/// Kod odpovedi z prvniho bajtu prijateho ramce.
pub fn decode_reply(first: u8, native: bool) -> Reply {
    let code = first >> 4;
    // Nativni cast (bity 1-0) plati vzdy; I2C cast (bity 3-2) jen u I2C.
    match code & 3 {
        1 => return Reply::Nack,
        2 | 3 => return Reply::Defer,
        _ => {}
    }
    if native {
        return Reply::Ack;
    }
    match (code >> 2) & 3 {
        0 => Reply::Ack,
        1 => Reply::Nack,
        _ => Reply::Defer,
    }
}

/// Fyzicky kanal AUX (ovladac zdroje). `transfer` posle pozadavek
/// `tx` (zakodovany [`Msg::encode`]) a do `rx` ulozi odpoved vcetne
/// prvniho bajtu s kodem; vraci delku odpovedi. Chyba = selhani prenosu
/// (timeout, chyba prijmu) - to se opakuje take.
pub trait Aux {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8; 20]) -> Result<usize>;
    /// Pauza mezi pokusy (us).
    fn delay_us(&mut self, us: u32);
}

/// Jedna transakce s opakovanim pri DEFER a chybe prenosu. Vraci pocet
/// prenesenych bajtu dat (u cteni muze byt mene, nez se zadalo).
pub fn transact<A: Aux + ?Sized>(aux: &mut A, msg: &Msg, data: &mut [u8]) -> Result<usize> {
    if msg.len > MAX_PAYLOAD || data.len() < msg.len {
        return Err(Error::Invalid);
    }
    let mut tx = [0u8; 20];
    let n = msg.encode(data, &mut tx);
    let mut io_errors = 0;
    let mut defers = 0;
    loop {
        let mut rx = [0u8; 20];
        let got = match aux.transfer(&tx[..n], &mut rx) {
            Ok(g) if g >= 1 => g,
            Ok(_) | Err(_) => {
                io_errors += 1;
                if io_errors >= 5 {
                    return Err(Error::Io);
                }
                aux.delay_us(500);
                continue;
            }
        };
        match decode_reply(rx[0], msg.is_native()) {
            Reply::Ack => {
                if msg.is_read() {
                    let k = (got - 1).min(msg.len);
                    data[..k].copy_from_slice(&rx[1..1 + k]);
                    return Ok(k);
                }
                // Zapis I2C: prijemce muze potvrdit jen cast (bajt M).
                if !msg.is_native() && got >= 2 {
                    return Ok((rx[1] as usize).min(msg.len));
                }
                return Ok(msg.len);
            }
            Reply::Nack => return Err(Error::Nack),
            Reply::Defer => {
                defers += 1;
                if defers > DEFER_RETRIES {
                    return Err(Error::Timeout);
                }
                aux.delay_us(500);
            }
        }
    }
}

/// Cteni DPCD od `address` (po 16 B).
pub fn dpcd_read<A: Aux + ?Sized>(aux: &mut A, address: u32, buf: &mut [u8]) -> Result<()> {
    let mut done = 0;
    while done < buf.len() {
        let len = (buf.len() - done).min(MAX_PAYLOAD);
        let msg = Msg {
            cmd: cmd::NATIVE_READ,
            address: address + done as u32,
            len,
        };
        let k = transact(aux, &msg, &mut buf[done..done + len])?;
        if k == 0 {
            return Err(Error::Io);
        }
        done += k;
    }
    Ok(())
}

/// Zapis do DPCD od `address`.
pub fn dpcd_write<A: Aux + ?Sized>(aux: &mut A, address: u32, data: &[u8]) -> Result<()> {
    for (i, chunk) in data.chunks(MAX_PAYLOAD).enumerate() {
        let mut tmp = [0u8; MAX_PAYLOAD];
        tmp[..chunk.len()].copy_from_slice(chunk);
        let msg = Msg {
            cmd: cmd::NATIVE_WRITE,
            address: address + (i * MAX_PAYLOAD) as u32,
            len: chunk.len(),
        };
        transact(aux, &msg, &mut tmp)?;
    }
    Ok(())
}

pub fn dpcd_read_byte<A: Aux + ?Sized>(aux: &mut A, address: u32) -> Result<u8> {
    let mut b = [0u8; 1];
    dpcd_read(aux, address, &mut b)?;
    Ok(b[0])
}

/// Cteni bloku EDID pres I2C pres AUX (E-DDC: segment, offset, data).
pub fn read_edid_block<A: Aux + ?Sized>(aux: &mut A, block: u8, buf: &mut [u8]) -> Result<()> {
    let mot = cmd::I2C_MOT;
    if block >= 2 {
        let mut seg = [block / 2];
        let m = Msg { cmd: cmd::I2C_WRITE | mot, address: I2C_SEGMENT, len: 1 };
        transact(aux, &m, &mut seg)?;
    }
    let mut off = [(block % 2).wrapping_mul(128)];
    let m = Msg { cmd: cmd::I2C_WRITE | mot, address: I2C_EDID, len: 1 };
    transact(aux, &m, &mut off)?;
    let total = buf.len().min(128);
    let mut done = 0;
    while done < total {
        let len = (total - done).min(MAX_PAYLOAD);
        let m = Msg { cmd: cmd::I2C_READ | mot, address: I2C_EDID, len };
        let k = transact(aux, &m, &mut buf[done..done + len])?;
        if k == 0 {
            return Err(Error::Io);
        }
        done += k;
    }
    // STOP: transakce jen s adresou bez MOT.
    let mut none = [0u8; 0];
    let m = Msg { cmd: cmd::I2C_READ, address: I2C_EDID, len: 0 };
    transact(aux, &m, &mut none)?;
    Ok(())
}

/// Cely EDID pres AUX.
pub fn read_edid<A: Aux + ?Sized>(aux: &mut A) -> Result<alloc::vec::Vec<u8>> {
    crate::edid::read(|i, b| read_edid_block(aux, i, b))
}

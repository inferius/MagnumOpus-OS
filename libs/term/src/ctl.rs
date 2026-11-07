//! Ridici zpravy mezi termd, procesy relaci (`shsess`) a registrem relaci.
//!
//! Kazda relace bezi ve vlastnim procesu. termd ji posila zpravy okna
//! (`SessMsg::Client`) po hlavnim kanalu; relace odpovida primo zpravami
//! `ServerMsg`. Aby relace prezila konec termd, ma druhy, nahradni kanal:
//! jeho druhy konec drzi registr (proces, ktery termd spustil - dnes test
//! v initu, pozdeji supervisor). Novy termd si u registru vyzvedne
//! nahradni konce osirelych relaci (`Claim`) a posle na ne `Adopt`.
//!
//! ```text
//!  okno <-> termd <-hlavni-> shsess (relace 1)
//!              |  `-hlavni-> shsess (relace 2)
//!              `-> registr: nahradni konce relaci 1, 2
//! ```

use alloc::string::String;
use alloc::vec::Vec;

use crate::proto::{ClientMsg, DecodeError, R, W};

const S_CLIENT: u8 = 0;
const S_INIT: u8 = 1;
const S_ADOPT: u8 = 2;

/// termd -> relace.
#[derive(Debug, Clone, PartialEq)]
pub enum SessMsg {
    /// Prvni zprava po spusteni; handly: [nahradni kanal, kanal fsd].
    Init { session: u64, tag: String, home: String },
    /// Na nahradnim kanalu: novy termd relaci prevzal; handly: [novy
    /// nahradni kanal]. Nahradni kanal se stava hlavnim.
    Adopt,
    /// Zprava okna pro tuto relaci.
    Client(ClientMsg),
}

impl SessMsg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W(Vec::new());
        match self {
            SessMsg::Init { session, tag, home } => {
                w.u8(S_INIT);
                w.u64(*session);
                w.str(tag);
                w.str(home);
            }
            SessMsg::Adopt => w.u8(S_ADOPT),
            SessMsg::Client(m) => {
                w.u8(S_CLIENT);
                w.0.extend_from_slice(&m.encode());
            }
        }
        w.0
    }

    pub fn decode(b: &[u8]) -> Result<Self, DecodeError> {
        let (&tag, rest) = b.split_first().ok_or(DecodeError)?;
        match tag {
            S_CLIENT => Ok(SessMsg::Client(ClientMsg::decode(rest)?)),
            S_INIT => {
                let mut r = R::new(rest);
                let m = SessMsg::Init { session: r.u64()?, tag: r.str()?, home: r.str()? };
                r.done()?;
                Ok(m)
            }
            S_ADOPT if rest.is_empty() => Ok(SessMsg::Adopt),
            _ => Err(DecodeError),
        }
    }
}

const G_PARK: u8 = 1;
const G_UNPARK: u8 = 2;
const G_CLAIM: u8 = 3;
const G_PARKED: u8 = 4;

/// Zpravy registru relaci.
#[derive(Debug, Clone, PartialEq)]
pub enum RegMsg {
    /// termd -> registr: uschovej nahradni konec relace (1 handle).
    Park { session: u64, tag: String },
    /// termd -> registr: relace skoncila, konec zahod.
    Unpark { session: u64 },
    /// termd -> registr (po startu): vydej uschovane konce.
    Claim,
    /// registr -> termd: (relace, znacka); handly ve stejnem poradi.
    Parked { sessions: Vec<(u64, String)> },
}

/// Nejvic relaci v jedne odpovedi `Parked`.
pub const MAX_PARKED: usize = 256;

impl RegMsg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W(Vec::new());
        match self {
            RegMsg::Park { session, tag } => {
                w.u8(G_PARK);
                w.u64(*session);
                w.str(tag);
            }
            RegMsg::Unpark { session } => {
                w.u8(G_UNPARK);
                w.u64(*session);
            }
            RegMsg::Claim => w.u8(G_CLAIM),
            RegMsg::Parked { sessions } => {
                w.u8(G_PARKED);
                w.u32(sessions.len() as u32);
                for (id, tag) in sessions {
                    w.u64(*id);
                    w.str(tag);
                }
            }
        }
        w.0
    }

    pub fn decode(b: &[u8]) -> Result<Self, DecodeError> {
        let mut r = R::new(b);
        let m = match r.u8()? {
            G_PARK => RegMsg::Park { session: r.u64()?, tag: r.str()? },
            G_UNPARK => RegMsg::Unpark { session: r.u64()? },
            G_CLAIM => RegMsg::Claim,
            G_PARKED => {
                let n = r.u32()? as usize;
                if n > MAX_PARKED {
                    return Err(DecodeError);
                }
                let mut sessions = Vec::with_capacity(n);
                for _ in 0..n {
                    sessions.push((r.u64()?, r.str()?));
                }
                RegMsg::Parked { sessions }
            }
            _ => return Err(DecodeError),
        };
        r.done()?;
        Ok(m)
    }
}

/// Registr relaci: uschovane nahradni konce (`H` = handle kanalu). Drzi ho
/// proces, ktery termd spousti; prezije konec termd.
#[derive(Debug)]
pub struct Registry<H> {
    parked: Vec<(u64, String, H)>,
}

impl<H> Default for Registry<H> {
    fn default() -> Self {
        Self { parked: Vec::new() }
    }
}

impl<H> Registry<H> {
    pub fn len(&self) -> usize {
        self.parked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parked.is_empty()
    }

    /// Zpracuje zpravu od termd. `handles` = handly zpravy. Vrati odpoved
    /// (jen u `Claim`) s handly k odeslani.
    pub fn handle(&mut self, msg: RegMsg, mut handles: Vec<H>) -> Option<(RegMsg, Vec<H>)> {
        match msg {
            RegMsg::Park { session, tag } => {
                // Bez handlu neni co uschovat; stary konec relace nahradit.
                let h = handles.pop()?;
                self.parked.retain(|(id, _, _)| *id != session);
                self.parked.push((session, tag, h));
                None
            }
            RegMsg::Unpark { session } => {
                self.parked.retain(|(id, _, _)| *id != session);
                None
            }
            RegMsg::Claim => {
                let n = self.parked.len().min(MAX_PARKED);
                let taken: Vec<(u64, String, H)> = self.parked.drain(..n).collect();
                let mut sessions = Vec::new();
                let mut hs = Vec::new();
                for (id, tag, h) in taken {
                    sessions.push((id, tag));
                    hs.push(h);
                }
                Some((RegMsg::Parked { sessions }, hs))
            }
            RegMsg::Parked { .. } => None,
        }
    }
}

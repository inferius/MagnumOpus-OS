//! Uzel B+stromu (16 KiB = NODE_BLOCKS bloku) a jeho kodovani.
//!
//! ```text
//! 0   magic u16 "BT", uroven u8 (0 = list), rezerva u8, pocet polozek u16
//! 8   generace vzniku u64
//! 32  polozky:
//!       list (32 B):     klic (oid u64, druh u8 + 7 B, offset u64),
//!                        offset hodnoty u16, delka u16, rezerva
//!       vnitrni (64 B):  klic (24 B), ukazatel na potomka (BlockPtr, 40 B)
//!     hodnoty listu jsou ulozene od konce bloku smerem dolu
//! ```
//!
//! Uzel nema vlastni kontrolni soucet: overuje ho soucet v ukazateli rodice
//! (u korene v superbloku). Odhali to i "ztraceny zapis", kdy na miste zustal
//! stary, sam o sobe platny uzel.

use alloc::vec::Vec;

use abi::{Error, Result};

use crate::layout::{BlockPtr, Key, NODE_SIZE, PTR_SIZE, u64_at};

const HEADER: usize = 32;
const LEAF_ENTRY: usize = 32;
const INTERNAL_ENTRY: usize = 24 + PTR_SIZE;
const MAGIC: u16 = u16::from_le_bytes(*b"BT");

/// Nejvetsi hodnota polozky. Polozka (hlavicka + hodnota) smi zabrat nejvys
/// polovinu uzitecneho mista listu - jen tak jde preplneny list vzdy rozdelit
/// na dve casti, ktere se obe vejdou.
pub const MAX_VALUE: usize = (NODE_SIZE - HEADER) / 2 - LEAF_ENTRY;
/// Nejvic potomku vnitrniho uzlu.
pub const MAX_CHILDREN: usize = (NODE_SIZE - HEADER) / INTERNAL_ENTRY;

/// Potomek vnitrniho uzlu: ulozeny na disku, nebo rozpracovany v pameti
/// (blok a soucet dostane az pri commitu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Child {
    Disk(BlockPtr),
    Mem(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Leaf(Vec<(Key, Vec<u8>)>),
    /// (nejmensi klic v podstromu, potomek); prvni klic se pri hledani
    /// ignoruje (pokryva vse mensi).
    Internal(Vec<(Key, Child)>),
}

/// Nejvetsi uroven uzlu (hloubka stromu). Pri 255 potomcich na uzel staci
/// 8 urovni na vic polozek, nez se vejde na jakykoli disk; vic = poskozeni.
pub const MAX_LEVEL: u8 = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub generation: u64,
    /// Uroven (0 = list); potomek vnitrniho uzlu ma vzdy uroven o 1 nizsi.
    /// Kontrola pri kazdem sestupu zarucuje, ze se poskozeny strom nezacykli.
    pub level: u8,
    pub body: Body,
}

fn put_key(b: &mut [u8], k: &Key) {
    b[0..8].copy_from_slice(&k.oid.to_le_bytes());
    b[8] = k.kind;
    b[16..24].copy_from_slice(&k.offset.to_le_bytes());
}

fn get_key(b: &[u8]) -> Key {
    Key {
        oid: u64_at(b, 0),
        kind: b[8],
        offset: u64_at(b, 16),
    }
}

impl Node {
    /// Velikost listu po zakodovani (pro rozhodnuti o rozdeleni).
    pub fn leaf_size(items: &[(Key, Vec<u8>)]) -> usize {
        HEADER
            + items
                .iter()
                .map(|(_, v)| LEAF_ENTRY + v.len())
                .sum::<usize>()
    }

    /// Zakoduje uzel; vsichni potomci uz musi byt na disku.
    pub fn encode(&self) -> Vec<u8> {
        let mut b = alloc::vec![0u8; NODE_SIZE];
        b[0..2].copy_from_slice(&MAGIC.to_le_bytes());
        b[8..16].copy_from_slice(&self.generation.to_le_bytes());
        match &self.body {
            Body::Leaf(items) => {
                debug_assert!(Self::leaf_size(items) <= NODE_SIZE);
                b[4..6].copy_from_slice(&(items.len() as u16).to_le_bytes());
                let mut end = NODE_SIZE;
                for (i, (k, v)) in items.iter().enumerate() {
                    end -= v.len();
                    b[end..end + v.len()].copy_from_slice(v);
                    let e = HEADER + i * LEAF_ENTRY;
                    put_key(&mut b[e..e + 24], k);
                    b[e + 24..e + 26].copy_from_slice(&(end as u16).to_le_bytes());
                    b[e + 26..e + 28].copy_from_slice(&(v.len() as u16).to_le_bytes());
                }
            }
            Body::Internal(children) => {
                debug_assert!(children.len() <= MAX_CHILDREN && self.level > 0);
                b[2] = self.level;
                b[4..6].copy_from_slice(&(children.len() as u16).to_le_bytes());
                for (i, (k, child)) in children.iter().enumerate() {
                    let e = HEADER + i * INTERNAL_ENTRY;
                    put_key(&mut b[e..e + 24], k);
                    let Child::Disk(ptr) = child else {
                        panic!("uzel s neulozenym potomkem")
                    };
                    ptr.put(&mut b[e + 24..e + INTERNAL_ENTRY]);
                }
            }
        }
        b
    }

    /// Dekoduje blok (kontrolni soucet uz overil volajici).
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != NODE_SIZE || u16::from_le_bytes([b[0], b[1]]) != MAGIC {
            return Err(Error::Corrupted);
        }
        let level = b[2];
        if level > MAX_LEVEL {
            return Err(Error::Corrupted);
        }
        let n = u16::from_le_bytes([b[4], b[5]]) as usize;
        let generation = u64_at(b, 8);
        let body = if level == 0 {
            if HEADER + n * LEAF_ENTRY > NODE_SIZE {
                return Err(Error::Corrupted);
            }
            let mut items = Vec::with_capacity(n);
            for i in 0..n {
                let e = HEADER + i * LEAF_ENTRY;
                let off = u16::from_le_bytes([b[e + 24], b[e + 25]]) as usize;
                let len = u16::from_le_bytes([b[e + 26], b[e + 27]]) as usize;
                let v = b.get(off..off + len).ok_or(Error::Corrupted)?;
                items.push((get_key(&b[e..e + 24]), v.to_vec()));
            }
            Body::Leaf(items)
        } else {
            if n == 0 || HEADER + n * INTERNAL_ENTRY > NODE_SIZE {
                return Err(Error::Corrupted);
            }
            let mut children = Vec::with_capacity(n);
            for i in 0..n {
                let e = HEADER + i * INTERNAL_ENTRY;
                children.push((
                    get_key(&b[e..e + 24]),
                    Child::Disk(BlockPtr::get(&b[e + 24..e + INTERNAL_ENTRY])),
                ));
            }
            Body::Internal(children)
        };
        Ok(Self {
            generation,
            level,
            body,
        })
    }
}

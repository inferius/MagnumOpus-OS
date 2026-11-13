//! Seznam datasetu svazku, ktere relace vidi (`DATASETS`).
//!
//! Pro postranni panel spravce souboru a prikazy shellu: kazda polozka
//! nese cestu korene datasetu (cesta relace), `Stat` korene (souhrny
//! podstromu s maskami `known` / `hidden` jako u `STAT`) a puvod klonu ve
//! trech stavech - znamy (cesta), skryty (relace puvodni dataset nevidi),
//! nedostupny (puvodni dataset uz neexistuje).
//!
//! Prava: polozka je jen pro dataset, jehoz koren relace vidi (pruchod
//! k nemu projde) a ktery lezi v pripojenych cestach relace. Korenovy
//! dataset (`/`) je v seznamu, kdyz relace vidi cely koren svazku.
//!
//! Cena: fsd hleda cesty datasetu pruchodem adresaru zive verze (podle
//! poctu adresaru ve svazku); volat pri startu a po zmene, ne ve smycce.

use alloc::string::String;
use alloc::vec::Vec;

use libos::{Error, Result};

use crate::{FsClient, Reader, Stat, Writer};

/// Kod operace: (nic) -> pocet u32 + [`DatasetEntry`].
pub const OP: u32 = 93;

/// Nejvic polozek v odpovedi (zbytek se nevrati; ochrana pameti).
pub const MAX_DATASETS: usize = 1024;

/// Puvod datasetu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// Neni klon.
    None,
    /// Klon datasetu s korenem na teto ceste (cesta relace).
    Known(String),
    /// Klon datasetu, ktery relace nevidi (bez prava nebo mimo pripojene
    /// cesty) - hodnota existuje, ale je skryta.
    Hidden,
    /// Klon datasetu, ktery uz neexistuje - hodnota neni k dispozici.
    Gone,
}

impl Origin {
    fn put(&self, w: &mut Writer) {
        match self {
            Origin::None => {
                w.u8(0);
            }
            Origin::Known(p) => {
                w.u8(1).str(p);
            }
            Origin::Hidden => {
                w.u8(2);
            }
            Origin::Gone => {
                w.u8(3);
            }
        }
    }

    fn get(r: &mut Reader) -> Result<Self> {
        Ok(match r.u8()? {
            0 => Origin::None,
            1 => Origin::Known(r.str()?),
            2 => Origin::Hidden,
            3 => Origin::Gone,
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Polozka seznamu datasetu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetEntry {
    /// Cesta korene (cesta relace, `/` = korenovy dataset).
    pub path: String,
    /// Cislo datasetu ve svazku (stabilni po dobu zivota datasetu).
    pub id: u64,
    /// `Stat` korene datasetu (velikost a pocty z `tree_*`, kdyz je
    /// v `known` bit `avail::TREE`).
    pub root: Stat,
    pub origin: Origin,
}

impl DatasetEntry {
    pub fn put(&self, w: &mut Writer) {
        w.str(&self.path).u64(self.id);
        self.root.put(w);
        self.origin.put(w);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self { path: r.str()?, id: r.u64()?, root: Stat::get(r)?, origin: Origin::get(r)? })
    }
}

/// Zakoduje odpoved (nejvic [`MAX_DATASETS`] polozek).
pub fn encode(w: &mut Writer, list: &[DatasetEntry]) {
    let n = list.len().min(MAX_DATASETS);
    w.u32(n as u32);
    for d in &list[..n] {
        d.put(w);
    }
}

/// Precte odpoved; pocet nad [`MAX_DATASETS`] nebo zbytek dat =
/// `INVALID_ARGS`.
pub fn decode(body: &[u8]) -> Result<Vec<DatasetEntry>> {
    let mut r = Reader::new(body);
    let n = r.u32()? as usize;
    if n > MAX_DATASETS {
        return Err(Error::InvalidArgs);
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(DatasetEntry::get(&mut r)?);
    }
    if !r.is_empty() {
        return Err(Error::InvalidArgs);
    }
    Ok(out)
}

impl FsClient {
    /// Datasety, ktere relace vidi, serazene podle cesty.
    pub fn datasets(&self) -> Result<Vec<DatasetEntry>> {
        decode(&self.call(Self::req(OP))?)
    }
}

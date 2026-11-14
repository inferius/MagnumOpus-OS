//! Blok parametru procesu (`abi::params`): argumenty, prostredi
//! a pocatecni pracovni adresar.
//!
//! Rodic blok vytvori ([`Params::to_vmo`]) a preda ho ve startovni zprave
//! ([`Params::start_handle`]); potomek ho najde v `Startup::params`
//! a precte ([`Params::read`]). Aplikace se `std` to nepotrebuji volat:
//! `std::env::args` a spol. ctou blok samy.

use alloc::vec;
use alloc::vec::Vec;

use abi::params as fmt;
use abi::{Error, Result, Rights};

use crate::Vmo;
use crate::spawn::StartHandle;

/// Parametry procesu (retezce jako bajty; v systemu UTF-8).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Params {
    /// Pocatecni pracovni adresar (prazdny = koren).
    pub cwd: Vec<u8>,
    /// Argumenty (prvni je obvykle jmeno programu).
    pub args: Vec<Vec<u8>>,
    /// Polozky prostredi `JMENO=HODNOTA`.
    pub env: Vec<Vec<u8>>,
    /// Priznaky (`abi::params::flags`, napr. zahodit stdout).
    pub flags: u32,
}

impl Params {
    /// Precte blok z objektu pameti.
    pub fn read(vmo: &Vmo) -> Result<Self> {
        let mut header = [0u8; fmt::HEADER_SIZE];
        vmo.read(0, &mut header)?;
        let size = fmt::Params::size(&header).ok_or(Error::Corrupted)?;
        let mut data = vec![0u8; size];
        vmo.read(0, &mut data)?;
        let p = fmt::Params::parse(&data).ok_or(Error::Corrupted)?;
        Ok(Self {
            cwd: p.cwd.to_vec(),
            flags: p.flags,
            args: p.args().map(<[u8]>::to_vec).collect(),
            env: p.env().map(<[u8]>::to_vec).collect(),
        })
    }

    /// Zakoduje blok (format `abi::params`).
    pub fn encode(&self) -> Result<Vec<u8>> {
        let args = self.args.iter().map(Vec::as_slice);
        let env = self.env.iter().map(Vec::as_slice);
        let len = fmt::encoded_len(&self.cwd, args.clone(), env.clone());
        if len > fmt::MAX_SIZE {
            return Err(Error::InvalidArgs);
        }
        let mut out = vec![0u8; len];
        fmt::encode(&mut out, self.flags, &self.cwd, args, env).ok_or(Error::InvalidArgs)?;
        Ok(out)
    }

    /// Objekt pameti s blokem (handle jen pro cteni a predani).
    pub fn to_vmo(&self) -> Result<Vmo> {
        let data = self.encode()?;
        let vmo = Vmo::create(data.len() as u64)?;
        vmo.write(0, &data)?;
        let ro = vmo.handle().duplicate(Some(Rights::READ | Rights::TRANSFER | Rights::DUPLICATE))?;
        Ok(Vmo::from_handle(ro))
    }

    /// Polozka startovni zpravy s blokem (`kind::PARAMS`).
    pub fn start_handle(&self) -> Result<StartHandle> {
        Ok(StartHandle::new(abi::bootstrap::kind::PARAMS, 0, self.to_vmo()?.into_handle()))
    }
}

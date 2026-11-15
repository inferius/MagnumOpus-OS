//! Relace aplikace u `audiod` (docs/cs/zvuk.md, kapitola Relace aplikaci
//! a nahravani).
//!
//! Init vytvori kazde aplikaci s oknem relaci pres supervisor
//! (`Supervisor::session("audiod", recept)`, listek pro obnovu) a preda ji
//! jako handle se stitkem `abi::params::tag::AUDIO`. Recept rika, kdo
//! klient je a co smi; po restartu `audiod` ho supervisor posle znovu, takze
//! prava preziji obnovu. Prazdny recept = systemova relace (init, lista,
//! shell, nastaveni) se vsemi pravy jako dosud.
//!
//! Aplikace smi jen vlastni streamy: `OPEN`, `START`, `STOP`, `CLOSE`,
//! `DRAIN`, `STREAM_VOLUME`, `STREAM_INFO`, `STATUS`, `DEVICES`. Hlavni
//! hlasitost, ztlumeni, vyber zarizeni a nove kanaly (`CONNECT`) jen
//! system. Nahravani: bez klice `[audio] capture = true` v manifestu
//! `ACCESS_DENIED`; s nim se `audiod` pri prvnim `OPEN` vstupu zepta sluzby
//! uctu (ulozene rozhodnuti, jinak systemovy dialog).

use alloc::string::String;
use alloc::vec::Vec;

use wire::{Reader, Writer};

use crate::client::{Request, op};
use crate::{Direction, Error, Result};

/// Nahravani podle manifestu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Capture {
    /// Manifest nahravani nechce: `OPEN` vstupu = `ACCESS_DENIED`.
    No = 0,
    /// Manifest chce: rozhodne uzivatel (ulozene rozhodnuti nebo dialog).
    Ask = 1,
}

/// Recept relace aplikace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    /// Identita z manifestu (`[app] id`), pod ni se uklada rozhodnuti.
    pub app_id: String,
    /// Jmeno pro dialog (`[app] name`).
    pub app_name: String,
    /// Uzivatel relace (identita z `accountd`).
    pub uid: u64,
    pub capture: Capture,
}

/// Znacka receptu aplikace (prazdny recept = systemova relace).
const APP_TAG: u8 = 1;
/// Nejdelsi identita a jmeno aplikace v receptu.
pub const MAX_NAME: usize = 128;

impl Recipe {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(APP_TAG).str(&self.app_id).str(&self.app_name).u64(self.uid).u8(self.capture as u8);
        w.finish()
    }

    /// `None` = systemova relace (prazdny recept).
    pub fn decode(m: &[u8]) -> Result<Option<Self>> {
        if m.is_empty() {
            return Ok(None);
        }
        let mut r = Reader::new(m);
        if r.u8()? != APP_TAG {
            return Err(Error::InvalidArgs);
        }
        let app_id = r.str()?;
        let app_name = r.str()?;
        let uid = r.u64()?;
        let capture = match r.u8()? {
            0 => Capture::No,
            1 => Capture::Ask,
            _ => return Err(Error::InvalidArgs),
        };
        if app_id.is_empty() || app_id.len() > MAX_NAME || app_name.len() > MAX_NAME || !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(Some(Self { app_id, app_name, uid, capture }))
    }
}

/// Smi relace aplikace poslat tento pozadavek? (`OPEN` vstupu resi
/// [`capture_rule`].)
pub fn app_allowed(req: &Request) -> bool {
    !matches!(req, Request::MasterVolume { .. } | Request::Mute { .. } | Request::Select { .. } | Request::Connect)
}

/// Co udelat s `OPEN` vstupu podle receptu a rozhodnuti, ktere uz relace
/// ma (`granted`: `Some(true)` povoleno, `Some(false)` zamitnuto, `None`
/// jeste se nikdo neptal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureStep {
    Open,
    Deny,
    /// Zeptat se sluzby uctu (a az pak odpovedet).
    Ask,
}

pub fn capture_rule(dir: Direction, capture: Capture, granted: Option<bool>) -> CaptureStep {
    match (dir, capture, granted) {
        (Direction::Output, _, _) => CaptureStep::Open,
        (Direction::Input, Capture::No, _) => CaptureStep::Deny,
        (Direction::Input, Capture::Ask, Some(true)) => CaptureStep::Open,
        (Direction::Input, Capture::Ask, Some(false)) => CaptureStep::Deny,
        (Direction::Input, Capture::Ask, None) => CaptureStep::Ask,
    }
}

/// Kod operace pozadavku (log, chyba).
pub fn op_of(req: &Request) -> u32 {
    match req {
        Request::Open { .. } => op::OPEN,
        Request::Start { .. } => op::START,
        Request::Stop { .. } => op::STOP,
        Request::Close { .. } => op::CLOSE,
        Request::Drain { .. } => op::DRAIN,
        Request::StreamVolume { .. } => op::STREAM_VOLUME,
        Request::MasterVolume { .. } => op::MASTER_VOLUME,
        Request::Mute { .. } => op::MUTE,
        Request::Status => op::STATUS,
        Request::Connect => op::CONNECT,
        Request::StreamInfo { .. } => op::STREAM_INFO,
        Request::Devices => op::DEVICES,
        Request::Select { .. } => op::SELECT,
    }
}

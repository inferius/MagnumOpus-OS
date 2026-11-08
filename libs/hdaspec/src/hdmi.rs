//! HDMI a DisplayPort: ELD (co umi monitor) a audio infoframe.
//!
//! ELD (EDID-Like Data) cte ovladac z pinu verbem GET_HDMI_ELDD po
//! bajtech; platna je, kdyz detekce pinu hlasi ELDV (bit 30). Infoframe
//! rika prijimaci, kolik kanalu a jake rozlozeni posilame; zapisuje se
//! do datoveho ostrova pinu (DIP) a vysila stale (best effort).
//!
//! Format podle specifikace HDA (7.3.3.34-36) a CEA-861 (infoframe typ
//! 0x84); rozlozeni DP podle DisplayPort 1.2 (sekundarni paket).

use alloc::string::String;
use alloc::vec::Vec;

/// Verby HDMI (12bitove).
pub mod verb {
    pub const GET_DIP_SIZE: u16 = 0xF2E;
    pub const GET_ELDD: u16 = 0xF2F;
    pub const SET_DIP_INDEX: u16 = 0x730;
    pub const SET_DIP_DATA: u16 = 0x731;
    pub const SET_DIP_XMIT: u16 = 0x732;
    pub const SET_CVT_CHAN_COUNT: u16 = 0x72D;
    pub const SET_CHAN_SLOT: u16 = 0x734;
    /// GET_DIP_SIZE s timto bitem = velikost bufferu ELD.
    pub const DIPSIZE_ELD: u8 = 1 << 3;
    /// Vysilat infoframe porad (best effort).
    pub const XMIT_BEST: u8 = 0xC0;
}

/// Detekce pinu HDMI: monitor pripojen (bit 31) a ELD platne (bit 30).
pub const SENSE_PRESENT: u32 = 1 << 31;
pub const SENSE_ELDV: u32 = 1 << 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eld {
    pub monitor: String,
    /// Monitor prijima DisplayPort (jinak HDMI).
    pub dp: bool,
    /// Nejvic kanalu LPCM podle SAD (0 = monitor PCM nehlasi).
    pub lpcm_channels: u8,
}

/// Rozebere ELD (verze 2). `None` = kratke nebo neplatne.
pub fn parse_eld(b: &[u8]) -> Option<Eld> {
    if b.len() < 20 || b[0] >> 3 != 2 {
        return None;
    }
    let mnl = (b[4] & 0x1F) as usize;
    let sads = (b[5] >> 4) as usize;
    let dp = (b[5] >> 2) & 3 == 1;
    let name = b.get(20..20 + mnl)?;
    let monitor = name.iter().map(|&c| if c.is_ascii_graphic() || c == b' ' { c as char } else { '?' }).collect();
    let mut lpcm = 0;
    for i in 0..sads {
        let off = 20 + mnl + i * 3;
        let Some(sad) = b.get(off..off + 3) else { break };
        if (sad[0] >> 3) & 0xF == 1 {
            lpcm = lpcm.max((sad[0] & 7) + 1);
        }
    }
    Some(Eld { monitor, dp, lpcm_channels: lpcm })
}

/// Audio infoframe pro `channels` kanalu (rozlozeni CA = 0, tj. FL FR
/// a pripadne dalsi podle poctu). HDMI: typ, verze, delka, soucet,
/// 10 bajtu dat; DP: typ, delka, verze, data.
pub fn audio_infoframe(channels: u8, dp: bool) -> Vec<u8> {
    let cc = channels.clamp(1, 8) - 1;
    if dp {
        // Sekundarni datovy paket DP: 0x84, delka 0x1B, verze 0x11 << 2.
        return alloc::vec![0x84, 0x1B, 0x11 << 2, cc, 0, 0, 0, 0];
    }
    let mut f = alloc::vec![0x84, 0x01, 0x0A, 0, cc, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let sum: u32 = f.iter().map(|&b| b as u32).sum();
    f[3] = (0x100 - (sum & 0xFF)) as u8;
    f
}

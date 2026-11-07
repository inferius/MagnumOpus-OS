//! Korekce od klienta synchronizace (`ntpd`) a pravidla pro ne.
//!
//! Zprava `sync`: `sync` + posun i64 (ns, kladny = mistni hodiny jsou
//! pozadu) + frekvence i64 (ppb, celkova odhadnuta oprava krystalu) +
//! priznaky u8 (bit 0 = frekvence je platna). Odpoved: `ok:step`,
//! `ok:slew` nebo `ok:none` a za ni 24 B stavu (`abi::time::AdjustStatus`),
//! nebo `err:<popis>`.
//!
//! Pravidla (timed je jedina strana s pravem k hodinam, ntpd jen navrhuje):
//! - **Prvni** synchronizace smi skocit, kdyz je posun nad
//!   [`STEP_THRESHOLD_NS`] (hodiny z RTC muzou byt o minuty vedle a plynule
//!   dorovnani by trvalo dny).
//! - Potom uz **nikdy skok**: posun se dorovna plynule (nejvys 500 ppm),
//!   realny cas nikdy nejde pozpatku.
//! - Posun nad [`MAX_SYNC_OFFSET_NS`] po prvni synchronizaci se odmitne:
//!   hodiny se tolik samy nerozejdou, spis jde o podvrzenou odpoved.
//! - Frekvence se omezi na 500 ppm a jedna zmena nejvys o
//!   [`MAX_FREQ_STEP_PPB`].

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libos::hw::Resource;
use libos::time::{self, zone::Settings};
use libos::{Error, println};

/// Nad tento posun smi prvni synchronizace skocit (jako ntpd: 128 ms).
pub const STEP_THRESHOLD_NS: i64 = 128_000_000;
/// Nejvetsi posun, ktery se po prvni synchronizaci jeste dorovna.
pub const MAX_SYNC_OFFSET_NS: i64 = 60_000_000_000;
/// Nejvetsi zmena frekvence jednou korekci.
pub const MAX_FREQ_STEP_PPB: i64 = 100_000;

pub struct Sync {
    synced: bool,
    last_offset: i64,
    updates: u64,
}

impl Sync {
    pub fn new() -> Self {
        Self {
            synced: false,
            last_offset: 0,
            updates: 0,
        }
    }

    pub fn status(&self, clock: &Resource) -> String {
        let s = time::adjust(clock, None, None).unwrap_or_default();
        format!(
            "synced={} updates={} last_offset_ns={} remaining_ns={} freq_ppb={} slew_ppb={}",
            self.synced as u8, self.updates, self.last_offset, s.remaining_ns, s.freq_ppb, s.slew_ppb
        )
    }

    pub fn handle(&mut self, clock: &Resource, settings: &Settings, msg: &[u8]) -> Vec<u8> {
        let Some(rest) = msg.strip_prefix(b"sync").filter(|r| r.len() == 17) else {
            return b"err:neznama zprava".to_vec();
        };
        let offset = i64::from_le_bytes(rest[0..8].try_into().unwrap());
        let freq = i64::from_le_bytes(rest[8..16].try_into().unwrap());
        let freq_valid = rest[16] & 1 != 0;
        match self.apply(clock, settings, offset, freq_valid.then_some(freq)) {
            Ok((kind, status)) => {
                let mut out = format!("ok:{kind}").into_bytes();
                out.extend_from_slice(&status.encode());
                out
            }
            Err(e) => format!("err:{e}").into_bytes(),
        }
    }

    fn apply(
        &mut self,
        clock: &Resource,
        settings: &Settings,
        offset: i64,
        freq: Option<i64>,
    ) -> Result<(&'static str, libos::abi::time::AdjustStatus), Error> {
        use libos::abi::time::adjust::{MAX_FREQ_PPB, MAX_OFFSET_NS};
        let current = time::adjust(clock, None, None)?;
        let freq = freq.map(|f| {
            let step = (f - current.freq_ppb).clamp(-MAX_FREQ_STEP_PPB, MAX_FREQ_STEP_PPB);
            (current.freq_ppb + step).clamp(-MAX_FREQ_PPB, MAX_FREQ_PPB)
        });
        let kind = if !self.synced && offset.abs() > STEP_THRESHOLD_NS {
            let now = time::wall().ok_or(Error::BadState)?;
            time::set(clock, now.add_ns(offset as i128), settings, false)?;
            println!("timed: prvni synchronizace: skok o {} ms", offset / 1_000_000);
            if let Some(f) = freq {
                time::adjust(clock, None, Some(f))?;
            }
            "step"
        } else if offset.abs() > MAX_SYNC_OFFSET_NS.min(MAX_OFFSET_NS) {
            println!(
                "timed: korekce o {} ms odmitnuta (hodiny se tolik nerozejdou, podvrzena odpoved?)",
                offset / 1_000_000
            );
            return Err(Error::OutOfRange);
        } else {
            time::adjust(clock, Some(offset), freq)?;
            if !self.synced {
                println!("timed: prvni synchronizace: dorovnani o {} ms", offset / 1_000_000);
            }
            "slew"
        };
        self.synced = true;
        self.last_offset = offset;
        self.updates += 1;
        Ok((kind, time::adjust(clock, None, None)?))
    }
}

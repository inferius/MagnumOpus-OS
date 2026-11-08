//! Seznam rezimu monitoru ze vsech zdroju EDID: detailni casovani
//! (zakladni blok, CTA, DisplayID), kody VIC, standardni a ustalena
//! casovani (tabulka DMT, jinak vypocet CVT).
//!
//! Poradi: preferovany rezim prvni, pak od nejvetsiho rozliseni, pri
//! shode od nejvyssi obnovy. Duplicity (stejne rozmery a obnova do 0.5 Hz)
//! se slouci - vyhrava presnejsi zdroj (detailni > VIC > DMT > CVT).

use alloc::vec::Vec;

use crate::cvt::{Blanking, cvt};
use crate::dmt;
use crate::edid::Edid;
use crate::timing::{Source, Timing, flags};

/// Rezim s puvodem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub timing: Timing,
    pub source: Source,
}

/// Omezeni zdroje (ovladace): nejvyssi pixel clock a rozmery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_clock_khz: u32,
    pub max_w: u16,
    pub max_h: u16,
    /// Pripustit prokladane rezimy.
    pub interlaced: bool,
}

impl Limits {
    pub const fn new(max_clock_khz: u32, max_w: u16, max_h: u16) -> Self {
        Self {
            max_clock_khz,
            max_w,
            max_h,
            interlaced: false,
        }
    }

    fn allows(&self, t: &Timing) -> bool {
        t.is_valid()
            && t.clock_khz <= self.max_clock_khz
            && t.hactive <= self.max_w
            && t.vactive <= self.max_h
            && (self.interlaced || !t.interlaced())
    }
}

/// Casovani pro rozmery a obnovu bez detailniho popisu: DMT, jinak CVT.
pub fn resolve(w: u16, h: u16, refresh_hz: u32) -> Option<Mode> {
    if let Some(e) = dmt::find(w as u32, h as u32, refresh_hz, None) {
        return Some(Mode {
            timing: e.timing,
            source: Source::Dmt,
        });
    }
    cvt(w as u32, h as u32, refresh_hz * 1000, Blanking::Standard).map(|timing| Mode { timing, source: Source::Cvt })
}

/// Vsechny rezimy monitoru v mezich `limits`, serazene (viz modul).
pub fn modes(edid: &Edid, limits: &Limits) -> Vec<Mode> {
    let mut all: Vec<Mode> = Vec::new();
    let detailed = |t: Timing| Mode {
        timing: t,
        source: Source::Detailed,
    };
    all.extend(edid.detailed.iter().map(|d| detailed(d.timing)));
    if let Some(c) = edid.cta() {
        all.extend(c.detailed.iter().map(|d| detailed(d.timing)));
        all.extend(c.vic_timings().into_iter().map(|t| Mode {
            timing: t,
            source: Source::Vic,
        }));
    }
    for d in edid.displayid() {
        all.extend(d.timings.iter().map(|t| detailed(*t)));
    }
    for s in &edid.standard {
        all.extend(resolve(s.w, s.h, s.refresh_hz as u32));
    }
    for &(w, h, r, interlaced) in &edid.established {
        if !interlaced {
            all.extend(resolve(w, h, r as u32));
        }
    }
    // Monitor sam omezuje pixel clock (deskriptor rozsahu).
    let monitor_max = edid.range.map(|r| r.max_clock_khz).filter(|&c| c > 0).unwrap_or(u32::MAX);
    all.retain(|m| limits.allows(&m.timing) && m.timing.clock_khz <= monitor_max.max(edid.preferred().map_or(0, |p| p.clock_khz)));
    // Slouceni: stabilni razeni podle zdroje, pak prvni vyskyt vyhrava.
    all.sort_by_key(|m| m.source);
    let mut out: Vec<Mode> = Vec::new();
    for m in all {
        if let Some(prev) = out.iter_mut().find(|p| p.timing.same_mode(&m.timing)) {
            // Priznak preferovaneho z kterehokoli zdroje.
            prev.timing.flags |= m.timing.flags & flags::PREFERRED;
            continue;
        }
        out.push(m);
    }
    // Preferovany je jen jeden: prvni detailni s priznakem.
    let mut seen = false;
    let pref = edid.preferred().copied();
    for m in &mut out {
        let is = pref.is_some_and(|p| p.same_mode(&m.timing));
        if is && !seen {
            m.timing.flags |= flags::PREFERRED;
            seen = true;
        } else {
            m.timing.flags &= !flags::PREFERRED;
        }
    }
    out.sort_by(|a, b| {
        let key = |m: &Mode| (!m.timing.preferred(), core::cmp::Reverse(m.timing.hactive as u32 * m.timing.vactive as u32), core::cmp::Reverse(m.timing.refresh_mhz()));
        key(a).cmp(&key(b))
    });
    out
}

/// Nejlepsi rezim pro pozadavek `w` x `h` (obnova `refresh_mhz`, 0 =
/// nejvyssi): presna shoda rozmeru, obnova nejblize pozadavku.
pub fn pick(modes: &[Mode], w: u16, h: u16, refresh_mhz: u32) -> Option<&Mode> {
    modes
        .iter()
        .filter(|m| m.timing.hactive == w && m.timing.vactive == h)
        .min_by_key(|m| if refresh_mhz == 0 { u32::MAX - m.timing.refresh_mhz() } else { m.timing.refresh_mhz().abs_diff(refresh_mhz) })
}

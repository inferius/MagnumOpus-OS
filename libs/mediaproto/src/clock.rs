//! Hodiny pro A/V synchronizaci (bez OS, testovatelne na hostiteli).
//!
//! Hlavni hodiny jsou zvuk: kotva [`AudioAnchor`] (udalost
//! `AUDIO_BASE`) rika, ze ramec kruhu na pozici `ring_pos` ma cas
//! `pts_us`. Slyset je ramec `played - latence * frekvence`, takze cas,
//! ktery prave zni, je `pts_us + (slysene - ring_pos) / frekvence`. Bez
//! zvuku jsou hodinami monotonni cas od startu ([`WallClock`]).

/// Velky skok mezi hodinami a videem: vyrovnat skokem, ne zahazovanim.
pub const RESYNC_US: u64 = 200_000;

/// Kotva zvuku: pozice v kruhu (ramce) a jeji cas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioAnchor {
    pub ring_pos: u64,
    pub pts_us: u64,
}

/// Cas zvuku, ktery prave zni. `played` = prehrane ramce z hlavicky kruhu,
/// `latency_us` = latence vystupu za nim. Pred kotvou (zvuk jeste nehraje
/// od teto pozice) vraci cas kotvy.
pub fn audio_now_us(a: AudioAnchor, played: u64, latency_us: u64, rate: u32) -> u64 {
    if rate == 0 {
        return a.pts_us;
    }
    let lat_frames = latency_us.saturating_mul(rate as u64) / 1_000_000;
    let heard = played.saturating_sub(lat_frames);
    if heard <= a.ring_pos {
        return a.pts_us;
    }
    let frames = heard - a.ring_pos;
    a.pts_us.saturating_add(frames.saturating_mul(1_000_000) / rate as u64)
}

/// Hodiny bez zvuku: cas medii = `base_pts + (ted - start)`, zastavene
/// pri pauze.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WallClock {
    base_pts: u64,
    start_ns: u64,
    running: bool,
}

impl WallClock {
    pub fn new(pts_us: u64) -> Self {
        Self { base_pts: pts_us, start_ns: 0, running: false }
    }

    pub fn now_us(&self, now_ns: u64) -> u64 {
        if !self.running {
            return self.base_pts;
        }
        self.base_pts.saturating_add(now_ns.saturating_sub(self.start_ns) / 1000)
    }

    pub fn play(&mut self, now_ns: u64) {
        if !self.running {
            self.start_ns = now_ns;
            self.running = true;
        }
    }

    pub fn pause(&mut self, now_ns: u64) {
        self.base_pts = self.now_us(now_ns);
        self.running = false;
    }

    pub fn seek(&mut self, pts_us: u64, now_ns: u64) {
        self.base_pts = pts_us;
        self.start_ns = now_ns;
    }

    pub fn running(&self) -> bool {
        self.running
    }
}

/// Co udelat se snimkem pri vymene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Snimek patri na tuto vymenu.
    Show,
    /// Snimek je napred: nechat na pristi vymenu.
    Hold,
    /// Snimek je pozadu: zahodit (uvolnit buffer) a vzit dalsi.
    Drop,
    /// Odchylka nad [`RESYNC_US`]: ukazat hned a srovnat hodiny / kruh.
    Resync,
}

/// Rozhodne o snimku `pts_us` s trvanim `duration_us`, kdyz vymena
/// probehne v case medii `at_us` (hodiny + cas do deadline kompozitoru).
/// Snimek patri na vymenu, kdyz `at_us` padne do jeho intervalu
/// `[pts - dur/2, pts + dur)`; pozdeji = pozadu, drive = napred.
pub fn decide(pts_us: u64, duration_us: u32, at_us: u64) -> Decision {
    let dur = (duration_us as u64).max(1);
    let diff = pts_us.abs_diff(at_us);
    if diff > RESYNC_US {
        // Napred o hodne (napr. po skoku hodin) i pozadu o hodne: ukazat.
        return Decision::Resync;
    }
    if pts_us.saturating_add(dur) <= at_us {
        return Decision::Drop;
    }
    if pts_us > at_us.saturating_add(dur / 2) {
        return Decision::Hold;
    }
    Decision::Show
}

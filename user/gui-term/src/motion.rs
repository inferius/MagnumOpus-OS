//! Pohyb v okne terminalu: novy blok se prolne (`motion.base`, krivka
//! `ease_enter`; navrh `.blk` `blk-in`). Posun o 6 px z navrhu mrizka
//! bunek nedela; prolnuti je zaroven nahrada posunu pro "Omezit pohyb"
//! (`motion.reduced`). Bloky z prehrani pri pripojeni se neprolinaji.

use alloc::vec::Vec;

use term::ui::draw::Line;
use theme::Motion;

use crate::blockact::block_of;

/// Kolik novych bloku najednou je jeste beh prikazu; vic = prehrani
/// relace po pripojeni (bez animace).
const MAX_NEW: usize = 3;

/// Kdy se ktery blok objevil (ns); `0` = bez animace.
#[derive(Default)]
pub struct Fade {
    born: Vec<(u64, u64)>,
}

impl Fade {
    /// Zapise bloky snimku: nove dostanou cas `now`, zmizele se zapomenou.
    pub fn update(&mut self, lines: &[Line], now: u64) {
        let mut seen: Vec<u64> = lines.iter().filter_map(|l| block_of(&l.role)).collect();
        seen.dedup();
        let new: Vec<u64> = seen.iter().copied().filter(|b| !self.born.iter().any(|(x, _)| x == b)).collect();
        let at = if new.len() > MAX_NEW || self.born.is_empty() && new.len() > 1 { 0 } else { now };
        self.born.retain(|(b, _)| seen.contains(b));
        self.born.extend(new.into_iter().map(|b| (b, at)));
    }

    /// Kryti prekryvu barvou pozadi (0..=255) pro kazdy radek: 255 =
    /// blok jeste neviditelny, 0 = hotovo.
    pub fn veils(&self, lines: &[Line], now: u64, m: &Motion) -> Vec<u32> {
        let dur = m.base as u64 * 1_000_000;
        lines
            .iter()
            .map(|l| {
                let Some(b) = block_of(&l.role) else { return 0 };
                match self.born.iter().find(|(x, _)| *x == b) {
                    Some(&(_, t)) if t > 0 && now < t + dur && dur > 0 => {
                        let p = (now - t) as f32 / dur as f32;
                        ((1.0 - m.ease_enter.at(p)) * 255.0) as u32
                    }
                    _ => 0,
                }
            })
            .collect()
    }

    /// Bezi nejake prolnuti (okno potrebuje dalsi snimky)?
    pub fn active(&self, now: u64, m: &Motion) -> bool {
        let dur = m.base as u64 * 1_000_000;
        self.born.iter().any(|&(_, t)| t > 0 && now < t + dur)
    }
}

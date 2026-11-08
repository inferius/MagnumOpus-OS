//! fsd jako producent prehledu uloh (`taskd`): scrub (kontrola dat
//! s prubehem a vysledkem) a deduplikace s defragmentaci v necinnosti.
//!
//! Kanal producenta posle init po startu taskd na ridici (bootstrap)
//! kanal zpravou `tasks` s handlem. Bez nej fsd ulohy nehlasi.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libos::{Channel, time};
use taskproto::{Control, Origin, Produce, State, actions};

/// Jak casto posilat prubeh scrubu.
const UPDATE_NS: u64 = 2_000_000_000;

#[derive(Default)]
struct Slot {
    /// Cislo pozadavku registrace a cislo ulohy (po REGISTERED).
    req: u32,
    id: Option<u64>,
    /// Konec driv, nez prislo REGISTERED.
    early: Option<State>,
    last_update: u64,
}

#[derive(Default)]
pub struct FsTasks {
    pub chan: Option<Channel>,
    next_req: u32,
    scrub: Option<Slot>,
    optimize: Option<Slot>,
    /// Pozadavek na zruseni scrubu od taskd.
    pub cancel_scrub: bool,
    /// Ulohy, ktere skoncily pred REGISTERED.
    pending_end: Vec<Slot>,
    /// Posledni hlaseni (fsd ho dela nejvys jednou za `REPORT_NS`).
    last_report: u64,
}

/// Jak casto nejvys hlasit stav (mimo cestu pozadavku).
const REPORT_NS: u64 = 500_000_000;

impl FsTasks {
    /// Je cas na hlaseni? (A posune cas posledniho.)
    pub fn due(&mut self) -> bool {
        let now = time::now();
        if now.saturating_sub(self.last_report) < REPORT_NS {
            return false;
        }
        self.last_report = now;
        true
    }
}

impl FsTasks {
    fn send(&mut self, m: &Produce) {
        if let Some(c) = &self.chan
            && c.write(&m.encode(), Vec::new()).is_err()
        {
            self.chan = None;
        }
    }

    fn start(&mut self, name: &str, acts: u32) -> Option<Slot> {
        self.chan.as_ref()?;
        self.next_req = self.next_req.wrapping_add(1).max(1);
        let req = self.next_req;
        self.send(&Produce::Register {
            req,
            parent: 0,
            name: name.into(),
            origin: Origin::Service,
            who: String::from("fsd"),
            agent: false,
            job: None,
            attach: None,
            actions: acts,
        });
        Some(Slot { req, ..Slot::default() })
    }

    fn end(&mut self, slot: Option<Slot>, state: State) {
        let Some(mut s) = slot else { return };
        match s.id {
            Some(id) => self.send(&Produce::Finish { id, state }),
            None => {
                // Pocka na REGISTERED (`pump`).
                s.early = Some(state);
                self.pending_end.push(s);
            }
        }
    }

    /// Odpovedi taskd (REGISTERED, CONTROL).
    pub fn pump(&mut self) {
        loop {
            let Some(c) = &self.chan else { return };
            let m = match c.try_read() {
                Ok(m) => m,
                Err(libos::Error::ShouldWait) => return,
                Err(_) => {
                    self.chan = None;
                    return;
                }
            };
            if let Ok((req, id)) = taskproto::decode_registered(&m.data) {
                for s in [self.scrub.as_mut(), self.optimize.as_mut()].into_iter().flatten() {
                    if s.req == req {
                        s.id = Some(id);
                    }
                }
                if let Some(i) = self.pending_end.iter().position(|s| s.req == req) {
                    let s = self.pending_end.remove(i);
                    if let Some(st) = s.early {
                        self.send(&Produce::Finish { id, state: st });
                    }
                }
            } else if let Ok(Control::Cancel(id)) = Control::decode(&m.data)
                && self.scrub.as_ref().is_some_and(|s| s.id == Some(id))
            {
                self.cancel_scrub = true;
            }
        }
    }

    /// Stav scrubu po kroku nebo po kole: zacatek, prubeh, konec.
    pub fn scrub(&mut self, st: &fs::datarec::ScrubState, used_blocks: u64) {
        let running = st.state == fs::datarec::SCRUB_RUNNING;
        match (running, self.scrub.is_some()) {
            (true, false) => {
                self.scrub = self.start("Scrub: kontrola všech dat", actions::CANCEL);
            }
            (true, true) => {
                let now = time::now();
                let s = self.scrub.as_mut().unwrap();
                if let Some(id) = s.id
                    && now.saturating_sub(s.last_update) >= UPDATE_NS
                {
                    s.last_update = now;
                    let pct = (st.scanned.saturating_mul(100) / used_blocks.max(1)).min(99) as u8;
                    self.send(&Produce::Update { id, state: State::Running, progress: Some(pct), eta_ms: None });
                }
            }
            (false, true) => {
                let slot = self.scrub.take();
                let text = format!("zkontrolováno {} bloků, opraveno {}, neopravitelné {}", st.scanned, st.repaired, st.lost);
                let state = if st.state == fs::datarec::SCRUB_STOPPED {
                    State::Cancelled
                } else if st.lost > 0 {
                    State::Failed { code: libos::Error::Corrupted as i32, message: text }
                } else {
                    State::Done(text)
                };
                self.end(slot, state);
            }
            (false, false) => {}
        }
    }

    /// Prace v necinnosti (defragmentace a deduplikace) zacala nebo skoncila.
    pub fn optimize(&mut self, running: bool) {
        match (running, self.optimize.is_some()) {
            (true, false) => self.optimize = self.start("Defragmentace a deduplikace", 0),
            (false, true) => {
                let slot = self.optimize.take();
                self.end(slot, State::Done(String::new()));
            }
            _ => {}
        }
    }
}

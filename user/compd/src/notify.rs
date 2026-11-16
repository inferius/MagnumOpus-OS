//! Prohlizece oznameni (`notifyd`, `docs/cs/oznameni.md`): jeden kanal na
//! uzivatele relace (prikaz initu `notify <uid>`). Zmeny jdou do
//! `Comp::notify_event` (vyskakovaci karty `comp::toast`), pozadavky
//! (`Comp::take_notify_requests`: krizek, akce, Seen) zpet prohlizeci
//! uzivatele. Kompozitor necheka: zprava do plne fronty se zahodi.

use alloc::vec::Vec;

use libos::{Channel, Error, Signals};
use notifyproto::{View, ViewReply};

#[derive(Default)]
pub struct Viewers {
    pub list: Vec<(u64, Channel)>,
}

impl Viewers {
    /// Novy prohlizec uzivatele (nahradi stary: restart notifyd nebo initu).
    pub fn add(&mut self, uid: u64, ch: Channel) {
        let _ = ch.write(&View::Subscribe.encode(), Vec::new());
        self.list.retain(|(u, _)| *u != uid);
        self.list.push((uid, ch));
    }

    /// Precte zmeny prohlizecu se signalem (`obs` ve stejnem poradi jako
    /// `list`); zavrene kanaly odebere.
    pub fn read(&mut self, obs: &[Signals], comp: &mut comp::Comp, now: u64) {
        let mut dead = Vec::new();
        for (i, (uid, ch)) in self.list.iter().enumerate() {
            if !obs.get(i).is_some_and(|s| s.intersects(Signals::READABLE | Signals::PEER_CLOSED)) {
                continue;
            }
            loop {
                match ch.try_read() {
                    Ok(m) => match ViewReply::decode(&m.data) {
                        Ok(r) => comp.notify_event(*uid, r, now),
                        Err(e) => libos::println!("compd: neplatna zprava notifyd: {e}"),
                    },
                    Err(Error::ShouldWait) => break,
                    Err(_) => {
                        dead.push(i);
                        break;
                    }
                }
            }
        }
        for i in dead.into_iter().rev() {
            self.list.remove(i);
        }
    }

    /// Pozadavky karet do notifyd.
    pub fn send(&self, reqs: Vec<(u64, View)>) {
        for (uid, v) in reqs {
            if let Some((_, ch)) = self.list.iter().find(|(u, _)| *u == uid) {
                let _ = ch.write(&v.encode(), Vec::new());
            }
        }
    }
}

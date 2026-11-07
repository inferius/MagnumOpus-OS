//! Takt snimku, animace, skupiny transakci, terminy `CONFIGURE` a ping.
//!
//! Kompozitor tika jen tehdy, kdyz neco ceka (poskozena oblast, animace,
//! zadost o snimek). Tik: vyprsene terminy, animace, slozeni poskozenych
//! oblasti, `FRAME` klientum, kteri o nej zadali. Jeden snimek je vzdy
//! nejvys jeden "na ceste" k displeji.

use alloc::vec::Vec;

use guiproto::Rect;
use guiproto::window::{ClientMsg, ServerMsg};

use crate::scene::anim_value;
use crate::{COMPOSE_BUDGET, ClientId, Comp, PERIOD, SYNC_TIMEOUT};

/// Interval pingu a doba bez odpovedi, po ktere je aplikace nereagujici.
pub const PING_INTERVAL: u64 = 2_000_000_000;
pub const PING_TIMEOUT: u64 = 3_000_000_000;

/// Podrzeny commit clena skupiny.
pub(crate) struct Held {
    client: ClientId,
    ops: Vec<ClientMsg>,
    txn: u32,
    flags: u32,
    window: u32,
    serial: u32,
}

pub(crate) struct Group {
    owner: ClientId,
    token: u128,
    count: u32,
    deadline: u64,
    held: Vec<Held>,
}

#[derive(Default)]
pub struct FrameState {
    pub frames: u64,
    pub presented: u64,
    pub seq: u64,
    pub last_tick: u64,
    /// Snimek odeslany displeji, cekame na `PRESENTED`.
    pub in_flight: bool,
    pub(crate) groups: Vec<Group>,
    /// Kdo ceka na ustaleni (inspekce `Settle`).
    pub(crate) settle: Vec<crate::PeerId>,
}

/// Nejvic soucasnych skupin a clenu skupiny.
const MAX_GROUPS: usize = 64;
const MAX_GROUPS_PER_CLIENT: usize = 4;
const MAX_GROUP_SIZE: u32 = 16;

impl Comp {
    pub(crate) fn new_group(&mut self, owner: ClientId, count: u32, now: u64) -> u128 {
        let token = (self.token_source)();
        // Limit na klienta: uvolni se jen jeho vlastni nejstarsi skupina
        // (cizi skupiny nikdo predcasne uvolnit nemuze).
        let own = self.frame.groups.iter().filter(|g| g.owner == owner).count();
        if own >= MAX_GROUPS_PER_CLIENT || self.frame.groups.len() >= MAX_GROUPS {
            if let Some(i) = self.frame.groups.iter().position(|g| g.owner == owner) {
                let g = self.frame.groups.remove(i);
                self.release_group(g, now);
            } else {
                self.error(owner, guiproto::window::op::SYNC_GROUP, guiproto::Error::NoResources);
                return 0;
            }
        }
        self.frame.groups.push(Group {
            owner,
            token,
            count: count.clamp(1, MAX_GROUP_SIZE),
            deadline: now + SYNC_TIMEOUT,
            held: Vec::new(),
        });
        token
    }

    /// Commit clena skupiny: `true` = podrzen (nebo uplatnen se skupinou).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn frame_hold(
        &mut self,
        client: ClientId,
        ops: Vec<ClientMsg>,
        txn: u32,
        flags: u32,
        window: u32,
        serial: u32,
        token: u128,
        now: u64,
    ) -> bool {
        let Some(i) = self.frame.groups.iter().position(|g| g.token == token) else {
            self.error(client, guiproto::window::op::COMMIT, guiproto::Error::NotFound);
            return false;
        };
        let g = &mut self.frame.groups[i];
        g.held.push(Held {
            client,
            ops,
            txn,
            flags,
            window,
            serial,
        });
        if g.held.len() as u32 >= g.count {
            let g = self.frame.groups.remove(i);
            self.release_group(g, now);
        }
        true
    }

    fn release_group(&mut self, g: Group, now: u64) {
        // Vsechny commity skupiny najednou (pred dalsim skladanim).
        for h in g.held {
            if self.clients.contains_key(&h.client) {
                self.apply(h.client, h.ops, h.txn, h.flags, h.window, h.serial, now);
            }
        }
    }

    /// Nejblizsi okamzik, kdy ma sluzba zavolat [`Comp::tick`] (`None` =
    /// nic neceka, spat do udalosti).
    pub fn next_wakeup(&self, now: u64) -> Option<u64> {
        let mut t: Option<u64> = None;
        let mut at = |v: u64| t = Some(t.map_or(v, |x| x.min(v)));
        let animating = self.clients.values().any(|c| c.layers.values().any(|l| !l.anim.is_empty()));
        let frames = self.clients.values().any(|c| c.frame_wanted);
        if !self.frame.in_flight && (!self.dirty.is_empty() || animating || frames) {
            at(self.next_tick_time(now));
        }
        for g in &self.frame.groups {
            at(g.deadline);
        }
        for w in self.windows.values() {
            if let Some((_, _, _, d)) = w.configure {
                at(d);
            }
            if w.badge_until > now {
                at(w.badge_until);
            }
        }
        for c in self.clients.values() {
            match c.ping {
                Some((_, sent)) if !c.unresponsive => at(sent + PING_TIMEOUT),
                None => at(c.last_ping + PING_INTERVAL),
                _ => {}
            }
        }
        if let Some(d) = self.dialogs.next_wakeup() {
            at(d);
        }
        t.map(|v| v.max(now))
    }

    fn next_tick_time(&self, now: u64) -> u64 {
        let next = self.frame.last_tick + PERIOD;
        if self.frame.last_tick == 0 || next <= now { now } else { next }
    }

    /// Casove veci, ktere nezavisi na snimku (skupiny, configure, ping).
    pub fn timers(&mut self, now: u64) {
        self.input_now = now;
        let (late, keep): (Vec<_>, Vec<_>) = core::mem::take(&mut self.frame.groups).into_iter().partition(|g| g.deadline <= now);
        self.frame.groups = keep;
        for g in late {
            self.release_group(g, now);
        }
        // Aplikace nestihla potvrdit zmenu velikosti: novy ram, stary obsah.
        let late: Vec<u32> = self
            .windows
            .iter()
            .filter(|(_, w)| w.configure.is_some_and(|(_, _, _, d)| d <= now))
            .map(|(&g, _)| g)
            .collect();
        for g in late {
            self.damage_window(g);
            let w = self.windows.get_mut(&g).unwrap();
            let (_, nw, nh, _) = w.configure.take().unwrap();
            w.rect.w = nw;
            w.rect.h = nh;
            self.damage_window(g);
        }
        let badges: Vec<u32> = self.windows.iter().filter(|(_, w)| w.badge_until != 0 && w.badge_until <= now).map(|(&g, _)| g).collect();
        for g in badges {
            self.windows.get_mut(&g).unwrap().badge_until = 0;
            self.damage_window(g);
        }
        let ids: Vec<ClientId> = self.clients.keys().copied().collect();
        for id in ids {
            let c = self.clients.get_mut(&id).unwrap();
            match c.ping {
                None if now >= c.last_ping + PING_INTERVAL => {
                    c.ping_serial = c.ping_serial.wrapping_add(1);
                    c.ping = Some((c.ping_serial, now));
                    c.last_ping = now;
                    let s = c.ping_serial;
                    self.send(id, ServerMsg::Ping { serial: s });
                }
                Some((_, sent)) if !c.unresponsive && now >= sent + PING_TIMEOUT => {
                    c.unresponsive = true;
                    let wins: Vec<u32> = c.windows.values().copied().collect();
                    for g in wins {
                        self.damage_window(g);
                    }
                }
                _ => {}
            }
        }
        self.dialogs_timers(now);
    }

    /// Snimek: animace, slozeni poskozenych oblasti do `canvas`, `FRAME`.
    /// Vraci obdelniky, ktere je treba poslat displeji (`None` = nic se
    /// neslozilo; pak sluzba nema cekat na `PRESENTED`).
    pub fn tick(&mut self, now: u64, canvas: &mut gfx::Canvas<'_>) -> Option<Vec<Rect>> {
        self.timers(now);
        if self.frame.in_flight {
            return None;
        }
        self.frame.last_tick = now;
        self.animate(now);
        let rects = if self.dirty.is_empty() {
            None
        } else {
            let rects = self.dirty.take();
            self.compose(canvas, &rects, now);
            self.frame.frames += 1;
            Some(rects)
        };
        // Transakce slozene v tomto snimku cekaji na zobrazeni.
        for c in self.clients.values_mut() {
            let w = core::mem::take(&mut c.txn_waiting);
            c.txn_in_flight.extend(w);
        }
        self.frame.seq += 1;
        let seq = self.frame.seq;
        let ids: Vec<ClientId> = self.clients.iter().filter(|(_, c)| c.frame_wanted).map(|(&i, _)| i).collect();
        for id in ids {
            self.clients.get_mut(&id).unwrap().frame_wanted = false;
            self.send(
                id,
                ServerMsg::Frame {
                    seq,
                    deadline: now + PERIOD - COMPOSE_BUDGET,
                    period: PERIOD,
                },
            );
        }
        match rects {
            Some(r) => {
                self.frame.in_flight = true;
                Some(r)
            }
            None => {
                self.presented(now);
                None
            }
        }
    }

    /// Displej snimek zobrazil.
    pub fn presented(&mut self, time: u64) {
        if self.frame.in_flight {
            self.frame.presented += 1;
        }
        self.frame.in_flight = false;
        let ids: Vec<ClientId> = self.clients.keys().copied().collect();
        for id in ids {
            let txns = core::mem::take(&mut self.clients.get_mut(&id).unwrap().txn_in_flight);
            for txn in txns {
                self.send(id, ServerMsg::Presented { txn, time });
            }
        }
        if self.is_settled() {
            for p in core::mem::take(&mut self.frame.settle) {
                self.out.push(crate::Out::Inspect(p, guiproto::control::InspectReply::Done));
            }
        }
    }

    pub(crate) fn is_settled(&self) -> bool {
        self.dirty.is_empty()
            && !self.frame.in_flight
            && self.frame.groups.is_empty()
            && self.windows.values().all(|w| w.configure.is_none())
            && self.clients.values().all(|c| c.staged.is_empty() && c.layers.values().all(|l| l.anim.is_empty()))
    }

    fn animate(&mut self, now: u64) {
        let mut done: Vec<(ClientId, u32, u32)> = Vec::new();
        for (&cid, c) in self.clients.iter_mut() {
            for (&lid, l) in c.layers.iter_mut() {
                if l.anim.is_empty() {
                    continue;
                }
                let window = l.window;
                l.anim.retain(|a| {
                    let el = now.saturating_sub(a.start) / 1_000_000;
                    let t = (el * 1000 / a.ms as u64).min(1000) as u32;
                    let k = a.curve.eval(t) as i64;
                    let v = a.from as i64 + (a.to as i64 - a.from as i64) * k / 1000;
                    set_anim(&mut l.props, a.prop, v as i32);
                    t < 1000
                });
                // Konec vsech animaci vrstvy hlasi aplikaci.
                if l.anim.is_empty() {
                    done.push((cid, lid, window));
                } else {
                    done.push((cid, 0, window));
                }
            }
        }
        for (c, lid, w) in done {
            self.damage_local_window(c, w);
            if lid != 0 {
                self.send(c, ServerMsg::AnimationDone { layer: lid });
            }
        }
        let _ = anim_value;
    }
}

fn set_anim(p: &mut guiproto::window::LayerProps, prop: guiproto::window::AnimProp, v: i32) {
    use guiproto::window::AnimProp::*;
    match prop {
        Opacity => p.opacity = v.clamp(0, 255) as u8,
        X => p.rect.x = v,
        Y => p.rect.y = v,
        ScrollX => p.scroll.0 = v,
        ScrollY => p.scroll.1 = v,
    }
}

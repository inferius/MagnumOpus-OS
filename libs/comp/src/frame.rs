//! Takt snimku, animace, skupiny transakci, terminy `CONFIGURE` a ping.
//!
//! Kompozitor tika jen tehdy, kdyz neco ceka (poskozena oblast, animace,
//! zadost o snimek). Tik: vyprsene terminy, animace, slozeni poskozenych
//! oblasti, `FRAME` klientum, kteri o nej zadali. Jeden snimek je vzdy
//! nejvys jeden "na ceste" k displeji.

use alloc::vec::Vec;

use guiproto::Rect;
use guiproto::window::{ClientMsg, ServerMsg};

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
    /// Kdo ceka na ustaleni (inspekce `Settle`) a od kolikateho
    /// zobrazeneho snimku je jeho pozadavek na obrazovce.
    pub(crate) settle: Vec<(crate::PeerId, u64)>,
    /// Perioda snimku z obnovovaci frekvence vystupu (0 = vychozi 60 Hz).
    pub(crate) period: u64,
    /// Od kdy nekdo ceka na ustaleni, a uz ohlaseno, proc ne.
    pub(crate) settle_since: Option<u64>,
    pub(crate) settle_reported: bool,
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
        if !self.frame.in_flight && (!self.dirty.is_empty() || animating || frames || self.theme_animating()) {
            at(self.next_tick_time(now));
        }
        for g in &self.frame.groups {
            at(g.deadline);
        }
        if let Some(d) = self.max_menu_due() {
            at(d);
        }
        if let Some(d) = self.fs_esc_due() {
            at(d);
        }
        if let Some(d) = self.appearance_wakeup(now) {
            at(d);
        }
        if let Some(d) = self.tip_due() {
            at(d);
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
                // Nereaguje: dalsi ping (predchozi se mohl ztratit).
                Some(_) | None => at(c.last_ping + PING_INTERVAL),
            }
        }
        if let Some(d) = self.dialogs.next_wakeup() {
            at(d);
        }
        if let Some(d) = self.greeter_wakeup(now) {
            at(d);
        }
        t.map(|v| v.max(now))
    }

    /// Perioda snimku vystupu (120 Hz = 8,3 ms, 240 Hz = 4,2 ms).
    pub fn period(&self) -> u64 {
        if self.frame.period == 0 { PERIOD } else { self.frame.period }
    }

    /// Obnovovaci frekvence vystupu v mHz (od ovladace displeje); 0 = nevi.
    pub fn set_refresh(&mut self, mhz: u32) {
        // 1 s = 10^12 ns * mHz / 1000; rozumny rozsah 24-500 Hz.
        self.frame.period = if (24_000..=500_000).contains(&mhz) { 1_000_000_000_000 / mhz as u64 } else { 0 };
    }

    /// Rezerva na skladani pred koncem periody.
    fn compose_budget(&self) -> u64 {
        (self.period() / 4).min(COMPOSE_BUDGET)
    }

    fn next_tick_time(&self, now: u64) -> u64 {
        let next = self.frame.last_tick + self.period();
        if self.frame.last_tick == 0 || next <= now { now } else { next }
    }

    /// Casove veci, ktere nezavisi na snimku (skupiny, configure, ping).
    pub fn timers(&mut self, now: u64) {
        self.max_menu_timer(now);
        self.fs_esc_timer(now);
        self.appearance_timer(now);
        self.tip_timer(now);
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
            w.set_size(nw, nh);
            let (pending, anchor) = (w.pending_size.take(), w.anchor);
            self.damage_window(g);
            if let Some((pw, ph)) = pending {
                self.configure(g, pw, ph, now);
                if let Some(w) = self.windows.get_mut(&g) {
                    w.anchor = anchor;
                }
            }
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
                // Nereagujici aplikace dostava novy ping kazdy interval:
                // zprava se mohla ztratit (plna fronta kanalu, kdyz aplikace
                // dlouho necetla) a bez dalsiho pingu by ztlumeni zustalo
                // navzdy. Odpoved na kterykoli z nich ho zrusi.
                Some(_) if c.unresponsive && now >= c.last_ping + PING_INTERVAL => {
                    c.ping_serial = c.ping_serial.wrapping_add(1);
                    c.ping = Some((c.ping_serial, now));
                    c.last_ping = now;
                    let s = c.ping_serial;
                    self.send(id, ServerMsg::Ping { serial: s });
                }
                Some((_, sent)) if !c.unresponsive && now >= sent + PING_TIMEOUT => {
                    c.unresponsive = true;
                    let name = c.info.id.clone();
                    let wins: Vec<u32> = c.windows.values().copied().collect();
                    for g in wins {
                        self.damage_window(g);
                    }
                    self.request_auto_snapshot(false, &alloc::format!("{name} nereaguje"), now);
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
        self.tick_with(now, |c, rects| c.compose(canvas, rects, now))
    }

    /// Spolecny tik obou backendu: `compose` slozi poskozene obdelniky.
    pub(crate) fn tick_with(&mut self, now: u64, compose: impl FnOnce(&mut Self, &[Rect])) -> Option<Vec<Rect>> {
        self.timers(now);
        if self.frame.in_flight {
            return None;
        }
        self.frame.last_tick = now;
        self.animate(now);
        self.places_changed();
        let rects = if self.dirty.is_empty() {
            None
        } else {
            let rects = self.dirty.take();
            compose(self, &rects);
            self.dialog_composed();
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
        let (period, budget) = (self.period(), self.compose_budget());
        for id in ids {
            self.clients.get_mut(&id).unwrap().frame_wanted = false;
            self.send(
                id,
                ServerMsg::Frame {
                    seq,
                    deadline: now + period - budget,
                    period,
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
            self.dialog_presented(time);
        }
        self.frame.in_flight = false;
        let ids: Vec<ClientId> = self.clients.keys().copied().collect();
        for id in ids {
            let txns = core::mem::take(&mut self.clients.get_mut(&id).unwrap().txn_in_flight);
            for txn in txns {
                self.send(id, ServerMsg::Presented { txn, time });
            }
        }
        // Ustaleno pro cekajici: vse, co bylo pred dotazem, je zobrazene
        // (dalsi cely snimek po dotazu) a nic nepotvrzeneho neceka. Prubezne
        // kreslici aplikace (animace, bezici uloha v terminalu) uz ustaleni
        // neblokuje.
        let pending = !self.frame.groups.is_empty()
            || self.windows.values().any(|w| w.configure.is_some())
            || self.clients.values().any(|c| !c.staged.is_empty());
        let (settled, presented) = (self.is_settled(), self.frame.presented);
        let (done, wait): (Vec<_>, Vec<_>) = core::mem::take(&mut self.frame.settle).into_iter().partition(|&(_, target)| settled || (!pending && presented >= target));
        self.frame.settle = wait;
        for (p, _) in done {
            self.out.push(crate::Out::Inspect(p, guiproto::control::InspectReply::Done));
        }
    }

    /// Ceka na ustaleni (inspekce `Settle`): odpoved az po dalsim celem
    /// snimku (snimek na ceste mohl zacit pred dotazem).
    pub(crate) fn settle_wait(&mut self, peer: crate::PeerId) {
        let target = self.frame.presented + if self.frame.in_flight { 2 } else { 1 };
        self.frame.settle.push((peer, target));
    }

    /// Ceka-li nekdo na ustaleni dele nez 5 s: jednou radek, co ho drzi
    /// (diagnostika testu a zaseknutych snimku; sluzba ho vypise).
    pub fn settle_report(&mut self, now: u64) -> Option<alloc::string::String> {
        if self.frame.settle.is_empty() {
            self.frame.settle_since = None;
            self.frame.settle_reported = false;
            return None;
        }
        let since = *self.frame.settle_since.get_or_insert(now);
        if self.frame.settle_reported || now < since + 5_000_000_000 {
            return None;
        }
        self.frame.settle_reported = true;
        let cfg: Vec<u32> = self.windows.iter().filter(|(_, w)| w.configure.is_some()).map(|(&g, _)| g).collect();
        let staged: Vec<_> = self.clients.iter().filter(|(_, c)| !c.staged.is_empty()).map(|(&i, c)| (i, c.staged.len())).collect();
        let anim: Vec<_> = self.clients.iter().filter(|(_, c)| c.layers.values().any(|l| !l.anim.is_empty())).map(|(&i, _)| i).collect();
        Some(alloc::format!(
            "compd: ustaleni ceka {} ms: poskozeno {:?}, snimek na ceste {}, skupiny {}, CONFIGURE bez potvrzeni okna {cfg:?}, rozpracovane zpravy klientu {staged:?}, animace klientu {anim:?}",
            (now - since) / 1_000_000,
            self.dirty.rects(),
            self.frame.in_flight,
            self.frame.groups.len()
        ))
    }

    pub(crate) fn is_settled(&self) -> bool {
        self.dirty.is_empty()
            && !self.theme_animating()
            && !self.frame.in_flight
            && self.frame.groups.is_empty()
            && self.windows.values().all(|w| w.configure.is_none())
            && self.clients.values().all(|c| c.staged.is_empty() && c.layers.values().all(|l| l.anim.is_empty()))
    }

    fn animate(&mut self, now: u64) {
        self.animate_theme(now);
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

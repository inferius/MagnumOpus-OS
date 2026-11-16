//! Jadro sluzby oznameni `notifyd` (`docs/cs/oznameni.md`): seznam
//! oznameni, identity odesilatelu, limity, Nerusit, odber prohlizecu.
//! Bez systemovych volani (testy na hostiteli); sluzba jen predava zpravy
//! kanalu a cas.
//!
//! - **Identita** (aplikace z manifestu, uzivatel, systemova) patri kanalu
//!   odesilatele; klice oznameni (`key`) jsou v prostoru identity, takze
//!   aplikace po restartu nahradi nebo zavre sve starsi oznameni.
//! - **Viditelnost:** prohlizec uzivatele `uid` vidi oznameni identit
//!   s `uid` a systemova (`uid` 0).
//! - **Limity:** nejvic [`MAX_PER_IDENTITY`] oznameni identity (starsi
//!   vytlaci novejsi), [`MAX_PER_USER`] na uzivatele; vyskakovani omezuje
//!   rychlost na identitu (zasobnik [`BURST`] po jednom za [`REFILL`]):
//!   nad limitem jde oznameni potichu do centra.
//! - **Nerusit** je na uzivatele; vyskoci jen `Urgent` (smi jen systemova
//!   identita, aplikaci se snizi na `High`). Oznameni z doby Nerusit
//!   zustanou v centru a nevyskoci ani po jeho konci.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

#[cfg(test)]
mod tests;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use notifyproto::{
    Closed, DndUntil, Error, MAX_ACTION_ID, MAX_ACTION_LABEL, MAX_BODY, MAX_PER_MSG, MAX_TITLE, Notification, Priority, Send, SendReply, View,
    ViewReply, clean, op,
};

pub const MAX_PER_IDENTITY: usize = 20;
pub const MAX_PER_USER: usize = 200;
/// Kolik oznameni identity smi vyskocit hned za sebou.
pub const BURST: u32 = 5;
/// Za jak dlouho se doplni jedno vyskoceni (ns monotonniho casu).
pub const REFILL: u64 = 3_000_000_000;

pub type SenderId = u32;
pub type ViewerId = u32;

/// Kdo oznameni posila (urcuje init pri vydani kanalu).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub app_id: String,
    pub app_name: String,
    /// Uzivatel relace; `0` = systemova sluzba (vidi vsichni).
    pub uid: u64,
    /// Systemova identita: smi `Urgent`.
    pub system: bool,
}

/// Zprava ven: komu a co.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    Sender(SenderId, Vec<u8>),
    Viewer(ViewerId, Vec<u8>),
}

struct Entry {
    n: Notification,
    uid: u64,
    key: u64,
    /// Vyskakovani podle priority a rychlosti (bez Nerusit).
    popup: bool,
    /// Uz vyskocilo a zmizelo do centra.
    seen: bool,
}

/// Rychlost vyskakovani identity.
struct Bucket {
    tokens: u32,
    at: u64,
}

pub struct Core {
    senders: BTreeMap<SenderId, Identity>,
    viewers: BTreeMap<ViewerId, (u64, bool)>,
    list: Vec<Entry>,
    buckets: BTreeMap<(String, u64), Bucket>,
    dnd: BTreeMap<u64, DndUntil>,
    next_id: u64,
    next_conn: u32,
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}

impl Core {
    pub fn new() -> Self {
        Self { senders: BTreeMap::new(), viewers: BTreeMap::new(), list: Vec::new(), buckets: BTreeMap::new(), dnd: BTreeMap::new(), next_id: 1, next_conn: 1 }
    }

    pub fn add_sender(&mut self, id: Identity) -> SenderId {
        let c = self.next_conn;
        self.next_conn += 1;
        self.senders.insert(c, id);
        c
    }

    /// Odesilatel skoncil: jeho oznameni zustanou (centrum), akce uz
    /// dorucit nejdou, dokud se identita znovu nepripoji.
    pub fn remove_sender(&mut self, s: SenderId) {
        self.senders.remove(&s);
    }

    pub fn add_viewer(&mut self, uid: u64) -> ViewerId {
        let c = self.next_conn;
        self.next_conn += 1;
        self.viewers.insert(c, (uid, false));
        c
    }

    pub fn remove_viewer(&mut self, v: ViewerId) {
        self.viewers.remove(&v);
    }

    /// Pocet oznameni (testy, stav).
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    fn dnd_on(&self, uid: u64, now: u64) -> bool {
        self.dnd.get(&uid).is_some_and(|&u| u > now)
    }

    /// Prohlizec `v` oznameni uzivatele `uid` vidi.
    fn sees(viewer_uid: u64, uid: u64) -> bool {
        uid == 0 || uid == viewer_uid
    }

    /// Oznameni pro prohlizec uzivatele `vuid` (vyskoceni podle jeho Nerusit).
    fn view_of(&self, e: &Entry, vuid: u64, now: u64) -> Notification {
        let mut n = e.n.clone();
        n.popup = e.popup && !e.seen && (n.priority == Priority::Urgent || !self.dnd_on(vuid, now));
        n
    }

    fn broadcast(&self, uid: u64, out: &mut Vec<Out>, mut msg: impl FnMut(u64) -> ViewReply) {
        for (&v, &(vuid, sub)) in &self.viewers {
            if sub && Self::sees(vuid, uid) {
                out.push(Out::Viewer(v, msg(vuid).encode()));
            }
        }
    }

    /// Aktivni kanal odesilatele identity (posledni pripojeny).
    fn sender_of(&self, app_id: &str, uid: u64) -> Option<SenderId> {
        self.senders.iter().rev().find(|(_, i)| i.app_id == app_id && i.uid == uid).map(|(&s, _)| s)
    }

    /// Odebere oznameni na indexu `i`; odesilatel se dozvi `why`
    /// (`Some`), prohlizece vzdy.
    fn remove_at(&mut self, i: usize, why: Closed, tell_sender: bool, out: &mut Vec<Out>) {
        let e = self.list.remove(i);
        if tell_sender && let Some(s) = self.sender_of(&e.n.app_id, e.uid) {
            out.push(Out::Sender(s, SendReply::Closed { key: e.key, why }.encode()));
        }
        let id = e.n.id;
        self.broadcast(e.uid, out, |_| ViewReply::Removed { id, why });
    }

    /// Doplni a vezme jedno vyskoceni identity.
    fn take_token(&mut self, app_id: &str, uid: u64, mono: u64) -> bool {
        let b = self.buckets.entry((String::from(app_id), uid)).or_insert(Bucket { tokens: BURST, at: mono });
        let add = (mono.saturating_sub(b.at) / REFILL) as u32;
        if add > 0 {
            b.tokens = (b.tokens + add).min(BURST);
            b.at += add as u64 * REFILL;
        }
        if b.tokens == BURST {
            b.at = mono;
        }
        if b.tokens > 0 {
            b.tokens -= 1;
            true
        } else {
            false
        }
    }

    /// Zprava odesilatele. `wall` = realny cas (ns), `mono` = monotonni.
    pub fn sender_msg(&mut self, s: SenderId, data: &[u8], wall: u64, mono: u64) -> Vec<Out> {
        let mut out = Vec::new();
        let Some(ident) = self.senders.get(&s).cloned() else { return out };
        match Send::decode(data) {
            Ok(Send::Notify { key, title, body, priority, actions }) => match self.notify(&ident, key, &title, &body, priority, actions, wall, mono, &mut out) {
                Ok(popup) => out.insert(0, Out::Sender(s, SendReply::Sent { key, popup }.encode())),
                Err(error) => out.push(Out::Sender(s, SendReply::Error { key, error }.encode())),
            },
            Ok(Send::Close { key }) => {
                if let Some(i) = self.list.iter().position(|e| e.n.app_id == ident.app_id && e.uid == ident.uid && e.key == key) {
                    self.remove_at(i, Closed::Sender, false, &mut out);
                }
            }
            Err(e) => out.push(Out::Sender(s, SendReply::Error { key: 0, error: e }.encode())),
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &mut self,
        ident: &Identity,
        key: u64,
        title: &str,
        body: &str,
        priority: Priority,
        actions: Vec<notifyproto::Action>,
        wall: u64,
        mono: u64,
        out: &mut Vec<Out>,
    ) -> Result<bool, Error> {
        let title = clean(title, MAX_TITLE, false);
        if title.trim().is_empty() {
            return Err(Error::InvalidArgs);
        }
        let mut acts = Vec::new();
        for a in actions {
            let id = clean(&a.id, MAX_ACTION_ID, false);
            let label = clean(&a.label, MAX_ACTION_LABEL, false);
            if id.is_empty() || label.trim().is_empty() || acts.iter().any(|x: &notifyproto::Action| x.id == id) {
                return Err(Error::InvalidArgs);
            }
            acts.push(notifyproto::Action { id, label });
        }
        let priority = if priority == Priority::Urgent && !ident.system { Priority::High } else { priority };
        let existing = self.list.iter().position(|e| e.n.app_id == ident.app_id && e.uid == ident.uid && e.key == key);
        let (id, popup, seen, replaced) = match existing {
            // Nahrazeni: stejne cislo, nevyskakuje znovu (jen prekresleni).
            Some(i) => {
                let e = self.list.remove(i);
                (e.n.id, e.popup, e.seen, true)
            }
            None => {
                let id = self.next_id;
                self.next_id += 1;
                // V Nerusit jen do centra (ani po jeho konci nevyskoci).
                let quiet = priority != Priority::Urgent && self.dnd_on(ident.uid, wall);
                let popup = priority != Priority::Low && !quiet && self.take_token(&ident.app_id, ident.uid, mono);
                (id, popup, false, false)
            }
        };
        let n = Notification {
            id,
            app_id: ident.app_id.clone(),
            app_name: ident.app_name.clone(),
            title,
            body: clean(body, MAX_BODY, true),
            priority,
            actions: acts,
            time: wall,
            popup,
            replaced,
        };
        self.list.push(Entry { n, uid: ident.uid, key, popup, seen });
        // Limity: nejstarsi oznameni identity, pak uzivatele.
        let same = |e: &Entry| e.n.app_id == ident.app_id && e.uid == ident.uid;
        if self.list.iter().filter(|e| same(e)).count() > MAX_PER_IDENTITY
            && let Some(i) = self.list.iter().position(same)
        {
            self.remove_at(i, Closed::Limit, true, out);
        }
        if self.list.iter().filter(|e| e.uid == ident.uid).count() > MAX_PER_USER
            && let Some(i) = self.list.iter().position(|e| e.uid == ident.uid)
        {
            self.remove_at(i, Closed::Limit, true, out);
        }
        let last = self.list.len() - 1;
        let uid = ident.uid;
        let views: Vec<(ViewerId, Vec<u8>)> = self
            .viewers
            .iter()
            .filter(|(_, (vuid, sub))| *sub && Self::sees(*vuid, uid))
            .map(|(&v, &(vuid, _))| (v, ViewReply::Added(self.view_of(&self.list[last], vuid, wall)).encode()))
            .collect();
        out.extend(views.into_iter().map(|(v, d)| Out::Viewer(v, d)));
        Ok(popup && (priority == Priority::Urgent || !self.dnd_on(uid, wall)))
    }

    /// Zprava prohlizece.
    pub fn viewer_msg(&mut self, v: ViewerId, data: &[u8], wall: u64) -> Vec<Out> {
        let mut out = Vec::new();
        let Some(&(vuid, _)) = self.viewers.get(&v) else { return out };
        let req = match View::decode(data) {
            Ok(r) => r,
            Err(e) => {
                out.push(Out::Viewer(v, ViewReply::Result { op: 0, error: Some(e) }.encode()));
                return out;
            }
        };
        let visible = |c: &Core, id: u64| c.list.iter().position(|e| e.n.id == id && Self::sees(vuid, e.uid));
        let (o, r) = match req {
            View::Subscribe => {
                self.viewers.insert(v, (vuid, true));
                let all: Vec<Notification> = self.list.iter().filter(|e| Self::sees(vuid, e.uid)).map(|e| self.view_of(e, vuid, wall)).collect();
                let mut chunks: Vec<&[Notification]> = all.chunks(MAX_PER_MSG).collect();
                if chunks.is_empty() {
                    chunks.push(&[]);
                }
                let n = chunks.len();
                for (i, c) in chunks.into_iter().enumerate() {
                    out.push(Out::Viewer(v, ViewReply::Snapshot { list: c.to_vec(), last: i + 1 == n }.encode()));
                }
                out.push(Out::Viewer(v, ViewReply::Dnd { until: self.dnd.get(&vuid).copied().filter(|&u| u > wall).unwrap_or(0) }.encode()));
                return out;
            }
            View::Dismiss { id } => (
                op::DISMISS,
                match visible(self, id) {
                    Some(i) => {
                        self.remove_at(i, Closed::User, true, &mut out);
                        None
                    }
                    None => Some(Error::NotFound),
                },
            ),
            View::Invoke { id, action } => (
                op::INVOKE,
                match visible(self, id) {
                    Some(i) if action.is_empty() || self.list[i].n.actions.iter().any(|a| a.id == action) => {
                        let e = &self.list[i];
                        if let Some(s) = self.sender_of(&e.n.app_id, e.uid) {
                            out.push(Out::Sender(s, SendReply::Action { key: e.key, action }.encode()));
                        }
                        self.remove_at(i, Closed::User, false, &mut out);
                        None
                    }
                    Some(_) => Some(Error::InvalidArgs),
                    None => Some(Error::NotFound),
                },
            ),
            View::DismissAll => {
                while let Some(i) = self.list.iter().position(|e| Self::sees(vuid, e.uid)) {
                    self.remove_at(i, Closed::User, true, &mut out);
                }
                (op::DISMISS_ALL, None)
            }
            View::SetDnd { until } => {
                if until == 0 {
                    self.dnd.remove(&vuid);
                } else {
                    self.dnd.insert(vuid, until);
                }
                self.broadcast(vuid, &mut out, |_| ViewReply::Dnd { until });
                (op::SET_DND, None)
            }
            View::Seen { id } => (
                op::SEEN,
                match visible(self, id) {
                    Some(i) => {
                        self.list[i].seen = true;
                        None
                    }
                    None => Some(Error::NotFound),
                },
            ),
        };
        out.insert(0, Out::Viewer(v, ViewReply::Result { op: o, error: r }.encode()));
        out
    }

    /// Konec Nerusit: prohlizece se to dozvi. Vraci nejblizsi dalsi konec.
    pub fn tick(&mut self, wall: u64) -> (Vec<Out>, Option<u64>) {
        let mut out = Vec::new();
        let ended: Vec<u64> = self.dnd.iter().filter(|(_, u)| **u <= wall).map(|(&k, _)| k).collect();
        for uid in ended {
            self.dnd.remove(&uid);
            // Jen prohlizece toho uzivatele (systemova oznameni nema Nerusit).
            for (&v, &(vuid, sub)) in &self.viewers {
                if sub && vuid == uid {
                    out.push(Out::Viewer(v, ViewReply::Dnd { until: 0 }.encode()));
                }
            }
        }
        let next = self.dnd.values().copied().filter(|&u| u != DndUntil::MAX).min();
        (out, next)
    }
}

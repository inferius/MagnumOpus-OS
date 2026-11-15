//! Povoleni aplikaci (`accountproto::perm`, docs/cs/ucty.md, kapitola
//! Povoleni aplikaci).
//!
//! Kanal `permissions` (sluzba zarizeni, dnes `audiod`) se pta "smi
//! aplikace X uzivatele U pouzit mikrofon". Ulozene rozhodnuti odpovi hned;
//! jinak systemovy dialog Povoleni u kompozitoru (Povolit / Nepovolit / Jen
//! tentokrat, Esc = Nepovolit). "Povolit" a "Nepovolit" se ulozi trvale po uzivateli
//! a aplikaci do `/db/permissions/<uid>` (relace FS sluzby uctu: aplikace
//! ho nevidi ani nezmeni), "Jen tentokrat" ne. Bez kompozitoru (zadny
//! duveryhodny dialog) a pro neznameho uzivatele = Nepovolit.
//!
//! Uzivatel sam (kanal self, prikaz shellu `povoleni`) sva rozhodnuti
//! vypise a odvola.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use accountproto::perm::{self, Answer, Ask, ChanMsg, DIALOG_OPTIONS, Decision, Perm};
use guiproto::control::{DialogKind, DialogReply, DialogRequest};
use libos::{Channel, Error, Result, println};

use crate::service::Service;

/// Dotaz cekajici na odpoved dialogu.
pub struct PendingAsk {
    pub dialog_id: u32,
    pub ask: Ask,
}

#[derive(Default)]
pub struct Perms {
    /// Kanal `permissions` (od initu; po restartu sluzby zarizeni novy):
    /// jen dotazy `Ask`.
    pub chan: Option<Channel>,
    /// Kanal `permissions-admin` (init jmenem relace shellu, prikaz
    /// `povoleni`): jen `List` a `Revoke`.
    pub admin: Option<Channel>,
    pub pending: Vec<PendingAsk>,
}

impl Service {
    fn answer(&mut self, id: u32, decision: Decision) {
        if let Some(c) = &self.perms.chan
            && c.write(&Answer { id, decision }.encode(), Vec::new()).is_err()
        {
            self.perms.chan = None;
        }
    }

    /// Dotazy sluzby zarizeni (kanal `permissions`).
    pub fn perm_requests(&mut self) {
        loop {
            let Some(c) = &self.perms.chan else { return };
            let m = match c.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => return,
                Err(_) => {
                    self.perms.chan = None;
                    return;
                }
            };
            if let Ok(ChanMsg::Ask(ask)) = ChanMsg::decode(&m.data) {
                self.perm_ask(ask);
            }
        }
    }

    /// Sprava rozhodnuti (kanal `permissions-admin`).
    pub fn perm_admin(&mut self) {
        loop {
            let Some(c) = &self.perms.admin else { return };
            let m = match c.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => return,
                Err(_) => {
                    self.perms.admin = None;
                    return;
                }
            };
            let r = match ChanMsg::decode(&m.data) {
                Ok(ChanMsg::List { uid }) => self.perm_list(uid),
                Ok(ChanMsg::Revoke { uid, app_id, perm }) => self.perm_revoke(uid, &app_id, perm),
                Ok(ChanMsg::Ask(_)) => Err(Error::AccessDenied),
                Err(e) => Err(e),
            };
            let reply = match r {
                Ok(d) => accountproto::reply_ok(&d),
                Err(e) => accountproto::reply_err(e),
            };
            if let Some(c) = &self.perms.admin {
                let _ = c.write(&reply, Vec::new());
            }
        }
    }

    fn perm_ask(&mut self, ask: Ask) {
        if self.eng.account(ask.uid).is_none() {
            return self.answer(ask.id, Decision::Deny);
        }
        match self.eng.store.load_perms(ask.uid).map(|v| perm::lookup(&v, &ask.app_id, ask.perm)) {
            Ok(Some(allow)) => return self.answer(ask.id, if allow { Decision::Allow } else { Decision::Deny }),
            Ok(None) => {}
            Err(e) => {
                println!("accountd: povoleni uzivatele {}: {e}", ask.uid);
                return self.answer(ask.id, Decision::Deny);
            }
        }
        if self.admin.dialog.is_none() {
            // Bez kompozitoru neni duveryhodny dialog: nic se nepovoli.
            return self.answer(ask.id, Decision::Deny);
        }
        let id = self.admin.next_dialog_id();
        let Some(dialog) = self.admin.dialog.as_ref() else { return };
        let req = DialogRequest::Ask {
            id,
            kind: DialogKind::Permission,
            app: ask.app_name.clone(),
            text: format!("{} {}", ask.app_name, ask.perm.question()),
            options: DIALOG_OPTIONS.iter().map(|s| String::from(*s)).collect(),
        };
        if let Err(e) = dialog.write(&req.encode(), Vec::new()) {
            println!("accountd: dialog povoleni: {e}");
            self.admin.dialog = None;
            return self.answer(ask.id, Decision::Deny);
        }
        self.perms.pending.push(PendingAsk { dialog_id: id, ask });
    }

    /// Odpoved dialogu (`dialog_answers`): je nase? Pak ulozit a odpovedet.
    pub fn perm_dialog_reply(&mut self, r: &DialogReply) -> bool {
        let (id, choice) = match r {
            DialogReply::Answer { id, choice, .. } => (*id, *choice),
            DialogReply::Cancelled { id } => (*id, u32::MAX),
        };
        let Some(i) = self.perms.pending.iter().position(|p| p.dialog_id == id) else { return false };
        let p = self.perms.pending.remove(i);
        let d = perm::decision_of_choice(choice);
        if d != Decision::Once {
            let r = self.eng.store.load_perms(p.ask.uid).and_then(|mut v| {
                perm::set(&mut v, &p.ask.app_id, p.ask.perm, d == Decision::Allow);
                self.eng.store.save_perms(p.ask.uid, &v)
            });
            if let Err(e) = r {
                println!("accountd: ulozeni povoleni: {e}");
            }
        }
        println!("accountd: povoleni {} pro {} (uzivatel {}): {:?}", p.ask.perm.name(), p.ask.app_id, p.ask.uid, d);
        self.answer(p.ask.id, d);
        true
    }

    /// Kompozitor skoncil: cekajici dotazy zamitnout.
    pub fn perm_dialog_gone(&mut self) {
        for p in core::mem::take(&mut self.perms.pending) {
            self.answer(p.ask.id, Decision::Deny);
        }
    }

    /// Kanal self: vlastni rozhodnuti.
    pub fn perm_list(&self, uid: u64) -> Result<Vec<u8>> {
        self.eng.store.load_perms(uid).map(|v| perm::encode_entries(&v))
    }

    pub fn perm_revoke(&mut self, uid: u64, app_id: &str, p: Perm) -> Result<Vec<u8>> {
        let mut v = self.eng.store.load_perms(uid)?;
        let before = v.len();
        v.retain(|e| !(e.app_id == app_id && e.perm == p));
        if v.len() == before {
            return Err(Error::NotFound);
        }
        self.eng.store.save_perms(uid, &v)?;
        println!("accountd: povoleni {} pro {app_id} (uzivatel {uid}) odvolano", p.name());
        Ok(Vec::new())
    }
}

//! Sprava pocitace: `ELEVATE` s duveryhodnym dialogem kompozitoru, ukony
//! uctu, granty pro jine sluzby (`CHECK_GRANT`), rezim spravy a audit.
//!
//! Model (schvaleny, `docs/cs/ucty.md`, kapitola Sprava pocitace): ukon pro
//! cely pocitac potvrdi clovek s roli spravy heslem nebo PINem v dialogu,
//! ktery aplikace nepodvrhne. Ma-li roli sam prihlaseny, potvrzuje on; jinak
//! prvni ucet s roli ("pres rameno"; vyber z vice uctu prijde s nastavenim).
//! Zaskrtavatko v dialogu zapne rezim spravy: dalsich 5 minut se ukony z teto
//! relace nepotvrzuji (vychozi vypnuto).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use accountproto::elevate::{ADMIN_MODE_SECS, GRANT_TTL_SECS, Params, TOKEN_LEN, action, decode_check};
use accountproto::{Method, Secret, ToInit, flags};
use accounts::Outcome;
use guiproto::control::{DialogKind, DialogReply, DialogRequest};
use libos::{Channel, Error, Result, println};

use crate::os::OsRng;
use crate::service::{Service, now};

/// Ukon cekajici na odpoved dialogu.
struct Pending {
    dialog_id: u32,
    session: u64,
    action: u32,
    params: Vec<u8>,
    confirm_uid: u64,
}

struct Grant {
    token: [u8; TOKEN_LEN],
    action: u32,
    params: Vec<u8>,
    expires: i64,
}

#[derive(Default)]
pub struct Admin {
    /// Kanal `dialog` od kompozitoru (dava ho init; bez nej `NOT_SUPPORTED`).
    pub dialog: Option<Channel>,
    next_id: u32,
    pending: Vec<Pending>,
    grants: Vec<Grant>,
    /// Rezim spravy: (relace, do kdy).
    mode: Vec<(u64, i64)>,
}

/// Odpoved na `ELEVATE` pro kanal self relace (az po dialogu).
pub struct Answer {
    pub session: u64,
    pub reply: Vec<u8>,
}

fn wipe(b: &mut [u8]) {
    for x in b.iter_mut() {
        // SAFETY: platny jedinecny odkaz na bajt; volatile zapis neni vynechan.
        unsafe { core::ptr::write_volatile(x, 0) };
    }
}

impl Service {
    fn display(&self, uid: u64) -> String {
        self.eng.account(uid).map_or_else(|| format!("{uid}"), |a| format!("{} ({})", a.display, a.name))
    }

    /// Popis ukonu pro dialog (skutecne parametry, ne jen nazev).
    fn describe(&self, act: u32, params: &[u8]) -> String {
        match Params::decode(act, params) {
            Ok(Params::AccountCreate { name, display, admin, .. }) => {
                format!("Přidat účet {name} ({display}){}", if admin { " s rolí správy" } else { "" })
            }
            Ok(Params::AccountDelete { uid }) => format!("Smazat účet {}; domov se přesune do koše správy", self.display(uid)),
            Ok(Params::ResetPassword { uid, .. }) => {
                let enc = self.eng.account(uid).is_some_and(|a| a.crypt.is_some());
                format!("Nastavit nové heslo účtu {}{}", self.display(uid), if enc { ". Šifrovaný domov se tím neotevře." } else { "" })
            }
            Ok(Params::RoleSet { uid, on }) => format!("{} roli správy: {}", if on { "Dát" } else { "Odebrat" }, self.display(uid)),
            _ => format!("Úkon {act} jiné služby (parametry {} B)", params.len()),
        }
    }

    /// Kdo ukon potvrdi: prihlaseny, ma-li roli, jinak prvni ucet s roli.
    fn confirmer(&self, uid: u64) -> Option<u64> {
        let has = |a: &accounts::Account| a.flags & flags::ADMIN != 0 && a.flags & flags::DISABLED == 0;
        if self.eng.account(uid).is_some_and(has) {
            return Some(uid);
        }
        self.eng.accounts().iter().find(|a| has(a)).map(|a| a.uid)
    }

    /// `ELEVATE` z relace `i`. `None` = ceka se na dialog (odpoved prijde
    /// pozdeji pres [`Service::dialog_answers`]).
    pub fn elevate(&mut self, i: usize, act: u32, params: Vec<u8>) -> Option<Vec<u8>> {
        let (session, uid) = (self.sessions[i].id, self.sessions[i].uid);
        if !(action::ACCOUNT_CREATE..=action::LAST).contains(&act) {
            return Some(accountproto::reply_err(Error::InvalidArgs));
        }
        let t = now();
        self.admin.mode.retain(|(_, until)| *until > t);
        if self.admin.mode.iter().any(|(s, _)| *s == session) {
            println!("accountd: ukon {act} v rezimu spravy (relace {session})");
            let r = self.perform(session, uid, act, &params, true);
            return Some(reply(r));
        }
        let Some(confirm_uid) = self.confirmer(uid) else {
            return Some(accountproto::reply_err(Error::AccessDenied));
        };
        if self.admin.dialog.is_none() {
            // Bez kompozitoru neni duveryhodny dialog: nic se nepovoli.
            return Some(accountproto::reply_err(Error::NotSupported));
        }
        let text = format!(
            "{}\nPotvrzuje: {} (smí spravovat tento počítač)",
            self.describe(act, &params),
            self.display(confirm_uid)
        );
        let phrase = self.eng.account(confirm_uid).map(|a| a.phrase.clone()).unwrap_or_default();
        let app = format!("Aplikace uživatele {}", self.eng.account(uid).map_or("?", |a| a.display.as_str()));
        self.admin.next_id = self.admin.next_id.wrapping_add(1).max(1);
        let id = self.admin.next_id;
        let dialog = self.admin.dialog.as_ref()?;
        let sent = dialog
            .write(&DialogRequest::Phrase { text: phrase }.encode(), Vec::new())
            .and_then(|_| {
                let ask = DialogRequest::Ask {
                    id,
                    kind: DialogKind::Elevate,
                    app,
                    text,
                    options: vec![String::from("Povolit změnu"), String::from("Zrušit")],
                };
                dialog.write(&ask.encode(), Vec::new())
            });
        if let Err(e) = sent {
            println!("accountd: dialog spravy: {e}");
            self.admin.dialog = None;
            return Some(accountproto::reply_err(Error::NotSupported));
        }
        self.admin.pending.push(Pending { dialog_id: id, session, action: act, params, confirm_uid });
        None
    }

    /// Odpovedi kompozitoru na dialogy spravy.
    pub fn dialog_answers(&mut self) -> Vec<Answer> {
        let mut out = Vec::new();
        loop {
            let Some(d) = self.admin.dialog.as_ref() else { break };
            let mut m = match d.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => break,
                Err(_) => {
                    // Kompozitor skoncil: cekajici ukony zrusit.
                    self.admin.dialog = None;
                    for p in core::mem::take(&mut self.admin.pending) {
                        out.push(Answer { session: p.session, reply: accountproto::reply_err(Error::Canceled) });
                    }
                    break;
                }
            };
            let r = DialogReply::decode(&m.data);
            wipe(&mut m.data);
            let (id, choice, text) = match r {
                Ok(DialogReply::Answer { id, choice, text }) => (id, choice, text),
                Ok(DialogReply::Cancelled { id }) => (id, 1, String::new()),
                Err(_) => continue,
            };
            let Some(k) = self.admin.pending.iter().position(|p| p.dialog_id == id) else { continue };
            let p = self.admin.pending.remove(k);
            let mut bytes = text.into_bytes();
            let secret = Secret::from_slice(&bytes);
            wipe(&mut bytes);
            let r = if choice == 0 || choice == 2 {
                self.confirm(&p, &secret, choice == 2)
            } else {
                Err(Error::Canceled)
            };
            out.push(Answer { session: p.session, reply: reply(r) });
        }
        out
    }

    /// Overi tajemstvi potvrzujiciho (heslo nebo PIN) a provede ukon.
    fn confirm(&mut self, p: &Pending, secret: &Secret, mode: bool) -> Result<Vec<u8>> {
        let uid = self.sessions.iter().find(|s| s.id == p.session).map(|s| s.uid).ok_or(Error::NotFound)?;
        let pin = !secret.is_empty()
            && secret.expose().iter().all(u8::is_ascii_digit)
            && self.eng.account(p.confirm_uid).is_some_and(|a| a.pin.is_some());
        let method = if pin { Method::Pin } else { Method::Password };
        match self.eng.authenticate(p.confirm_uid, method, secret, None, now())? {
            Outcome::Ok { .. } => {}
            Outcome::Denied { .. } => return Err(Error::AccessDenied),
            Outcome::LockedOut { .. } => return Err(Error::ShouldWait),
        }
        if mode {
            self.admin.mode.retain(|(s, _)| *s != p.session);
            self.admin.mode.push((p.session, now() + ADMIN_MODE_SECS));
        }
        self.audit(uid, p.confirm_uid, p.action, mode);
        self.perform(p.session, uid, p.action, &p.params, false)
    }

    /// Provede potvrzeny ukon: ukony uctu hned, ostatni = grant.
    fn perform(&mut self, session: u64, uid: u64, act: u32, params: &[u8], in_mode: bool) -> Result<Vec<u8>> {
        if in_mode {
            self.audit(uid, uid, act, true);
        }
        let _ = session;
        match Params::decode(act, params) {
            Ok(Params::AccountCreate { name, display, password, admin }) => {
                let a = self.eng.prepare(&name, &display, Some(&password), admin, now())?;
                let new = self.create_account(a)?;
                Ok(new.to_le_bytes().to_vec())
            }
            Ok(Params::AccountDelete { uid: target }) => {
                if self.sessions.iter().any(|s| s.uid == target) {
                    // Prihlaseny ucet nejdriv odhlasit.
                    return Err(Error::BadState);
                }
                let a = self.eng.delete(target)?;
                self.to_init(&ToInit::DeleteHome { uid: target, name: a.name.clone() }, Vec::new())?;
                println!("accountd: ucet {} ({target}) smazan", a.name);
                Ok(Vec::new())
            }
            Ok(Params::ResetPassword { uid: target, password }) => self.eng.reset_password(target, &password).map(|_| Vec::new()),
            Ok(Params::RoleSet { uid: target, on }) => self.eng.set_admin(target, on).map(|_| Vec::new()),
            Err(Error::NotSupported) => {
                // Ukon jine sluzby: grant na presne tyto parametry.
                let mut token = [0u8; TOKEN_LEN];
                rand_core::RngCore::fill_bytes(&mut OsRng, &mut token);
                let t = now();
                self.admin.grants.retain(|g| g.expires > t);
                self.admin.grants.push(Grant { token, action: act, params: params.to_vec(), expires: t + GRANT_TTL_SECS });
                Ok(token.to_vec())
            }
            Err(e) => Err(e),
        }
    }

    /// `CHECK_GRANT` od sluzby ukonu: platny token na stejny ukon a stejne
    /// parametry (bajt po bajtu); spotrebuje se.
    pub fn check_grant(&mut self, b: &[u8]) -> Vec<u8> {
        let r = decode_check(b).and_then(|(token, act, params)| {
            let t = now();
            self.admin.grants.retain(|g| g.expires > t);
            let k = self
                .admin
                .grants
                .iter()
                .position(|g| accounts::kdf::ct_eq(&g.token, &token) && g.action == act && g.params == params)
                .ok_or(Error::AccessDenied)?;
            self.admin.grants.remove(k);
            Ok(Vec::new())
        });
        reply(r)
    }

    /// Zaznam do auditu (`/db/audit`): kdo zadal, kdo potvrdil, co.
    fn audit(&mut self, requester: u64, confirmer: u64, act: u32, mode: bool) {
        let line = format!("{}\t{requester}\t{confirmer}\t{act}\t{}\n", now(), if mode { "rezim" } else { "dialog" });
        if let Err(e) = self.eng.store.append_audit(line.as_bytes()) {
            println!("accountd: audit: {e}");
        }
    }
}

fn reply(r: Result<Vec<u8>>) -> Vec<u8> {
    match r {
        Ok(b) => accountproto::reply_ok(&b),
        Err(e) => accountproto::reply_err(e),
    }
}

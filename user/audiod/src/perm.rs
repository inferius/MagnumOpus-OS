//! Relace aplikaci a povoleni nahravani (docs/cs/zvuk.md, kapitola Relace
//! aplikaci a nahravani).
//!
//! - Init vytvari relace aplikaci zpravou `app\0<recept>` na privilegovanem
//!   kanalu (`audioproto::session::Recipe`); odpoved = kanal relace.
//! - Relace aplikace smi jen vlastni streamy (`session::app_allowed`).
//! - Prvni `OPEN` vstupu s `Capture::Ask`: `audiod` se zepta sluzby uctu
//!   (kanal `permissions` od initu, zprava `perm` s handlem) a odpovi
//!   aplikaci, az prijde rozhodnuti (ulozene hned, jinak po dialogu). Bez
//!   sluzby uctu nahravani pro aplikace neni (`ACCESS_DENIED`).

use alloc::string::String;
use alloc::vec::Vec;

use accountproto::perm::{Answer, Ask, Perm};
use audioproto::session::{CaptureStep, Recipe, app_allowed, capture_rule};
use audioproto::client::Request;
use audioproto::{Direction, Error, Format, Result, encode_reply};
use libos::{Channel, Handle, println};

use crate::service::{Client, Service};

/// Zprava initu: nova relace aplikace (za ni recept).
pub const APP_PREFIX: &[u8] = b"app\0";

/// `OPEN` vstupu cekajici na rozhodnuti uzivatele.
pub struct PendingOpen {
    pub ask: u32,
    pub client: u64,
    pub format: Format,
    pub latency_us: u32,
}

impl Service {
    /// Nova relace aplikace podle receptu (zprava initu).
    pub fn app_session(&mut self, recipe: &[u8]) -> Result<Vec<Handle>> {
        let r = Recipe::decode(recipe)?;
        let (ours, theirs) = Channel::create()?;
        let id = self.next_client_id();
        self.clients.push(Client { chan: ours, open: true, privileged: false, id, app: r, granted: None });
        Ok(alloc::vec![theirs.into_handle()])
    }

    pub fn next_client_id(&mut self) -> u64 {
        self.client_seq += 1;
        self.client_seq
    }

    /// Smi klient poslat pozadavek? (`None` = ano, jinak chyba.)
    pub fn check_rights(&self, client: usize, req: &Request) -> Option<Error> {
        let c = &self.clients[client];
        (c.app.is_some() && !app_allowed(req)).then_some(Error::AccessDenied)
    }

    /// `OPEN` relace aplikace: povolit, zamitnout, nebo zeptat se (odpoved
    /// pozdeji, `None`).
    pub fn open_checked(&mut self, client: usize, dir: Direction, format: Format, latency_us: u32) -> Option<Result<(Vec<u8>, Vec<Handle>)>> {
        let c = &self.clients[client];
        let Some(app) = c.app.as_ref() else { return Some(self.open(client, dir, format, latency_us)) };
        match capture_rule(dir, app.capture, c.granted) {
            CaptureStep::Open => Some(self.open(client, dir, format, latency_us)),
            CaptureStep::Deny => Some(Err(Error::AccessDenied)),
            CaptureStep::Ask => {
                let Some(perm) = self.perm.as_ref() else {
                    println!("audiod: nahravani pro {}: sluzba uctu neni, zamitnuto", app.app_id);
                    return Some(Err(Error::AccessDenied));
                };
                self.ask_seq = self.ask_seq.wrapping_add(1).max(1);
                let ask = Ask { id: self.ask_seq, uid: app.uid, app_id: app.app_id.clone(), app_name: app.app_name.clone(), perm: Perm::Microphone };
                if let Err(e) = perm.write(&accountproto::perm::ChanMsg::Ask(ask.clone()).encode(), Vec::new()) {
                    println!("audiod: dotaz na povoleni: {e}");
                    self.perm = None;
                    return Some(Err(Error::AccessDenied));
                }
                self.pending.push(PendingOpen { ask: ask.id, client: c.id, format, latency_us });
                None
            }
        }
    }

    /// Odpovedi sluzby uctu: rozhodnuti zapamatovat v relaci a cekajici
    /// `OPEN` dokoncit.
    pub fn perm_answers(&mut self) {
        loop {
            let Some(perm) = self.perm.as_ref() else { return };
            let m = match perm.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => return,
                Err(_) => {
                    // Sluzba uctu skoncila: cekajici dotazy zamitnout.
                    self.perm = None;
                    for p in core::mem::take(&mut self.pending) {
                        self.finish_open(p, false);
                    }
                    return;
                }
            };
            let Ok(a) = Answer::decode(&m.data) else { continue };
            let Some(i) = self.pending.iter().position(|p| p.ask == a.id) else { continue };
            let p = self.pending.remove(i);
            // "Jen tentokrat" plati do konce relace (behu aplikace).
            if let Some(c) = self.clients.iter_mut().find(|c| c.id == p.client) {
                c.granted = Some(a.decision.allows());
            }
            self.finish_open(p, a.decision.allows());
        }
    }

    fn finish_open(&mut self, p: PendingOpen, allowed: bool) {
        let Some(i) = self.clients.iter().position(|c| c.id == p.client && c.open) else { return };
        let (reply, hs) = if allowed {
            match self.open(i, Direction::Input, p.format, p.latency_us) {
                Ok((r, hs)) => (Ok(r), hs),
                Err(e) => (Err(e), Vec::new()),
            }
        } else {
            (Err(Error::AccessDenied), Vec::new())
        };
        if let Some(c) = self.clients.get(i) {
            println!("audiod: nahravani pro {}: {}", c.app.as_ref().map_or("system", |a| a.app_id.as_str()), if allowed { "povoleno" } else { "zamitnuto" });
        }
        if self.clients[i].chan.write(&encode_reply(reply), hs).is_err() {
            self.clients[i].open = false;
        }
    }

    /// Klient skoncil: jeho cekajici dotazy zahodit.
    pub fn forget_client(&mut self, id: u64) {
        self.pending.retain(|p| p.client != id);
    }

    /// Jmena klientu s bezicim nahravanim (indikator v liste).
    pub fn capturing(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for s in self.streams.iter().filter(|s| s.dir == Direction::Input && s.active()) {
            let name = self.clients.get(s.owner).and_then(|c| c.app.as_ref()).map_or_else(|| String::from("Systém"), |a| a.app_name.clone());
            if !v.contains(&name) {
                v.push(name);
            }
        }
        v
    }
}

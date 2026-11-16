//! Podpis za klienta SSH (`accountproto::agent`, `docs/cs/ssh.md`, Klient).
//!
//! - Kanal podpisu zaklada init (`agent` na bootstrap kanalu) jen pro
//!   systemovy program z initfs (`ssh`, `scp`, `sftp`) a jeho relaci; s nim
//!   prijde kanal systemove vyzvy do shellu te relace.
//! - Podepisuje se jen zprava prihlaseni SSH (RFC 4252) s blobem prave
//!   toho klice: sluzba neni obecny podpisovy orakl.
//! - Klice relace se odemykaji heslem uctu (pri prihlaseni heslem rovnou,
//!   jinak systemova vyzva); potvrzeni klice pro novy server je druha
//!   vyzva. Na odpoved se ceka bez blokovani sluzby (stav `Pending`).
//! - Kazdy podpis, odmitnuti a odemceni jde do logu (logd).

use alloc::string::String;
use alloc::vec::Vec;

use accountproto::agent::{AgentReq, Identity, Prompt, PromptAnswer, encode_identities};
use accountproto::{Secret, reply_err, reply_ok};
use libos::{Channel, Error, Handle, Result, println};
use zeroize::Zeroize;

use crate::service::{Service, now};

/// Kolikrat se zeptat na heslo, nez se podpis odmitne.
const UNLOCK_TRIES: u32 = 3;
/// Jak dlouho ceka vyzva na cloveka (ns).
const PROMPT_NS: u64 = 300_000_000_000;

/// Rozpracovany podpis (ceka na odpoved vyzvy).
struct Pending {
    key: u32,
    host: String,
    data: Vec<u8>,
    prompt: u32,
    unlock: bool,
    tries: u32,
    deadline: u64,
}

pub struct Agent {
    session: u64,
    uid: u64,
    app: String,
    pub chan: Channel,
    /// Systemova vyzva do shellu relace (`None` = jen klice odemcene
    /// prihlasenim, bez dotazu).
    pub prompt: Option<Channel>,
    pending: Option<Pending>,
    next_prompt: u32,
}

/// Odemcene klice relace (cislo klice, seed).
pub struct Unlocked {
    pub session: u64,
    pub seeds: Vec<(u32, Secret)>,
}

fn clean(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_graphic()).take(64).collect()
}

impl Service {
    /// `agent` od initu: data `u64 relace LE, u64 uid LE, id aplikace`,
    /// handly `[kanal podpisu, kanal vyzvy (volitelny)]`.
    pub fn add_agent(&mut self, data: &[u8], handles: Vec<Handle>) -> Result<()> {
        let (s, rest) = data.split_first_chunk::<8>().ok_or(Error::InvalidArgs)?;
        let (u, app) = rest.split_first_chunk::<8>().ok_or(Error::InvalidArgs)?;
        let (session, uid) = (u64::from_le_bytes(*s), u64::from_le_bytes(*u));
        let app = String::from(core::str::from_utf8(app).map_err(|_| Error::InvalidArgs)?);
        self.eng.account(uid).ok_or(Error::NotFound)?;
        let mut hs = handles.into_iter();
        let chan = hs.next().map(Channel::from_handle).ok_or(Error::InvalidArgs)?;
        let prompt = hs.next().map(Channel::from_handle);
        println!("accountd: ssh klient: {app} v relaci {session} (ucet {uid})");
        self.agents.push(Agent { session, uid, app, chan, prompt, pending: None, next_prompt: 1 });
        Ok(())
    }

    /// Po prihlaseni heslem: klice klienta relace rovnou odemcene.
    pub fn unlock_after_login(&mut self, session: u64, uid: u64, password: &Secret) {
        match self.eng.client_unlock_verified(uid, password) {
            Ok(seeds) if !seeds.is_empty() => {
                println!("accountd: ucet {uid}: klice klienta SSH odemceny prihlasenim (relace {session})");
                self.unlocked.push(Unlocked { session, seeds });
            }
            _ => {}
        }
    }

    /// Konec relace: klice z pameti pryc, kanaly podpisu zavrit.
    pub fn agent_session_closed(&mut self, session: u64) {
        self.unlocked.retain(|u| u.session != session);
        self.agents.retain(|a| a.session != session);
    }

    fn seed(&self, session: u64, key: u32) -> Option<[u8; 32]> {
        let u = self.unlocked.iter().find(|u| u.session == session)?;
        let (_, s) = u.seeds.iter().find(|(id, _)| *id == key)?;
        s.expose().try_into().ok()
    }

    /// Zpravy kanalu podpisu agenta `i`. `false` = kanal zavreny.
    pub fn agent_requests(&mut self, i: usize) -> bool {
        loop {
            if self.agents[i].pending.is_some() {
                return true;
            }
            let mut m = match self.agents[i].chan.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => return true,
                Err(_) => return false,
            };
            let r = AgentReq::decode(&m.data);
            m.data.zeroize();
            let reply = match r {
                Ok(AgentReq::Identities) => self.identities(i),
                Ok(AgentReq::Sign { id, host, data }) => match self.sign_start(i, id, host, data) {
                    Some(r) => r,
                    None => continue,
                },
                Err(e) => reply_err(e),
            };
            let _ = self.agents[i].chan.write(&reply, Vec::new());
        }
    }

    fn identities(&self, i: usize) -> Vec<u8> {
        match self.eng.client_keys(self.agents[i].uid) {
            Ok(list) => {
                let ids: Vec<Identity> = list.into_iter().map(|k| Identity { id: k.id, blob: k.public.blob(), comment: k.comment }).collect();
                reply_ok(&encode_identities(&ids))
            }
            Err(e) => reply_err(e),
        }
    }

    /// Zacatek podpisu; `None` = ceka na vyzvu.
    fn sign_start(&mut self, i: usize, key: u32, host: String, data: Vec<u8>) -> Option<Vec<u8>> {
        let a = &self.agents[i];
        let host = clean(&host);
        // Jen zprava prihlaseni s blobem tohoto klice.
        let blob = self.eng.client_keys(a.uid).ok().and_then(|l| l.into_iter().find(|k| k.id == key)).map(|k| k.public.blob());
        let ok = match (&blob, accounts::remote::parse_signed(&data)) {
            (Some(b), Ok((_, _, signed))) => signed == b.as_slice(),
            _ => false,
        };
        if !ok || !accounts::client::valid_host(&host) {
            return Some(reply_err(if blob.is_none() { Error::NotFound } else { Error::InvalidArgs }));
        }
        let mut p = Pending { key, host, data, prompt: 0, unlock: false, tries: 0, deadline: 0 };
        self.advance(i, &mut p)
    }

    /// Dalsi krok podpisu: odemceni, potvrzeni serveru, podpis.
    fn advance(&mut self, i: usize, p: &mut Pending) -> Option<Vec<u8>> {
        let (session, uid) = (self.agents[i].session, self.agents[i].uid);
        if self.seed(session, p.key).is_none() {
            if p.tries >= UNLOCK_TRIES {
                return Some(reply_err(Error::AccessDenied));
            }
            let q = alloc::format!("Klíče SSH účtu jsou zamčené. Heslo účtu pro přihlášení na {}:", p.host);
            return self.ask(i, p, true, q);
        }
        if !self.eng.client_host_known(uid, p.key, &p.host) {
            let fp = self.eng.client_keys(uid).ok().and_then(|l| l.into_iter().find(|k| k.id == p.key)).map(|k| k.fingerprint).unwrap_or_default();
            let q = alloc::format!("Použít klíč SSH {fp} pro přihlášení na {}? (program {})", p.host, self.agents[i].app);
            return self.ask(i, p, false, q);
        }
        let mut seed = self.seed(session, p.key)?;
        let sig = ssh::keys::ed25519_sign(&seed, &p.data);
        seed.zeroize();
        println!("accountd: ssh klient: ucet {uid} podepsal klicem {} pro {} ({})", p.key, p.host, self.agents[i].app);
        Some(reply_ok(&sig))
    }

    /// Systemova vyzva; bez kanalu vyzvy = zamceno.
    fn ask(&mut self, i: usize, p: &mut Pending, secret: bool, question: String) -> Option<Vec<u8>> {
        let a = &mut self.agents[i];
        let Some(ch) = &a.prompt else {
            return Some(reply_err(Error::Locked));
        };
        let id = a.next_prompt;
        a.next_prompt = a.next_prompt.wrapping_add(1).max(1);
        let m = if secret { Prompt::Secret { id, question } } else { Prompt::Confirm { id, question } };
        if ch.write(&m.encode(), Vec::new()).is_err() {
            return Some(reply_err(Error::Locked));
        }
        let mut next = Pending { key: p.key, host: core::mem::take(&mut p.host), data: core::mem::take(&mut p.data), prompt: id, unlock: secret, tries: p.tries, deadline: libos::time::now() + PROMPT_NS };
        if secret {
            next.tries += 1;
        }
        a.pending = Some(next);
        None
    }

    /// Odpovedi systemove vyzvy agenta `i`.
    pub fn agent_answers(&mut self, i: usize) {
        loop {
            let Some(ch) = &self.agents[i].prompt else { return };
            let mut m = match ch.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => break,
                Err(_) => {
                    // Shell vyzvu zavrel: rozpracovany podpis odmitnout.
                    self.agents[i].prompt = None;
                    if self.agents[i].pending.take().is_some() {
                        let _ = self.agents[i].chan.write(&reply_err(Error::Locked), Vec::new());
                    }
                    return;
                }
            };
            let a = PromptAnswer::decode(&m.data);
            m.data.zeroize();
            let Ok(a) = a else { continue };
            let Some(mut p) = self.agents[i].pending.take() else { continue };
            if a.id != p.prompt {
                self.agents[i].pending = Some(p);
                continue;
            }
            let (uid, session) = (self.agents[i].uid, self.agents[i].session);
            let reply = if !a.ok {
                println!("accountd: ssh klient: ucet {uid}: {} odmitnuto uzivatelem ({})", if p.unlock { "odemceni" } else { "klic pro server" }, p.host);
                Some(reply_err(Error::AccessDenied))
            } else if p.unlock {
                match self.eng.client_unlock(uid, &a.text, now()) {
                    Ok(seeds) => {
                        println!("accountd: ucet {uid}: klice klienta SSH odemceny (relace {session})");
                        self.unlocked.retain(|u| u.session != session);
                        self.unlocked.push(Unlocked { session, seeds });
                        self.advance(i, &mut p)
                    }
                    Err(Error::AccessDenied) => {
                        println!("accountd: ssh klient: ucet {uid}: spatne heslo k odemceni klicu");
                        self.advance(i, &mut p)
                    }
                    Err(e) => Some(reply_err(e)),
                }
            } else {
                match self.eng.client_host_approve(uid, p.key, &p.host) {
                    Ok(()) => self.advance(i, &mut p),
                    Err(e) => Some(reply_err(e)),
                }
            };
            if let Some(r) = reply {
                let _ = self.agents[i].chan.write(&r, Vec::new());
            }
        }
    }

    /// Vyzvy bez odpovedi po terminu: podpis odmitnout.
    pub fn agent_timeouts(&mut self) {
        let t = libos::time::now();
        for a in &mut self.agents {
            if a.pending.as_ref().is_some_and(|p| t > p.deadline) {
                a.pending = None;
                let _ = a.chan.write(&reply_err(Error::TimedOut), Vec::new());
            }
        }
    }
}

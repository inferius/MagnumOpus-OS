//! Opravneni relace (drzi ho sluzba `netd`, klient ho nevidi).
//!
//! Relace ma **prava**, **vrstvy omezeni** (kazde zuzeni prida vrstvu, cil
//! musi projit vsemi), **rozpocet** (sockety a pamet bufferu, sdileny
//! s nadrazenymi relacemi jako joby jadra) a **zapamatovane preklady jmen**
//! ze systemoveho resolveru (pravidla se jmenem hostitele).
//!
//! Vrstva muze nest identitu aplikace (`app`, z manifestu) a priznak
//! **zeptat se** (`ask`): cil mimo pravidla vrstvy pak neni hned odmitnut,
//! ale rozhodne uzivatel pres sluzbu firewallu (`fwd`). Trvala rozhodnuti
//! uzivatele ([`UserRules`]) plati jen pro vrstvy s `ask` a jen pro
//! aplikaci, ktere patri. Vrstva bez `ask` je pevna hranice: rodic tak
//! muze dite uzavrit, at uzivatel povoli cokoli.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;
use core::net::{IpAddr, SocketAddr};

use abi::{Error, Result};

use crate::codec::{Reader, Writer};
use crate::policy::{MAX_LAYERS, Policy, Rule, Target, normalize_name, rights};

/// Nejvic zapamatovanych prekladu jmen jedne relace (vyprsene se mazou
/// prvni, pak nejstarsi).
pub const MAX_LEARNED: usize = 4096;
/// Preklad plati nejmene tak dlouho (s), i kdyz ma DNS kratsi TTL: program
/// se po prekladu pripojuje pozdeji a kratke TTL (CDN) by spojeni zbytecne
/// rozbilo. Nejdele den.
pub const LEARNED_MIN_TTL: u64 = 300;
pub const LEARNED_MAX_TTL: u64 = 86_400;

/// Pamet sluzby za jednu relaci (vrstvy, preklady): strhava se rodicovske
/// relaci jako jeden socket, takze ani zakladani relaci neni bez meze.
pub const SESSION_COST: u64 = 2048;

/// Bez limitu (socketu, bajtu).
pub const UNLIMITED_SOCKETS: u32 = u32::MAX;
pub const UNLIMITED_BYTES: u64 = u64::MAX;
/// Vychozi limit soubeznych dotazu na relaci (connect, resolve, ping, ...).
/// Plati i pro relaci "bez limitu": rozpracovany dotaz drzi pamet sluzby.
pub const DEFAULT_OPS: u32 = 32;
/// Nejvic soubeznych dotazu, ktere si relace muze vyzadat.
pub const MAX_OPS: u32 = 1024;

/// Priznaky vrstvy ([`Restriction::flags`]).
pub mod layer_flags {
    /// Cil mimo pravidla vrstvy rozhodne uzivatel (firewall).
    pub const ASK: u8 = 1;
    pub const ALL: u8 = ASK;
}

/// Rozpocet relace: nejvic socketu a bajtu bufferu (i za potomky) a nejvic
/// soubeznych dotazu na kanalu relace (jen tato relace; meze
/// 1..=[`MAX_OPS`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    pub sockets: u32,
    pub bytes: u64,
    pub ops: u32,
}

impl Quota {
    pub const UNLIMITED: Self = Self {
        sockets: UNLIMITED_SOCKETS,
        bytes: UNLIMITED_BYTES,
        ops: DEFAULT_OPS,
    };
}

impl Default for Quota {
    fn default() -> Self {
        Self::UNLIMITED
    }
}

/// Citac rozpoctu jedne relace (strom: strzeni plati i pro predky).
#[derive(Debug)]
pub struct Budget {
    parent: Option<Rc<Budget>>,
    quota: Quota,
    sockets: Cell<u32>,
    bytes: Cell<u64>,
    /// Rozpracovane dotazy na kanalu teto relace.
    ops: Cell<u32>,
}

impl Budget {
    /// Rozpocet identity aplikace (sockety a bajty vsech jejich relaci,
    /// `Auth::set_identity`): sluzba ho drzi v registru podle identity.
    pub fn identity(quota: Quota) -> Rc<Self> {
        Self::new(None, quota)
    }

    fn new(parent: Option<Rc<Budget>>, quota: Quota) -> Rc<Self> {
        Rc::new(Self {
            parent,
            quota,
            sockets: Cell::new(0),
            bytes: Cell::new(0),
            ops: Cell::new(0),
        })
    }

    /// Nejvic soubeznych dotazu teto relace.
    pub fn ops_limit(&self) -> u32 {
        self.quota.ops.clamp(1, MAX_OPS)
    }

    /// Rozpracovane dotazy teto relace.
    pub fn ops_used(&self) -> u32 {
        self.ops.get()
    }

    fn fits(&self, bytes: u64) -> bool {
        let mut node = Some(self);
        while let Some(b) = node {
            if b.sockets.get() >= b.quota.sockets
                || b.bytes.get().saturating_add(bytes) > b.quota.bytes
            {
                return false;
            }
            node = b.parent.as_deref();
        }
        true
    }

    fn add(&self, sockets: i64, bytes: i64) {
        let mut node = Some(self);
        while let Some(b) = node {
            b.sockets.set((b.sockets.get() as i64 + sockets) as u32);
            b.bytes.set((b.bytes.get() as i64 + bytes) as u64);
            node = b.parent.as_deref();
        }
    }

    /// (sockety, bajty) teto relace vcetne potomku.
    pub fn used(&self) -> (u32, u64) {
        (self.sockets.get(), self.bytes.get())
    }
}

/// Jeden rozpracovany dotaz relace; pri zaniku se vrati.
#[derive(Debug)]
pub struct OpTicket {
    budget: Rc<Budget>,
}

impl Drop for OpTicket {
    fn drop(&mut self) {
        self.budget.ops.set(self.budget.ops.get() - 1);
    }
}

/// Strzeni jednoho socketu z rozpoctu relace (a identity); vrati se pri
/// zaniku.
#[derive(Debug)]
pub struct Usage {
    budget: Rc<Budget>,
    identity: Option<Rc<Budget>>,
    bytes: u64,
}

impl Drop for Usage {
    fn drop(&mut self) {
        self.budget.add(-1, -(self.bytes as i64));
        if let Some(i) = &self.identity {
            i.add(-1, -(self.bytes as i64));
        }
    }
}

/// Vrstva omezeni s identitou aplikace.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Layer {
    pub policy: Policy,
    /// Identita aplikace (z manifestu), prazdna = bez identity.
    pub app: String,
    pub ask: bool,
}

/// Popis zuzeni relace (`op::RESTRICT`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Restriction {
    /// Prava nove relace (jen podmnozina).
    pub rights: u32,
    pub policy: Policy,
    /// Identita aplikace pro firewall a audit (nejvys 128 znaku).
    pub app: String,
    /// [`layer_flags`].
    pub flags: u8,
    pub quota: Quota,
}

/// Nejdelsi identita aplikace.
pub const MAX_APP_ID: usize = 128;

impl Restriction {
    /// Jen prava a pravidla (bez identity, dotazu a rozpoctu).
    pub fn new(rights: u32, policy: Policy) -> Self {
        Self {
            rights,
            policy,
            ..Self::default()
        }
    }

    pub fn put(&self, w: &mut Writer) {
        w.u32(self.rights);
        self.policy.put(w);
        w.u8(self.flags).str(&self.app);
        w.u32(self.quota.sockets).u64(self.quota.bytes).u32(self.quota.ops);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let rights = r.u32()?;
        let policy = Policy::get(r)?;
        let flags = r.u8()?;
        let app = r.str()?;
        let quota = Quota {
            sockets: r.u32()?,
            bytes: r.u64()?,
            ops: r.u32()?,
        };
        if flags & !layer_flags::ALL != 0 || app.len() > MAX_APP_ID || !valid_app_id(&app) {
            return Err(Error::InvalidArgs);
        }
        Ok(Self {
            rights,
            policy,
            app,
            flags,
            quota,
        })
    }
}

/// Identita aplikace: prazdna, nebo znaky `a-z A-Z 0-9 . - _`.
pub fn valid_app_id(app: &str) -> bool {
    app.len() <= MAX_APP_ID
        && app
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
}

/// Trvala rozhodnuti uzivatele pro aplikace (spravuje `fwd`, `netd` je
/// dostane v konfiguraci firewallu). Plati jen pro vrstvy s `ask`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserRules {
    /// (aplikace, pravidlo, povolit)
    pub rules: Vec<(String, Rule, bool)>,
}

/// Vysledek kontroly cile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Deny,
    /// Rozhodne uzivatel: aplikace `app` chce cil mimo sva pravidla.
    Ask { app: String },
}

#[derive(Debug, Clone)]
struct Learned {
    name: String,
    ip: IpAddr,
    /// Konec platnosti (ns monotonickeho casu).
    expires: u64,
}

#[derive(Debug, Clone)]
pub struct Auth {
    pub rights: u32,
    layers: Vec<Layer>,
    learned: Vec<Learned>,
    budget: Rc<Budget>,
    /// Rozpocet identity aplikace: sockety vsech relaci stejne identity
    /// (procesy s dedenou siti ho neobejdou).
    identity: Option<Rc<Budget>>,
    /// Strzeni teto relace z rozpoctu rodice (koren nema).
    _held: Option<Rc<Usage>>,
}

impl Default for Auth {
    fn default() -> Self {
        Self::root()
    }
}

impl Auth {
    /// Korenova relace (ma ji init): vsechna prava, bez omezeni.
    pub fn root() -> Self {
        Self::root_with(Quota::UNLIMITED)
    }

    /// Korenova relace s rozpoctem (napr. podle pameti sluzby).
    pub fn root_with(quota: Quota) -> Self {
        Self {
            rights: rights::ALL,
            layers: Vec::new(),
            learned: Vec::new(),
            budget: Budget::new(None, quota),
            identity: None,
            _held: None,
        }
    }

    /// Uzsi relace: prava jen podmnozina, k vrstvam pribude nova a rozpocet
    /// je podrizeny rozpoctu teto relace. Zapamatovane preklady se zdedi.
    pub fn restrict_with(&self, r: Restriction) -> Result<Self> {
        r.policy.validate()?;
        if self.layers.len() >= MAX_LAYERS {
            return Err(Error::NoResources);
        }
        if r.flags & layer_flags::ASK != 0 && r.app.is_empty() {
            // Dotaz bez identity by uzivatel nemel s cim spojit.
            return Err(Error::InvalidArgs);
        }
        // Cena relace jde jen do stromu relaci (rozpocet identity pocita
        // sockety aplikace, ne jeji relace).
        if !self.budget.fits(SESSION_COST) {
            return Err(Error::NoResources);
        }
        self.budget.add(1, SESSION_COST as i64);
        let held = Usage { budget: self.budget.clone(), identity: None, bytes: SESSION_COST };
        let mut layers = self.layers.clone();
        layers.push(Layer {
            policy: r.policy,
            app: r.app,
            ask: r.flags & layer_flags::ASK != 0,
        });
        Ok(Self {
            rights: self.rights & r.rights,
            layers,
            learned: self.learned.clone(),
            budget: Budget::new(Some(self.budget.clone()), r.quota),
            // Stejna identita (zuzeni bez nove identity) = stejny rozpocet.
            identity: self.identity.clone(),
            _held: Some(Rc::new(held)),
        })
    }

    /// Rozpocet identity (sluzba ho sdili mezi relacemi stejne identity).
    pub fn set_identity(&mut self, budget: Rc<Budget>) {
        self.identity = Some(budget);
    }

    pub fn identity_budget(&self) -> Option<&Rc<Budget>> {
        self.identity.as_ref()
    }

    /// Uzsi relace jen s pravy a pravidly.
    pub fn restrict(&self, rights: u32, policy: Policy) -> Result<Self> {
        self.restrict_with(Restriction::new(rights, policy))
    }

    pub fn has(&self, right: u32) -> bool {
        self.rights & right == right
    }

    /// Ma pravo `right`? Jinak `ACCESS_DENIED`.
    pub fn require(&self, right: u32) -> Result<()> {
        if self.has(right) {
            Ok(())
        } else {
            Err(Error::AccessDenied)
        }
    }

    /// Identita aplikace (nejvnitrnejsi vrstva s identitou), pro audit.
    pub fn app(&self) -> &str {
        self.layers
            .iter()
            .rev()
            .find(|l| !l.app.is_empty())
            .map_or("", |l| l.app.as_str())
    }

    /// Strhne socket s `bytes` bajty bufferu z rozpoctu (`NO_RESOURCES`).
    /// Zacatek dotazu na kanalu relace: nad limit [`Quota::ops`]
    /// `NO_RESOURCES`.
    pub fn begin_op(&self) -> Result<OpTicket> {
        let b = &self.budget;
        if b.ops.get() >= b.ops_limit() {
            return Err(Error::NoResources);
        }
        b.ops.set(b.ops.get() + 1);
        Ok(OpTicket { budget: b.clone() })
    }

    pub fn charge(&self, bytes: u64) -> Result<Usage> {
        if !self.budget.fits(bytes) || self.identity.as_ref().is_some_and(|i| !i.fits(bytes)) {
            return Err(Error::NoResources);
        }
        self.budget.add(1, bytes as i64);
        if let Some(i) = &self.identity {
            i.add(1, bytes as i64);
        }
        Ok(Usage {
            budget: self.budget.clone(),
            identity: self.identity.clone(),
            bytes,
        })
    }

    pub fn budget(&self) -> &Budget {
        &self.budget
    }

    fn rule_matches(&self, rule: &Rule, proto: u8, dest: SocketAddr, now: u64) -> bool {
        rule.matches_with(proto, dest, |pattern, ip| {
            self.learned.iter().any(|l| {
                l.ip == ip && l.expires > now && crate::policy::host_matches(pattern, &l.name)
            })
        })
    }

    fn layer_allows(&self, l: &Layer, proto: u8, dest: SocketAddr, now: u64) -> bool {
        match &l.policy.connect {
            None => true,
            Some(rules) => rules.iter().any(|r| self.rule_matches(r, proto, dest, now)),
        }
    }

    /// Smi relace komunikovat s `dest` protokolem `proto` (jeden bit
    /// z `proto`)? Bez rozhodnuti uzivatele (vrstva s `ask` = ne).
    pub fn allows(&self, proto: u8, dest: SocketAddr, now: u64) -> bool {
        self.check(proto, dest, now, &UserRules::default(), &[]) == Verdict::Allow
    }

    /// Kontrola cile vcetne trvalych rozhodnuti uzivatele. `once` =
    /// aplikace, kterym uzivatel prave povolil tento jeden pozadavek.
    pub fn check(
        &self,
        proto: u8,
        dest: SocketAddr,
        now: u64,
        user: &UserRules,
        once: &[String],
    ) -> Verdict {
        let mut ask = None;
        for l in &self.layers {
            if self.layer_allows(l, proto, dest, now) {
                continue;
            }
            if !l.ask {
                return Verdict::Deny;
            }
            if once.contains(&l.app) {
                continue;
            }
            // Zakaz ma prednost pred povolenim (stejne jako ve fwd).
            let decided = user
                .rules
                .iter()
                .filter(|(app, rule, _)| *app == l.app && self.rule_matches(rule, proto, dest, now))
                .fold(None, |acc, (_, _, allow)| match (acc, *allow) {
                    (Some(false), _) | (_, false) => Some(false),
                    _ => Some(true),
                });
            match decided {
                Some(true) => {}
                Some(false) => return Verdict::Deny,
                None => {
                    ask.get_or_insert_with(|| l.app.clone());
                }
            }
        }
        match ask {
            Some(app) => Verdict::Ask { app },
            None => Verdict::Allow,
        }
    }

    /// Smi relace prijimat spojeni ze site (ne jen z loopbacku)? Vsechny
    /// vrstvy musi mit `BindScope::Any`.
    pub fn allows_bind_net(&self) -> bool {
        self.layers.iter().all(|l| l.policy.bind_scope == crate::policy::BindScope::Any)
    }

    /// Smi relace naslouchat / prijimat na mistnim portu `port`?
    pub fn allows_bind(&self, port: u16) -> bool {
        port == 0
            || self.layers.iter().all(|l| match &l.policy.bind {
                None => true,
                Some(ranges) => ranges.iter().any(|(lo, hi)| (*lo..=*hi).contains(&port)),
            })
    }

    /// Zapamatuje si preklad jmena od systemoveho resolveru (pro pravidla
    /// se jmenem hostitele) s platnosti podle TTL (`ttl` s, `now` ns).
    pub fn learn(&mut self, name: &str, addrs: &[IpAddr], ttl: u32, now: u64) {
        let name = normalize_name(name);
        let life = (ttl as u64).clamp(LEARNED_MIN_TTL, LEARNED_MAX_TTL) * 1_000_000_000;
        let expires = now.saturating_add(life);
        for a in addrs {
            if let Some(l) = self.learned.iter_mut().find(|l| l.name == name && l.ip == *a) {
                l.expires = l.expires.max(expires);
                continue;
            }
            if self.learned.len() >= MAX_LEARNED {
                self.learned.retain(|l| l.expires > now);
            }
            if self.learned.len() >= MAX_LEARNED {
                // Nejdriv koncici zaznam uvolni misto.
                if let Some(i) = (0..self.learned.len()).min_by_key(|&i| self.learned[i].expires) {
                    self.learned.swap_remove(i);
                }
            }
            self.learned.push(Learned {
                name: name.clone(),
                ip: *a,
                expires,
            });
        }
    }

    /// Jmeno hostitele, pro ktere relace adresu `ip` dostala (pro dotaz
    /// uzivateli: "chce na api.example.com" je srozumitelnejsi nez adresa).
    pub fn name_for(&self, ip: IpAddr, now: u64) -> Option<&str> {
        self.learned
            .iter()
            .rev()
            .find(|l| l.ip == ip && l.expires > now)
            .map(|l| l.name.as_str())
    }

    /// Ma nektera vrstva (nebo rozhodnuti uzivatele) pravidlo se jmenem
    /// hostitele? Jen pak ma smysl preklady zapamatovat.
    pub fn has_host_rules(&self, user: &UserRules) -> bool {
        let host = |r: &Rule| matches!(r.target, Target::Host(_));
        self.layers.iter().any(|l| {
            l.policy.connect.as_ref().is_some_and(|r| r.iter().any(host))
                || (l.ask && user.rules.iter().any(|(app, r, _)| *app == l.app && host(r)))
        })
    }
}

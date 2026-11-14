//! Pravidla obnovy pro knihovny klientu (odsouhlasena uzivatelem, viz
//! `docs/cs/obnova-sluzeb.md`, kapitola "Pravidla pro knihovny klientu").
//!
//! - operace bez vedlejsiho ucinku ([`Kind::Idempotent`]) knihovna po padu
//!   sluzby sama zopakuje na nove instanci,
//! - operace s ucinkem ([`Kind::Effect`]) a kazdy pozadavek s handly vrati
//!   `SERVICE_RESTARTED` (nevime, jestli je stara instance provedla),
//! - na novou instanci se ceka [`DEFAULT_WAIT`] (klient smi nastavit
//!   vlastni), pak `SERVICE_UNAVAILABLE`.

use core::time::Duration;

use abi::Error;

/// Vychozi cekani na novou instanci sluzby.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(10);

/// Rezerva nad cekanim klienta na odpoved listku: supervisor odpovi
/// nejpozdeji po `cekat_ms`, rezerva pokryje jeho zpozdeni ve smycce.
pub const REPLY_MARGIN: Duration = Duration::from_secs(5);

/// Nejvic pokusu o jeden pozadavek (sluzba muze padat opakovane; obnova
/// pak skonci chybou drive, tohle je jen pojistka proti smycce).
pub const MAX_ATTEMPTS: u32 = 4;

/// Jak dlouho po vydani relace jeste povazovat zavreni kanalu za mozny
/// pad (ne za umyslne zavreni sluzbou): supervisor ceka, jestli instance
/// skonci.
pub const CLOSE_GRACE: Duration = Duration::from_millis(200);

/// Druh operace podle toho, jestli ji jde bezpecne zopakovat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Opakovani nic nezmeni (cteni, stat, vypis, info, dotaz).
    Idempotent,
    /// Ma vedlejsi ucinek (zapis, prejmenovani, smazani, zamek, podminene
    /// ulozeni, odeslani dat, akce okna).
    Effect,
}

/// Udalost obnovy pro aplikaci (UI ukaze "sluzba se restartuje").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Kanal ke sluzbe zanikl, knihovna ceka na novou instanci.
    Restarting,
    /// Relace obnovena na instanci `generation`.
    Restored { generation: u64 },
    /// Obnova se nepovedla (`SERVICE_UNAVAILABLE`, `PEER_CLOSED`, ...).
    Unavailable(Error),
}

/// Co udelat s pozadavkem, jehoz kanal zanikl, kdyz se relace obnovila.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum After {
    /// Poslat znovu na novou instanci.
    Resend,
    /// Vratit chybu (aplikace rozhodne).
    Fail(Error),
}

/// Rozhodnuti po ztrate kanalu. `delivered` = zapis pozadavku se povedl
/// (stara instance ho mohla zpracovat); `had_handles` = pozadavek nesl
/// handly (predaly se stare instanci a zanikly s ni).
pub fn after_loss(kind: Kind, delivered: bool, had_handles: bool) -> After {
    if had_handles {
        return After::Fail(Error::ServiceRestarted);
    }
    match (kind, delivered) {
        // Zapis selhal: pozadavek ke sluzbe nedorazil, jde poslat znovu.
        (_, false) => After::Resend,
        (Kind::Idempotent, true) => After::Resend,
        (Kind::Effect, true) => After::Fail(Error::ServiceRestarted),
    }
}

/// Cekani klienta v ms pro pole `cekat_ms` (horni mez protokolu).
pub fn wait_ms(wait: Duration) -> u32 {
    (wait.as_millis().min(crate::proto::MAX_WAIT_MS as u128)) as u32
}

/// Omezovac `ADOPT` jednoho listku (supervisor): nejvic
/// `leaseproto::adopt::MAX_LIVE` zijicich prevzatych listku a rychlost
/// pres tokeny (davka `BURST`, doplneni `PER_SEC` za sekundu). Cas
/// v nanosekundach (monotonni hodiny).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdoptLimit {
    /// Zijici prevzate listky.
    pub live: u32,
    /// Tokeny v tisicinach (presne doplnovani po malych krocich).
    milli_tokens: u64,
    last_ns: u64,
}

impl AdoptLimit {
    pub const fn new(now_ns: u64) -> Self {
        Self { live: 0, milli_tokens: leaseproto::adopt::BURST as u64 * 1000, last_ns: now_ns }
    }

    /// Smi listek prevzit dalsi? `true` = ano (a zapocita ho).
    pub fn take(&mut self, now_ns: u64) -> bool {
        use leaseproto::adopt::{BURST, MAX_LIVE, PER_SEC};
        let dt = now_ns.saturating_sub(self.last_ns);
        self.last_ns = self.last_ns.max(now_ns);
        // PER_SEC tokenu za 1e9 ns = PER_SEC tisicin tokenu za 1e6 ns.
        let add = dt / 1_000_000 * u64::from(PER_SEC);
        self.milli_tokens = (self.milli_tokens + add).min(u64::from(BURST) * 1000);
        if self.live >= MAX_LIVE || self.milli_tokens < 1000 {
            return false;
        }
        self.milli_tokens -= 1000;
        self.live += 1;
        true
    }

    /// Prevzaty listek zanikl.
    pub fn release(&mut self) {
        self.live = self.live.saturating_sub(1);
    }
}

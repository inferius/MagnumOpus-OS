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

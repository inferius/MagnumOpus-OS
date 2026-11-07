//! Nastaveni casu systemu: pasmo a v jakem case jsou hardwarove hodiny.
//!
//! Textovy tvar (radky `klic = hodnota`, `#` = komentar):
//!
//! ```text
//! zone = Europe/Prague   # jmeno z tabulky nebo UTC+2, UTC-03:30
//! rtc = local            # utc (vychozi; Linux) nebo local (Windows)
//! ```

use abi::time::{Reading, WallTime, flags, source};

use crate::Zone;

/// Nastaveni casu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Pasmo pro zobrazeni mistniho casu.
    pub zone: Zone,
    /// Hardwarove hodiny (RTC) jsou v mistnim case pasma `zone`, ne v UTC.
    /// Typicky dual-boot s Windows. Pak se cas pri startu opravi
    /// ([`rtc_local_correction`]) a do RTC se zapisuje mistni cas.
    pub rtc_local: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { zone: Zone::UTC, rtc_local: false }
    }
}

/// Chyba v textu nastaveni: cislo radku (od 1) a co je spatne.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub what: &'static str,
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "radek {}: {}", self.line, self.what)
    }
}

impl Settings {
    /// Precte nastaveni z textu; chybejici klice maji vychozi hodnotu.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let mut s = Self::default();
        for (i, line) in text.lines().enumerate() {
            let err = |what| ParseError { line: i + 1, what };
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (key, value) = line.split_once('=').ok_or(err("chybi '='"))?;
            let value = value.trim();
            match key.trim() {
                "zone" => s.zone = Zone::parse(value).ok_or(err("nezname pasmo"))?,
                "rtc" => {
                    s.rtc_local = match value {
                        "utc" => false,
                        "local" => true,
                        _ => return Err(err("rtc musi byt utc nebo local")),
                    }
                }
                _ => return Err(err("neznamy klic")),
            }
        }
        Ok(s)
    }

    /// Posun pro zapis do hardwarovych hodin v okamziku `t`
    /// (`abi::time::SetRequest::rtc_offset`): posun pasma, kdyz je RTC
    /// v mistnim case, jinak 0.
    pub fn rtc_offset(&self, t: WallTime) -> i32 {
        if self.rtc_local { self.zone.offset_at(t.secs).secs } else { 0 }
    }
}

/// Oprava casu pri startu, kdyz je RTC v mistnim case.
///
/// Jadro cte firmware/RTC a bez udaje o pasmu cas bere jako UTC (priznak
/// `ZONE_ASSUMED_UTC`). Kdyz nastaveni rika, ze RTC je v mistnim case,
/// precteny cas je ve skutecnosti mistni: spravne UTC = mistni - posun
/// pasma. Vraci cas k nastaveni (`CLOCK_SET`), nebo `None`, kdyz oprava
/// neni potreba: RTC v UTC, cas uz byl nastaven za behu (`source::SET`,
/// oprava se tak neprovede dvakrat), nebo firmware pasmo uvedl.
///
/// V hodine, ktera se pri konci letniho casu opakuje, nejde poznat, ktera
/// z nich to je; bere se drivejsi (letni cas). Chyba je pak 1 hodina, dokud
/// cas neopravi sit.
pub fn rtc_local_correction(settings: &Settings, reading: &Reading) -> Option<WallTime> {
    let from_boot = reading.source == source::UEFI || reading.source == source::RTC;
    if !settings.rtc_local || !from_boot || reading.flags & flags::ZONE_ASSUMED_UTC == 0 {
        return None;
    }
    let utc = settings.zone.local_secs_to_utc(reading.time.secs).resolve();
    Some(WallTime { secs: utc, nanos: reading.time.nanos })
}

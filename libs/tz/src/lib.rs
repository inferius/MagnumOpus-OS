//! Casova pasma: prevod UTC na mistni cas a zpet.
//!
//! Jadro drzi realny cas jen v UTC; pasmo je nastaveni systemu (sluzba casu,
//! programy). Tato knihovna nema tzdata: pasma popisuje kompaktni tabulka
//! ([`table`]) se **soucasnymi** pravidly - pevny posun od UTC a nejvys jedno
//! pravidlo letniho casu (zacatek a konec jako "n-ta / posledni nedele
//! v mesici v danou hodinu"). Pouziti v programech pres `libos::time::zone`.
//!
//! Omezeni (vedome):
//! - Historie se nepocita: pravidla plati pro vsechny roky. Napr. v Praze
//!   plati pravidla EU od roku 1996; pro starsi data (a pred 1979, kdy se
//!   letni cas nepouzival) vyjde mistni cas o hodinu jinak nez skutecne byl.
//! - Pasma s vic nez dvema prechody za rok nebo se zapornym letnim casem
//!   (Irsko v tzdata) tabulka neumi; posun je ale spravny.
//! - Prestupne sekundy se nepocitaji (stejne jako `abi::time`).

#![cfg_attr(not(test), no_std)]

mod settings;
mod table;
#[cfg(test)]
mod tests;

use core::fmt;

pub use abi::time::{Civil, WallTime, weekday};
pub use settings::{ParseError, Settings, rtc_local_correction};
pub use table::ZONES;

/// Nejvetsi posun od UTC (absolutne), ktery prijmeme: 18 hodin (stejna mez
/// jako `rtc_offset` v `CLOCK_SET`).
pub const MAX_OFFSET: i32 = abi::time::MAX_ZONE_OFFSET;

/// Vuci jakemu casu je zadana hodina prechodu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum At {
    /// Hodina v UTC (EU: 01:00 UTC ve vsech pasmech naraz).
    Utc,
    /// Mistni standardni (zimni) cas.
    Standard,
    /// Mistni cas platny tesne pred prechodem (USA: 02:00 "na hodinach").
    Wall,
}

/// Okamzik prechodu v roce: `week`-ta nedele (1..=4, [`LAST`] = posledni)
/// mesice `month`, `secs` sekund po pulnoci podle `at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub month: u8,
    pub week: u8,
    /// Den v tydnu, 0 = nedele.
    pub weekday: u8,
    pub secs: i32,
    pub at: At,
}

/// Hodnota `Transition::week` = posledni takovy den v mesici.
pub const LAST: u8 = 5;

impl Transition {
    /// Posledni den `weekday` mesice `month` v `secs` sekund po pulnoci.
    pub const fn last(month: u8, weekday: u8, secs: i32, at: At) -> Self {
        Self { month, week: LAST, weekday, secs, at }
    }

    /// `week`-ty (1..=4) den `weekday` mesice `month`.
    pub const fn nth(week: u8, month: u8, weekday: u8, secs: i32, at: At) -> Self {
        Self { month, week, weekday, secs, at }
    }

    /// Den prechodu v roce `year` (dny od 1970); `None` = rok mimo rozsah.
    pub fn day(&self, year: i64) -> Option<i64> {
        if self.week == LAST {
            let last = abi::time::days_in_month(year, self.month);
            let d = abi::time::days_from_civil(year, self.month, last)?;
            Some(d - (weekday(d) as i64 - self.weekday as i64).rem_euclid(7))
        } else {
            let d = abi::time::days_from_civil(year, self.month, 1)?;
            let first = d + (self.weekday as i64 - weekday(d) as i64).rem_euclid(7);
            Some(first + (self.week as i64 - 1) * 7)
        }
    }

    /// Okamzik prechodu v UTC (sekundy od 1970). `std` = standardni posun
    /// pasma, `before` = posun platny tesne pred prechodem.
    pub fn utc(&self, year: i64, std: i32, before: i32) -> Option<i64> {
        let base = match self.at {
            At::Utc => 0,
            At::Standard => std,
            At::Wall => before,
        };
        self.day(year)?
            .checked_mul(86_400)?
            .checked_add(self.secs as i64 - base as i64)
    }
}

/// Letni cas: posun navic (`save`, kladny) od `start` do `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dst {
    pub save: i32,
    pub abbrev: &'static str,
    pub start: Transition,
    pub end: Transition,
}

/// Casove pasmo: standardni posun od UTC (vychod kladne) a volitelne letni cas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zone {
    /// Jmeno podle IANA (`Europe/Prague`); pevny posun ma prazdne jmeno.
    pub name: &'static str,
    pub std_offset: i32,
    /// Zkratka standardniho casu (`CET`); prazdna = zobrazit posun.
    pub std_abbrev: &'static str,
    pub dst: Option<Dst>,
}

/// Posun platny v jednom okamziku.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offset {
    /// Sekundy pridane k UTC (Praha v lete: 7200).
    pub secs: i32,
    pub dst: bool,
    /// Zkratka (`CEST`); prazdna = zadna.
    pub abbrev: &'static str,
}

/// Mistni cas: kalendarni datum a posun, ktery plati.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    pub civil: Civil,
    pub offset: Offset,
}

/// Vysledek prevodu mistniho casu na UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalResult {
    /// Jednoznacny okamzik (sekundy UTC od 1970).
    Single(i64),
    /// Mistni cas nastal dvakrat (konec letniho casu, hodina se opakuje).
    Ambiguous { earlier: i64, later: i64 },
    /// Mistni cas neexistoval (zacatek letniho casu, hodina se preskocila).
    /// Hodnota = prevod posunem platnym pred mezerou (02:30 v Praze pri
    /// zacatku letniho casu -> 01:30 UTC, tj. 03:30 SELC).
    Gap(i64),
}

impl LocalResult {
    /// Jeden okamzik: pri dvojznacnosti drivejsi (jeste letni cas), pri
    /// mezere prevod posunem pred ni. Stejne jako `mktime` s `tm_isdst = -1`
    /// ve vetsine implementaci.
    pub fn resolve(self) -> i64 {
        match self {
            Self::Single(t) | Self::Gap(t) => t,
            Self::Ambiguous { earlier, .. } => earlier,
        }
    }
}

impl Zone {
    pub const UTC: Self = Self {
        name: "UTC",
        std_offset: 0,
        std_abbrev: "UTC",
        dst: None,
    };

    /// Pasmo z tabulky podle jmena IANA (velikost pismen nehraje roli).
    pub fn by_name(name: &str) -> Option<Self> {
        ZONES.iter().find(|z| z.name.eq_ignore_ascii_case(name)).copied()
    }

    /// Pevny posun od UTC v sekundach (bez letniho casu), nejvys +/- 18 h.
    pub const fn fixed(secs: i32) -> Option<Self> {
        if secs < -MAX_OFFSET || secs > MAX_OFFSET {
            return None;
        }
        Some(Self { name: "", std_offset: secs, std_abbrev: "", dst: None })
    }

    /// Jmeno z tabulky (`Europe/Prague`) nebo pevny posun `UTC+2`,
    /// `UTC-03:30`, `GMT+05:45`, `+01:00`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if let Some(z) = Self::by_name(text) {
            return Some(z);
        }
        let rest = ["UTC", "GMT", "utc", "gmt"]
            .iter()
            .find_map(|p| text.strip_prefix(p))
            .unwrap_or(text);
        let (sign, rest) = match rest.as_bytes().first()? {
            b'+' => (1, &rest[1..]),
            b'-' => (-1, &rest[1..]),
            _ => return None,
        };
        let (h, m) = match rest.split_once(':') {
            Some((h, m)) => (h, m),
            None => (rest, "0"),
        };
        let digits = |s: &str, max: u32| -> Option<i32> {
            let ok = !s.is_empty() && s.len() <= 2 && s.bytes().all(|b| b.is_ascii_digit());
            let v: u32 = s.parse().ok().filter(|_| ok)?;
            (v <= max).then_some(v as i32)
        };
        Self::fixed(sign * (digits(h, 18)? * 3600 + digits(m, 59)? * 60))
    }

    /// Okamziky zacatku a konce letniho casu v roce `year` (podle
    /// standardniho mistniho casu), UTC. `None` = pasmo letni cas nema.
    pub fn transitions(&self, year: i64) -> Option<(i64, i64)> {
        let d = self.dst?;
        let std = self.std_offset;
        let start = d.start.utc(year, std, std)?;
        let end = d.end.utc(year, std, std + d.save)?;
        Some((start, end))
    }

    /// Posun platny v okamziku `utc` (sekundy od 1970).
    pub fn offset_at(&self, utc: i64) -> Offset {
        let std = Offset { secs: self.std_offset, dst: false, abbrev: self.std_abbrev };
        let Some(d) = self.dst else {
            return std;
        };
        let local_std = utc.saturating_add(self.std_offset as i64);
        let (year, _, _) = abi::time::civil_from_days(local_std.div_euclid(86_400));
        let Some((start, end)) = self.transitions(year) else {
            return std;
        };
        // Severni polokoule: leto uvnitr roku; jizni: leto pres novy rok.
        let summer = if start < end {
            utc >= start && utc < end
        } else {
            utc >= start || utc < end
        };
        if summer {
            Offset { secs: self.std_offset + d.save, dst: true, abbrev: d.abbrev }
        } else {
            std
        }
    }

    /// Mistni cas v okamziku `t`.
    pub fn to_local(&self, t: WallTime) -> Local {
        let offset = self.offset_at(t.secs);
        let shifted = WallTime { secs: t.secs.saturating_add(offset.secs as i64), nanos: t.nanos };
        Local { civil: shifted.to_civil(), offset }
    }

    /// Mistni cas `local` (sekundy od 1970 "na mistnich hodinach") na UTC.
    pub fn local_secs_to_utc(&self, local: i64) -> LocalResult {
        let std = self.std_offset;
        let Some(d) = self.dst else {
            return LocalResult::Single(local.saturating_sub(std as i64));
        };
        let as_std = local.saturating_sub(std as i64);
        let as_dst = local.saturating_sub((std + d.save) as i64);
        let std_ok = self.offset_at(as_std).secs == std;
        let dst_ok = self.offset_at(as_dst).secs == std + d.save;
        match (std_ok, dst_ok) {
            (true, true) => LocalResult::Ambiguous { earlier: as_dst, later: as_std },
            (true, false) => LocalResult::Single(as_std),
            (false, true) => LocalResult::Single(as_dst),
            // Mezera: pred ni platil standardni cas.
            (false, false) => LocalResult::Gap(as_std),
        }
    }

    /// Kalendarni mistni cas na UTC; `None` = neplatne datum.
    pub fn local_to_utc(&self, local: &Civil) -> Option<LocalResult> {
        let t = WallTime::from_civil(local)?;
        Some(self.local_secs_to_utc(t.secs))
    }

    /// Popis pro zobrazeni: jmeno, nebo posun (`UTC+05:30`).
    pub fn display(&self) -> impl fmt::Display + '_ {
        ZoneDisplay(self)
    }
}

struct ZoneDisplay<'a>(&'a Zone);

impl fmt::Display for ZoneDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.name.is_empty() {
            write!(f, "UTC{}", FormatOffset(self.0.std_offset))
        } else {
            f.write_str(self.0.name)
        }
    }
}

/// Posun ve tvaru `+02:00` / `-03:30`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatOffset(pub i32);

impl fmt::Display for FormatOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { '-' } else { '+' };
        let a = self.0.unsigned_abs();
        write!(f, "{sign}{:02}:{:02}", a / 3600, a / 60 % 60)
    }
}

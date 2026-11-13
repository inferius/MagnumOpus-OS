//! Jednotky jako data: kazdy radek tabulky [`TABLE`] je zapis, rozmer
//! (velicina) a nasobek zakladni jednotky rozmeru (zlomek `num / den`).
//! Nova velicina (delka, teplota, rychlost prenosu) = novy [`Dim`]
//! a radky tabulky; lexer, `convert`, vypis i doplnovani ctou tabulku.
//!
//! Hodnoty: velikost a doba jsou presna cela cisla (`Value::Size` v bajtech,
//! `Value::Duration` v ns) jako dosud; ostatni rozmery jsou
//! [`crate::value::Quantity`] (desetinne cislo v zakladni jednotce).
//!
//! Otevrena otazka: `m` je dnes minuta; az pribude delka, kolidoval by
//! s metrem (viz docs/cs/navrh-shell.md, kapitola Jednotky).

/// Rozmer (velicina).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dim {
    /// Velikost dat, zaklad bajt.
    Size,
    /// Doba, zaklad nanosekunda.
    Duration,
    /// Frekvence, zaklad hertz.
    Frequency,
    /// Pomer, zaklad 1 (procento = 1/100).
    Ratio,
}

impl Dim {
    /// Jmeno typu hodnoty (`type_name`, chybova hlaseni).
    pub const fn type_name(self) -> &'static str {
        match self {
            Dim::Size => "size",
            Dim::Duration => "duration",
            Dim::Frequency => "frequency",
            Dim::Ratio => "percent",
        }
    }

    /// Cesky nazev veliciny (napoveda, chyby).
    pub const fn name_cs(self) -> &'static str {
        match self {
            Dim::Size => "velikost",
            Dim::Duration => "doba",
            Dim::Frequency => "frekvence",
            Dim::Ratio => "procenta",
        }
    }

    /// Ciselny kod rozmeru v protokolu (stabilni, nove na konec).
    pub const fn code(self) -> u8 {
        match self {
            Dim::Size => 0,
            Dim::Duration => 1,
            Dim::Frequency => 2,
            Dim::Ratio => 3,
        }
    }

    pub const fn from_code(c: u8) -> Option<Dim> {
        Some(match c {
            0 => Dim::Size,
            1 => Dim::Duration,
            2 => Dim::Frequency,
            3 => Dim::Ratio,
            _ => return None,
        })
    }
}

/// Radek tabulky jednotek.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Def {
    /// Zapis za cislem (`MB`, `ms`, `kHz`, `%`).
    pub sym: &'static str,
    pub dim: Dim,
    /// Nasobek zakladni jednotky: `num / den`.
    pub num: u128,
    pub den: u128,
    /// Jednotka pro vypis (vyber nejvetsi, ve ktere je hodnota >= 1).
    pub show: bool,
}

const S: u128 = 1_000_000_000;

const fn d(sym: &'static str, dim: Dim, num: u128, den: u128, show: bool) -> Def {
    Def { sym, dim, num, den, show }
}

/// Tabulka jednotek. Poradi v ramci rozmeru: od nejmensi; zapis se hleda
/// presne (rozlisuje velka a mala pismena).
pub const TABLE: &[Def] = &[
    // velikosti (desitkove a dvojkove)
    d("B", Dim::Size, 1, 1, true),
    d("KB", Dim::Size, 1000, 1, true),
    d("MB", Dim::Size, 1_000_000, 1, true),
    d("GB", Dim::Size, 1_000_000_000, 1, true),
    d("TB", Dim::Size, 1_000_000_000_000, 1, true),
    d("KiB", Dim::Size, 1 << 10, 1, false),
    d("MiB", Dim::Size, 1 << 20, 1, false),
    d("GiB", Dim::Size, 1 << 30, 1, false),
    d("TiB", Dim::Size, 1 << 40, 1, false),
    // doby
    d("ns", Dim::Duration, 1, 1, true),
    d("us", Dim::Duration, 1000, 1, true),
    d("ms", Dim::Duration, 1_000_000, 1, true),
    d("s", Dim::Duration, S, 1, true),
    d("m", Dim::Duration, 60 * S, 1, true),
    d("min", Dim::Duration, 60 * S, 1, false),
    d("h", Dim::Duration, 3600 * S, 1, true),
    d("d", Dim::Duration, 86400 * S, 1, true),
    d("w", Dim::Duration, 7 * 86400 * S, 1, false),
    d("y", Dim::Duration, 365 * 86400 * S, 1, false),
    // frekvence
    d("Hz", Dim::Frequency, 1, 1, true),
    d("kHz", Dim::Frequency, 1000, 1, true),
    d("MHz", Dim::Frequency, 1_000_000, 1, true),
    d("GHz", Dim::Frequency, 1_000_000_000, 1, true),
    // pomery
    d("%", Dim::Ratio, 1, 100, true),
];

/// Jednotka = index do [`TABLE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unit(u8);

impl Unit {
    pub fn def(self) -> &'static Def {
        &TABLE[self.0 as usize]
    }

    pub fn name(self) -> &'static str {
        self.def().sym
    }

    pub fn dim(self) -> Dim {
        self.def().dim
    }

    pub fn parse(s: &str) -> Option<Unit> {
        TABLE.iter().position(|d| d.sym == s).map(|i| Unit(i as u8))
    }

    /// Je to velikost (bajty)?
    pub fn is_size(self) -> bool {
        self.dim() == Dim::Size
    }

    /// Nasobek zakladni jednotky pro celociselne rozmery (velikost, doba).
    pub fn factor(self) -> u128 {
        let d = self.def();
        d.num / d.den
    }

    /// Nasobek jako desetinne cislo (vsechny rozmery).
    pub fn scale(self) -> f64 {
        let d = self.def();
        d.num as f64 / d.den as f64
    }

    /// Jednotky rozmeru (napoveda, chyba "neznama jednotka").
    pub fn of(dim: Dim) -> impl Iterator<Item = Unit> {
        TABLE.iter().enumerate().filter(move |(_, d)| d.dim == dim).map(|(i, _)| Unit(i as u8))
    }

    /// Jednotka pro vypis hodnoty `base` (v zakladni jednotce): nejvetsi
    /// vypisovana, ve ktere je absolutni hodnota aspon 1.
    pub fn for_display(dim: Dim, base: f64) -> Unit {
        let a = if base < 0.0 { -base } else { base };
        let mut best = None;
        for u in Unit::of(dim).filter(|u| u.def().show) {
            if best.is_none() || a >= u.scale() {
                best = Some(u);
            }
        }
        best.unwrap_or(Unit(0))
    }
}

/// Napoveda k neznamem jednotce: vsechny zapisy po rozmerech.
pub fn hint() -> alloc::string::String {
    let mut out = alloc::string::String::new();
    for dim in [Dim::Size, Dim::Duration, Dim::Frequency, Dim::Ratio] {
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(dim.name_cs());
        out.push(':');
        for u in Unit::of(dim) {
            out.push(' ');
            out.push_str(u.name());
        }
    }
    out
}

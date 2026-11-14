//! Volby textoveho pole: jen pro cteni, nejvetsi delka a druh pole
//! (bezne, cislo, hledani). Logika cisla (filtr znaku, krok, meze) je bez
//! pisma a stromu, testuje se primo.

use alloc::string::String;

/// Druh pole.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum FieldKind {
    #[default]
    Text,
    /// Cislo: filtr znaku, krok sipkami, zarovnani vpravo (`number_input`).
    Number(NumberSpec),
    /// Hledani: lupa vlevo, krizek s textem, Esc smaze (`search_input`).
    Search,
}

/// Meze a krok cisla. Desetinna mista se odvodi z kroku (krok 0,5 = jedno).
#[derive(Debug, Clone, PartialEq)]
pub struct NumberSpec {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    /// Jednotka za cislem (jen kresleni, napr. "%").
    pub unit: String,
}

impl Default for NumberSpec {
    fn default() -> Self {
        Self { min: f64::MIN, max: f64::MAX, step: 1.0, unit: String::new() }
    }
}

/// Volby pole (spolecne pro jedno- i viceradkove, heslo, cislo, hledani).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FieldOpts {
    /// Jen pro cteni: text jde vybrat a kopirovat, ne menit.
    pub read_only: bool,
    /// Nejvic grafemu (vlozeni se zkrati, psani navic se ignoruje).
    pub max_len: Option<usize>,
    pub kind: FieldKind,
}

impl NumberSpec {
    /// Pocet desetinnych mist podle kroku (nejvys 6).
    pub fn decimals(&self) -> usize {
        let mut s = self.step.abs();
        let mut d = 0;
        while d < 6 && (s - libm_round(s)).abs() > 1e-9 {
            s *= 10.0;
            d += 1;
        }
        d
    }

    /// Hodnota textu (desetinna carka i tecka); `None` = neni cislo.
    pub fn parse(&self, text: &str) -> Option<f64> {
        let t: String = text.trim().chars().map(|c| if c == ',' { '.' } else { c }).filter(|c| !c.is_whitespace()).collect();
        if t.is_empty() || t == "-" {
            return None;
        }
        t.parse::<f64>().ok().filter(|v| v.is_finite())
    }

    /// Text hodnoty (desetinna carka jako v cestine, pocet mist podle
    /// kroku).
    pub fn format(&self, v: f64) -> String {
        let d = self.decimals();
        let s = alloc::format!("{:.*}", d, v);
        let s = if s.starts_with("-") && s[1..].chars().all(|c| c == '0' || c == '.') { String::from(&s[1..]) } else { s };
        s.replace('.', ",")
    }

    /// Hodnota v mezich.
    pub fn clamp(&self, v: f64) -> f64 {
        v.max(self.min).min(self.max)
    }

    /// Krok `n` (zaporny dolu) od textu; prazdny nebo neplatny text zacne
    /// od 0 (v mezich). Vysledek je zarovnany na mrizku kroku od `min`
    /// nebo 0 a v mezich.
    pub fn stepped(&self, text: &str, n: i32) -> String {
        let base = self.parse(text).unwrap_or_else(|| self.clamp(0.0));
        let origin = if self.min > f64::MIN { self.min } else { 0.0 };
        let k = libm_round((base - origin) / self.step);
        let v = origin + (k + n as f64) * self.step;
        self.format(self.clamp(v))
    }

    /// Smi znak do cisla? Cislice, desetinny oddelovac (jen s krokem
    /// mensim nez 1) a minus (jen se zapornym minimem).
    pub fn accepts(&self, c: char) -> bool {
        c.is_ascii_digit() || ((c == ',' || c == '.') && self.decimals() > 0) || (c == '-' && self.min < 0.0)
    }
}

/// Zaokrouhleni bez `std` (polovina od nuly).
fn libm_round(v: f64) -> f64 {
    let t = v as i64 as f64;
    let f = v - t;
    if f >= 0.5 {
        t + 1.0
    } else if f <= -0.5 {
        t - 1.0
    } else {
        t
    }
}

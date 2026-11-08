//! Ceny, limity utraty a jejich kontrola.
//!
//! Vse v celych cislech: cena v **mikrodolarech** (1 USD = 1 000 000),
//! cenik v mikrodolarech za milion tokenu. Zadne `f64` - soucty utraty
//! musi sedet na posledni misto i po tisicich volani.

use crate::msg::Usage;

/// Cenik modelu: mikrodolary za milion tokenu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Price {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
}

impl Price {
    pub const FREE: Price = Price { input: 0, output: 0, cache_write: 0, cache_read: 0 };

    /// Cena spotreby (zaokrouhleno nahoru na mikrodolar).
    pub fn cost(&self, u: &Usage) -> u64 {
        let total = u128::from(u.input) * u128::from(self.input)
            + u128::from(u.output) * u128::from(self.output)
            + u128::from(u.cache_write) * u128::from(self.cache_write)
            + u128::from(u.cache_read) * u128::from(self.cache_read);
        total.div_ceil(1_000_000).min(u128::from(u64::MAX)) as u64
    }

    /// Horni odhad ceny volani: vstup podle velikosti textu (bajty / 3
    /// tokeny - spis vic nez skutecne) a cely `max_tokens` vystup.
    pub fn estimate(&self, input_bytes: usize, max_tokens: u32) -> u64 {
        let input = (input_bytes as u64).div_ceil(3).saturating_add(64);
        self.cost(&Usage { input, output: u64::from(max_tokens), ..Default::default() })
    }

    pub fn put(&self, w: &mut wire::Writer) {
        w.u64(self.input).u64(self.output).u64(self.cache_write).u64(self.cache_read);
    }

    pub fn get(r: &mut wire::Reader) -> crate::Result<Self> {
        Ok(Self { input: r.u64()?, output: r.u64()?, cache_write: r.u64()?, cache_read: r.u64()? })
    }
}

/// Mikrodolary na text `1.234567 USD`.
pub fn usd(micros: u64) -> alloc::string::String {
    alloc::format!("{}.{:06} USD", micros / 1_000_000, micros % 1_000_000)
}

/// Limity relace (0 = vychozi hodnota pri otevreni relace).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Tokeny vystupu na jedno volani modelu.
    pub max_tokens: u32,
    /// Nejvic volani modelu na jeden dotaz.
    pub max_calls: u32,
    /// Utrata relace v mikrodolarech.
    pub session: u64,
    /// Utrata uzivatele za den v mikrodolarech.
    pub day: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_tokens: 16_000, max_calls: 24, session: 1_000_000, day: 5_000_000 }
    }
}

impl Limits {
    /// Nulove polozky nahradi vychozimi.
    pub fn or_default(self) -> Self {
        let d = Self::default();
        Self {
            max_tokens: if self.max_tokens == 0 { d.max_tokens } else { self.max_tokens },
            max_calls: if self.max_calls == 0 { d.max_calls } else { self.max_calls },
            session: if self.session == 0 { d.session } else { self.session },
            day: if self.day == 0 { d.day } else { self.day },
        }
    }

    pub fn put(&self, w: &mut wire::Writer) {
        w.u32(self.max_tokens).u32(self.max_calls).u64(self.session).u64(self.day);
    }

    pub fn get(r: &mut wire::Reader) -> crate::Result<Self> {
        Ok(Self { max_tokens: r.u32()?, max_calls: r.u32()?, session: r.u64()?, day: r.u64()? })
    }
}

/// Proc limit nepustil volani.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Over {
    Session,
    Day,
    Calls,
}

impl Over {
    pub fn describe(self) -> &'static str {
        match self {
            Over::Session => "limit utraty relace",
            Over::Day => "denni limit utraty uzivatele",
            Over::Calls => "limit volani modelu na jeden dotaz",
        }
    }
}

/// Stav utraty relace a dne.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Spend {
    pub session: u64,
    pub day: u64,
    /// Den (UTC, dny od 1970), ke kteremu patri `day`.
    pub day_index: u64,
}

impl Spend {
    /// Smi se udelat volani s horni cenou `estimate` a poradim `call`
    /// (1 = prvni volani dotazu)?
    pub fn check(&self, limits: &Limits, estimate: u64, call: u32) -> Result<(), Over> {
        if call > limits.max_calls {
            return Err(Over::Calls);
        }
        if self.session.saturating_add(estimate) > limits.session {
            return Err(Over::Session);
        }
        if self.day.saturating_add(estimate) > limits.day {
            return Err(Over::Day);
        }
        Ok(())
    }

    /// Zapocita skutecnou cenu; novy den vynuluje denni pocitadlo.
    pub fn add(&mut self, cost: u64, day_index: u64) {
        if day_index != self.day_index {
            self.day_index = day_index;
            self.day = 0;
        }
        self.session = self.session.saturating_add(cost);
        self.day = self.day.saturating_add(cost);
    }

    /// Prechod na novy den (pred kontrolou).
    pub fn roll(&mut self, day_index: u64) {
        if day_index != self.day_index {
            self.day_index = day_index;
            self.day = 0;
        }
    }
}

/// Ceniky znamych modelu Claude (prvni poskytovatel). Zapis cache = 1,25x
/// vstupu (TTL 5 minut).
pub fn claude_price(model: &str) -> Price {
    let m = |input: u64, output: u64, read: u64| Price { input, output, cache_write: input * 5 / 4, cache_read: read };
    match model {
        "claude-opus-5-5" => m(4_000_000, 20_000_000, 200_000),
        "claude-sonnet-5-5" | "claude-sonnet-5" => m(2_000_000, 10_000_000, 200_000),
        "claude-haiku-5-5" => m(100_000, 500_000, 10_000),
        "claude-fable-5-1" | "claude-fable-5" => m(10_000_000, 50_000_000, 250_000),
        "claude-opus-5" | "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6" => m(5_000_000, 25_000_000, 500_000),
        // Neznamy model: radeji vyssi cena (limity zastavi drive).
        _ => m(10_000_000, 50_000_000, 1_000_000),
    }
}

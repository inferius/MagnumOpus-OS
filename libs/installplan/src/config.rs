//! Nastaveni systemu `/system/config/system.toml` (zapise instalator, init
//! ho cte pri startu): jazyk, rozlozeni klavesnice (vic, prepinaji se),
//! casove pasmo, hodiny RTC a vychozi system pri startu.
//!
//! Format (podmnozina TOML, jeden klic na radek):
//!
//! ```text
//! jazyk = "cs"
//! klavesnice = ["cz", "us"]
//! pasmo = "Europe/Prague"
//! rtc = "utc"
//! vychozi-system = "magnumopus"
//! ```

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Cesta souboru na datovem oddilu.
pub const PATH: &str = "/system/config/system.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub language: String,
    /// Rozlozeni klavesnice (jmena z `keymap`), prvni je aktivni po startu.
    pub keyboards: Vec<String>,
    pub zone: String,
    /// Hodiny RTC v mistnim case (pocitac s Windows), jinak UTC.
    pub rtc_local: bool,
    /// Vychozi system pri startu je MagnumOpus (jinak druhy system).
    pub default_magnum: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            language: String::from("cs"),
            keyboards: alloc::vec![String::from("cz")],
            zone: String::from("Europe/Prague"),
            rtc_local: false,
            default_magnum: true,
        }
    }
}

fn quote(s: &str) -> String {
    let clean: String = s.chars().filter(|c| *c != '"' && *c != '\\' && !c.is_control()).collect();
    format!("\"{clean}\"")
}

impl Settings {
    pub fn to_toml(&self) -> String {
        let kb: Vec<String> = self.keyboards.iter().map(|k| quote(k)).collect();
        format!(
            "# Nastaveni systemu (zapsal instalator; zmeny pozdeji v Nastaveni).\njazyk = {}\nklavesnice = [{}]\npasmo = {}\nrtc = {}\nvychozi-system = {}\n",
            quote(&self.language),
            kb.join(", "),
            quote(&self.zone),
            quote(if self.rtc_local { "local" } else { "utc" }),
            quote(if self.default_magnum { "magnumopus" } else { "jiny" }),
        )
    }

    /// Rozbor; nezname klice a vadne radky preskoci (chybejici = vychozi).
    pub fn parse(text: &str) -> Self {
        let mut s = Settings::default();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            let unq = |v: &str| String::from(v.trim().trim_matches('"'));
            match k {
                "jazyk" => s.language = unq(v),
                "pasmo" => s.zone = unq(v),
                "rtc" => s.rtc_local = unq(v) == "local",
                "vychozi-system" => s.default_magnum = unq(v) != "jiny",
                "klavesnice" => {
                    let list: Vec<String> = v
                        .trim_start_matches('[')
                        .trim_end_matches(']')
                        .split(',')
                        .map(unq)
                        .filter(|x| !x.is_empty())
                        .collect();
                    if !list.is_empty() {
                        s.keyboards = list;
                    }
                }
                _ => {}
            }
        }
        s
    }

    /// Nastaveni pro sluzbu casu (`libos::time::zone::Settings`).
    pub fn time_settings(&self) -> String {
        format!("zone = {}\nrtc = {}\n", self.zone, if self.rtc_local { "local" } else { "utc" })
    }
}

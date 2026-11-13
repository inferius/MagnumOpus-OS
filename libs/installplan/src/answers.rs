//! Soubor odpovedi pro instalaci bez dotazu (`install --odpovedi
//! <soubor>`): skriptovana instalace a test v QEMU. Stejny format jako
//! `system.toml` (jeden klic na radek), navic disk, zpusob a ucet:
//!
//! ```text
//! disk = "QEMU NVMe Ctrl"      # model disku nebo jeho cislo v seznamu
//! zpusob = "cely-disk"         # cely-disk, volne-misto, preinstalovat
//! potvrzeni = "QEMU NVMe Ctrl" # u celeho disku opsany nazev disku
//! jmeno = "jan"
//! zobrazovane = "Jan Novák"
//! heslo = "dlouhe heslo"
//! pin = ""
//! klavesnice = ["cz", "us"]
//! pasmo = "Europe/Prague"
//! vychozi-system = "magnumopus"
//! ```
//!
//! Heslo v souboru je jen pro testy a skripty; soubor po instalaci smaz.

use alloc::string::String;

use crate::Mode;
use crate::proto::AccountInput;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answers {
    /// Model disku nebo jeho cislo.
    pub disk: String,
    pub mode: Mode,
    pub confirm: String,
    pub account: AccountInput,
    /// Cely text (nastaveni cte `config::Settings::parse`).
    pub settings: String,
}

/// Rozbor; chyba = popis chybejiciho nebo vadneho klice.
pub fn parse(text: &str) -> Result<Answers, String> {
    let get = |key: &str| -> Option<String> {
        text.lines().find_map(|l| {
            let l = l.split('#').next().unwrap_or("").trim();
            let (k, v) = l.split_once('=')?;
            (k.trim() == key).then(|| String::from(v.trim().trim_matches('"')))
        })
    };
    let disk = get("disk").ok_or_else(|| String::from("chybí klíč disk"))?;
    let mode = get("zpusob")
        .and_then(|m| Mode::from_key(&m))
        .ok_or_else(|| String::from("chybí nebo je neznámý klíč zpusob (cely-disk, volne-misto, preinstalovat)"))?;
    Ok(Answers {
        disk,
        mode,
        confirm: get("potvrzeni").unwrap_or_default(),
        account: AccountInput {
            name: get("jmeno").unwrap_or_default(),
            display: get("zobrazovane").unwrap_or_default(),
            password: get("heslo").unwrap_or_default(),
            pin: get("pin").unwrap_or_default(),
        },
        settings: String::from(text),
    })
}

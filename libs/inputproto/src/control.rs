//! Ridici prikazy `inputd` (kanal od rodice, tj. bootstrap kanal sluzby).
//!
//! Prikaz je text (prvni slovo = jmeno) a muze nest handly. Na kazdy
//! prikaz prijde jedna odpoved: `ok`, `ok <data>` nebo `err <KOD>` (jmeno
//! chyby z `abi::Error`, napr. `err INVALID_ARGS`).
//!
//! | Prikaz | Handly | Odpoved |
//! |---|---|---|
//! | `device` | kanal ke zdroji vstupu | `ok` |
//! | `focus` | kanal k novemu vlastnikovi fokusu | `ok` |
//! | `secure` | kanal pro bezpecnou zkratku | `ok` |
//! | `inject` | - | `ok` + kanal pro umely vstup |
//! | `layout <jmeno>` | - | `ok` / `err NOT_FOUND` |
//! | `status` | - | `ok` + [`Status`] v textu |
//!
//! Na kanalu `secure` posila jeho vlastnik `release` (konec bezpecneho
//! rezimu, fokus se vrati). Na kanalu fokusu klient zatim nic neposila.

/// Pripoji zdroj vstupu (ovladac). Handle: kanal (protokol [`crate::device`]).
pub const DEVICE: &str = "device";
/// Nastavi vlastnika fokusu (konzole, kompozitor). Predchozi dostane
/// `Focus(false)` a kanal se mu zavre.
pub const FOCUS: &str = "focus";
/// Nastavi prijemce bezpecne zkratky (system: odemceni, dialog opravneni,
/// spravce uloh).
pub const SECURE: &str = "secure";
/// Vytvori zdroj umeleho vstupu; v odpovedi je kanal (protokol
/// [`crate::device`]). Kdo ho dostane, smi vkladat vstup - je to capability.
pub const INJECT: &str = "inject";
/// Prepne rozlozeni: `layout us`, `layout cz`.
pub const LAYOUT: &str = "layout";
/// Stav sluzby.
pub const STATUS: &str = "status";
/// Kanal `secure`: konec bezpecneho rezimu.
pub const RELEASE: &str = "release";

/// Odpoved na `status`, v textu `ok devices=2 keyboards=1 pointers=1
/// layout=us injecting=0 dropped=0 secure=0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Status {
    /// Pripojene zdroje vstupu (vcetne umelych).
    pub devices: u32,
    /// Z toho fyzicke klavesnice.
    pub keyboards: u32,
    /// Z toho fyzicke mysi.
    pub pointers: u32,
    /// Id aktivniho rozlozeni.
    pub layout: [u8; 8],
    /// Aktivni zdroje umeleho vstupu.
    pub injecting: u32,
    /// Zahozene zpravy (plna fronta prijemce).
    pub dropped: u32,
    /// Bezpecny rezim (po bezpecne zkratce, do `release`).
    pub secure: bool,
}

impl Status {
    /// Jmeno rozlozeni jako text.
    pub fn layout_name(&self) -> &str {
        let end = self.layout.iter().position(|&b| b == 0).unwrap_or(8);
        core::str::from_utf8(&self.layout[..end]).unwrap_or("?")
    }

    /// Text odpovedi (bez `ok `).
    pub fn format(&self, out: &mut impl core::fmt::Write) -> core::fmt::Result {
        write!(
            out,
            "devices={} keyboards={} pointers={} layout={} injecting={} dropped={} secure={}",
            self.devices,
            self.keyboards,
            self.pointers,
            self.layout_name(),
            self.injecting,
            self.dropped,
            self.secure as u8
        )
    }

    /// Rozebere text odpovedi (s `ok ` nebo bez); nezname klice ignoruje.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.strip_prefix("ok").unwrap_or(text);
        let mut s = Status::default();
        for part in text.split_whitespace() {
            let (k, v) = part.split_once('=')?;
            match k {
                "devices" => s.devices = v.parse().ok()?,
                "keyboards" => s.keyboards = v.parse().ok()?,
                "pointers" => s.pointers = v.parse().ok()?,
                "injecting" => s.injecting = v.parse().ok()?,
                "dropped" => s.dropped = v.parse().ok()?,
                "secure" => s.secure = v == "1",
                "layout" => {
                    let b = v.as_bytes();
                    if b.len() > 8 {
                        return None;
                    }
                    s.layout = [0; 8];
                    s.layout[..b.len()].copy_from_slice(b);
                }
                _ => {}
            }
        }
        Some(s)
    }
}

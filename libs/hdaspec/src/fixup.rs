//! Opravy pro konkretni kodeky a desky ("quirks").
//!
//! Nektere desky maji ve vychozi konfiguraci pinu chyby, nebo potrebuji
//! GPIO (externi zesilovac reproduktoru notebooku) ci koeficienty
//! kodeku, ktere specifikace neresi. Tabulka je klicovana ID kodeku
//! (vyrobce:zarizeni) a ID subsystemu (vyrobce desky, z AFG verbem
//! GET_SUBSYSTEM_ID). Udaje jsou fakta o hardwaru (prevzit je smime
//! i z verejnych tabulek oprav, napr. Linuxu - ne jejich kod).
//!
//! Polozka se prida, az kdyz se overi na skutecnem pocitaci; tabulka je
//! zatim prazdna a mechanismus testuji testy. Obecna oprava bez tabulky:
//! u kodeku Realtek se zapne EAPD na vsech pinech, ktere ho maji
//! ([`generic_eapd`]), jako to dela vetsina ovladacu.

use crate::caps::PinConfig;
use crate::graph::Graph;

/// Verby pro opravy.
pub mod verb {
    pub const GET_SUBSYSTEM_ID: u16 = 0xF20;
    /// Koeficienty zpracovani (4bitove verby se 16 bity dat).
    pub const SET_COEF_INDEX: u8 = 0x5;
    pub const SET_PROC_COEF: u8 = 0x4;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gpio {
    pub mask: u8,
    pub dir: u8,
    pub data: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct Fixup {
    pub name: &'static str,
    /// ID kodeku (parametr VENDOR_ID).
    pub codec: u32,
    /// ID subsystemu a maska (0 = jakykoli).
    pub subsystem: u32,
    pub subsystem_mask: u32,
    /// Nova vychozi konfigurace pinu (nid, hodnota).
    pub pins: &'static [(u16, u32)],
    pub gpio: Option<Gpio>,
    /// Koeficienty (nid, index, hodnota), zapisuji se po nastaveni cest.
    pub coefs: &'static [(u16, u16, u16)],
}

/// Opravy overene na hardwaru (zatim zadne).
pub const FIXUPS: &[Fixup] = &[];

/// Prvni oprava z `table` pro kodek a subsystem.
pub fn find(table: &'static [Fixup], codec: u32, subsystem: u32) -> Option<&'static Fixup> {
    table.iter().find(|f| f.codec == codec && subsystem & f.subsystem_mask == f.subsystem & f.subsystem_mask)
}

/// Prepise vychozi konfigurace pinu v grafu (pred planovanim cest).
pub fn apply_pins(g: &mut Graph, f: &Fixup) {
    for &(nid, cfg) in f.pins {
        if let Some(w) = g.widgets.iter_mut().find(|w| w.nid == nid && w.pin_caps.is_some()) {
            w.pin_config = Some(PinConfig::decode(cfg));
        }
    }
}

/// Zapnout EAPD na vsech pinech, ktere ho umi (kodeky Realtek).
pub fn generic_eapd(codec: u32) -> bool {
    codec >> 16 == 0x10EC
}

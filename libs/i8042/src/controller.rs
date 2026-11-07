//! Registry a prikazy radice i8042 a zarizeni PS/2.

/// Datovy port (cteni: bajt od zarizeni nebo odpoved radice; zapis: bajt
/// pro zarizeni nebo parametr prikazu).
pub const DATA: u16 = 0x60;
/// Cteni: stav radice; zapis: prikaz radici.
pub const COMMAND: u16 = 0x64;

/// Linka preruseni klavesnice (prvni port).
pub const IRQ_KEYBOARD: u32 = 1;
/// Linka preruseni mysi (druhy port).
pub const IRQ_MOUSE: u32 = 12;

/// Bity stavoveho registru.
pub mod status {
    /// Vystupni buffer plny: na DATA ceka bajt.
    pub const OUTPUT_FULL: u8 = 1 << 0;
    /// Vstupni buffer plny: radic jeste nezpracoval posledni zapis.
    pub const INPUT_FULL: u8 = 1 << 1;
    /// Bajt ve vystupnim bufferu je od druheho portu (mys).
    pub const AUX_DATA: u8 = 1 << 5;
    /// Chyba casovani.
    pub const TIMEOUT: u8 = 1 << 6;
    /// Chyba parity.
    pub const PARITY: u8 = 1 << 7;
}

/// Prikazy radice (zapis na COMMAND).
pub mod cmd {
    pub const READ_CONFIG: u8 = 0x20;
    pub const WRITE_CONFIG: u8 = 0x60;
    pub const DISABLE_AUX: u8 = 0xA7;
    pub const ENABLE_AUX: u8 = 0xA8;
    /// Test druheho portu: 0x00 = v poradku.
    pub const TEST_AUX: u8 = 0xA9;
    /// Test radice: 0x55 = v poradku.
    pub const SELF_TEST: u8 = 0xAA;
    /// Test prvniho portu: 0x00 = v poradku.
    pub const TEST_KEYBOARD: u8 = 0xAB;
    pub const DISABLE_KEYBOARD: u8 = 0xAD;
    pub const ENABLE_KEYBOARD: u8 = 0xAE;
    /// Pristi bajt na DATA jde druhemu portu (mysi).
    pub const WRITE_AUX: u8 = 0xD4;
}

/// Bity konfiguracniho bajtu radice.
pub mod config {
    /// Preruseni od prvniho portu (IRQ 1).
    pub const KEYBOARD_IRQ: u8 = 1 << 0;
    /// Preruseni od druheho portu (IRQ 12).
    pub const AUX_IRQ: u8 = 1 << 1;
    /// Hodiny prvniho portu vypnute.
    pub const KEYBOARD_DISABLED: u8 = 1 << 4;
    /// Hodiny druheho portu vypnute (radic s jednim portem bit nemeni).
    pub const AUX_DISABLED: u8 = 1 << 5;
    /// Preklad scan kodu sady 2 na sadu 1.
    pub const TRANSLATE: u8 = 1 << 6;
}

/// Prikazy zarizeni (klavesnice a mys) a odpovedi.
pub mod dev {
    /// Rozsvitit kontrolky klavesnice (parametr: bity Scroll, Num, Caps).
    pub const SET_LEDS: u8 = 0xED;
    /// Mys: identifikace (0 standardni, 3 kolecko, 4 kolecko a 5 tlacitek).
    pub const GET_ID: u8 = 0xF2;
    /// Mys: vzorkovaci frekvence (parametr); posloupnosti zapinaji kolecko.
    pub const SET_SAMPLE_RATE: u8 = 0xF3;
    /// Zapnout posilani udalosti.
    pub const ENABLE: u8 = 0xF4;
    /// Vypnout posilani udalosti.
    pub const DISABLE: u8 = 0xF5;
    /// Reset a self-test (odpoved ACK, pak 0xAA; mys jeste ID).
    pub const RESET: u8 = 0xFF;

    /// Potvrzeni prikazu.
    pub const ACK: u8 = 0xFA;
    /// Zopakuj prikaz.
    pub const RESEND: u8 = 0xFE;
    /// Self-test po resetu prosel.
    pub const SELF_TEST_OK: u8 = 0xAA;
}

/// Posloupnost vzorkovacich frekvenci, ktera u mysi IntelliMouse zapne
/// kolecko (ID 3).
pub const WHEEL_MAGIC: [u8; 3] = [200, 100, 80];
/// Posloupnost, ktera po kolecku zapne 4. a 5. tlacitko (ID 4).
pub const FIVE_BUTTON_MAGIC: [u8; 3] = [200, 200, 80];

/// Bity kontrolek pro `SET_LEDS`.
pub fn leds(caps: bool, num: bool, scroll: bool) -> u8 {
    (scroll as u8) | (num as u8) << 1 | (caps as u8) << 2
}

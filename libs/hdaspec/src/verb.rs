//! Prikazy (verby) kodeku. Prikaz je 32bitovy: adresa kodeku 31:28,
//! uzel (NID) 27:20, verb a data 19:0. Verby s 12bitovym kodem nesou
//! 8 bitu dat, verby se 4bitovym kodem 16 bitu (specifikace 7.3).

/// Prikaz s 12bitovym verbem a 8 bity dat.
pub const fn cmd12(codec: u8, nid: u8, verb: u16, data: u8) -> u32 {
    (codec as u32 & 0xF) << 28 | (nid as u32) << 20 | (verb as u32 & 0xFFF) << 8 | data as u32
}

/// Prikaz se 4bitovym verbem a 16 bity dat.
pub const fn cmd4(codec: u8, nid: u8, verb: u8, data: u16) -> u32 {
    (codec as u32 & 0xF) << 28 | (nid as u32) << 20 | (verb as u32 & 0xF) << 16 | data as u32
}

// 12bitove verby.
pub const GET_PARAMETER: u16 = 0xF00;
pub const GET_CONN_SELECT: u16 = 0xF01;
pub const SET_CONN_SELECT: u16 = 0x701;
pub const GET_CONN_LIST: u16 = 0xF02;
pub const SET_POWER_STATE: u16 = 0x705;
pub const GET_POWER_STATE: u16 = 0xF05;
pub const SET_STREAM_CHANNEL: u16 = 0x706;
pub const SET_PIN_CONTROL: u16 = 0x707;
pub const GET_PIN_CONTROL: u16 = 0xF07;
pub const SET_UNSOL: u16 = 0x708;
pub const GET_PIN_SENSE: u16 = 0xF09;
pub const EXEC_PIN_SENSE: u16 = 0x709;
pub const SET_EAPD_BTL: u16 = 0x70C;
pub const GET_EAPD_BTL: u16 = 0xF0C;
pub const SET_DIGI_CONVERT_1: u16 = 0x70D;
pub const SET_GPIO_DATA: u16 = 0x715;
pub const SET_GPIO_MASK: u16 = 0x716;
pub const SET_GPIO_DIR: u16 = 0x717;
pub const GET_CONFIG_DEFAULT: u16 = 0xF1C;
pub const FUNCTION_RESET: u16 = 0x7FF;

// 4bitove verby.
pub const SET_CONVERTER_FORMAT: u8 = 0x2;
pub const SET_AMP_GAIN_MUTE: u8 = 0x3;
pub const GET_AMP_GAIN_MUTE: u8 = 0xB;

/// Parametry (data verbu GET_PARAMETER).
pub mod param {
    pub const VENDOR_ID: u8 = 0x00;
    pub const REVISION_ID: u8 = 0x02;
    /// Podrizene uzly: prvni NID 23:16, pocet 7:0.
    pub const NODE_COUNT: u8 = 0x04;
    /// Typ funkcni skupiny 7:0 (1 = audio, 2 = modem).
    pub const FUNCTION_TYPE: u8 = 0x05;
    pub const AFG_CAPS: u8 = 0x08;
    pub const WIDGET_CAPS: u8 = 0x09;
    /// Podporovane frekvence a bitove hloubky PCM.
    pub const PCM: u8 = 0x0A;
    pub const STREAM_FORMATS: u8 = 0x0B;
    pub const PIN_CAPS: u8 = 0x0C;
    pub const AMP_IN_CAPS: u8 = 0x0D;
    pub const CONN_LIST_LEN: u8 = 0x0E;
    pub const POWER_STATES: u8 = 0x0F;
    pub const GPIO_COUNT: u8 = 0x11;
    pub const AMP_OUT_CAPS: u8 = 0x12;
}

/// Typ funkcni skupiny: audio.
pub const FG_AUDIO: u8 = 0x01;

/// Rizeni pinu (SET_PIN_CONTROL): vystup, vstup, posilovac sluchatek.
pub const PIN_OUT: u8 = 1 << 6;
pub const PIN_IN: u8 = 1 << 5;
pub const PIN_HP: u8 = 1 << 7;
/// EAPD (externi zesilovac) zapnout.
pub const EAPD: u8 = 1 << 1;
/// Stav napajeni D0 (plny provoz).
pub const POWER_D0: u8 = 0;
/// Rizeni pinu: napajeni mikrofonu 80 % (bity 2:0).
pub const PIN_VREF80: u8 = 4;
/// Nevyzadane odpovedi (SET_UNSOL): zapnout + znacka v bitech 5:0.
pub const UNSOL_ENABLE: u8 = 1 << 7;

/// Znacka nevyzadane odpovedi (bity 31:26 odpovedi).
pub const fn unsol_tag(resp: u32) -> u8 {
    (resp >> 26) as u8
}

/// Data pro SET_AMP_GAIN_MUTE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmpSet {
    pub output: bool,
    pub input: bool,
    pub left: bool,
    pub right: bool,
    /// Index vstupu (u vstupnich zesilovacu mixeru).
    pub index: u8,
    pub mute: bool,
    pub gain: u8,
}

impl AmpSet {
    /// Vystupni zesilovac obou kanalu.
    pub const fn output(gain: u8, mute: bool) -> Self {
        Self { output: true, input: false, left: true, right: true, index: 0, mute, gain }
    }

    /// Vstupni zesilovac `index` obou kanalu.
    pub const fn input(index: u8, gain: u8, mute: bool) -> Self {
        Self { output: false, input: true, left: true, right: true, index, mute, gain }
    }

    pub const fn encode(&self) -> u16 {
        (self.output as u16) << 15
            | (self.input as u16) << 14
            | (self.left as u16) << 13
            | (self.right as u16) << 12
            | (self.index as u16 & 0xF) << 8
            | (self.mute as u16) << 7
            | (self.gain as u16 & 0x7F)
    }
}

/// Odpoved v RIRB: 32 bitu odpovedi + 32 bitu rozsireni (adresa kodeku 3:0,
/// bit 4 = nevyzadana odpoved).
pub const RESP_UNSOL: u32 = 1 << 4;

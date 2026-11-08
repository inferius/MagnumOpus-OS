//! Dekodovani odpovedi kodeku (specifikace 7.3.4).

use alloc::vec::Vec;

/// Typ widgetu (schopnosti widgetu, bity 23:20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetKind {
    /// Prevodnik D/A (DAC, "audio output").
    Output,
    /// Prevodnik A/D (ADC, "audio input").
    Input,
    Mixer,
    Selector,
    Pin,
    Power,
    VolumeKnob,
    Beep,
    Vendor,
    Unknown(u8),
}

impl WidgetKind {
    pub fn from_type(t: u8) -> Self {
        match t {
            0 => Self::Output,
            1 => Self::Input,
            2 => Self::Mixer,
            3 => Self::Selector,
            4 => Self::Pin,
            5 => Self::Power,
            6 => Self::VolumeKnob,
            7 => Self::Beep,
            0xF => Self::Vendor,
            t => Self::Unknown(t),
        }
    }
}

/// Schopnosti widgetu (parametr 0x09).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidgetCaps {
    pub kind: WidgetKind,
    pub stereo: bool,
    pub in_amp: bool,
    pub out_amp: bool,
    /// Zesilovac ma vlastni parametry (jinak plati vychozi z AFG).
    pub amp_override: bool,
    /// Prevodnik ma vlastni podporovane formaty (jinak plati z AFG).
    pub format_override: bool,
    pub conn_list: bool,
    pub digital: bool,
    pub power_control: bool,
    /// Pocet kanalu (1..=16).
    pub channels: u8,
}

impl WidgetCaps {
    pub fn decode(v: u32) -> Self {
        let ext = (v >> 13 & 7) as u8;
        Self {
            kind: WidgetKind::from_type((v >> 20 & 0xF) as u8),
            stereo: v & 1 != 0,
            in_amp: v & 1 << 1 != 0,
            out_amp: v & 1 << 2 != 0,
            amp_override: v & 1 << 3 != 0,
            format_override: v & 1 << 4 != 0,
            conn_list: v & 1 << 8 != 0,
            digital: v & 1 << 9 != 0,
            power_control: v & 1 << 10 != 0,
            channels: (ext << 1 | (v & 1) as u8) + 1,
        }
    }
}

/// Schopnosti pinu (parametr 0x0C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinCaps {
    /// Detekce zasunuti potrebuje spoustec (EXEC_PIN_SENSE) pred ctenim.
    pub trigger: bool,
    pub presence_detect: bool,
    pub headphone_drive: bool,
    pub output: bool,
    pub input: bool,
    pub hdmi: bool,
    pub eapd: bool,
    pub dp: bool,
}

impl PinCaps {
    pub fn decode(v: u32) -> Self {
        Self {
            trigger: v & 1 << 1 != 0,
            presence_detect: v & 1 << 2 != 0,
            headphone_drive: v & 1 << 3 != 0,
            output: v & 1 << 4 != 0,
            input: v & 1 << 5 != 0,
            hdmi: v & 1 << 7 != 0,
            eapd: v & 1 << 16 != 0,
            dp: v & 1 << 24 != 0,
        }
    }
}

/// Schopnosti zesilovace (parametry 0x0D a 0x12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AmpCaps {
    /// Hodnota zesileni odpovidajici 0 dB.
    pub offset: u8,
    /// Nejvyssi hodnota zesileni (pocet kroku - 1).
    pub steps: u8,
    /// Velikost kroku v 0,25 dB minus 1.
    pub step_size: u8,
    pub mute: bool,
}

impl AmpCaps {
    pub fn decode(v: u32) -> Self {
        Self {
            offset: (v & 0x7F) as u8,
            steps: (v >> 8 & 0x7F) as u8,
            step_size: (v >> 16 & 0x7F) as u8,
            mute: v & 1 << 31 != 0,
        }
    }

    /// Zesileni na 0 dB (nebo nejvyssi, kdyz 0 dB neumi) - vychozi pro
    /// cestu k vystupu; hlasitost resi sluzba softwarove.
    pub fn unity(&self) -> u8 {
        self.offset.min(self.steps)
    }
}

/// Pripojeni pinu (vychozi konfigurace, bity 31:30).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connectivity {
    Jack,
    None,
    Fixed,
    Both,
}

/// Vychozi zarizeni pinu (bity 23:20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultDevice {
    LineOut,
    Speaker,
    HeadphoneOut,
    Cd,
    SpdifOut,
    DigitalOtherOut,
    ModemLine,
    ModemHandset,
    LineIn,
    Aux,
    MicIn,
    Telephony,
    SpdifIn,
    DigitalOtherIn,
    Reserved,
    Other,
}

impl DefaultDevice {
    pub fn from_bits(v: u8) -> Self {
        use DefaultDevice::*;
        [
            LineOut, Speaker, HeadphoneOut, Cd, SpdifOut, DigitalOtherOut, ModemLine, ModemHandset, LineIn, Aux,
            MicIn, Telephony, SpdifIn, DigitalOtherIn, Reserved, Other,
        ][v as usize & 0xF]
    }
}

/// Vychozi konfigurace pinu (verb GET_CONFIG_DEFAULT): jak ho zapojil
/// vyrobce desky (BIOS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinConfig {
    pub raw: u32,
    pub connectivity: Connectivity,
    /// Umisteni (bity 29:24): 0x01 zadni panel, 0x02 predni, 0x18 HDMI
    /// u nekterych grafickych kodeku apod.
    pub location: u8,
    pub device: DefaultDevice,
    pub connection: u8,
    pub color: u8,
    /// Bit 0: konektor nema detekci zasunuti (prepis).
    pub misc: u8,
    /// Asociace: piny jednoho logickeho vystupu (0 = nepouzito).
    pub association: u8,
    pub sequence: u8,
}

impl PinConfig {
    pub fn decode(v: u32) -> Self {
        Self {
            raw: v,
            connectivity: match v >> 30 {
                0 => Connectivity::Jack,
                1 => Connectivity::None,
                2 => Connectivity::Fixed,
                _ => Connectivity::Both,
            },
            location: (v >> 24 & 0x3F) as u8,
            device: DefaultDevice::from_bits((v >> 20 & 0xF) as u8),
            connection: (v >> 16 & 0xF) as u8,
            color: (v >> 12 & 0xF) as u8,
            misc: (v >> 8 & 0xF) as u8,
            association: (v >> 4 & 0xF) as u8,
            sequence: (v & 0xF) as u8,
        }
    }

    /// Je pin zapojeny (ne `None`)?
    pub fn connected(&self) -> bool {
        self.connectivity != Connectivity::None
    }

    /// Ma konektor detekci zasunuti podle konfigurace (ne prepis misc).
    pub fn jack_detect_allowed(&self) -> bool {
        self.misc & 1 == 0 && matches!(self.connectivity, Connectivity::Jack | Connectivity::Both)
    }
}

/// Delka seznamu spojeni (parametr 0x0E): (pocet, dlouhy tvar).
pub fn conn_list_len(v: u32) -> (u8, bool) {
    ((v & 0x7F) as u8, v & 1 << 7 != 0)
}

/// Seznam spojeni: `fetch(index)` vrati odpoved GET_CONN_LIST pro dany
/// index (4 kratke nebo 2 dlouhe polozky). Rozsahy (bit "range") se
/// rozvinou; nesmyslny rozsah (sestupny, prilis velky) se vynecha.
pub fn conn_list(len: u8, long: bool, mut fetch: impl FnMut(u8) -> Option<u32>) -> Vec<u16> {
    let per = if long { 2 } else { 4 };
    let (bits, range_bit, mask) = if long { (16, 1u32 << 15, 0x7FFFu32) } else { (8, 1u32 << 7, 0x7Fu32) };
    let mut out: Vec<u16> = Vec::new();
    let mut i = 0u8;
    while i < len {
        let Some(resp) = fetch(i) else { break };
        for k in 0..per {
            if i + k >= len {
                break;
            }
            let e = resp >> (k as u32 * bits) & ((1 << bits) - 1);
            let nid = (e & mask) as u16;
            if e & range_bit != 0 {
                if let Some(&prev) = out.last()
                    && nid > prev
                    && nid - prev <= 64
                {
                    out.extend(prev + 1..=nid);
                }
            } else {
                out.push(nid);
            }
        }
        i = i.saturating_add(per);
    }
    out
}

/// Podporovane frekvence PCM (parametr 0x0A, bity 11:0) a hloubky.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmCaps {
    pub raw: u32,
}

/// Frekvence v poradi bitu parametru PCM.
pub const PCM_RATES: [u32; 12] = [8_000, 11_025, 16_000, 22_050, 32_000, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 384_000];

impl PcmCaps {
    pub fn rate(&self, hz: u32) -> bool {
        PCM_RATES.iter().position(|&r| r == hz).is_some_and(|i| self.raw & 1 << i != 0)
    }

    /// Bitove hloubky: 8, 16, 20, 24, 32 = bity 16..20.
    pub fn bits(&self, b: u32) -> bool {
        let i = match b {
            8 => 16,
            16 => 17,
            20 => 18,
            24 => 19,
            32 => 20,
            _ => return false,
        };
        self.raw & 1 << i != 0
    }
}

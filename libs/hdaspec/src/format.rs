//! Format streamu HDA (16 bitu, stejny v registru SDnFMT a ve verbu
//! SET_CONVERTER_FORMAT): bit 15 typ (0 = PCM), bit 14 zaklad (0 = 48 kHz,
//! 1 = 44,1 kHz), 13:11 nasobek - 1, 10:8 delitel - 1, 6:4 bity, 3:0
//! kanaly - 1.

/// Zakoduje format; `None` = frekvenci nejde slozit ze zakladu
/// 48/44,1 kHz s nasobkem 1-4 a delitelem 1-8.
pub fn encode(rate: u32, bits: u32, channels: u32) -> Option<u16> {
    let b = match bits {
        8 => 0,
        16 => 1,
        20 => 2,
        24 => 3,
        32 => 4,
        _ => return None,
    };
    if !(1..=16).contains(&channels) {
        return None;
    }
    for (base_bit, base) in [(0u16, 48_000u32), (1, 44_100)] {
        for mult in 1..=4u32 {
            for div in 1..=8u32 {
                if base * mult == rate * div {
                    return Some(
                        base_bit << 14 | ((mult - 1) as u16) << 11 | ((div - 1) as u16) << 8 | b << 4 | (channels - 1) as u16,
                    );
                }
            }
        }
    }
    None
}

/// Rozlozi format: (frekvence, bity, kanaly).
pub fn decode(f: u16) -> (u32, u32, u32) {
    let base = if f & 1 << 14 != 0 { 44_100 } else { 48_000 };
    let mult = (f >> 11 & 7) as u32 + 1;
    let div = (f >> 8 & 7) as u32 + 1;
    let bits = [8, 16, 20, 24, 32, 0, 0, 0][(f >> 4 & 7) as usize];
    (base * mult / div, bits, (f & 0xF) as u32 + 1)
}

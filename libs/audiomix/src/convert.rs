//! Prevod vzorku formatu klienta na vnitrni stereo Q23 a zpet.
//!
//! Mono jde do obou kanalu. Vic kanalu: downmix podle obvykleho rozlozeni
//! (poradi kanalu WAV/HDA, [`DOWNMIX`]): stred a zadni/bocni kanaly
//! s vahou 0,707 (-3 dB), LFE se vynecha (reproduktory ho obvykle
//! nehraji). Vahy se normuji na soucet 1, takze se nic neorizne.

use audioproto::{Format, Sample};

use crate::{Frame, Q23, Q23_MAX, Q23_MIN};

/// Jeden vzorek z bajtu -> Q23.
#[inline]
pub fn sample_to_q23(s: Sample, b: &[u8]) -> Q23 {
    match s {
        Sample::U8 => (b[0] as i32 - 128) << 16,
        Sample::S16 => (i16::from_le_bytes([b[0], b[1]]) as i32) << 8,
        Sample::S24 => {
            // 24 bitu v nizsich bitech slova: znamenko z bitu 23.
            let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) << 8;
            (v as i32) >> 8
        }
        Sample::S32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) >> 8,
        Sample::F32 => f32_to_q23(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
    }
}

/// f32 (-1.0..1.0) -> Q23 s orezanim; NaN = ticho.
fn f32_to_q23(v: f32) -> Q23 {
    if v.is_nan() {
        return 0;
    }
    let x = v * 8_388_608.0;
    if x >= Q23_MAX as f32 {
        Q23_MAX
    } else if x <= Q23_MIN as f32 {
        Q23_MIN
    } else {
        x as i32
    }
}

/// Snimek formatu `f` (prave `f.frame_bytes()` bajtu) -> stereo.
pub fn frame_to_stereo(f: &Format, b: &[u8]) -> Frame {
    let sb = f.sample.bytes() as usize;
    let ch = f.channels as usize;
    match ch {
        1 => {
            let v = sample_to_q23(f.sample, b);
            [v, v]
        }
        2 => [sample_to_q23(f.sample, b), sample_to_q23(f.sample, &b[sb..])],
        _ => {
            let w = DOWNMIX[ch.min(8)];
            let (mut l, mut r, mut sl, mut sr) = (0i64, 0i64, 0i64, 0i64);
            for (c, &(wl, wr)) in w.iter().enumerate().take(ch) {
                let v = sample_to_q23(f.sample, &b[c * sb..]) as i64;
                l += v * wl as i64;
                r += v * wr as i64;
                sl += wl as i64;
                sr += wr as i64;
            }
            [(l / sl.max(1)) as i32, (r / sr.max(1)) as i32]
        }
    }
}

/// Vaha (L, R) v Q15 pro kanaly v poradi WAV: FL FR FC LFE BL BR SL SR.
const FL: (i32, i32) = (32_768, 0);
const FR: (i32, i32) = (0, 32_768);
const FC: (i32, i32) = (23_170, 23_170);
const LFE: (i32, i32) = (0, 0);
const BL: (i32, i32) = (23_170, 0);
const BR: (i32, i32) = (0, 23_170);
const BC: (i32, i32) = (16_384, 16_384);

/// Rozlozeni podle poctu kanalu: 3 = FL FR FC, 4 = kvadro, 5 = 5.0,
/// 6 = 5.1, 7 = 6.1, 8 = 7.1.
pub const DOWNMIX: [&[(i32, i32)]; 9] = [
    &[],
    &[FC],
    &[FL, FR],
    &[FL, FR, FC],
    &[FL, FR, BL, BR],
    &[FL, FR, FC, BL, BR],
    &[FL, FR, FC, LFE, BL, BR],
    &[FL, FR, FC, LFE, BC, BL, BR],
    &[FL, FR, FC, LFE, BL, BR, BL, BR],
];

/// Q23 -> jeden vzorek formatu `s` (s orezanim).
pub fn q23_to_sample(s: Sample, v: i32, out: &mut [u8]) {
    let v = v.clamp(Q23_MIN, Q23_MAX);
    match s {
        Sample::U8 => out[0] = ((v >> 16) + 128) as u8,
        Sample::S16 => out[..2].copy_from_slice(&((v >> 8) as i16).to_le_bytes()),
        Sample::S24 => out[..4].copy_from_slice(&v.to_le_bytes()),
        Sample::S32 => out[..4].copy_from_slice(&(v << 8).to_le_bytes()),
        Sample::F32 => out[..4].copy_from_slice(&(v as f32 / 8_388_608.0).to_le_bytes()),
    }
}

/// Stereo -> snimek formatu `f` (nahravani): mono = prumer, vic kanalu =
/// L a R do prvnich dvou, ostatni ticho.
pub fn stereo_to_frame(f: &Format, fr: Frame, out: &mut [u8]) {
    let sb = f.sample.bytes() as usize;
    if f.channels == 1 {
        q23_to_sample(f.sample, ((fr[0] as i64 + fr[1] as i64) / 2) as i32, out);
        return;
    }
    q23_to_sample(f.sample, fr[0], out);
    q23_to_sample(f.sample, fr[1], &mut out[sb..]);
    for c in 2..f.channels as usize {
        q23_to_sample(f.sample, 0, &mut out[c * sb..]);
    }
}

/// Q23 -> s16 s orezanim (bez ditheringu).
#[inline]
pub fn q23_to_s16(v: i64) -> i16 {
    let s = v >> 8;
    s.clamp(i16::MIN as i64, i16::MAX as i64) as i16
}

//! Prevod vzorku formatu klienta na vnitrni stereo Q23 a zpet.
//!
//! Mono jde do obou kanalu. Vic nez dva kanaly: levy = prumer sudych
//! kanalu, pravy = prumer lichych (5.1 v poradi FL FR C LFE RL RR da
//! L = (FL + C + RL) / 3, R = (FR + LFE + RR) / 3) - jednoduchy downmix,
//! hlasitost se nezvysi a nic se neorizne.

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
            let (mut l, mut r, mut nl, mut nr) = (0i64, 0i64, 0i64, 0i64);
            for c in 0..ch {
                let v = sample_to_q23(f.sample, &b[c * sb..]) as i64;
                if c % 2 == 0 {
                    l += v;
                    nl += 1;
                } else {
                    r += v;
                    nr += 1;
                }
            }
            [(l / nl) as i32, (r / nr.max(1)) as i32]
        }
    }
}

/// Q23 -> s16 s orezanim (bez ditheringu).
#[inline]
pub fn q23_to_s16(v: i64) -> i16 {
    let s = v >> 8;
    s.clamp(i16::MIN as i64, i16::MAX as i64) as i16
}

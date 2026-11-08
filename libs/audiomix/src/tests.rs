//! Testy zpracovani zvuku na hostiteli.

extern crate std;

use alloc::vec;
use alloc::vec::Vec;

use audioproto::{Format, Sample};

use crate::convert::{frame_to_stereo, q23_to_s16, sample_to_q23};
use crate::mix::Mixer;
use crate::src::Resampler;
use crate::tone::{Tone, sin_q30};
use crate::wav::{self, WavError};
use crate::{Frame, Q23_MAX, Q23_MIN};

/// Energie frekvence `freq` v signalu (Goertzel), normovana na amplitudu.
fn goertzel(x: &[i32], freq: f64, rate: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * freq / rate;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &v in x {
        let s = v as f64 + c * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    let p = s1 * s1 + s2 * s2 - c * s1 * s2;
    2.0 * p.sqrt() / x.len() as f64
}

#[test]
fn sample_conversion() {
    assert_eq!(sample_to_q23(Sample::S16, &i16::MAX.to_le_bytes()), 32767 << 8);
    assert_eq!(sample_to_q23(Sample::S16, &i16::MIN.to_le_bytes()), Q23_MIN);
    assert_eq!(sample_to_q23(Sample::U8, &[128]), 0);
    assert_eq!(sample_to_q23(Sample::U8, &[0]), Q23_MIN);
    // S24: znamenko z bitu 23, horni bajt se ignoruje.
    assert_eq!(sample_to_q23(Sample::S24, &[0xFF, 0xFF, 0xFF, 0x00]), -1);
    assert_eq!(sample_to_q23(Sample::S24, &[0xFF, 0xFF, 0x7F, 0xAA]), Q23_MAX);
    assert_eq!(sample_to_q23(Sample::S32, &i32::MIN.to_le_bytes()), Q23_MIN);
    assert_eq!(sample_to_q23(Sample::F32, &1.0f32.to_le_bytes()), Q23_MAX);
    assert_eq!(sample_to_q23(Sample::F32, &(-2.0f32).to_le_bytes()), Q23_MIN);
    assert_eq!(sample_to_q23(Sample::F32, &0.5f32.to_le_bytes()), 1 << 22);
    assert_eq!(sample_to_q23(Sample::F32, &f32::NAN.to_le_bytes()), 0);
    assert_eq!(q23_to_s16(Q23_MAX as i64 * 4), i16::MAX);
    assert_eq!(q23_to_s16(-(1 << 30)), i16::MIN);
}

#[test]
fn channel_mapping() {
    let mono = Format::new(48_000, 1, Sample::S16);
    assert_eq!(frame_to_stereo(&mono, &100i16.to_le_bytes()), [100 << 8, 100 << 8]);
    let six = Format::new(48_000, 6, Sample::S16);
    let mut b = Vec::new();
    for v in [300i16, 30, 600, 60, 0, 0] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(frame_to_stereo(&six, &b), [300 << 8, 30 << 8]);
}

#[test]
fn resampler_same_rate_is_exact() {
    let input: Vec<Frame> = (0..100).map(|i| [i * 1000, -i * 1000]).collect();
    let mut r = Resampler::new(48_000, 48_000);
    let mut out = vec![[0; 2]; 100];
    let mut it = input.iter().copied();
    let n = r.process(&mut out, || it.next());
    // Prvni vystup = prvni vstup; posledni dva vstupy jeste v historii.
    assert_eq!(n, 98);
    assert_eq!(&out[..n], &input[..n]);
}

#[test]
fn resampler_ratio_and_continuation() {
    let mut r = Resampler::new(44_100, 48_000);
    let mut consumed = 0u32;
    let mut produced = 0usize;
    let mut out = vec![[0; 2]; 480];
    for _ in 0..100 {
        let need = r.input_needed(480);
        let mut left = need;
        produced += r.process(&mut out, || {
            if left == 0 {
                return None;
            }
            left -= 1;
            consumed += 1;
            Some([0, 0])
        });
    }
    // 100 period po 10 ms: 48 000 vystupu z ~44 100 vstupu.
    assert_eq!(produced, 48_000);
    assert!((44_090..=44_110).contains(&consumed), "spotrebovano {consumed}");
}

#[test]
fn resampler_keeps_tone_frequency() {
    // 1 kHz pri 44,1 kHz -> 48 kHz: ton zustane 1 kHz, jinde ticho.
    let mut tone = Tone::new(1_000, 44_100, 50);
    let mut r = Resampler::new(44_100, 48_000);
    let mut out = vec![[0; 2]; 4_800];
    let n = r.process(&mut out, || Some(tone.next_frame()));
    assert_eq!(n, 4_800);
    let left: Vec<i32> = out[100..].iter().map(|f| f[0]).collect();
    let amp = Q23_MAX as f64 * 0.5;
    let at = goertzel(&left, 1_000.0, 48_000.0);
    let off = goertzel(&left, 1_500.0, 48_000.0);
    assert!((at - amp).abs() < amp * 0.02, "amplituda {at}, ocekavano {amp}");
    assert!(off < amp * 0.01, "energie mimo ton {off}");
}

#[test]
fn resampler_reports_input_shortage() {
    let mut r = Resampler::new(48_000, 48_000);
    let mut out = vec![[0; 2]; 10];
    let mut left = 5;
    let n = r.process(&mut out, || {
        if left == 0 {
            None
        } else {
            left -= 1;
            Some([1, 1])
        }
    });
    assert_eq!(n, 3);
    // Po doplneni vstupu pokracuje.
    let n2 = r.process(&mut out, || Some([2, 2]));
    assert_eq!(n2, 10);
}

#[test]
fn sine_accuracy() {
    for i in 0..4096u32 {
        let phase = i.wrapping_mul(1 << 20);
        let exact = (2.0 * std::f64::consts::PI * phase as f64 / 4_294_967_296.0).sin();
        let got = sin_q30(phase) as f64 / (1u64 << 30) as f64;
        assert!((exact - got).abs() < 2e-4, "faze {phase}: {got} vs {exact}");
    }
}

#[test]
fn tone_frequency() {
    let mut t = Tone::new(1_000, 48_000, 100);
    let x: Vec<i32> = (0..4_800).map(|_| t.next_frame()[0]).collect();
    let at = goertzel(&x, 1_000.0, 48_000.0);
    assert!((at / Q23_MAX as f64 - 1.0).abs() < 0.01);
    assert!(goertzel(&x, 2_000.0, 48_000.0) < Q23_MAX as f64 * 0.001);
}

#[test]
fn mixer_sums_clips_and_applies_gain() {
    let mut m = Mixer::new(2);
    let loud = [[Q23_MAX, 1 << 20]; 2];
    m.add(&loud, 65_536);
    m.add(&loud, 65_536);
    let mut out = [0u8; 8];
    let clipped = m.finish_s16(65_536, &mut out);
    assert_eq!(i16::from_le_bytes([out[0], out[1]]), i16::MAX);
    assert_eq!(i16::from_le_bytes([out[2], out[3]]), ((2 << 20) >> 8) as i16);
    assert_eq!(clipped, 2);
    // Stejny mix s hlavni hlasitosti 1/4: bez orezu.
    let clipped = m.finish_s16(16_384, &mut out);
    assert_eq!(clipped, 0);
    assert_eq!(i16::from_le_bytes([out[2], out[3]]), ((2 << 20) / 4 >> 8) as i16);
    // Opacne faze se odectou drive, nez se orezava.
    m.clear();
    m.add(&[[Q23_MAX, 0]], 65_536);
    m.add(&[[Q23_MAX, 0]], 65_536);
    m.add(&[[-Q23_MAX, 0]], 65_536);
    m.finish_s16(65_536, &mut out);
    assert_eq!(i16::from_le_bytes([out[0], out[1]]), (Q23_MAX >> 8) as i16);
}

#[test]
fn wav_roundtrip_and_errors() {
    let mut file = wav::header_s16(44_100, 2, 8);
    file.extend_from_slice(&[1, 0, 2, 0, 3, 0, 4, 0]);
    let w = wav::parse(&file).unwrap();
    assert_eq!(w.format, Format::new(44_100, 2, Sample::S16));
    assert_eq!(w.data.len(), 8);
    // Delka dat vetsi nez soubor (nedokonceny zapis): zkrati se.
    let mut short = wav::header_s16(48_000, 2, 1_000);
    short.extend_from_slice(&[0; 10]);
    assert_eq!(wav::parse(&short).unwrap().data.len(), 8);
    assert_eq!(wav::parse(b"nic"), Err(WavError::NotRiff));
    let mut nodata = wav::header_s16(48_000, 2, 0);
    nodata.truncate(36);
    assert_eq!(wav::parse(&nodata), Err(WavError::Missing));
    // Komprese (ADPCM = 2) neumime.
    let mut adpcm = wav::header_s16(48_000, 2, 0);
    adpcm[20] = 2;
    assert_eq!(wav::parse(&adpcm), Err(WavError::Unsupported));
}

#[test]
fn wav_skips_unknown_chunks() {
    let mut file = Vec::new();
    file.extend_from_slice(b"RIFF\0\0\0\0WAVE");
    file.extend_from_slice(b"LIST");
    file.extend_from_slice(&3u32.to_le_bytes());
    file.extend_from_slice(&[1, 2, 3, 0]); // licha delka + vypln
    let h = wav::header_s16(8_000, 1, 2);
    file.extend_from_slice(&h[12..]);
    file.extend_from_slice(&[5, 0]);
    let w = wav::parse(&file).unwrap();
    assert_eq!(w.format.rate, 8_000);
    assert_eq!(w.data, &[5, 0]);
}

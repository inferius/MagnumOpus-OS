//! Zvuk v QEMU: radice Intel HDA s kodeky hda-output a hda-duplex
//! a kontrola zapsaneho zvuku po testu.
//!
//! V testu jde vystup prvniho kodeku do souboru WAV (`-audiodev wav`,
//! 48 kHz, 16 bitu, stereo). Test userspace `zvuk: ton 1 kHz do WAV` prehraje ton 1 kHz;
//! po konci QEMU se v souboru hleda (Goertzel po oknech 50 ms). Bez
//! okna: `--audio none`; interaktivne napr. `--audio dsound` (Windows),
//! `pa` nebo `alsa` (Linux), `coreaudio` (macOS).

use std::path::Path;
use std::process::Command;

use crate::{Options, Result};

/// Znacka testu v logu: kdyz je, musi byt ton i v souboru.
pub const TONE_MARKER: &str = "[ok] zvuk: ton 1 kHz do WAV";
/// Frekvence a minimalni delka tonu (testy ho hraji 1,5 s).
const TONE_HZ: f64 = 1_000.0;
const MIN_TONE_MS: u32 = 500;
const WINDOW: usize = 2_400;
const RATE: f64 = 48_000.0;

/// Backend zvuku pro beh: zadany `--audio`, jinak test = wav, beh = none.
pub fn backend(opts: &Options) -> &str {
    if !opts.audio.is_empty() {
        &opts.audio
    } else if opts.test {
        "wav"
    } else {
        "none"
    }
}

/// Prida radic HDA a kodeky. `wav` = soubor pro backend wav.
///
/// Test (wav): dva radice - prvni s `hda-output` do souboru WAV, druhy
/// s `hda-duplex` a backendem `none`, ktery umi i vstup (backend wav
/// nahravat neumi). Overi se tak vic zvukovych karet, vyber zarizeni
/// a nahravani z jine karty nez vystup. Dva kodeky na jedne lince QEMU
/// neumi spravne: kazdy kodek si z DMA bere vlastni data (na skutecnem
/// hardwaru se stream vysila vsem), takze by kazdy dostal jen kousky.
/// Interaktivne jeden radic s `hda-duplex` a zvolenym backendem.
pub fn qemu_args(cmd: &mut Command, opts: &Options, wav: &Path) {
    cmd.args(["-device", "intel-hda,id=hda0"]);
    match backend(opts) {
        "wav" => {
            let out = format!(
                "wav,id=snd0,path={},out.frequency=48000,out.channels=2,out.format=s16",
                wav.display()
            );
            cmd.args(["-audiodev", &out])
                .args(["-audiodev", "none,id=snd1"])
                .args(["-device", "hda-output,bus=hda0.0,audiodev=snd0"])
                .args(["-device", "intel-hda,id=hda1"])
                .args(["-device", "hda-duplex,bus=hda1.0,audiodev=snd1"]);
        }
        other => {
            cmd.args(["-audiodev", &format!("{other},id=snd0")])
                .args(["-device", "hda-duplex,bus=hda0.0,audiodev=snd0"]);
        }
    }
}

/// Data za kouskem `data` (QEMU pri konci pres isa-debug-exit nemusi
/// stihnout opravit delky v hlavicce: pak se bere zbytek souboru).
fn wav_samples(b: &[u8]) -> Option<Vec<i16>> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12;
    while pos + 8 <= b.len() {
        let len = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = pos + 8;
        if &b[pos..pos + 4] == b"data" {
            let end = if len == 0 || body + len > b.len() { b.len() } else { body + len };
            return Some(b[body..end].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect());
        }
        pos = body + len + (len & 1);
    }
    None
}

/// Amplituda frekvence `f` v okne (Goertzel).
fn goertzel(x: &[f64], f: f64) -> f64 {
    let c = 2.0 * (2.0 * std::f64::consts::PI * f / RATE).cos();
    let (mut s1, mut s2) = (0.0, 0.0);
    for &v in x {
        let s = v + c * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    2.0 * (s1 * s1 + s2 * s2 - c * s1 * s2).max(0.0).sqrt() / x.len() as f64
}

/// Kolik milisekund souboru obsahuje cisty ton `TONE_HZ` (levy kanal).
pub fn tone_ms(samples: &[i16]) -> u32 {
    let left: Vec<f64> = samples.iter().step_by(2).map(|&v| v as f64).collect();
    let mut windows = 0;
    for w in left.chunks_exact(WINDOW) {
        let rms = (w.iter().map(|v| v * v).sum::<f64>() / w.len() as f64).sqrt();
        let amp = goertzel(w, TONE_HZ);
        // Sinus amplitudy A ma RMS A / sqrt(2): cisty ton = pomer ~1.
        if amp > 1_000.0 && amp / std::f64::consts::SQRT_2 > rms * 0.9 {
            windows += 1;
        }
    }
    (windows * WINDOW as u32 * 1000) / RATE as u32
}

/// Po testu: kdyz test tonu probehl, musi byt ton v souboru.
pub fn check(wav: &Path, serial_log: &Path, opts: &Options) -> Result<()> {
    if backend(opts) != "wav" {
        return Ok(());
    }
    let log = std::fs::read_to_string(serial_log).unwrap_or_default();
    if !log.contains(TONE_MARKER) {
        println!("xtask: test tonu neprobehl, soubor WAV nekontroluji");
        return Ok(());
    }
    let data = std::fs::read(wav).map_err(|e| format!("soubor WAV {}: {e}", wav.display()))?;
    let samples = wav_samples(&data).ok_or("soubor WAV z QEMU nema platnou hlavicku")?;
    let ms = tone_ms(&samples);
    println!("xtask: WAV z QEMU: {} ms zvuku, ton 1 kHz {} ms", samples.len() as u64 * 1000 / 2 / RATE as u64, ms);
    if ms < MIN_TONE_MS {
        return Err(format!("v zapsanem zvuku chybi ton 1 kHz (jen {ms} ms, ocekavano aspon {MIN_TONE_MS} ms)").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(ms: u32, amp: f64) -> Vec<i16> {
        let n = (RATE as u32 * ms / 1000) as usize;
        (0..n)
            .flat_map(|i| {
                let v = (amp * (2.0 * std::f64::consts::PI * TONE_HZ * i as f64 / RATE).sin()) as i16;
                [v, v]
            })
            .collect()
    }

    #[test]
    fn detects_tone_and_rejects_noise() {
        let mut s = vec![0i16; 48_000];
        s.extend(tone(1_000, 12_000.0));
        assert!((950..=1_000).contains(&tone_ms(&s)), "{}", tone_ms(&s));
        // Jina frekvence ani ticho tonem nejsou.
        let other: Vec<i16> = (0..96_000)
            .map(|i| (12_000.0 * (2.0 * std::f64::consts::PI * 440.0 * (i / 2) as f64 / RATE).sin()) as i16)
            .collect();
        assert_eq!(tone_ms(&other), 0);
        assert_eq!(tone_ms(&vec![0; 96_000]), 0);
    }

    #[test]
    fn reads_unfinished_wav() {
        let mut f = Vec::new();
        f.extend_from_slice(b"RIFF\0\0\0\0WAVEfmt ");
        f.extend_from_slice(&16u32.to_le_bytes());
        f.extend_from_slice(&[1, 0, 2, 0, 0x80, 0xBB, 0, 0, 0, 0xEE, 2, 0, 4, 0, 16, 0]);
        f.extend_from_slice(b"data\0\0\0\0");
        f.extend_from_slice(&[1, 0, 2, 0]);
        assert_eq!(wav_samples(&f), Some(vec![1, 2]));
        assert_eq!(wav_samples(b"nic"), None);
    }
}

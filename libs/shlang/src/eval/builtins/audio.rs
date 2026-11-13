//! Prikaz `audio`: testovaci ton, prehrani WAV, hlasitost, ztlumeni, stav.
//!
//! ```text
//! audio tone 440 --duration 2s --level 30
//! audio play /hudba/ukazka.wav
//! audio volume 60          # bez cisla vrati aktualni hlasitost
//! audio mute               # unmute zrusi
//! audio status
//! audio devices            # zvukova zarizeni; audio device 1 vybere
//! ```
//!
//! Ton i soubor jdou do hostitele po kouscich [`CHUNK_MS`]: soubor se cte
//! po castech pres FS relace (prava jako `open`, velikost neomezena),
//! mezi kousky se kontroluje Ctrl-C (pak `Abort` a chyba `SH_INTERRUPTED`).

use alloc::vec::Vec;

use audioproto::{Format, Sample};
use audiomix::tone::Tone;
use audiomix::wav;

use super::super::call::Args;
use super::super::{Flow, Interp, R};
use super::{int_arg, str_arg};
use crate::error::{ShError, code};
use crate::host::{AudioReq, Host};
use crate::value::Value;

/// Nejdelsi ton (ms).
const MAX_TONE_MS: i64 = 3_600_000;
/// Velikost kousku (ms zvuku).
const CHUNK_MS: u32 = 100;
/// Kolik bajtu ze zacatku souboru staci na hlavicku WAV.
const HEADER_READ: usize = 64 << 10;

fn host_audio(host: &mut dyn Host, a: &Args, req: AudioReq) -> R<Value> {
    match host.audio(req) {
        Some(r) => r.map_err(|e| Flow::Err(e.at(a.span))),
        None => Err(a.err(abi::Error::NotSupported.name(), "zvuk tu není (žádná zvuková služba)")),
    }
}

pub(super) fn call(it: &mut Interp, host: &mut dyn Host, a: Args) -> R<Value> {
    let action = str_arg(&a, 0, "akce")?;
    match action.as_str() {
        "tone" => tone(host, &a),
        "play" => play(it, host, &a),
        "volume" => match int_arg(&a, 1, "hlasitost")? {
            Some(v) if (0..=100).contains(&v) => host_audio(host, &a, AudioReq::Volume(Some(v as u8))),
            Some(_) => Err(a.err(code::ARGS, "hlasitost musí být 0-100")),
            None => host_audio(host, &a, AudioReq::Volume(None)),
        },
        "mute" => host_audio(host, &a, AudioReq::Mute(true)),
        "unmute" => host_audio(host, &a, AudioReq::Mute(false)),
        "status" => host_audio(host, &a, AudioReq::Status),
        "devices" => host_audio(host, &a, AudioReq::Devices),
        "device" => match int_arg(&a, 1, "zařízení")? {
            Some(i) if (0..=255).contains(&i) => host_audio(host, &a, AudioReq::Select(i as u8)),
            _ => Err(a.err(code::ARGS, "audio device čeká číslo zařízení (audio devices je vypíše)")),
        },
        other => Err(Flow::Err(
            ShError::new(code::ARGS, alloc::format!("audio: neznámá akce '{other}'"))
                .at(a.span)
                .hint("akce: tone, play, volume, mute, unmute, status, devices, device"),
        )),
    }
}

/// Posle kousky z `next` (None = konec); mezi kousky Ctrl-C.
fn stream(host: &mut dyn Host, a: &Args, f: Format, mut next: impl FnMut(&mut dyn Host) -> R<Option<Vec<u8>>>) -> R<Value> {
    host_audio(host, a, AudioReq::Open { rate: f.rate, channels: f.channels, sample: f.sample as u8 })?;
    loop {
        if host.interrupted() {
            let _ = host.audio(AudioReq::Abort);
            return Err(Flow::Err(ShError::new(code::INTERRUPTED, "přerušeno (Ctrl-C)")));
        }
        let chunk = match next(host) {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                let _ = host.audio(AudioReq::Abort);
                return Err(e);
            }
        };
        if let Err(e) = host_audio(host, a, AudioReq::Write { data: chunk }) {
            let _ = host.audio(AudioReq::Abort);
            return Err(e);
        }
    }
    host_audio(host, a, AudioReq::Finish)
}

fn tone(host: &mut dyn Host, a: &Args) -> R<Value> {
    let freq = int_arg(a, 1, "frekvence")?.unwrap_or(440);
    if !(20..=20_000).contains(&freq) {
        return Err(a.err(code::ARGS, "frekvence tónu musí být 20-20000 Hz"));
    }
    let ms = match a.named("duration") {
        Some(Value::Duration(ns)) => ns / 1_000_000,
        Some(_) => return Err(a.err(code::TYPE, "--duration čeká dobu, např. 2s")),
        None => 1_000,
    };
    if !(1..=MAX_TONE_MS).contains(&ms) {
        return Err(a.err(code::ARGS, "délka tónu musí být 1 ms - 1 h"));
    }
    let level = match a.named("level") {
        Some(Value::Int(l)) if (0..=100).contains(l) => *l as u8,
        Some(_) => return Err(a.err(code::ARGS, "--level čeká 0-100")),
        None => 50,
    };
    let f = Format::new(48_000, 1, Sample::S16);
    let mut t = Tone::new(freq as u32, f.rate, level);
    let mut left = f.rate as u64 * ms as u64 / 1000;
    let per = (f.rate * CHUNK_MS / 1000) as u64;
    stream(host, a, f, |_| {
        if left == 0 {
            return Ok(None);
        }
        let n = left.min(per);
        left -= n;
        let mut c = Vec::with_capacity(n as usize * 2);
        for _ in 0..n {
            let [l, _] = t.next_frame();
            c.extend_from_slice(&((l >> 8) as i16).to_le_bytes());
        }
        Ok(Some(c))
    })
}

fn play(it: &mut Interp, host: &mut dyn Host, a: &Args) -> R<Value> {
    let Some(v) = a.pos(1) else {
        return Err(a.err(code::ARGS, "audio play: chybí soubor WAV"));
    };
    let p = it.paths(v)?.into_iter().next().ok_or_else(|| a.err(code::ARGS, "audio play: chybí soubor WAV"))?;
    let size = it.fs_stat(host, &p)?.size;
    let head = it.fs_read_range(host, &p, 0, HEADER_READ)?;
    let h = wav::parse_header(&head, size).map_err(|e| {
        let why = match e {
            wav::WavError::NotRiff => "není to soubor WAV (RIFF)",
            wav::WavError::Missing => "WAV bez části fmt nebo data",
            wav::WavError::Unsupported => "nepodporovaný formát WAV (umím PCM 8/16/32 bitů, 24 bitů ve 32 a float 32)",
        };
        a.err(code::TYPE, why)
    })?;
    let per = (h.format.frame_bytes() * h.format.rate * CHUNK_MS / 1000) as u64;
    let (mut pos, end) = (h.data_offset, h.data_offset + h.data_len);
    stream(host, a, h.format, |host| {
        if pos >= end {
            return Ok(None);
        }
        let n = (end - pos).min(per) as usize;
        let c = it.fs_read_range(host, &p, pos, n)?;
        if c.is_empty() {
            return Ok(None); // soubor se mezitim zkratil
        }
        pos += c.len() as u64;
        Ok(Some(c))
    })
}

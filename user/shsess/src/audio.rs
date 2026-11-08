//! Prikaz `audio` v relaci: klient zvukove sluzby `audiod`.
//!
//! Kanal ke sluzbe prijde ve zprave `Init` od termd (treti handle). Ton
//! se generuje po kouscich 100 ms (pamet nezavisi na delce), WAV se
//! prehraje ve svem formatu - prevod na format zarizeni dela sluzba.
//! Prikaz ceka, nez zvuk dohraje (Ctrl-C ho zatim neprerusi).

use alloc::vec::Vec;
use core::time::Duration;

use audioproto::conn::AudioClient;
use audioproto::{Direction, Format, Sample};
use audiomix::tone::Tone;
use audiomix::wav;
use libos::time;
use shlang::host::AudioReq;
use shlang::value::Value;
use shlang::{ShError, error::code};

/// Buffer streamu relace (vic nez rozpocet latence: shell nehraje
/// interaktivne, plynulost je dulezitejsi).
const STREAM_LATENCY_US: u32 = 100_000;
const TONE_RATE: u32 = 48_000;

fn sys(e: libos::Error, what: &str) -> ShError {
    ShError::sys(e, what)
}

pub fn handle(client: &AudioClient, req: AudioReq) -> Result<Value, ShError> {
    match req {
        AudioReq::Tone { freq, ms, level } => tone(client, freq, ms, level),
        AudioReq::Play { wav } => play(client, &wav),
        AudioReq::Volume(Some(p)) => client.master_volume(p).map(|_| Value::Nothing).map_err(|e| sys(e, "hlasitost")),
        AudioReq::Volume(None) => client.status().map(|s| Value::Int(s.master as i64)).map_err(|e| sys(e, "stav zvuku")),
        AudioReq::Mute(on) => client.mute(on).map(|_| Value::Nothing).map_err(|e| sys(e, "ztlumeni")),
        AudioReq::Status => status(client),
    }
}

fn status(client: &AudioClient) -> Result<Value, ShError> {
    let s = client.status().map_err(|e| sys(e, "stav zvuku"))?;
    Ok(Value::record(&[
        ("device", Value::Str(s.device)),
        ("outputs", Value::Str(s.outputs)),
        ("rate", Value::Int(s.format.rate as i64)),
        ("volume", Value::Int(s.master as i64)),
        ("muted", Value::Bool(s.muted)),
        ("streams", Value::Int(s.streams as i64)),
        ("latency", Value::Duration(s.latency_us as i64 * 1_000)),
        ("underruns", Value::Int(s.device_underruns as i64)),
    ]))
}

/// Vysledek prehrani: delka a podteceni.
fn played(s: &audioproto::conn::Stream<'_>, f: Format) -> Result<Value, ShError> {
    let info = s.stream_info().map_err(|e| sys(e, "stav streamu"))?;
    Ok(Value::record(&[
        ("duration", Value::Duration((f.us_for_frames(info.played) * 1_000) as i64)),
        ("underruns", Value::Int(info.underruns as i64)),
    ]))
}

fn tone(client: &AudioClient, freq: u32, ms: u32, level: u8) -> Result<Value, ShError> {
    let f = Format::new(TONE_RATE, 1, Sample::S16);
    let s = client.open(Direction::Output, f, STREAM_LATENCY_US).map_err(|e| sys(e, "zvukovy stream"))?;
    let mut t = Tone::new(freq, TONE_RATE, level);
    let total = TONE_RATE as u64 * ms as u64 / 1000;
    let mut done = 0u64;
    let mut chunk = Vec::new();
    while done < total {
        let n = (total - done).min(TONE_RATE as u64 / 10) as usize;
        chunk.clear();
        for _ in 0..n {
            let [l, _] = t.next_frame();
            chunk.extend_from_slice(&((l >> 8) as i16).to_le_bytes());
        }
        let deadline = time::deadline_after(Duration::from_secs(10));
        s.write_all(&chunk, true, Some(deadline)).map_err(|e| sys(e, "zapis zvuku"))?;
        done += n as u64;
    }
    s.drain().map_err(|e| sys(e, "dohrani"))?;
    played(&s, f)
}

fn play(client: &AudioClient, data: &[u8]) -> Result<Value, ShError> {
    let w = wav::parse(data).map_err(|e| {
        let why = match e {
            wav::WavError::NotRiff => "neni to soubor WAV (RIFF)",
            wav::WavError::Missing => "WAV bez casti fmt nebo data",
            wav::WavError::Unsupported => "nepodporovany format WAV (umim PCM 8/16/24/32 bitu a float 32)",
        };
        ShError::new(code::TYPE, why)
    })?;
    let s = client.open(Direction::Output, w.format, STREAM_LATENCY_US).map_err(|e| sys(e, "zvukovy stream"))?;
    // Po kouscich: termin zapisu se pocita od posledniho pokroku.
    let step = (w.format.frame_bytes() * w.format.rate / 10) as usize;
    for part in w.data.chunks(step.max(1)) {
        let deadline = time::deadline_after(Duration::from_secs(10));
        s.write_all(part, true, Some(deadline)).map_err(|e| sys(e, "zapis zvuku"))?;
    }
    s.drain().map_err(|e| sys(e, "dohrani"))?;
    played(&s, w.format)
}

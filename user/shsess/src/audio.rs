//! Prikaz `audio` v relaci: klient zvukove sluzby `audiod`.
//!
//! Kanal ke sluzbe (a listek, ktery prezije jeji restart) prijde ve zprave
//! `Init` od termd. Prehravani ridi interpret po kouscich (`Open`, `Write`,
//! `Finish`, pri Ctrl-C `Abort`); relace drzi otevreny stream mezi
//! kousky. Format prevadi sluzba.

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use audioproto::conn::{AudioClient, Stream};
use audioproto::{Direction, Format, Sample};
use libos::time;
use shlang::host::AudioReq;
use shlang::value::Value;
use shlang::{ShError, error::code};

/// Buffer streamu relace (vic nez rozpocet latence: shell nehraje
/// interaktivne, plynulost je dulezitejsi). Kousek interpretu je 100 ms.
const STREAM_LATENCY_US: u32 = 200_000;
/// Nejdel cekani na misto v bufferu pro jeden kousek.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

fn sys(e: libos::Error, what: &str) -> ShError {
    ShError::sys(e, what)
}

pub fn handle(client: &AudioClient, playing: &mut Option<Stream>, req: AudioReq) -> Result<Value, ShError> {
    match req {
        AudioReq::Open { rate, channels, sample } => {
            let sample = Sample::from_u8(sample).ok_or_else(|| ShError::new(code::TYPE, "neznamy typ vzorku"))?;
            let f = Format::new(rate, channels, sample);
            if !f.valid() {
                return Err(ShError::new(code::TYPE, alloc::format!("nepodporovany format {rate} Hz, {channels} kanalu")));
            }
            // Predchozi stream (prerusene prehravani) zavrit.
            *playing = None;
            *playing = Some(client.open(Direction::Output, f, STREAM_LATENCY_US).map_err(|e| sys(e, "zvukovy stream"))?);
            Ok(Value::Nothing)
        }
        AudioReq::Write { data } => {
            let s = playing.as_ref().ok_or_else(|| ShError::new(code::ARGS, "zadny otevreny zvukovy stream"))?;
            let deadline = time::deadline_after(WRITE_TIMEOUT);
            s.write_all(&data, true, Some(deadline)).map_err(|e| sys(e, "zapis zvuku"))?;
            Ok(Value::Nothing)
        }
        AudioReq::Finish => {
            let s = playing.take().ok_or_else(|| ShError::new(code::ARGS, "zadny otevreny zvukovy stream"))?;
            s.drain().map_err(|e| sys(e, "dohrani"))?;
            let info = s.stream_info().map_err(|e| sys(e, "stav streamu"))?;
            let f = s.info().format;
            Ok(Value::record(&[
                ("duration", Value::Duration((f.us_for_frames(info.played) * 1_000) as i64)),
                ("underruns", Value::Int(info.underruns as i64)),
            ]))
        }
        AudioReq::Abort => {
            if let Some(s) = playing.take() {
                let _ = s.stop();
            }
            Ok(Value::Nothing)
        }
        AudioReq::Volume(Some(p)) => client.master_volume(p).map(|_| Value::Nothing).map_err(|e| sys(e, "hlasitost")),
        AudioReq::Volume(None) => client.status().map(|s| Value::Int(s.master as i64)).map_err(|e| sys(e, "stav zvuku")),
        AudioReq::Mute(on) => client.mute(on).map(|_| Value::Nothing).map_err(|e| sys(e, "ztlumeni")),
        AudioReq::Status => status(client),
        AudioReq::Devices => devices(client),
        AudioReq::Select(i) => client.select(i).map(|_| Value::Nothing).map_err(|e| sys(e, "vyber zarizeni")),
    }
}

fn status(client: &AudioClient) -> Result<Value, ShError> {
    let s = client.status().map_err(|e| sys(e, "stav zvuku"))?;
    let bits = if s.format.sample == Sample::S16 { 16 } else { 24 };
    Ok(Value::record(&[
        ("device", Value::Str(s.device)),
        ("index", Value::Int(s.device_index as i64)),
        ("outputs", Value::Str(s.outputs)),
        ("input", Value::Bool(s.input)),
        ("rate", Value::Int(s.format.rate as i64)),
        ("bits", Value::Int(bits)),
        ("volume", Value::Int(s.master as i64)),
        ("muted", Value::Bool(s.muted)),
        ("streams", Value::Int(s.streams as i64)),
        ("latency", Value::Duration(s.latency_us as i64 * 1_000)),
        ("underruns", Value::Int(s.device_underruns as i64)),
    ]))
}

fn devices(client: &AudioClient) -> Result<Value, ShError> {
    let list = client.devices().map_err(|e| sys(e, "seznam zarizeni"))?;
    Ok(Value::List(
        list.into_iter()
            .map(|d| {
                Value::record(&[
                    ("index", Value::Int(d.index as i64)),
                    ("name", Value::Str(d.name)),
                    ("outputs", Value::Str(if d.outputs.is_empty() { String::from("-") } else { d.outputs })),
                    ("input", Value::Bool(d.input)),
                    ("active", Value::Bool(d.active)),
                ])
            })
            .collect::<Vec<_>>(),
    ))
}

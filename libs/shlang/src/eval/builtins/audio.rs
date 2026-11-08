//! Prikaz `audio`: testovaci ton, prehrani WAV, hlasitost, ztlumeni, stav.
//!
//! ```text
//! audio tone 440 --duration 2s --level 30
//! audio play /hudba/ukazka.wav
//! audio volume 60          # bez cisla vrati aktualni hlasitost
//! audio mute               # unmute zrusi
//! audio status
//! ```
//!
//! Soubor WAV cte interpret pres FS relace (prava jako `open`); zvuk sam
//! resi hostitel ([`crate::host::Host::audio`]).

use super::super::call::Args;
use super::super::{Flow, Interp, R};
use super::{int_arg, str_arg};
use crate::error::{ShError, code};
use crate::host::{AudioReq, Host};
use crate::value::Value;

/// Nejdelsi ton (ms) - prikaz ceka, nez dohraje.
const MAX_TONE_MS: i64 = 60_000;
/// Nejvetsi soubor WAV (cte se cely do pameti relace): 64 MiB, asi 6 min
/// CD kvality.
const MAX_WAV: u64 = 64 << 20;

pub(super) fn call(it: &mut Interp, host: &mut dyn Host, a: Args) -> R<Value> {
    let action = str_arg(&a, 0, "akce")?;
    let req = match action.as_str() {
        "tone" => {
            let freq = int_arg(&a, 1, "frekvence")?.unwrap_or(440);
            if !(20..=20_000).contains(&freq) {
                return Err(a.err(code::ARGS, "frekvence tonu musi byt 20-20000 Hz"));
            }
            let ms = match a.named("duration") {
                Some(Value::Duration(ns)) => ns / 1_000_000,
                Some(_) => return Err(a.err(code::TYPE, "--duration ceka dobu, napr. 2s")),
                None => 1_000,
            };
            if !(1..=MAX_TONE_MS).contains(&ms) {
                return Err(a.err(code::ARGS, "delka tonu musi byt 1 ms - 60 s"));
            }
            let level = match a.named("level") {
                Some(Value::Int(l)) if (0..=100).contains(l) => *l as u8,
                Some(_) => return Err(a.err(code::ARGS, "--level ceka 0-100")),
                None => 50,
            };
            AudioReq::Tone { freq: freq as u32, ms: ms as u32, level }
        }
        "play" => {
            let Some(v) = a.pos(1) else {
                return Err(a.err(code::ARGS, "audio play: chybi soubor WAV"));
            };
            let p = it.paths(v)?.into_iter().next().ok_or_else(|| a.err(code::ARGS, "audio play: chybi soubor WAV"))?;
            let size = it.fs_stat(host, &p)?.size;
            if size > MAX_WAV {
                return Err(a.err(code::ARGS, alloc::format!("audio play: soubor ma {} MiB, nejvic {} MiB", size >> 20, MAX_WAV >> 20)));
            }
            AudioReq::Play { wav: it.fs_read(host, &p)? }
        }
        "volume" => match int_arg(&a, 1, "hlasitost")? {
            Some(v) if (0..=100).contains(&v) => AudioReq::Volume(Some(v as u8)),
            Some(_) => return Err(a.err(code::ARGS, "hlasitost musi byt 0-100")),
            None => AudioReq::Volume(None),
        },
        "mute" => AudioReq::Mute(true),
        "unmute" => AudioReq::Mute(false),
        "status" => AudioReq::Status,
        other => {
            return Err(Flow::Err(
                ShError::new(code::ARGS, alloc::format!("audio: neznama akce '{other}'"))
                    .at(a.span)
                    .hint("akce: tone, play, volume, mute, unmute, status"),
            ));
        }
    };
    match host.audio(req) {
        Some(r) => r.map_err(|e| Flow::Err(e.at(a.span))),
        None => Err(a.err(abi::Error::NotSupported.name(), "zvuk tu neni (zadna zvukova sluzba)")),
    }
}

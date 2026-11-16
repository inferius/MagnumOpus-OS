//! Rezimy role `Audio` (relace zvuku zdedena od rodice, `docs/cs/zvuk.md`,
//! kapitola Relace zvuku pro potomky; test `user/init/src/tests/audio_role.rs`).
//! Zvuk jde pres spolecne jadro `libs/mowin` (`moaudio`), stejne jako
//! u SDL a Wine.
//!
//! - `hraj`: otevre vystup a hraje ton (440 Hz); kdyz sluzba odebrala vic
//!   nez cely buffer (opravdu hraje), vypise `gui-probe: hraje`. Kdyz
//!   zapis prestane projit (odvolani zvuku), vypise `gui-probe: zvuk
//!   prestal` a skonci s 0. Kod 3 = relaci
//!   nedostal, 4 = sluzba odmitla stream, 6 = za 60 s nic.
//! - `hraj-kratce`: jako `hraj`, ale skonci po 0,3 s tonu (kod 0).
//! - `zvuk-dite`: spusti sam sebe v rezimu `hraj-kratce` jednou s
//!   `MO_AUDIO` (ceka 0) a jednou bez ni (ceka 3). Kod 0 = obe podle
//!   ocekavani.
//! - `zvuk-odvolani`: spusti dite v rezimu `hraj` (s roli Audio) a vrati
//!   jeho kod.

use std::time::{Duration, Instant};

use mowin::audio;

use super::{NO_SESSION, REFUSED, code, me};

const RATE: u32 = 48_000;
/// Vzorky S16 (`MOAUDIO_SAMPLE_S16`), stereo.
const S16: u32 = 1;
const CHANNELS: u32 = 2;
const STOPPED_AFTER: Duration = Duration::from_secs(2);

/// Blok tonu 440 Hz (S16 stereo) od snimku `start`.
fn tone(start: u64, frames: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(frames * 4);
    for i in 0..frames as u64 {
        let t = (start + i) as f32 / RATE as f32;
        let v = ((t * 440.0 * core::f32::consts::TAU).sin() * 8000.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Hraje ton; `limit` = jak dlouho (None = do zastaveni sluzbou).
fn play(limit: Option<Duration>) -> i32 {
    if !audio::available() {
        return NO_SESSION;
    }
    let (id, period) = match audio::open(0, RATE, CHANNELS, S16, 20_000) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("gui-probe: zvuk: stream odmitnut: {e:?}");
            return REFUSED;
        }
    };
    let frames = period.max(256) as usize;
    // Hraje = zapsano vic nez buffer a jeste blok: sluzba data odebira.
    let cap = audio::fill(id).map_or(RATE as u64, |(_, c)| c as u64);
    let audible = cap + 4 * frames as u64;
    let begin = Instant::now();
    let mut at = 0u64;
    let mut playing = false;
    loop {
        let block = tone(at, frames);
        // Bez odberu sluzbou (odvolani) se buffer zaplni a zapis do lhuty
        // neprojde.
        match audio::write(id, &block, STOPPED_AFTER.as_nanos() as i64) {
            Ok(()) => {
                at += frames as u64;
                if !playing && at >= audible {
                    playing = true;
                    println!("gui-probe: hraje");
                }
            }
            Err(e) if playing => {
                println!("gui-probe: zvuk prestal ({e:?})");
                audio::close(id);
                return 0;
            }
            Err(e) => {
                eprintln!("gui-probe: zvuk: zapis (zapsano {at} snimku): {e:?}");
                return REFUSED;
            }
        }
        if let Some(l) = limit
            && begin.elapsed() >= l
        {
            audio::close(id);
            return 0;
        }
        if begin.elapsed() > Duration::from_secs(60) {
            eprintln!("gui-probe: zvuk: za 60 s se nic nestalo");
            return 6;
        }
    }
}

pub(super) fn run(mode: &str) -> Option<i32> {
    Some(match mode {
        "hraj" => play(None),
        "hraj-kratce" => play(Some(Duration::from_millis(300))),
        "zvuk-dite" => {
            let with = code(&mut me("hraj-kratce"));
            let without = code(me("hraj-kratce").env_remove("MO_AUDIO"));
            println!("gui-probe: zvuk dite s roli {with}, bez role {without}");
            if with == 0 && without == NO_SESSION { 0 } else { 1 }
        }
        "zvuk-odvolani" => code(&mut me("hraj")),
        _ => return None,
    })
}

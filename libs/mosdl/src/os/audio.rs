//! Zvuk pro SDL (`mosdl_audio_*`): jen obalky nad spolecnym jadrem
//! `libs/mowin/src/os/audio.rs` (`moaudio`, relace od runtime
//! `osrt::audio`, streamy `audiod`). Vlakna: SDL pise a cte z vlakna
//! sveho zvukoveho zarizeni, cekani na misto (`WaitDevice`) bezi bez zamku.

use libos::Error;
use mowin::audio as core_audio;

use super::code;

fn status(r: Result<(), Error>) -> i32 {
    r.map_or_else(code, |_| 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn mosdl_audio_available() -> i32 {
    core_audio::available() as i32
}

/// Otevre stream (`dir` 0 vystup, 1 vstup). Vrati id > 0, nebo zaporny kod
/// chyby (`ACCESS_DENIED` = nahravani nepovoleno). `period_frames` =
/// perioda sluzby ve snimcich (velikost bloku pro SDL).
///
/// # Safety
///
/// `out_frames` je platny ukazatel nebo NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdl_audio_open(dir: i32, rate: u32, channels: u32, sample_fmt: u32, latency_us: u32, out_frames: *mut u32) -> i32 {
    match core_audio::open(dir, rate, channels, sample_fmt, latency_us) {
        Ok((id, period)) => {
            if !out_frames.is_null() {
                // SAFETY: platny podle smlouvy.
                unsafe { out_frames.write(period) };
            }
            id
        }
        Err(e) => code(e),
    }
}

/// # Safety
///
/// `data` ukazuje na `bytes` citelnych bajtu.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdl_audio_write(id: i32, data: *const u8, bytes: u32, timeout_ns: i64) -> i32 {
    if data.is_null() {
        return code(Error::InvalidArgs);
    }
    // SAFETY: delka podle smlouvy.
    let buf = unsafe { core::slice::from_raw_parts(data, bytes as usize) };
    status(core_audio::write(id, buf, timeout_ns))
}

/// # Safety
///
/// `data` ukazuje na `bytes` zapisovatelnych bajtu.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdl_audio_read(id: i32, data: *mut u8, bytes: u32) -> i32 {
    if data.is_null() {
        return code(Error::InvalidArgs);
    }
    // SAFETY: delka podle smlouvy.
    let buf = unsafe { core::slice::from_raw_parts_mut(data, bytes as usize) };
    core_audio::read(id, buf).map_or_else(code, |n| n as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn mosdl_audio_wait_space(id: i32, bytes: u32, timeout_ns: i64) -> i32 {
    status(core_audio::wait_space(id, bytes, timeout_ns))
}

#[unsafe(no_mangle)]
pub extern "C" fn mosdl_audio_wait(id: i32, timeout_ns: i64) -> i32 {
    status(core_audio::wait(id, timeout_ns))
}

#[unsafe(no_mangle)]
pub extern "C" fn mosdl_audio_start(id: i32) -> i32 {
    status(core_audio::start(id))
}

#[unsafe(no_mangle)]
pub extern "C" fn mosdl_audio_close(id: i32) {
    core_audio::close(id)
}

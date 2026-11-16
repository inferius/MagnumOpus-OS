//! Zvuk aplikace (`moaudio_*`, `include/moaudio.h`): streamy `audiod`
//! pres relaci aplikace (`audioproto::session`). **Spolecny modul**: C API
//! `moaudio_*` (ovladac zvuku Wine `winemo.drv`) i `mosdl_audio_*`
//! (`libs/mosdl/src/os/audio.rs` jen vola funkce odsud).
//!
//! Relace: od runtime (`osrt::audio`, role `Audio` od rodice nebo relace
//! aplikace od initu, stitky `AUDIO` a `AUDIO_LEASE`). Listek si vezme
//! tento modul (obnova po restartu audiod, `svcclient::Conn`) a runtime
//! dostane hacek pro relace potomku (`NARROW` na tomto listku).
//!
//! Relace ma prava aplikace: vlastni streamy prehravani; nahravani jen
//! s `[audio] capture = true` v manifestu a po povoleni uzivatelem (dialog
//! sluzby uctu, `OPEN` vstupu do te doby ceka; indikator nahravani v liste).
//! Vzorky jdou pres sdileny kruhovy buffer (`audioproto::ring`).
//!
//! Zamky: stav (`AUDIO`) jen na kratke operace; zapis a cekani na misto
//! bezi **bez zamku** na klonu streamu (blokujici zapis nesmi drzet zamek,
//! jinak by zastavil otevirani a zavirani z jinych vlaken, napr. vlakna
//! `mmdevapi` ve Wine).

use std::os::magnumopus::runtime as rt;
use std::sync::{Arc, Mutex, OnceLock};

use audioproto::conn::{AudioClient, Stream};
use audioproto::{Direction, Format, Sample};
use libos::{Channel, Error, Handle};

use super::code;

/// Stream: kruhovy buffer je jeden producent a jeden konzument. Volajici
/// (SDL: vlakno zvukoveho zarizeni; Wine: vlakno streamu `mmdevapi`) pise
/// i cte z jednoho vlakna, ostatni vlakna jen otviraji a zaviraji.
struct Shared(Stream);

// SAFETY: `Stream` drzi mapovani sdileneho bufferu (surove ukazatele);
// volani sluzby jdou pres `AudioClient` (zamek), ring pouziva jedine vlakno
// (viz vyse), takze sdileni odkazu mezi vlakny je bezpecne.
unsafe impl Send for Shared {}
// SAFETY: viz `Send` - soubezne `write`/`read` z vic vlaken se nedela.
unsafe impl Sync for Shared {}

struct Audio {
    client: Option<AudioClient>,
    /// Otevrene streamy (index + 1 = id pro C).
    streams: Vec<Option<Arc<Shared>>>,
}

static AUDIO: OnceLock<Mutex<Audio>> = OnceLock::new();

fn lock() -> std::sync::MutexGuard<'static, Audio> {
    audio().lock().unwrap_or_else(|e| e.into_inner())
}

fn audio() -> &'static Mutex<Audio> {
    AUDIO.get_or_init(|| {
        // Druha kopie libos (mapovani ringu) potrebuje handle procesu.
        libos::rt::set_self_process(rt::start::self_process());
        let client = rt::audio::take_client_raw().map(|(s, l)| {
            let chan = Channel::from_handle(Handle::from_raw(s));
            match l {
                Some(l) => {
                    rt::audio::set_child_hook(child_session);
                    AudioClient::with_lease(chan, Channel::from_handle(Handle::from_raw(l)))
                }
                // Relace bez listku (testy): bez obnovy a bez potomku.
                None => AudioClient::new(chan),
            }
        });
        Mutex::new(Audio { client, streams: Vec::new() })
    })
}

/// Hacek runtime: relace a listek pro potomka (`NARROW` na listku).
fn child_session() -> core::result::Result<(u32, u32), i32> {
    let client = lock().client.clone().ok_or(Error::NotSupported.code())?;
    let (s, l) = client.narrow().map_err(|e| e.code())?;
    Ok((s.into_handle().into_raw(), l.into_handle().into_raw()))
}

fn stream(id: i32) -> Option<Arc<Shared>> {
    let a = lock();
    a.streams.get((id as usize).checked_sub(1)?)?.clone()
}

/// Format C (`MOAUDIO_SAMPLE_*` = `MOSDL_SAMPLE_*`) -> format sluzby.
fn sample(v: u32) -> Option<Sample> {
    match v {
        0 => Some(Sample::U8),
        1 => Some(Sample::S16),
        2 => Some(Sample::S32),
        3 => Some(Sample::F32),
        _ => None,
    }
}

fn deadline(timeout_ns: i64) -> Option<u64> {
    (timeout_ns >= 0).then(|| libos::time::now().saturating_add(timeout_ns as u64))
}

/// Ma proces relaci zvuku?
pub fn available() -> bool {
    lock().client.is_some()
}

/// Otevre stream (`dir` 0 vystup, 1 vstup); id > 0 a perioda sluzby ve
/// snimcich.
pub fn open(dir: i32, rate: u32, channels: u32, sample_fmt: u32, latency_us: u32) -> Result<(i32, u32), Error> {
    let s = sample(sample_fmt).ok_or(Error::InvalidArgs)?;
    let ch = u8::try_from(channels).map_err(|_| Error::InvalidArgs)?;
    let dir = if dir == 1 { Direction::Input } else { Direction::Output };
    let client = lock().client.clone().ok_or(Error::NotFound)?;
    // Bez zamku: OPEN vstupu muze cekat na rozhodnuti uzivatele.
    let st = client.open(dir, Format::new(rate, ch, s), latency_us)?;
    let period = st.info().period;
    let mut a = lock();
    a.streams.push(Some(Arc::new(Shared(st))));
    Ok((a.streams.len() as i32, period))
}

/// Zapise vsechna data (cele snimky), pri plnem bufferu ceka.
pub fn write(id: i32, data: &[u8], timeout_ns: i64) -> Result<(), Error> {
    stream(id).ok_or(Error::NotFound)?.0.write_all(data, true, deadline(timeout_ns))
}

/// Zapise jen to, co se do bufferu hned vejde (cele snimky), bez cekani
/// a bez volani sluzby (stream nespousti); vrati pocet zapsanych snimku.
/// Pro volajici, kteri pisou pod svym zamkem (ovladac Wine) a stream
/// spousti sami mimo nej (`start`).
pub fn put(id: i32, data: &[u8]) -> Result<u32, Error> {
    Ok(stream(id).ok_or(Error::NotFound)?.0.ring().write(data))
}

/// Precte dostupna nahrana data; vrati pocet bajtu.
pub fn read(id: i32, data: &mut [u8]) -> Result<usize, Error> {
    let s = stream(id).ok_or(Error::NotFound)?;
    let frames = s.0.read(data);
    Ok(frames as usize * s.0.ring().frame_bytes() as usize)
}

/// Pocka, az se do bufferu vejde `bytes` bajtu.
pub fn wait_space(id: i32, bytes: u32, timeout_ns: i64) -> Result<(), Error> {
    stream(id).ok_or(Error::NotFound)?.0.wait_space(bytes as usize, deadline(timeout_ns))
}

/// Pocka na udalost streamu (misto / nova data); vyprseni neni chyba.
pub fn wait(id: i32, timeout_ns: i64) -> Result<(), Error> {
    match stream(id).ok_or(Error::NotFound)?.0.wait(deadline(timeout_ns)) {
        Ok(()) | Err(Error::TimedOut) => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn start(id: i32) -> Result<(), Error> {
    stream(id).ok_or(Error::NotFound)?.0.start()
}

pub fn stop(id: i32) -> Result<(), Error> {
    stream(id).ok_or(Error::NotFound)?.0.stop()
}

/// Hlasitost streamu (0-100 %).
pub fn volume(id: i32, percent: u8) -> Result<(), Error> {
    stream(id).ok_or(Error::NotFound)?.0.volume(percent)
}

/// Snimky ve sdilenem bufferu, ktere sluzba jeste neprehrala (vystup)
/// nebo ktere ceka na precteni (vstup), a velikost bufferu ve snimcich.
pub fn fill(id: i32) -> Result<(u32, u32), Error> {
    let s = stream(id).ok_or(Error::NotFound)?;
    let r = s.0.ring();
    Ok((r.readable(), r.capacity()))
}

/// Snimky streamu, ktere uz zazneli (odhad sluzby podle pozice DMA), a
/// snimky zapsane celkem (pozice zapisu).
pub fn position(id: i32) -> Result<(u64, u64), Error> {
    let s = stream(id).ok_or(Error::NotFound)?;
    let r = s.0.ring();
    Ok((r.played(), r.write_pos()))
}

/// Zavre stream (zruseni posledniho odkazu = `CLOSE`; prehrani zbytku
/// neceka).
pub fn close(id: i32) {
    let mut a = lock();
    if let Some(slot) = (id as usize).checked_sub(1).and_then(|i| a.streams.get_mut(i)) {
        *slot = None;
    }
}

fn status(r: Result<(), Error>) -> i32 {
    r.map_or_else(code, |_| 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn moaudio_available() -> i32 {
    available() as i32
}

/// # Safety
///
/// `out_frames` je platny ukazatel nebo NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moaudio_open(dir: i32, rate: u32, channels: u32, sample_fmt: u32, latency_us: u32, out_frames: *mut u32) -> i32 {
    match open(dir, rate, channels, sample_fmt, latency_us) {
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
pub unsafe extern "C" fn moaudio_write(id: i32, data: *const u8, bytes: u32, timeout_ns: i64) -> i32 {
    if data.is_null() {
        return code(Error::InvalidArgs);
    }
    // SAFETY: delka podle smlouvy.
    let buf = unsafe { core::slice::from_raw_parts(data, bytes as usize) };
    status(write(id, buf, timeout_ns))
}

/// # Safety
///
/// `data` ukazuje na `bytes` citelnych bajtu.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moaudio_put(id: i32, data: *const u8, bytes: u32) -> i32 {
    if data.is_null() {
        return code(Error::InvalidArgs);
    }
    // SAFETY: delka podle smlouvy.
    let buf = unsafe { core::slice::from_raw_parts(data, bytes as usize) };
    put(id, buf).map_or_else(code, |n| n as i32)
}

/// # Safety
///
/// `data` ukazuje na `bytes` zapisovatelnych bajtu.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moaudio_read(id: i32, data: *mut u8, bytes: u32) -> i32 {
    if data.is_null() {
        return code(Error::InvalidArgs);
    }
    // SAFETY: delka podle smlouvy.
    let buf = unsafe { core::slice::from_raw_parts_mut(data, bytes as usize) };
    read(id, buf).map_or_else(code, |n| n as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn moaudio_wait_space(id: i32, bytes: u32, timeout_ns: i64) -> i32 {
    status(wait_space(id, bytes, timeout_ns))
}

#[unsafe(no_mangle)]
pub extern "C" fn moaudio_wait(id: i32, timeout_ns: i64) -> i32 {
    status(wait(id, timeout_ns))
}

#[unsafe(no_mangle)]
pub extern "C" fn moaudio_start(id: i32) -> i32 {
    status(start(id))
}

#[unsafe(no_mangle)]
pub extern "C" fn moaudio_stop(id: i32) -> i32 {
    status(stop(id))
}

#[unsafe(no_mangle)]
pub extern "C" fn moaudio_volume(id: i32, percent: u32) -> i32 {
    status(volume(id, percent.min(100) as u8))
}

/// # Safety
///
/// `queued` a `capacity` jsou platne ukazatele nebo NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moaudio_fill(id: i32, queued: *mut u32, capacity: *mut u32) -> i32 {
    match fill(id) {
        Ok((q, c)) => {
            // SAFETY: platne podle smlouvy.
            unsafe {
                if !queued.is_null() {
                    queued.write(q);
                }
                if !capacity.is_null() {
                    capacity.write(c);
                }
            }
            0
        }
        Err(e) => code(e),
    }
}

/// # Safety
///
/// `played` a `written` jsou platne ukazatele nebo NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moaudio_position(id: i32, played: *mut u64, written: *mut u64) -> i32 {
    match position(id) {
        Ok((p, w)) => {
            // SAFETY: platne podle smlouvy.
            unsafe {
                if !played.is_null() {
                    played.write(p);
                }
                if !written.is_null() {
                    written.write(w);
                }
            }
            0
        }
        Err(e) => code(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn moaudio_close(id: i32) {
    close(id)
}

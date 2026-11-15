//! Zvuk pro SDL (`mosdl_audio_*`): streamy `audiod` pres relaci aplikace
//! ze startovni zpravy (handle se stitkem `abi::params::tag::AUDIO`,
//! `audioproto::session`).
//!
//! Relace ma prava aplikace: vlastni streamy prehravani; nahravani jen
//! s `[audio] capture = true` v manifestu a po povoleni uzivatelem (dotaz
//! resi `audiod` se sluzbou uctu, `OPEN` vstupu do te doby ceka). Vzorky
//! jdou pres sdileny kruhovy buffer (`audioproto::ring`), ne pres kanal.
//!
//! Stav je za vlastnim zamkem (SDL vola zvuk z vlakna zvukoveho zarizeni,
//! nezavisle na videu).

use std::sync::{Arc, Mutex, OnceLock};

use audioproto::conn::{AudioClient, Stream};
use audioproto::{Direction, Format, Sample};
use libos::{Channel, Error, Handle};

use super::code;

/// Stream jednoho zarizeni SDL. Kruhovy buffer je jeden producent a jeden
/// konzument: SDL ho pouziva jen z vlakna sveho zvukoveho zarizeni (zapis
/// i cteni), ostatni vlakna jen otviraji a zaviraji.
struct Shared(Stream);

// SAFETY: `Stream` drzi mapovani sdileneho bufferu (surove ukazatele);
// volani sluzby jdou pres `AudioClient` (zamek), ring pouziva jedine vlakno
// zarizeni SDL (viz vyse), takze sdileni odkazu mezi vlakny je bezpecne.
unsafe impl Send for Shared {}
// SAFETY: viz `Send` - soubezne `write`/`read` z vic vlaken SDL nedela.
unsafe impl Sync for Shared {}

struct Audio {
    client: Option<AudioClient>,
    /// Otevrene streamy (index + 1 = id pro C).
    streams: Vec<Option<Arc<Shared>>>,
}

static AUDIO: OnceLock<Mutex<Audio>> = OnceLock::new();

fn audio() -> &'static Mutex<Audio> {
    AUDIO.get_or_init(|| {
        // Handle relace (a listek) ze startovni zpravy; bez nich SDL zvoli
        // ovladac `dummy`.
        let client = super::rt::start::take_tagged(libos::abi::params::tag::AUDIO).map(|h| {
            let chan = Channel::from_handle(Handle::from_raw(h));
            match super::rt::start::take_tagged(libos::abi::params::tag::AUDIO_LEASE) {
                Some(l) => AudioClient::with_lease(chan, Channel::from_handle(Handle::from_raw(l))),
                None => AudioClient::new(chan),
            }
        });
        Mutex::new(Audio { client, streams: Vec::new() })
    })
}

fn stream(id: i32) -> Option<Arc<Shared>> {
    let a = audio().lock().unwrap_or_else(|e| e.into_inner());
    a.streams.get((id as usize).checked_sub(1)?)?.clone()
}

/// Format SDL -> format sluzby (`MOSDL_SAMPLE_*`).
fn sample(v: u32) -> Option<Sample> {
    match v {
        0 => Some(Sample::U8),
        1 => Some(Sample::S16),
        2 => Some(Sample::S32),
        3 => Some(Sample::F32),
        _ => None,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mosdl_audio_available() -> i32 {
    super::with_state(|_| ());
    audio().lock().unwrap_or_else(|e| e.into_inner()).client.is_some() as i32
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
    super::with_state(|_| ());
    let Some(s) = sample(sample_fmt) else { return code(Error::InvalidArgs) };
    let Ok(ch) = u8::try_from(channels) else { return code(Error::InvalidArgs) };
    let dir = if dir == 1 { Direction::Input } else { Direction::Output };
    let client = match audio().lock().unwrap_or_else(|e| e.into_inner()).client.clone() {
        Some(c) => c,
        None => return code(Error::NotFound),
    };
    // Bez zamku: OPEN vstupu muze cekat na rozhodnuti uzivatele.
    let st = match client.open(dir, Format::new(rate, ch, s), latency_us) {
        Ok(s) => s,
        Err(e) => return code(e),
    };
    if !out_frames.is_null() {
        // SAFETY: platny podle smlouvy.
        unsafe { out_frames.write(st.info().period) };
    }
    let mut a = audio().lock().unwrap_or_else(|e| e.into_inner());
    a.streams.push(Some(Arc::new(Shared(st))));
    a.streams.len() as i32
}

/// Zapise vsechna data (cele snimky); kdyz je buffer plny, pocka na misto
/// (nejdel `timeout_ns`, < 0 = bez limitu). Stream se spusti, jakmile se
/// buffer poprve naplni.
///
/// # Safety
///
/// `data` ukazuje na `bytes` citelnych bajtu.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdl_audio_write(id: i32, data: *const u8, bytes: u32, timeout_ns: i64) -> i32 {
    let Some(s) = stream(id) else { return code(Error::NotFound) };
    if data.is_null() {
        return code(Error::InvalidArgs);
    }
    // SAFETY: delka podle smlouvy.
    let buf = unsafe { core::slice::from_raw_parts(data, bytes as usize) };
    let deadline = (timeout_ns >= 0).then(|| libos::time::now().saturating_add(timeout_ns as u64));
    s.0.write_all(buf, true, deadline).map_or_else(code, |_| 0)
}

/// Precte dostupna nahrana data; vrati pocet bajtu.
///
/// # Safety
///
/// `data` ukazuje na `bytes` zapisovatelnych bajtu.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mosdl_audio_read(id: i32, data: *mut u8, bytes: u32) -> i32 {
    let Some(s) = stream(id) else { return code(Error::NotFound) };
    if data.is_null() {
        return code(Error::InvalidArgs);
    }
    // SAFETY: delka podle smlouvy.
    let buf = unsafe { core::slice::from_raw_parts_mut(data, bytes as usize) };
    let frames = s.0.read(buf);
    (frames as usize * s.0.ring().frame_bytes() as usize) as i32
}

/// Pocka, az se do bufferu vejde `bytes` bajtu (nejdel `timeout_ns`, < 0 =
/// bez limitu); 0 = misto je, zaporne = chyba (`TIMED_OUT`: sluzba stoji).
/// SDL ho vola z `WaitDevice` mimo zamek zarizeni, `mosdl_audio_write`
/// pak neceka.
#[unsafe(no_mangle)]
pub extern "C" fn mosdl_audio_wait_space(id: i32, bytes: u32, timeout_ns: i64) -> i32 {
    let Some(s) = stream(id) else { return code(Error::NotFound) };
    let deadline = (timeout_ns >= 0).then(|| libos::time::now().saturating_add(timeout_ns as u64));
    s.0.wait_space(bytes as usize, deadline).map_or_else(code, |_| 0)
}

/// Pocka na udalost streamu (misto v bufferu / nova data).
#[unsafe(no_mangle)]
pub extern "C" fn mosdl_audio_wait(id: i32, timeout_ns: i64) -> i32 {
    let Some(s) = stream(id) else { return code(Error::NotFound) };
    let deadline = (timeout_ns >= 0).then(|| libos::time::now().saturating_add(timeout_ns as u64));
    match s.0.wait(deadline) {
        Ok(()) | Err(Error::TimedOut) => 0,
        Err(e) => code(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mosdl_audio_start(id: i32) -> i32 {
    stream(id).map_or(code(Error::NotFound), |s| s.0.start().map_or_else(code, |_| 0))
}

/// Zavre stream (prehrani zbytku neceka; SDL dohrava samo).
#[unsafe(no_mangle)]
pub extern "C" fn mosdl_audio_close(id: i32) {
    let mut a = audio().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(slot) = (id as usize).checked_sub(1).and_then(|i| a.streams.get_mut(i)) {
        // Zruseni posledniho odkazu stream zavre (CLOSE).
        *slot = None;
    }
}

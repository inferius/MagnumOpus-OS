//! Media: sluzba `mediad` a testovaci dekoder `mediatest`
//! (`docs/cs/multimedia.md`).
//!
//! Testy hraji roli prehravace: vlastni relace (`CONNECT`), testovaci
//! soubor `MTM1` ve VMO nebo za kanalem zdroje, pool bufferu NV12 s plotem
//! snimku, kruh zvuku od audiod. Obsah snimku se overuje kontrolnim
//! souctem proti stejnemu generatoru, jaky pouziva dekoder.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use audioproto::conn::AudioClient;
use audioproto::{Direction, Format, Sample};
use bufdesc::{BufDesc, format as fourcc};
use libos::abi::vm::MAP_READ;
use libos::{Channel, Error, Fence, Mapping, Startup, Vmo, time};
use mediaproto::clock::{AudioAnchor, audio_now_us};
use mediaproto::conn::MediaClient;
use mediaproto::source::{SourceRequest, clamp};
use mediaproto::testmedia::{self, Header};
use mediaproto::{AudioRing, Event, Kind};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("otevreni VMO: popis proudu", open_vmo),
    ("snimky do poolu s plotem, obsah a poradi", frames),
    ("skok na klicovy a presny snimek", seek),
    ("zdroj kanalem aplikace", source_channel),
    ("zvuk do kruhu audiod, hodiny zvuku", audio),
    ("pad dekoderu: udalost a obnova relace", decoder_crash),
    ("neplatne pozadavky a poskozeny soubor", bad_requests),
];

const W: u32 = 64;
const H: u32 = 32;
const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

fn header() -> Header {
    Header { width: W, height: H, fps_num: 25, fps_den: 1, frames: 50, rate: 48_000, channels: 2, tone_hz: 440, crash_at: 0 }
}

fn session() -> Result<MediaClient, String> {
    crate::media::connect().map(MediaClient::new).map_err(|e| fail("pripojeni k mediad", e))
}

fn file_vmo(h: &Header) -> Result<(Vmo, u64), String> {
    let data = testmedia::build(h);
    let vmo = Vmo::create(data.len() as u64).map_err(|e| fail("VMO souboru", e))?;
    vmo.write(0, &data).map_err(|e| fail("zapis souboru", e))?;
    Ok((vmo, data.len() as u64))
}

fn open(h: &Header) -> Result<MediaClient, String> {
    let m = session()?;
    let (vmo, size) = file_vmo(h)?;
    m.open_vmo(&vmo, size).map_err(|e| fail("OPEN", e))?;
    Ok(m)
}

struct Pool {
    descs: Vec<BufDesc>,
    vmos: Vec<Vmo>,
    fence: Fence,
}

fn pool(m: &MediaClient, n: usize) -> Result<Pool, String> {
    let desc = BufDesc::linear(fourcc::NV12, W, H, 64).map_err(|e| format!("popis bufferu: {e:?}"))?;
    let size = desc.min_size().map_err(|e| format!("velikost bufferu: {e:?}"))?;
    let mut vmos = Vec::new();
    for _ in 0..n {
        vmos.push(Vmo::create(size).map_err(|e| fail("VMO bufferu", e))?);
    }
    let descs = alloc::vec![desc; n];
    let fence = Fence::create(0).map_err(|e| fail("plot", e))?;
    m.pool(descs.clone(), &vmos, &fence).map_err(|e| fail("POOL", e))?;
    Ok(Pool { descs, vmos, fence })
}

/// Kontrolni soucet bufferu podle popisu.
fn checksum(p: &Pool, i: usize) -> Result<u64, String> {
    let d = &p.descs[i];
    let map = Mapping::new(&p.vmos[i], 0, d.min_size().unwrap(), MAP_READ).map_err(|e| fail("mapovani bufferu", e))?;
    // SAFETY: mapovani jen pro cteni; dekoder buffer drzi az po RELEASE.
    let mem = unsafe { map.as_slice() };
    let (y, uv) = (d.planes[0], d.planes[1]);
    Ok(testmedia::checksum_nv12(W, H, &mem[y.offset as usize..], y.pitch as usize, &mem[uv.offset as usize..], uv.pitch as usize))
}

/// Ocekavany soucet snimku se seminkem `seed`.
fn expected(p: &Pool, seed: u32) -> u64 {
    let d = &p.descs[0];
    let mut buf = alloc::vec![0u8; d.min_size().unwrap() as usize];
    let (y, uv) = (d.planes[0], d.planes[1]);
    let (ys, uvs) = buf.split_at_mut(uv.offset as usize);
    testmedia::fill_nv12(W, H, &mut ys[y.offset as usize..], y.pitch as usize, uvs, uv.pitch as usize, seed);
    testmedia::checksum_nv12(W, H, &buf[y.offset as usize..], y.pitch as usize, &buf[uv.offset as usize..], uv.pitch as usize)
}

fn next_event(m: &MediaClient) -> Result<Event, String> {
    m.next_event(Some(time::deadline_after(EVENT_TIMEOUT))).map_err(|e| fail("cekani na udalost", e))
}

/// Dalsi snimek (preskoci ostatni udalosti); overi plot a obsah, vrati
/// (buffer, pts) a buffer uvolni.
fn take_frame(m: &MediaClient, p: &Pool) -> Result<(u8, u64), String> {
    loop {
        match next_event(m)? {
            Event::Frame { buffer, pts_us, duration_us, serial } => {
                check_eq!(duration_us, 40_000, "trvani snimku");
                p.fence
                    .wait(serial, Some(time::deadline_after(EVENT_TIMEOUT)))
                    .map_err(|e| fail("plot snimku", e))?;
                let i = (pts_us / 40_000) as u32;
                check_eq!(checksum(p, buffer as usize)?, expected(p, testmedia::seed(i)), "obsah snimku");
                m.release(buffer).map_err(|e| fail("RELEASE", e))?;
                return Ok((buffer, pts_us));
            }
            Event::Error { code, .. } => return Err(format!("chyba dekoderu {code}")),
            _ => {}
        }
    }
}

fn open_vmo(_: &Startup) -> TestResult {
    let m = session()?;
    let (vmo, size) = file_vmo(&header())?;
    let info = m.open_vmo(&vmo, size).map_err(|e| fail("OPEN", e))?;
    check_eq!(info.duration_us, 2_000_000, "delka");
    check_eq!(info.streams.len(), 2, "pocet proudu");
    let v = &info.streams[0];
    check!(v.kind == Kind::Video && v.width == W && v.height == H && v.fps_num == 25, "popis videa");
    let a = &info.streams[1];
    check!(a.kind == Kind::Audio && a.rate == 48_000 && a.channels == 2, "popis zvuku");
    check!(info.streams.iter().all(|s| s.available), "dostupnost kodeku");
    // Druhe otevreni ve stejne relaci nejde.
    check_eq!(m.open_vmo(&vmo, size).err(), Some(Error::AlreadyExists), "druhe OPEN");
    m.close().map_err(|e| fail("CLOSE", e))?;
    Ok(())
}

fn frames(_: &Startup) -> TestResult {
    let m = open(&header())?;
    m.select(0, -1, -1).map_err(|e| fail("SELECT", e))?;
    let p = pool(&m, 3)?;
    m.play().map_err(|e| fail("PLAY", e))?;
    for i in 0..12u64 {
        let (_, pts) = take_frame(&m, &p)?;
        check_eq!(pts, i * 40_000, "poradi snimku");
    }
    // Plot skoncil na poctu dokoncenych snimku (dekoder muze byt napred
    // o volne buffery).
    let v = p.fence.query().map_err(|e| fail("stav plotu", e))?.value;
    check!((12..=15).contains(&v), "hodnota plotu");
    m.pause().map_err(|e| fail("PAUSE", e))?;
    Ok(())
}

fn seek(_: &Startup) -> TestResult {
    let m = open(&header())?;
    m.select(0, -1, -1).map_err(|e| fail("SELECT", e))?;
    let p = pool(&m, 2)?;
    // Klicove snimky kazdou sekundu: 1,3 s -> 1,0 s; presne -> 1,28 s.
    check_eq!(m.seek(1_300_000, false).map_err(|e| fail("SEEK", e))?, 1_000_000, "klicovy snimek");
    m.play().map_err(|e| fail("PLAY", e))?;
    check_eq!(take_frame(&m, &p)?.1, 1_000_000, "prvni snimek po skoku");
    m.pause().map_err(|e| fail("PAUSE", e))?;
    // Snimky dekodovane pred skokem jeste muzou prijit: vratit buffery.
    while let Ok(e) = m.try_event() {
        if let Event::Frame { buffer, .. } = e {
            m.release(buffer).map_err(|e| fail("RELEASE", e))?;
        }
    }
    check_eq!(m.seek(1_300_000, true).map_err(|e| fail("SEEK presne", e))?, 1_280_000, "presny snimek");
    m.play().map_err(|e| fail("PLAY", e))?;
    check_eq!(take_frame(&m, &p)?.1, 1_280_000, "snimek po presnem skoku");
    Ok(())
}

/// Obsluha kanalu zdroje v testu (jako aplikace nad souborem).
fn serve(ch: Channel, data: Vec<u8>) {
    while let Ok(m) = ch.read(None) {
        let reply = match SourceRequest::decode(&m.data) {
            Ok(SourceRequest::ReadAt { offset, len }) => {
                let (o, n) = clamp(offset, len, data.len() as u64);
                mediaproto::source::encode_reply(Ok(&data[o as usize..o as usize + n]))
            }
            Ok(SourceRequest::Size) => mediaproto::source::encode_reply(Ok(&(data.len() as u64).to_le_bytes())),
            Err(e) => mediaproto::source::encode_reply(Err(e)),
        };
        if ch.write(&reply, Vec::new()).is_err() {
            return;
        }
    }
}

fn source_channel(_: &Startup) -> TestResult {
    let m = session()?;
    let (ours, theirs) = Channel::create().map_err(|e| fail("kanal zdroje", e))?;
    let data = testmedia::build(&header());
    let t = libos::thread::spawn("zdroj-medii", move || serve(ours, data)).map_err(|e| fail("vlakno zdroje", e))?;
    let info = m.open_channel(theirs).map_err(|e| fail("OPEN kanalem", e))?;
    check_eq!(info.duration_us, 2_000_000, "delka");
    m.select(0, -1, -1).map_err(|e| fail("SELECT", e))?;
    let p = pool(&m, 2)?;
    m.play().map_err(|e| fail("PLAY", e))?;
    for i in 0..5u64 {
        check_eq!(take_frame(&m, &p)?.1, i * 40_000, "snimek ze zdroje kanalem");
    }
    // Zavreni relace zavre i kanal zdroje u sluzby: vlakno skonci.
    m.close().map_err(|e| fail("CLOSE", e))?;
    drop(m);
    t.join(Some(time::deadline_after(EVENT_TIMEOUT))).map_err(|e| fail("konec vlakna zdroje", e))?;
    Ok(())
}

fn audio(_: &Startup) -> TestResult {
    let h = Header { frames: 25, ..header() };
    let m = open(&h)?;
    m.select(-1, 1, -1).map_err(|e| fail("SELECT", e))?;
    let a = crate::audio::connect().map(AudioClient::new).map_err(|e| fail("pripojeni k audiod", e))?;
    let s = a
        .open(Direction::Output, Format::new(48_000, 2, Sample::S16), 100_000)
        .map_err(|e| fail("stream audiod", e))?;
    let (vmo, ev) = s.producer_handles().map_err(|e| fail("handly kruhu", e))?;
    let ring = AudioRing { rate: 48_000, channels: 2, sample: Sample::S16 as u8, capacity: s.info().capacity };
    m.audio(ring, vmo, ev).map_err(|e| fail("AUDIO", e))?;
    m.play().map_err(|e| fail("PLAY", e))?;
    let anchor = match next_event(&m)? {
        Event::AudioBase { ring_pos, pts_us } => AudioAnchor { ring_pos, pts_us },
        e => return Err(format!("prvni udalost: {e:?}")),
    };
    check_eq!(anchor.pts_us, 0, "cas kotvy");
    s.start().map_err(|e| fail("START", e))?;
    // Ton 1 s: hodiny zvuku musi dojit pres polovinu a prijde END.
    let deadline = time::deadline_after(Duration::from_secs(5));
    let mut now = 0;
    while now < 500_000 {
        check!(time::now() < deadline, "hodiny zvuku se nehybou");
        libos::time::sleep(Duration::from_millis(20));
        now = audio_now_us(anchor, s.ring().played(), s.ring().latency_us(), 48_000);
    }
    loop {
        match next_event(&m)? {
            Event::End => break,
            Event::Error { code, .. } => return Err(format!("chyba dekoderu {code}")),
            _ => {}
        }
    }
    check!(s.ring().write_pos() >= anchor.ring_pos + 48_000, "dekoder nezapsal celou sekundu");
    Ok(())
}

fn decoder_crash(_: &Startup) -> TestResult {
    let m = open(&Header { crash_at: 4, ..header() })?;
    m.select(0, -1, -1).map_err(|e| fail("SELECT", e))?;
    let p = pool(&m, 2)?;
    m.play().map_err(|e| fail("PLAY", e))?;
    let mut seen = 0;
    loop {
        match next_event(&m)? {
            Event::Frame { buffer, .. } => {
                seen += 1;
                // Uvolneni po padu dekoderu muze selhat (PEER_CLOSED).
                let _ = m.release(buffer);
            }
            Event::Error { decoder_lost: true, .. } => break,
            e => return Err(format!("neocekavana udalost {e:?}")),
        }
    }
    check_eq!(seen, 3, "snimky pred padem");
    check_eq!(m.play().err(), Some(Error::BadState), "PLAY bez dekoderu");
    let info = m.reopen().map_err(|e| fail("REOPEN", e))?;
    check_eq!(info.streams.len(), 2, "proudy po obnove");
    check_eq!(m.seek(0, true).map_err(|e| fail("SEEK po obnove", e))?, 0, "skok po obnove");
    m.play().map_err(|e| fail("PLAY po obnove", e))?;
    check_eq!(take_frame(&m, &p)?.1, 0, "snimek po obnove");
    Ok(())
}

fn bad_requests(_: &Startup) -> TestResult {
    let m = session()?;
    check_eq!(m.play().err(), Some(Error::BadState), "PLAY bez zdroje");
    // Poskozeny soubor: OPEN selze, relace zustane pouzitelna.
    let mut data = testmedia::build(&header());
    data[0] = b'X';
    let vmo = Vmo::create(data.len() as u64).map_err(|e| fail("VMO", e))?;
    vmo.write(0, &data).map_err(|e| fail("zapis", e))?;
    check_eq!(m.open_vmo(&vmo, data.len() as u64).err(), Some(Error::NotSupported), "poskozena magie");
    let (good, size) = file_vmo(&header())?;
    m.open_vmo(&good, size).map_err(|e| fail("OPEN po chybe", e))?;
    check_eq!(m.select(0, 5, -1).err(), Some(Error::InvalidArgs), "proud mimo rozsah");
    check_eq!(m.play().err(), Some(Error::BadState), "PLAY bez poolu");
    check_eq!(m.release(0).err(), Some(Error::BadState), "RELEASE bez poolu");
    // Pool bez plotu (o handle min).
    let desc = BufDesc::linear(fourcc::NV12, W, H, 64).unwrap();
    let v = Vmo::create(desc.min_size().unwrap()).map_err(|e| fail("VMO", e))?;
    let h = v.handle().duplicate(None).map_err(|e| fail("dup", e))?;
    let r = m.call(&mediaproto::Request::Pool { descs: alloc::vec![desc] }, alloc::vec![h]);
    check_eq!(r.err(), Some(Error::InvalidArgs), "POOL bez plotu");
    Ok(())
}

//! Media nad FFmpeg: proces dekoderu `ffdec` (PIE nad `libc.so`
//! a `libav*.so`) pres `mediad` (`docs/cs/multimedia.md`).
//!
//! Testovaci vektory AV1 (AOM) a VP9 (libvpx) jsou v initfs (`media/`)
//! i s MD5 kazdeho dekodovaneho snimku I420: test slozi I420 zpet z bufferu
//! NV12 (prevod je bezztratovy - jen proplete chroma) a porovna, takze
//! overi dekodovani bit po bitu. Zvuk: WAV vygenerovany v testu jde pres
//! kanal zdroje do kruhu audiod.

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use audioproto::conn::AudioClient;
use audioproto::{Direction, Format, Sample};
use bufdesc::{BufDesc, format as fourcc};
use libos::abi::vm::MAP_READ;
use libos::initfs::Initfs;
use libos::{Channel, Error, Fence, Mapping, Startup, Vmo, time};
use mediaproto::conn::MediaClient;
use mediaproto::source::{SourceRequest, clamp};
use mediaproto::{AudioRing, Event, Kind};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("ffdec: popis proudu AV1 a VP9", describe),
    ("ffdec: AV1 (dav1d) bit po bitu podle MD5 AOM", av1_frames),
    ("ffdec: VP9 bit po bitu podle MD5 libvpx, skok po konci", vp9_frames),
    ("ffdec: zvuk WAV kanalem zdroje do audiod", wav_audio),
    ("ffdec: poskozeny a neznamy vstup", bad_input),
];

const EVENT_TIMEOUT: Duration = Duration::from_secs(20);

fn media(s: &Startup, name: &str) -> Result<&'static [u8], String> {
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    fs.find(&format!("media/{name}")).ok_or_else(|| format!("media/{name} neni v initfs"))
}

fn session(s: &Startup) -> Result<MediaClient, String> {
    libd_ready(s)?;
    crate::media::connect().map(MediaClient::new).map_err(|e| fail("pripojeni k mediad", e))
}

/// Generace libd, se kterou uz mediad spustil ffdec (0 = zatim nikdy).
static LIBD_READY: AtomicU64 = AtomicU64::new(0);

/// mediad spousti `ffdec` (PIE) pres svou relaci s libd. Po padu libd (sada
/// `procesy`, test obnovy libd) ji obnovi az pri dalsim spusteni pres listek
/// (`RECONNECT` u supervisoru initu) - jenze supervisor behem testu ridi jen
/// test, hlavni smycka initu nebezi. Jednou za generaci libd proto test
/// spusti ffdec (nesmyslny vstup) ve vedlejsim vlakne a mezitim ridi
/// supervisor. V bezicim systemu supervisor ridi hlavni smycka sama.
pub(super) fn libd_ready(s: &Startup) -> TestResult {
    let generation = crate::supervisor::SUPERVISOR.lock().generation("libd").unwrap_or(0);
    if LIBD_READY.load(Ordering::Relaxed) == generation {
        return Ok(());
    }
    let result: Arc<spin::Mutex<Option<Result<(), Error>>>> = Arc::new(spin::Mutex::new(None));
    let r = result.clone();
    let t = libos::thread::spawn("media-libd", move || {
        let probe = || -> Result<(), Error> {
            let m = MediaClient::new(crate::media::connect()?);
            let vmo = Vmo::create(64)?;
            vmo.write(0, &[0u8; 64])?;
            // Nuly FFmpeg nepozna: chyba je spravne, jde jen o start ffdec.
            match m.open_vmo(&vmo, 64) {
                Err(Error::TimedOut) => Err(Error::TimedOut),
                _ => Ok(()),
            }
        };
        *r.lock() = Some(probe());
    })
    .map_err(|e| fail("vlakno", e))?;
    let end = time::deadline_after(Duration::from_secs(30));
    while result.lock().is_none() {
        if time::now() > end {
            return Err(String::from("mediad po restartu libd nespustil ffdec do 30 s"));
        }
        crate::supervisor::SUPERVISOR.lock().step(s, Some(time::deadline_after(Duration::from_millis(20))));
    }
    let _ = t.join(None);
    result.lock().take().unwrap_or(Err(Error::IoError)).map_err(|e| fail("spusteni ffdec (obnova libd)", e))?;
    LIBD_READY.store(generation, Ordering::Relaxed);
    Ok(())
}

fn open_bytes(m: &MediaClient, data: &[u8]) -> Result<mediaproto::Info, String> {
    let vmo = Vmo::create(data.len() as u64).map_err(|e| fail("VMO", e))?;
    vmo.write(0, data).map_err(|e| fail("zapis", e))?;
    m.open_vmo(&vmo, data.len() as u64).map_err(|e| fail("OPEN", e))
}

/// MD5 snimku z oficialniho seznamu (`<md5>  <jmeno>`), v poradi.
fn md5_list(text: &[u8]) -> Vec<String> {
    core::str::from_utf8(text).unwrap_or("").lines().filter_map(|l| l.split_whitespace().next()).map(String::from).collect()
}

/// Pool bufferu NV12 predany dekoderu.
struct Pool {
    desc: BufDesc,
    vmos: Vec<Vmo>,
    fence: Fence,
}

fn pool(m: &MediaClient, w: u32, h: u32) -> Result<Pool, String> {
    let desc = BufDesc::linear(fourcc::NV12, w, h, 64).map_err(|e| format!("popis: {e:?}"))?;
    let size = desc.min_size().map_err(|e| format!("velikost: {e:?}"))?;
    let vmos: Vec<Vmo> = (0..3).map(|_| Vmo::create(size)).collect::<libos::Result<_>>().map_err(|e| fail("VMO", e))?;
    let fence = Fence::create(0).map_err(|e| fail("plot", e))?;
    m.pool(alloc::vec![desc.clone(); 3], &vmos, &fence).map_err(|e| fail("POOL", e))?;
    Ok(Pool { desc, vmos, fence })
}

/// Dalsi snimek: (MD5 I420, pts); buffer vrati. `None` = konec.
fn next_frame(m: &MediaClient, p: &Pool) -> Result<Option<(String, u64)>, String> {
    loop {
        match m.next_event(Some(time::deadline_after(EVENT_TIMEOUT))).map_err(|e| fail("udalost", e))? {
            Event::Frame { buffer, serial, pts_us, .. } => {
                p.fence.wait(serial, Some(time::deadline_after(EVENT_TIMEOUT))).map_err(|e| fail("plot snimku", e))?;
                let sum = i420_md5(&p.desc, &p.vmos[buffer as usize])?;
                m.release(buffer).map_err(|e| fail("RELEASE", e))?;
                return Ok(Some((sum, pts_us)));
            }
            Event::End => return Ok(None),
            Event::Error { code, .. } => return Err(format!("chyba dekoderu {code}")),
            Event::AudioBase { .. } => {}
        }
    }
}

/// Dekoduje vsechny snimky a vrati jejich MD5 (I420 slozene z NV12).
fn decode_all(m: &MediaClient, p: &Pool) -> Result<Vec<String>, String> {
    m.play().map_err(|e| fail("PLAY", e))?;
    let mut out = Vec::new();
    while let Some((sum, _)) = next_frame(m, p)? {
        out.push(sum);
    }
    Ok(out)
}

/// I420 (Y, U, V za sebou) z bufferu NV12 a jeho MD5 (hex).
fn i420_md5(d: &BufDesc, vmo: &Vmo) -> Result<String, String> {
    let map = Mapping::new(vmo, 0, d.min_size().unwrap(), MAP_READ).map_err(|e| fail("mapovani", e))?;
    // SAFETY: mapovani jen pro cteni; dekoder buffer drzi az po RELEASE.
    let mem = unsafe { map.as_slice() };
    let (w, h) = (d.width as usize, d.height as usize);
    let (yp, uvp) = (d.planes[0], d.planes[1]);
    let mut i420 = Vec::with_capacity(w * h * 3 / 2);
    for r in 0..h {
        let o = yp.offset as usize + r * yp.pitch as usize;
        i420.extend_from_slice(&mem[o..o + w]);
    }
    for k in 0..2 {
        for r in 0..h.div_ceil(2) {
            let o = uvp.offset as usize + r * uvp.pitch as usize;
            i420.extend(mem[o..o + w.div_ceil(2) * 2].iter().skip(k).step_by(2));
        }
    }
    Ok(md5::hex(&i420))
}

fn describe(s: &Startup) -> TestResult {
    for (name, codec) in [("av1-64x64.ivf", "av1"), ("vp9-64x64.webm", "vp9")] {
        let m = session(s)?;
        let info = open_bytes(&m, media(s, name)?)?;
        let v = info.first(Kind::Video).ok_or(format!("{name}: chybi video"))?;
        let st = &info.streams[v];
        check_eq!(st.codec.as_str(), codec, "kodek");
        check_eq!((st.width, st.height), (64, 64), "rozmer");
        check!(st.available, "dekoder je v obrazu");
        m.close().map_err(|e| fail("CLOSE", e))?;
    }
    Ok(())
}

fn frames(s: &Startup, file: &str, md5: &str) -> Result<(MediaClient, Pool, Vec<String>, Vec<String>), String> {
    let want = md5_list(media(s, md5)?);
    check!(!want.is_empty(), "prazdny seznam MD5");
    let m = session(s)?;
    let info = open_bytes(&m, media(s, file)?)?;
    let v = info.first(Kind::Video).ok_or("chybi video")? as i32;
    m.select(v, -1, -1).map_err(|e| fail("SELECT", e))?;
    let p = pool(&m, 64, 64)?;
    let got = decode_all(&m, &p)?;
    Ok((m, p, got, want))
}

fn av1_frames(s: &Startup) -> TestResult {
    let (_, _, got, want) = frames(s, "av1-64x64.ivf", "av1-64x64.ivf.md5")?;
    check_eq!(got, want, "MD5 snimku AV1");
    Ok(())
}

fn vp9_frames(s: &Startup) -> TestResult {
    let (m, p, got, want) = frames(s, "vp9-64x64.webm", "vp9-64x64.webm.md5")?;
    check_eq!(got, want, "MD5 snimku VP9");
    // Po konci: pauza, skok na zacatek, prvni snimek znovu stejny.
    m.pause().map_err(|e| fail("PAUSE", e))?;
    check_eq!(m.seek(0, true).map_err(|e| fail("SEEK", e))?, 0, "skok na zacatek");
    m.play().map_err(|e| fail("PLAY po skoku", e))?;
    let (sum, pts) = next_frame(&m, &p)?.ok_or("po skoku zadny snimek")?;
    check_eq!(pts, 0, "pts prvniho snimku po skoku");
    check_eq!(sum, want[0], "prvni snimek po skoku");
    Ok(())
}

/// WAV s16 stereo 48 kHz: `ms` milisekund tonu.
fn wav(ms: u32) -> Vec<u8> {
    let frames = 48 * ms;
    let mut tone = audiomix::tone::Tone::new(440, 48_000, 30);
    let mut pcm = alloc::vec![0u8; frames as usize * 4];
    tone.fill_s16(&mut pcm);
    let mut v = Vec::new();
    let le32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
    v.extend_from_slice(b"RIFF");
    le32(&mut v, 36 + pcm.len() as u32);
    v.extend_from_slice(b"WAVEfmt ");
    le32(&mut v, 16);
    v.extend_from_slice(&[1, 0, 2, 0]);
    le32(&mut v, 48_000);
    le32(&mut v, 48_000 * 4);
    v.extend_from_slice(&[4, 0, 16, 0]);
    v.extend_from_slice(b"data");
    le32(&mut v, pcm.len() as u32);
    v.extend_from_slice(&pcm);
    v
}

/// Obsluha kanalu zdroje (jako aplikace nad souborem).
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

fn wav_audio(s: &Startup) -> TestResult {
    let m = session(s)?;
    let (ours, theirs) = Channel::create().map_err(|e| fail("kanal zdroje", e))?;
    let data = wav(500);
    libos::thread::spawn("zdroj-wav", move || serve(ours, data)).map_err(|e| fail("vlakno zdroje", e))?;
    let info = m.open_channel(theirs).map_err(|e| fail("OPEN kanalem", e))?;
    let a = info.first(Kind::Audio).ok_or("chybi zvuk")?;
    check_eq!(info.streams[a].codec.as_str(), "pcm_s16le", "kodek");
    check_eq!(info.streams[a].rate, 48_000, "frekvence");
    let ac = crate::audio::connect().map(AudioClient::new).map_err(|e| fail("pripojeni k audiod", e))?;
    let st = ac.open(Direction::Output, Format::new(48_000, 2, Sample::S16), 100_000).map_err(|e| fail("stream audiod", e))?;
    let (vmo, ev) = st.producer_handles().map_err(|e| fail("handly kruhu", e))?;
    let ring = AudioRing { rate: 48_000, channels: 2, sample: Sample::S16 as u8, capacity: st.info().capacity };
    m.audio(ring, vmo, ev).map_err(|e| fail("AUDIO", e))?;
    m.select(-1, a as i32, -1).map_err(|e| fail("SELECT", e))?;
    m.play().map_err(|e| fail("PLAY", e))?;
    st.start().map_err(|e| fail("START", e))?;
    let mut base = None;
    loop {
        match m.next_event(Some(time::deadline_after(EVENT_TIMEOUT))).map_err(|e| fail("udalost", e))? {
            Event::AudioBase { ring_pos, pts_us } => base = Some((ring_pos, pts_us)),
            Event::End => break,
            Event::Error { code, .. } => return Err(format!("chyba dekoderu {code}")),
            Event::Frame { .. } => return Err(String::from("snimek videa u zvuku")),
        }
    }
    let (pos, pts) = base.ok_or("chybi AUDIO_BASE")?;
    check_eq!(pts, 0, "cas kotvy");
    check!(st.ring().write_pos() >= pos + 24_000, "dekoder nezapsal 0,5 s zvuku");
    Ok(())
}

fn bad_input(s: &Startup) -> TestResult {
    // Nahodne bajty: FFmpeg format nepozna (NOT_SUPPORTED), relace zije.
    let junk: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    let m = session(s)?;
    let r = open_bytes(&m, &junk);
    check!(r.is_err(), "nahodne bajty se otevrely");
    // Useknuty WebM: otevreni muze projit, dekodovani pak skonci chybou
    // nebo koncem, nikdy padem sluzby.
    let m2 = session(s)?;
    let cut = &wav(100)[..30];
    match open_bytes(&m2, cut) {
        Ok(_) | Err(_) => {}
    }
    check!(session(s).is_ok(), "mediad zije");
    Ok(())
}

/// MD5 (RFC 1321) pro porovnani s testovacimi vektory.
mod md5 {
    use alloc::string::String;

    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4,
        11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];

    fn k(i: usize) -> u32 {
        // floor(|sin(i + 1)| * 2^32) bez libm: tabulka.
        const K: [u32; 64] = [
            0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122,
            0xfd987193, 0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6,
            0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60,
            0xbebfbc70, 0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039,
            0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1, 0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
        ];
        K[i]
    }

    pub fn hex(data: &[u8]) -> String {
        let mut st = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476];
        let mut msg = data.to_vec();
        let bits = (data.len() as u64).wrapping_mul(8);
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bits.to_le_bytes());
        for chunk in msg.chunks_exact(64) {
            let m: [u32; 16] = core::array::from_fn(|i| u32::from_le_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap()));
            let [mut a, mut b, mut c, mut d] = st;
            for i in 0..64 {
                let (f, g) = match i / 16 {
                    0 => ((b & c) | (!b & d), i),
                    1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                    2 => (b ^ c ^ d, (3 * i + 5) % 16),
                    _ => (c ^ (b | !d), (7 * i) % 16),
                };
                let t = d;
                d = c;
                c = b;
                b = b.wrapping_add(a.wrapping_add(f).wrapping_add(k(i)).wrapping_add(m[g]).rotate_left(S[i]));
                a = t;
            }
            st = [st[0].wrapping_add(a), st[1].wrapping_add(b), st[2].wrapping_add(c), st[3].wrapping_add(d)];
        }
        let mut s = String::new();
        for w in st {
            for byte in w.to_le_bytes() {
                s.push_str(&alloc::format!("{byte:02x}"));
            }
        }
        s
    }
}

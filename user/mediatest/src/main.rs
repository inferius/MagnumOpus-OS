//! mediatest: testovaci dekoder s rozhranim procesu `ffdec` (bez FFmpeg).
//!
//! Spousti ho `mediad` jednou na relaci bez jakychkoli dalsich handlu:
//! bootstrap kanal = ridici kanal od `mediad` (`mediaproto::Request`,
//! odpovedi a udalosti se znackou). Vstup cte jen pres kanal zdroje
//! (`mediaproto::source`), ktery dostane v `PROBE`. Umi testovaci format
//! `MTM1` (`mediaproto::testmedia`): obraz NV12 do poolu bufferu s plotem
//! snimku, ton s16 stereo do kruhu `audiod`. Na snimku `crash_at` schvalne
//! skonci chybou (test obnovy relace).

#![no_std]
#![no_main]

extern crate alloc;

mod audio;
mod video;

use alloc::vec::Vec;
use core::time::Duration;

use libos::{Channel, Error, Result, Signals, Startup};
use mediaproto::source::{SourceRequest, decode_reply};
use mediaproto::testmedia::{self, Header, Record};
use mediaproto::{Event, Request, encode_reply, encode_u64};

use audio::Audio;
use video::Pool;

libos::entry!(main);

/// Jak dlouho cekat na bajty zdroje.
const SOURCE_TIMEOUT: Duration = Duration::from_secs(5);

/// Kod ukonceni pri schvalnem padu (`crash_at`).
const CRASH_CODE: i64 = 70;

struct Dec {
    ctl: Channel,
    src: Option<Channel>,
    hdr: Option<Header>,
    video_on: bool,
    audio_on: bool,
    pool: Option<Pool>,
    audio: Option<Audio>,
    playing: bool,
    /// Dalsi snimek k dekodovani.
    next: u32,
    end_sent: bool,
}

fn main(startup: Startup) -> i64 {
    let mut d = Dec {
        ctl: startup.bootstrap,
        src: None,
        hdr: None,
        video_on: true,
        audio_on: true,
        pool: None,
        audio: None,
        playing: false,
        next: 0,
        end_sent: false,
    };
    loop {
        if d.playing {
            if let Err(e) = d.produce() {
                d.event(Event::Error { code: e.code(), decoder_lost: false });
                d.playing = false;
            }
        }
        let mut items = Vec::new();
        items.push((d.ctl.handle(), Signals::READABLE | Signals::PEER_CLOSED));
        let audio_wait = d.playing && d.audio_on && d.audio.as_ref().is_some_and(|a| !a.done());
        if audio_wait {
            items.push((d.audio.as_ref().unwrap().event().handle(), Signals::SIGNALED));
        }
        let observed = match libos::wait_many(&items, None) {
            Ok((_, o)) => o,
            Err(_) => return 1,
        };
        if audio_wait && observed[1].intersects(Signals::SIGNALED) {
            let _ = d.audio.as_ref().unwrap().event().reset();
        }
        loop {
            match d.ctl.try_read() {
                Ok(m) => {
                    let (reply, quit) = match Request::decode(&m.data) {
                        Ok(req) => d.request(req, m.handles),
                        Err(e) => (Err(e), false),
                    };
                    if d.ctl.write(&encode_reply(reply), Vec::new()).is_err() || quit {
                        return 0;
                    }
                }
                Err(Error::ShouldWait) => break,
                Err(_) => return 0,
            }
        }
        if observed[0].intersects(Signals::PEER_CLOSED) && d.ctl.try_read().is_err() {
            return 0;
        }
    }
}

impl Dec {
    fn event(&self, e: Event) {
        let _ = self.ctl.write(&e.encode(), Vec::new());
    }

    fn hdr(&self) -> Result<Header> {
        self.hdr.ok_or(Error::BadState)
    }

    /// Precte bajty zdroje (presne `len`, jinak chyba).
    fn read_at(&self, offset: u64, len: u32) -> Result<Vec<u8>> {
        let src = self.src.as_ref().ok_or(Error::BadState)?;
        src.write(&SourceRequest::ReadAt { offset, len }.encode(), Vec::new())?;
        let m = src.read(Some(libos::time::deadline_after(SOURCE_TIMEOUT)))?;
        let data = decode_reply(&m.data)?;
        if data.len() != len as usize {
            return Err(Error::Corrupted);
        }
        Ok(data.to_vec())
    }

    fn record(&self, i: u32) -> Result<Record> {
        Record::parse(&self.read_at(testmedia::record_offset(i), testmedia::RECORD_LEN as u32)?)
    }

    /// Vrati (odpoved, skoncit).
    fn request(&mut self, req: Request, mut hs: Vec<libos::Handle>) -> (Result<Vec<u8>>, bool) {
        let r = match req {
            Request::Probe { size } => {
                let Some(h) = hs.pop() else { return (Err(Error::InvalidArgs), false) };
                self.src = Some(Channel::from_handle(h));
                self.probe(size)
            }
            Request::Select { video, audio, subtitle } => {
                // Testovaci soubor: video = proud 0, zvuk = proud 1.
                let has_audio = self.hdr.is_some_and(|h| h.rate != 0);
                if !matches!(video, -1 | 0) || !matches!(audio, -1 | 1) || subtitle != -1 || (audio == 1 && !has_audio) {
                    Err(Error::InvalidArgs)
                } else {
                    self.video_on = video == 0;
                    self.audio_on = audio == 1;
                    Ok(Vec::new())
                }
            }
            Request::Pool { descs } => Pool::new(descs, hs).map(|p| {
                self.pool = Some(p);
                Vec::new()
            }),
            Request::Audio(ring) => self.hdr().and_then(|h| Audio::new(ring, hs, &h)).map(|a| {
                self.audio = Some(a);
                Vec::new()
            }),
            Request::Play => self.play().map(|_| Vec::new()),
            Request::Pause => {
                self.playing = false;
                Ok(Vec::new())
            }
            Request::Seek { pts_us, exact } => self.seek(pts_us, exact).map(encode_u64),
            Request::Release { buffer } => match self.pool.as_mut() {
                Some(p) => p.release(buffer).map(|_| Vec::new()),
                None => Err(Error::BadState),
            },
            Request::Close => return (Ok(Vec::new()), true),
            _ => Err(Error::NotSupported),
        };
        (r, false)
    }

    fn probe(&mut self, size: u64) -> Result<Vec<u8>> {
        let head = self.read_at(0, testmedia::HEADER_LEN as u32)?;
        let h = Header::parse(&head)?;
        if size != mediaproto::source::UNKNOWN_SIZE && size < h.file_len() {
            return Err(Error::Corrupted);
        }
        self.hdr = Some(h);
        Ok(h.info().encode())
    }

    fn play(&mut self) -> Result<()> {
        let h = self.hdr()?;
        if self.video_on && self.pool.is_none() {
            return Err(Error::BadState);
        }
        if !self.playing {
            if let Some(a) = self.audio.as_mut().filter(|_| self.audio_on) {
                if !a.anchored() {
                    let pts = self.next.min(h.frames) as u64 * h.frame_us() as u64;
                    let base = a.anchor(pts, &h);
                    self.event(base);
                }
            }
        }
        self.playing = true;
        Ok(())
    }

    fn seek(&mut self, pts_us: u64, exact: bool) -> Result<u64> {
        let h = self.hdr()?;
        let mut i = ((pts_us as u128 * h.fps_num as u128) / (1_000_000u128 * h.fps_den as u128)).min(h.frames as u128 - 1) as u32;
        if !exact {
            while i > 0 && self.record(i)?.flags & testmedia::KEY == 0 {
                i -= 1;
            }
        }
        let actual = self.record(i)?.pts_us;
        self.next = i;
        self.end_sent = false;
        if let Some(a) = self.audio.as_mut().filter(|_| self.audio_on) {
            let base = a.anchor(actual, &h);
            self.event(base);
        }
        Ok(actual)
    }

    /// Dekoduje, co jde: snimky do volnych bufferu, ton do volneho mista
    /// v kruhu. Na konci vsech proudu posle `END`.
    fn produce(&mut self) -> Result<()> {
        let h = self.hdr()?;
        if self.video_on {
            while self.next < h.frames {
                let Some(slot) = self.pool.as_ref().and_then(Pool::free_slot) else { break };
                if h.crash_at != 0 && self.next + 1 == h.crash_at {
                    libos::exit(CRASH_CODE);
                }
                let rec = self.record(self.next)?;
                let pool = self.pool.as_mut().ok_or(Error::BadState)?;
                let serial = pool.fill(slot, &h, rec.seed)?;
                self.event(Event::Frame { buffer: slot, pts_us: rec.pts_us, duration_us: h.frame_us(), serial });
                self.next += 1;
            }
        }
        if self.audio_on {
            if let Some(a) = self.audio.as_mut() {
                a.fill();
            }
        }
        let video_done = !self.video_on || self.next >= h.frames;
        let audio_done = !self.audio_on || self.audio.as_ref().is_none_or(Audio::done);
        if video_done && audio_done && !self.end_sent {
            self.end_sent = true;
            self.event(Event::End);
        }
        Ok(())
    }
}

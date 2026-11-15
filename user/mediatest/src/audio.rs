//! Ton do kruhu `audiod` (VMO a udalost periody od aplikace pres `mediad`).

use alloc::vec;
use alloc::vec::Vec;

use audiomix::tone::Tone;
use audioproto::Sample;
use audioproto::ring::{self, Ring};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Error, Event, Handle, Mapping, Result, Vmo};
use mediaproto::testmedia::Header;
use mediaproto::{AudioRing, Event as MediaEvent};

/// Hlasitost testovaciho tonu (procenta plneho rozsahu).
const VOLUME: u8 = 20;

pub struct Audio {
    ring: Ring,
    _map: Mapping,
    event: Event,
    tone: Tone,
    rate: u32,
    /// Ramce zbyvajici do konce souboru od posledni kotvy.
    left: u64,
    anchored: bool,
    buf: Vec<u8>,
}

impl Audio {
    /// Handly: [VMO kruhu, udalost periody].
    pub fn new(r: AudioRing, mut hs: Vec<Handle>, h: &Header) -> Result<Self> {
        // Testovaci dekoder umi jen s16 stereo; frekvenci kruhu zvolila
        // aplikace (typicky podle `Info`), ton se generuje v ni.
        if hs.len() != 2 || r.sample != Sample::S16 as u8 || r.channels != 2 {
            return Err(Error::NotSupported);
        }
        let event = Event::from_handle(hs.pop().ok_or(Error::InvalidArgs)?);
        let vmo = Vmo::from_handle(hs.pop().ok_or(Error::InvalidArgs)?);
        let len = ring::vmo_size(r.capacity, 4) as u64;
        let map = Mapping::new(&vmo, 0, len, MAP_READ | MAP_WRITE)?;
        // SAFETY: mapovani zije v `_map` stejne dlouho jako `ring`; kruh je
        // sdileny jen s audiod, ktery dodrzuje protokol pozic (atomicke).
        let ring = unsafe { Ring::attach(map.addr() as *mut u8, map.len() as usize)? };
        if ring.capacity() != r.capacity || ring.frame_bytes() != 4 {
            return Err(Error::InvalidArgs);
        }
        let rate = r.rate;
        Ok(Self {
            ring,
            _map: map,
            event,
            tone: Tone::new(h.tone_hz.max(1), rate, VOLUME),
            rate,
            left: 0,
            anchored: false,
            buf: vec![0u8; 4096 * 4],
        })
    }

    pub fn event(&self) -> &Event {
        &self.event
    }

    pub fn anchored(&self) -> bool {
        self.anchored
    }

    /// Novy cas od soucasne pozice zapisu (start, `SEEK`): udalost
    /// `AUDIO_BASE` pro aplikaci.
    pub fn anchor(&mut self, pts_us: u64, h: &Header) -> MediaEvent {
        let end = h.duration_us();
        self.left = end.saturating_sub(pts_us) * self.rate as u64 / 1_000_000;
        self.anchored = true;
        MediaEvent::AudioBase { ring_pos: self.ring.write_pos(), pts_us }
    }

    pub fn done(&self) -> bool {
        self.anchored && self.left == 0
    }

    /// Zapise ton do volneho mista v kruhu (nejvic do konce souboru).
    pub fn fill(&mut self) {
        while self.left > 0 {
            let frames = (self.ring.writable() as u64).min(self.left).min(self.buf.len() as u64 / 4) as usize;
            if frames == 0 {
                return;
            }
            let chunk = &mut self.buf[..frames * 4];
            self.tone.fill_s16(chunk);
            let wrote = self.ring.write(chunk) as u64;
            self.left -= wrote;
            if wrote < frames as u64 {
                return;
            }
        }
    }
}
